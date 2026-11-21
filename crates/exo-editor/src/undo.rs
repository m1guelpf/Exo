//! Squire's undo stack: whole-document snapshots taken at boundaries.
//!
//! Squire records the document (with the selection) *before* a boundary action: Enter, Space,
//! Backspace, typing over a selection, a format or block change, a paste. Plain typing between
//! boundaries records nothing, so it undoes in one step. `in_undo_state` is true right after a
//! snapshot or an undo, false as soon as the document changes. Undo first snapshots the current
//! state if it changed since the last snapshot, so redo can come back to it.
//!
//! Every snapshot is kept, with no limit, so each shares the blocks that didn't change
//! with the one before it: a keystroke in a long draft keeps one block, not the whole draft.

use std::sync::Arc;

use crate::model::{Block, ContainerId, Document, Selection};

/// Something the caller tied to an undo step (a recipient change made from the body), handed
/// back when that step is undone or redone.
pub type UndoMark = u64;

#[derive(Clone, Debug)]
struct Snapshot {
    blocks: Vec<Arc<Block>>,
    next_id: ContainerId,
    selection: Selection,
    /// The marks of changes made right after this snapshot.
    marks: Vec<UndoMark>,
}

impl Snapshot {
    /// `document`, sharing the blocks it has in common with `previous` (the unchanged start and
    /// end of the draft, around what an edit touched).
    fn new(document: &Document, selection: Selection, previous: Option<&Snapshot>) -> Self {
        let old = previous.map_or(&[][..], |previous| &previous.blocks[..]);
        let new = &document.blocks;
        let prefix = old
            .iter()
            .zip(new)
            .take_while(|(old, new)| ***old == **new)
            .count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(old, new)| ***old == **new)
            .count();
        let mut blocks = Vec::with_capacity(new.len());
        blocks.extend_from_slice(&old[..prefix]);
        blocks.extend(
            new[prefix..new.len() - suffix]
                .iter()
                .cloned()
                .map(Arc::new),
        );
        blocks.extend_from_slice(&old[old.len() - suffix..]);
        Self {
            blocks,
            next_id: document.next_id,
            selection,
            marks: Vec::new(),
        }
    }

    fn document(&self) -> Document {
        Document {
            blocks: self
                .blocks
                .iter()
                .map(|block| Block::clone(block))
                .collect(),
            next_id: self.next_id,
        }
    }
}

/// The undo stack.
#[derive(Clone, Debug, Default)]
pub(crate) struct UndoStack {
    stack: Vec<Snapshot>,
    index: usize,
    in_undo_state: bool,
}

impl UndoStack {
    /// Starts over with `document` as the only state: the stack cleared, then the first record.
    pub(crate) fn reset(&mut self, document: &Document, selection: Selection) {
        self.stack = vec![Snapshot::new(document, selection, None)];
        self.index = 0;
        self.in_undo_state = true;
    }

    /// Records the state before a boundary action. With `replace` the latest entry is overwritten;
    /// without it nothing happens while already in an undo state.
    pub(crate) fn record(&mut self, document: &Document, selection: Selection, replace: bool) {
        if self.in_undo_state && !replace {
            return;
        }
        let snapshot = Snapshot::new(document, selection, self.stack.get(self.index));
        if !replace || self.stack.is_empty() {
            self.index += usize::from(!self.stack.is_empty());
        }
        self.stack.truncate(self.index);
        self.stack.push(snapshot);
        self.in_undo_state = true;
    }

    /// Records, replacing the latest entry if nothing changed since it.
    pub(crate) fn save(&mut self, document: &Document, selection: Selection) {
        let replace = self.in_undo_state;
        self.record(document, selection, replace);
    }

    /// The document changed.
    pub(crate) fn changed(&mut self) {
        self.in_undo_state = false;
    }

    /// Ties `mark` to the change being made after the latest snapshot.
    pub(crate) fn mark(&mut self, mark: UndoMark) {
        if let Some(snapshot) = self.stack.get_mut(self.index) {
            snapshot.marks.push(mark);
        }
    }

    /// Steps back: the state to restore and the marks of the change undone.
    pub(crate) fn undo(
        &mut self,
        document: &Document,
        selection: Selection,
    ) -> Option<(Document, Selection, Vec<UndoMark>)> {
        if self.index == 0 && self.in_undo_state {
            return None;
        }
        self.record(document, selection, false);
        self.index = self.index.checked_sub(1)?;
        self.in_undo_state = true;
        let snapshot = &self.stack[self.index];
        Some((
            snapshot.document(),
            snapshot.selection,
            snapshot.marks.clone(),
        ))
    }

    /// Steps forward again: the state to restore and the marks of the change redone.
    pub(crate) fn redo(&mut self) -> Option<(Document, Selection, Vec<UndoMark>)> {
        if self.index + 1 >= self.stack.len() || !self.in_undo_state {
            return None;
        }
        let marks = self.stack[self.index].marks.clone();
        self.index += 1;
        let snapshot = &self.stack[self.index];
        Some((snapshot.document(), snapshot.selection, marks))
    }

    /// Whether there's anything to undo.
    pub(crate) fn can_undo(&self) -> bool {
        self.index > 0 || !self.in_undo_state
    }

    /// Whether there's anything to redo.
    pub(crate) fn can_redo(&self) -> bool {
        self.in_undo_state && self.index + 1 < self.stack.len()
    }
}
