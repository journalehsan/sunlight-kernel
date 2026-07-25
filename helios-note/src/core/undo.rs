//! Bounded undo and redo transaction history.

#[derive(Debug, Clone)]
pub struct UndoHistory {
    undo_stack: Vec<Vec<String>>,
    redo_stack: Vec<Vec<String>>,
    max_depth: usize,
}

impl Default for UndoHistory {
    fn default() -> Self {
        Self::new(50)
    }
}

impl UndoHistory {
    pub fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_depth,
        }
    }

    /// Record snapshot of lines before edit. Clears redo stack.
    pub fn push_snapshot(&mut self, lines: Vec<String>) {
        if self.undo_stack.len() >= self.max_depth {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(lines);
        self.redo_stack.clear();
    }

    /// Undo: pop from undo_stack onto redo_stack, return previous lines snapshot.
    pub fn undo(&mut self, current_lines: Vec<String>) -> Option<Vec<String>> {
        let prev = self.undo_stack.pop()?;
        self.redo_stack.push(current_lines);
        Some(prev)
    }

    /// Redo: pop from redo_stack onto undo_stack, return next lines snapshot.
    pub fn redo(&mut self, current_lines: Vec<String>) -> Option<Vec<String>> {
        let next = self.redo_stack.pop()?;
        self.undo_stack.push(current_lines);
        Some(next)
    }

    #[cfg(test)]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    #[cfg(test)]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_workflow() {
        let mut history = UndoHistory::new(5);
        let s0 = vec!["Line 1".to_string()];
        let s1 = vec!["Line 1".to_string(), "Line 2".to_string()];

        history.push_snapshot(s0.clone());
        assert!(history.can_undo());
        assert!(!history.can_redo());

        let undone = history.undo(s1.clone()).unwrap();
        assert_eq!(undone, s0);
        assert!(history.can_redo());

        let redone = history.redo(s0.clone()).unwrap();
        assert_eq!(redone, s1);
    }
}
