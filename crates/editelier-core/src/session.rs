//! Undo/redo over copy-on-write project snapshots.

use crate::{Command, Error, Id, Project};
use std::sync::Arc;

struct Entry {
    label: &'static str,
    state: Arc<Project>,
    drag: Option<u64>,
}

/// The open project plus its history. Every edit goes through [`Session::execute`].
// ponytail: unbounded history; cap it (drop oldest) if memory ever matters, snapshots share Arcs.
pub struct Session {
    project: Arc<Project>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

impl Session {
    pub fn new(project: Project) -> Self {
        Session {
            project: Arc::new(project),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn project(&self) -> &Arc<Project> {
        &self.project
    }

    /// Apply `cmd` as one undo step; a failed command leaves everything untouched. Consecutive
    /// commands with the same `drag` id (trims from one mouse drag) share a single undo step.
    pub fn execute(&mut self, cmd: &Command) -> Result<Option<Id>, Error> {
        let mut next = (*self.project).clone();
        let created = cmd.apply(&mut next)?;
        let drag = cmd.drag();
        let coalesce = drag.is_some()
            && self.redo.is_empty()
            && self.undo.last().is_some_and(|e| e.drag == drag);
        let before = std::mem::replace(&mut self.project, Arc::new(next));
        if !coalesce {
            self.undo.push(Entry {
                label: cmd.name(),
                state: before,
                drag,
            });
        }
        self.redo.clear();
        Ok(created)
    }

    /// Undo the last step; returns its label, or `None` when there's nothing to undo.
    pub fn undo(&mut self) -> Option<&'static str> {
        let e = self.undo.pop()?;
        let after = std::mem::replace(&mut self.project, e.state);
        self.redo.push(Entry {
            label: e.label,
            state: after,
            drag: e.drag,
        });
        Some(e.label)
    }

    /// Redo the last undone step; returns its label.
    pub fn redo(&mut self) -> Option<&'static str> {
        let e = self.redo.pop()?;
        let before = std::mem::replace(&mut self.project, e.state);
        self.undo.push(Entry {
            label: e.label,
            state: before,
            drag: e.drag,
        });
        Some(e.label)
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|e| e.label)
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|e| e.label)
    }
}
