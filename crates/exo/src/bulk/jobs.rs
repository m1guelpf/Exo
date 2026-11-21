//! The account's bulk actions: queued and run one at a time on the account's runtime, straight
//! against Gmail (exo-store's `Store::bulk`), each undoable while it waits (which cancels it),
//! while it runs (which stops it and puts back what it did) and after.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex, MutexGuard},
};

use exo_store::{BulkTarget, Mutation, Progress, Relabelled, Store};
use tokio::sync::Notify;

use crate::account::{Repaint, UndoToken};

/// Where a bulk action is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the one before it.
    Pending,
    /// Changing threads.
    Running,
    /// Every thread changed.
    Done,
    /// Being put back, or waiting to be.
    Undoing,
    /// Put back, or cancelled before it started.
    Undone,
    /// Gmail failed it; what it changed by then can still be put back.
    Failed,
}

/// A bulk action as the UI shows it.
#[derive(Clone, Debug)]
pub struct Summary {
    /// The toast's undo token, which also names it.
    pub token: UndoToken,
    /// What it does: `Mark Done`.
    pub name: &'static str,
    /// What it's on, as the Bulk Actions dialog shows it.
    pub query: Option<String>,
    /// Where it is.
    pub status: Status,
    /// Its work is under way, not waiting: what the footer reports.
    pub active: bool,
    /// The loaded rows it's for, which show `Pending:` while it waits or runs.
    pub threads: Arc<HashSet<String>>,
    /// Threads changed (or put back, while undoing) so far.
    pub done: usize,
    /// Of how many: none until they're all found.
    pub total: usize,
}

/// A bulk action to queue.
#[derive(Clone, Debug)]
pub struct Request {
    /// What it does.
    pub name: &'static str,
    /// What it's on, for the Bulk Actions dialog.
    pub query: Option<String>,
    /// The action.
    pub mutation: Mutation,
    /// Its threads.
    pub target: BulkTarget,
    /// The loaded rows it's for.
    pub threads: HashSet<String>,
}

struct Job {
    summary: Summary,
    mutation: Mutation,
    target: BulkTarget,
    progress: Arc<Progress>,
    /// What it changed, to put back.
    changed: Relabelled,
}

/// The bulk actions, shared between the UI and the task running them.
#[derive(Clone)]
pub struct Jobs {
    shared: Arc<Shared>,
}

struct Shared {
    jobs: Mutex<Vec<Job>>,
    wake: Notify,
    repaint: Repaint,
}

/// What the runner does next.
enum Work {
    Run(BulkTarget, Mutation, Arc<Progress>),
    Undo(Relabelled, Arc<Progress>),
}

impl Jobs {
    /// Starts the task that runs `store`'s bulk actions, on the current runtime, again after a
    /// panic as `restarts` says; `repaint` asks for frames as they move on.
    pub(crate) fn start(
        store: Store,
        repaint: Repaint,
        restarts: &Arc<crate::recovery::Restarts>,
    ) -> Self {
        let jobs = Self {
            shared: Arc::new(Shared {
                jobs: Mutex::new(Vec::new()),
                wake: Notify::new(),
                repaint,
            }),
        };
        crate::recovery::spawn(restarts, {
            let jobs = jobs.clone();
            move || jobs.clone().run(store.clone())
        });
        jobs
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Job>> {
        self.shared
            .jobs
            .lock()
            .expect("bulk jobs lock is never poisoned")
    }

    /// Queues `request`; [`Jobs::undo`] with `token` undoes it.
    pub fn submit(&self, token: UndoToken, request: Request) {
        let Request {
            name,
            query,
            mutation,
            target,
            threads,
        } = request;
        let summary = Summary {
            token,
            name,
            query,
            status: Status::Pending,
            active: false,
            threads: Arc::new(threads),
            done: 0,
            total: 0,
        };
        self.lock().push(Job {
            summary,
            mutation,
            target,
            progress: Arc::default(),
            changed: Relabelled::default(),
        });
        self.shared.wake.notify_one();
    }

    /// Undoes the bulk action `token`: a waiting one is dropped, a running one stops and is put
    /// back, a finished one is put back. Returns whether `token` names one.
    pub fn undo(&self, token: UndoToken) -> bool {
        let mut jobs = self.lock();
        let Some(job) = jobs.iter_mut().find(|job| job.summary.token == token) else {
            return false;
        };
        match job.summary.status {
            Status::Pending => job.summary.status = Status::Undone,
            Status::Running => {
                job.progress.stop();
                job.summary.status = Status::Undoing;
            }
            Status::Done | Status::Failed => {
                job.summary.status = Status::Undoing;
                self.shared.wake.notify_one();
            }
            Status::Undoing | Status::Undone => {}
        }
        true
    }

    /// Every bulk action of this session, oldest first.
    pub fn summaries(&self) -> Vec<Summary> {
        self.lock()
            .iter()
            .map(|job| Summary {
                done: job.progress.done(),
                total: job.progress.total(),
                ..job.summary.clone()
            })
            .collect()
    }

    /// No bulk action is waiting or under way.
    pub fn is_idle(&self) -> bool {
        self.lock().iter().all(|job| {
            matches!(
                job.summary.status,
                Status::Done | Status::Undone | Status::Failed
            )
        })
    }

    /// The next piece of work, marked under way: the oldest waiting action, or undo.
    fn next(&self) -> Option<(UndoToken, Work)> {
        let mut jobs = self.lock();
        let job = jobs.iter_mut().find(|job| {
            !job.summary.active && matches!(job.summary.status, Status::Pending | Status::Undoing)
        })?;
        job.summary.active = true;
        let work = if job.summary.status == Status::Pending {
            job.summary.status = Status::Running;
            Work::Run(
                job.target.clone(),
                job.mutation.clone(),
                job.progress.clone(),
            )
        } else {
            job.progress = Arc::default();
            Work::Undo(std::mem::take(&mut job.changed), job.progress.clone())
        };
        Some((job.summary.token, work))
    }

    async fn run(self, store: Store) {
        loop {
            let Some((token, work)) = self.next() else {
                self.shared.wake.notified().await;
                continue;
            };
            self.shared.repaint.request();
            // What it changed (still to put back, when an undo failed), and whether it worked.
            let (changed, ran, ok) = match work {
                Work::Run(target, mutation, progress) => {
                    let mut changed = Relabelled::default();
                    let result = store
                        .bulk(&target, &mutation, &progress, &mut changed)
                        .await;
                    if let Err(error) = &result {
                        log::error!("bulk {mutation:?}: {error}");
                    }
                    (changed, true, result.is_ok())
                }
                Work::Undo(changed, progress) => match store.undo_bulk(&changed, &progress).await {
                    Ok(()) => (Relabelled::default(), false, true),
                    Err(error) => {
                        log::error!("undoing a bulk action: {error}");
                        (changed, false, false)
                    }
                },
            };
            let mut jobs = self.lock();
            if let Some(job) = jobs.iter_mut().find(|job| job.summary.token == token) {
                job.summary.active = false;
                job.changed = changed;
                job.summary.status = match (job.summary.status, ran, ok) {
                    // Undone while it ran: the next round puts back what it did.
                    (Status::Undoing, true, _) => Status::Undoing,
                    (_, true, true) => Status::Done,
                    (_, false, true) => Status::Undone,
                    (_, _, false) => Status::Failed,
                };
            }
            drop(jobs);
            self.shared.repaint.request();
        }
    }
}
