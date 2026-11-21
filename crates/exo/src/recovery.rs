//! Crash recovery, for Exo's one process. What panics starts again, and keeps what it held:
//!
//! - An account's background (its sync, sends, reminders, connection, bulk and calendar loops,
//!   and the worker that answers the UI) restarts 0, 5, 10, 30 and 60 s after its successive
//!   failures and gives up at the sixth with `The background process has crashed repeatedly.`
//!   A loop that panics runs again after that wait, its wakes and channels kept; a request that
//!   panics the worker is dropped, and the worker answers the next one after it. What's queued
//!   and stored (held sends, drafts, queued changes) waits meanwhile.
//! - The body renderer's thread is restarted the same way.
//! - The UI thread recovers at once: a frame that panics is dropped and the next one drawn. As
//!   that can loop, Exo gives up at the sixth frame in a row, as for a background.
//!
//! Giving up shows the crash dialog and quits, through the quit flush, without asking first.
//! Every panic is logged where it happened (`logging`).
//!
//! An account that closes stops its loops first ([`Restarts::stop`]), then its runtime: the
//! store's own tasks (toasty's connections) end with the runtime, in no set order, and a loop
//! still at work then would find its connection gone and panic.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How long to wait before each restart, in seconds; one more failure gives up.
const DELAYS: [u64; 5] = [0, 5, 10, 30, 60];

/// What `The background process has crashed repeatedly.` reports: the `{type}` of a background
/// that died, and how many times it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaveUp {
    /// The failures, the last included.
    pub attempts: usize,
}

impl GaveUp {
    /// The error line's type: a background that died.
    pub const KIND: &'static str = "process-gone";
}

/// Exo gave up on something that kept failing: the app shows the dialog and quits.
static GAVE_UP: Mutex<Option<GaveUp>> = Mutex::new(None);

/// Whether something failed too often since the last call; the app then shows the crash dialog
/// and quits.
pub fn gave_up() -> Option<GaveUp> {
    GAVE_UP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
}

/// The failures of one thing that restarts (an account's background, the body renderer), counted
/// for the session, and the loops it supervises.
pub(crate) struct Restarts {
    failures: AtomicUsize,
    /// Woken to show the dialog once it gives up.
    ctx: egui::Context,
    /// The tasks supervising each loop ([`spawn`], [`spawn_once`]).
    loops: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// The short tasks the worker and the loader start ([`Restarts::spawn_task`]).
    tasks: Mutex<tokio::task::JoinSet<()>>,
    /// Turns true when they're to stop.
    stopping: tokio::sync::watch::Sender<bool>,
}

impl Restarts {
    /// No failures yet; `ctx` is the window's.
    pub(crate) fn new(ctx: egui::Context) -> Self {
        Self {
            failures: AtomicUsize::new(0),
            ctx,
            loops: Mutex::default(),
            tasks: Mutex::default(),
            stopping: tokio::sync::watch::Sender::new(false),
        }
    }

    /// Runs `task` (a fetch, a page of search, a write) on `runtime` until it ends or
    /// [`Restarts::stop`].
    pub(crate) fn spawn_task(
        &self,
        runtime: &tokio::runtime::Handle,
        task: impl Future<Output = ()> + Send + 'static,
    ) {
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        // Those that ended go, so the set holds what runs.
        while tasks.try_join_next().is_some() {}
        tasks.spawn_on(task, runtime);
    }

    /// Stops every loop it supervises and every task it started, and waits, `grace` at most,
    /// until they're gone: each is dropped at its next `.await`, what it was doing with the store
    /// with it.
    pub(crate) async fn stop(&self, grace: Duration) {
        self.stopping.send_replace(true);
        let loops = std::mem::take(&mut *self.loops.lock().unwrap_or_else(PoisonError::into_inner));
        let mut tasks =
            std::mem::take(&mut *self.tasks.lock().unwrap_or_else(PoisonError::into_inner));
        let _ = tokio::time::timeout(grace, async {
            futures::future::join_all(loops).await;
            tasks.shutdown().await;
        })
        .await;
    }

    /// Another failure: how long to wait before starting again, or `None` once it has failed more
    /// than five times, when Exo gives up.
    pub(crate) fn failed(&self) -> Option<Duration> {
        let attempts = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        let wait = DELAYS
            .get(attempts - 1)
            .map(|&seconds| Duration::from_secs(seconds));
        match wait {
            Some(wait) => log::error!("restarting after a panic in {wait:?} ({attempts} attempts)"),
            None => {
                log::error!("giving up after {attempts} panics");
                *GAVE_UP.lock().unwrap_or_else(PoisonError::into_inner) = Some(GaveUp { attempts });
                self.ctx.request_repaint();
            }
        }
        wait
    }
}

