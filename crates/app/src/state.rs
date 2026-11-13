use std::path::PathBuf;

/// Which image is on screen and whether it is also the edit reference.
///
/// Text-to-image runs keep `reference` empty even after producing a result, so
/// pressing Generate again makes a fresh image from the prompt. Edits promote
/// their result to the next reference, which makes iteration natural.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AppState {
    pub display: Option<PathBuf>,
    pub reference: Option<PathBuf>,
}

impl AppState {
    /// Start a text-to-image run: clear the canvas and the reference.
    pub fn new_image(&mut self) {
        self.display = None;
        self.reference = None;
    }

    /// Load an existing image (history click or file picker) for editing.
    pub fn load(&mut self, path: PathBuf) {
        self.display = Some(path.clone());
        self.reference = Some(path);
    }

    /// Apply a finished generation.
    pub fn on_result(&mut self, path: PathBuf, was_edit: bool) {
        self.display = Some(path.clone());
        self.reference = if was_edit { Some(path) } else { None };
    }
}

/// A state snapshot plus the region that was cropped out of it.
#[derive(Debug, Clone)]
pub(crate) struct UndoEntry {
    pub state: AppState,
    /// Selection in the previous image's pixel coordinates.
    pub selection: (f64, f64, f64, f64),
}

/// A small LIFO history for crop/zoom steps.
#[derive(Debug, Default)]
pub(crate) struct UndoStack {
    entries: Vec<UndoEntry>,
}

impl UndoStack {
    const LIMIT: usize = 20;

    pub fn push(&mut self, state: &AppState, selection: (f64, f64, f64, f64)) {
        self.entries.push(UndoEntry {
            state: state.clone(),
            selection,
        });
        if self.entries.len() > Self::LIMIT {
            self.entries.remove(0);
        }
    }

    pub fn undo(&mut self) -> Option<UndoEntry> {
        self.entries.pop()
    }

    pub fn can_undo(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_to_image_result_does_not_become_a_reference() {
        let mut state = AppState::default();
        state.on_result(PathBuf::from("/tmp/a.png"), false);
        assert_eq!(state.display, Some(PathBuf::from("/tmp/a.png")));
        assert_eq!(state.reference, None);
    }

    #[test]
    fn edit_result_becomes_the_next_reference() {
        let mut state = AppState::default();
        state.load(PathBuf::from("/tmp/ref.png"));
        state.on_result(PathBuf::from("/tmp/out.png"), true);
        assert_eq!(state.display, Some(PathBuf::from("/tmp/out.png")));
        assert_eq!(state.reference, Some(PathBuf::from("/tmp/out.png")));
    }

    #[test]
    fn new_image_clears_everything() {
        let mut state = AppState::default();
        state.load(PathBuf::from("/tmp/ref.png"));
        state.new_image();
        assert_eq!(state.display, None);
        assert_eq!(state.reference, None);
    }

    #[test]
    fn undo_restores_crops_in_reverse_order() {
        let mut stack = UndoStack::default();
        let first = AppState {
            display: Some(PathBuf::from("/tmp/a.png")),
            reference: None,
        };
        let second = AppState {
            display: Some(PathBuf::from("/tmp/b.png")),
            reference: Some(PathBuf::from("/tmp/b.png")),
        };
        stack.push(&first, (1.0, 2.0, 3.0, 4.0));
        stack.push(&second, (5.0, 6.0, 7.0, 8.0));
        assert!(stack.can_undo());

        let entry = stack.undo().unwrap();
        assert_eq!(entry.state, second);
        assert_eq!(entry.selection, (5.0, 6.0, 7.0, 8.0));
        let entry = stack.undo().unwrap();
        assert_eq!(entry.state, first);
        assert_eq!(entry.selection, (1.0, 2.0, 3.0, 4.0));
        assert!(!stack.can_undo());
        assert!(stack.undo().is_none());
    }

    #[test]
    fn undo_stack_clears() {
        let mut stack = UndoStack::default();
        stack.push(&AppState::default(), (0.0, 0.0, 1.0, 1.0));
        stack.clear();
        assert!(!stack.can_undo());
    }
}