/// Runs the loop `run` makes on the current Tokio runtime, and makes it again whenever it panics,
/// after `restarts`' wait, until Exo gives up or [`Restarts::stop`]. What `run` captures (wakes,
/// channels, the store) outlives each run, so a restarted loop picks up where the last left off.
pub(crate) fn spawn<F>(restarts: &Arc<Restarts>, mut run: impl FnMut() -> F + Send + 'static)
where
    F: Future<Output = ()> + Send + 'static,
{
    let supervisor = restarts.clone();
    supervise(restarts, async move {
        loop {
            match until_stopped(&supervisor, run()).await {
                Some(Err(error)) if error.is_panic() => match supervisor.failed() {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => return,
                },
                // It ended, it was stopped, or the runtime is shutting down.
                _ => return,
            }
        }
    });
}

/// Runs `task` (the worker, which catches its own panics) on the current Tokio runtime until it
/// ends or [`Restarts::stop`].
pub(crate) fn spawn_once<F>(restarts: &Arc<Restarts>, task: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let supervisor = restarts.clone();
    supervise(restarts, async move {
        until_stopped(&supervisor, task).await;
    });
}

/// Spawns `supervisor`, which [`Restarts::stop`] waits for.
fn supervise(restarts: &Restarts, supervisor: impl Future<Output = ()> + Send + 'static) {
    let handle = tokio::spawn(supervisor);
    restarts
        .loops
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(handle);
}

/// Runs `task` as a task of its own until it ends (how, unless the runtime is shutting down) or
/// `restarts` is stopping, when it's aborted and waited for: `None`.
async fn until_stopped<F>(
    restarts: &Restarts,
    task: F,
) -> Option<Result<(), tokio::task::JoinError>>
where
    F: Future<Output = ()> + Send + 'static,
{
    let mut stopping = restarts.stopping.subscribe();
    let mut running = tokio::spawn(task);
    let stopped = async move {
        // Never `Err`: `restarts` holds the sender.
        let _ = stopping.wait_for(|stop| *stop).await;
    };
    tokio::select! {
        ended = &mut running => return Some(ended),
        () = stopped => {}
    }
    running.abort();
    let _ = running.await;
    None
}

/// The UI thread's frames: a frame that panics is dropped and the next one drawn; the sixth in a
/// row gives up.
#[derive(Default)]
pub(crate) struct Frames {
    /// Frames in a row that panicked.
    panicked: usize,
}

impl Frames {
    /// Draws a frame with `draw`, surviving a panic in it.
    pub(crate) fn draw(&mut self, ctx: &egui::Context, draw: impl FnOnce()) {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(draw)).is_ok() {
            self.panicked = 0;
            return;
        }
        self.panicked += 1;
        if self.panicked > DELAYS.len() {
            log::error!("giving up after {} frames in a row panicked", self.panicked);
            *GAVE_UP.lock().unwrap_or_else(PoisonError::into_inner) = Some(GaveUp {
                attempts: self.panicked,
            });
        } else {
            log::error!("a frame panicked; drawing the next");
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests that give up, one at a time: giving up is the process's.
    static GIVING_UP: Mutex<()> = Mutex::new(());

    #[test]
    fn a_background_restarts_after_growing_waits_then_gives_up() {
        let _one = GIVING_UP.lock().unwrap_or_else(PoisonError::into_inner);
        let restarts = Restarts::new(egui::Context::default());
        let waits: Vec<Option<u64>> = (0..6)
            .map(|_| restarts.failed().map(|wait| wait.as_secs()))
            .collect();
        assert_eq!(
            waits,
            [Some(0), Some(5), Some(10), Some(30), Some(60), None]
        );
        assert_eq!(gave_up(), Some(GaveUp { attempts: 6 }));
        assert_eq!(gave_up(), None, "once");
    }

    #[test]
    fn a_frame_that_panics_is_dropped_and_the_sixth_in_a_row_gives_up() {
        let _one = GIVING_UP.lock().unwrap_or_else(PoisonError::into_inner);
        let ctx = egui::Context::default();
        let mut frames = Frames::default();
        let mut drawn = 0;
        frames.draw(&ctx, || panic!("a test frame"));
        frames.draw(&ctx, || drawn += 1);
        assert_eq!((drawn, frames.panicked), (1, 0), "the next frame draws");
        for _ in 0..5 {
            frames.draw(&ctx, || panic!("a test frame"));
        }
        assert_eq!(frames.panicked, 5);
        frames.draw(&ctx, || panic!("a test frame"));
        assert_eq!(gave_up().map(|gave_up| gave_up.attempts), Some(6));
    }

    #[tokio::test]
    async fn stopping_waits_until_every_loop_is_gone() {
        /// Counts the loops running: one each from its start until it's dropped.
        struct Running(Arc<AtomicUsize>);
        impl Drop for Running {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let running = Arc::new(AtomicUsize::new(0));
        let run = |running: Arc<AtomicUsize>| async move {
            running.fetch_add(1, Ordering::SeqCst);
            let _running = Running(running);
            std::future::pending::<()>().await;
        };
        let restarts = Arc::new(Restarts::new(egui::Context::default()));
        for _ in 0..3 {
            let running = running.clone();
            spawn(&restarts, move || run(running.clone()));
        }
        spawn_once(&restarts, run(running.clone()));
        restarts.spawn_task(&tokio::runtime::Handle::current(), run(running.clone()));
        while running.load(Ordering::SeqCst) < 5 {
            tokio::task::yield_now().await;
        }
        restarts.stop(Duration::from_secs(5)).await;
        assert_eq!(
            running.load(Ordering::SeqCst),
            0,
            "each was dropped before `stop` returned"
        );
    }

    #[tokio::test]
    async fn a_loop_that_panics_runs_again_with_what_it_kept() {
        let restarts = Arc::new(Restarts::new(egui::Context::default()));
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<u32>();
        let receiver = Arc::new(tokio::sync::Mutex::new(receiver));
        let (done, finished) = tokio::sync::oneshot::channel();
        let done = Arc::new(Mutex::new(Some(done)));
        spawn(&restarts, move || {
            let (receiver, done) = (receiver.clone(), done.clone());
            async move {
                while let Some(value) = receiver.lock().await.recv().await {
                    assert_ne!(value, 1, "a test panic");
                    if let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() {
                        let _ = done.send(value);
                    }
                }
            }
        });
        sender.send(1).expect("sent");
        sender.send(2).expect("sent");
        assert_eq!(
            finished.await,
            Ok(2),
            "the value after the panic, by the restarted loop"
        );
        assert_eq!(restarts.failures.load(Ordering::SeqCst), 1);
    }
}
