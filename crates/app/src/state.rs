use std::path::PathBuf;

/// Which image is on screen and which images are used as edit references.
///
/// Text-to-image runs keep `references` empty even after producing a result, so
/// pressing Generate again makes a fresh image from the prompt. Edits promote
/// their result to the next reference, which makes iteration natural.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AppState {
    pub display: Option<PathBuf>,
    /// Images passed to the model as `ref_images`, in prompt order.
    pub references: Vec<PathBuf>,
    /// Masked inpaint prepared from the canvas, waiting for a prompt.
    pub inpaint: Option<PendingInpaint>,
}

/// A source image plus the rasterised mask that should be repainted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingInpaint {
    pub source: PathBuf,
    pub mask: PathBuf,
    pub width: u32,
    pub height: u32,
}

impl AppState {
    /// Start a text-to-image run: clear the canvas and the references.
    pub fn new_image(&mut self) {
        self.display = None;
        self.references.clear();
        self.inpaint = None;
    }

    /// Load an existing image as the only edit reference.
    pub fn load(&mut self, path: PathBuf) {
        self.display = Some(path.clone());
        self.references = vec![path];
        self.inpaint = None;
    }

    /// Show an image without changing the edit references.
    pub fn select_display(&mut self, path: PathBuf) {
        self.display = Some(path);
        self.inpaint = None;
    }

    /// Browse an image: show it and drop the edit references.
    pub fn preview(&mut self, path: PathBuf) {
        self.display = Some(path);
        self.references.clear();
        self.inpaint = None;
    }

    /// Apply a finished generation.
    pub fn on_result(&mut self, path: PathBuf, was_edit: bool) {
        self.display = Some(path.clone());
        self.references = if was_edit { vec![path] } else { Vec::new() };
        self.inpaint = None;
    }

    /// Whether at least one reference is selected.
    pub fn is_editing(&self) -> bool {
        !self.references.is_empty()
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
        assert!(state.references.is_empty());
    }

    #[test]
    fn edit_result_becomes_the_next_reference() {
        let mut state = AppState::default();
        state.load(PathBuf::from("/tmp/ref.png"));
        state.on_result(PathBuf::from("/tmp/out.png"), true);
        assert_eq!(state.display, Some(PathBuf::from("/tmp/out.png")));
        assert_eq!(state.references, vec![PathBuf::from("/tmp/out.png")]);
    }

    #[test]
    fn preview_shows_an_image_without_keeping_references() {
        let mut state = AppState::default();
        state.load(PathBuf::from("/tmp/ref.png"));
        state.preview(PathBuf::from("/tmp/other.png"));
        assert_eq!(state.display, Some(PathBuf::from("/tmp/other.png")));
        assert!(state.references.is_empty());
    }

    #[test]
    fn select_display_keeps_the_references() {
        let mut state = AppState::default();
        state.load(PathBuf::from("/tmp/ref.png"));
        state.references.push(PathBuf::from("/tmp/second.png"));
        state.select_display(PathBuf::from("/tmp/second.png"));
        assert_eq!(state.display, Some(PathBuf::from("/tmp/second.png")));
        assert_eq!(state.references.len(), 2);
    }

    #[test]
    fn load_replaces_the_reference_set() {
        let mut state = AppState {
            references: vec![PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")],
            ..Default::default()
        };
        state.load(PathBuf::from("/tmp/c.png"));
        assert_eq!(state.references, vec![PathBuf::from("/tmp/c.png")]);
    }

    #[test]
    fn new_image_clears_everything() {
        let mut state = AppState::default();
        state.load(PathBuf::from("/tmp/ref.png"));
        state.new_image();
        assert_eq!(state.display, None);
        assert!(state.references.is_empty());
    }

    #[test]
    fn a_new_image_clears_a_pending_mask() {
        let mut state = AppState {
            inpaint: Some(PendingInpaint {
                source: PathBuf::from("/tmp/a.png"),
                mask: PathBuf::from("/tmp/mask.png"),
                width: 64,
                height: 64,
            }),
            ..Default::default()
        };
        state.new_image();
        assert!(state.inpaint.is_none());
    }

    #[test]
    fn loading_an_image_clears_a_pending_mask() {
        let mut state = AppState {
            inpaint: Some(PendingInpaint {
                source: PathBuf::from("/tmp/a.png"),
                mask: PathBuf::from("/tmp/mask.png"),
                width: 64,
                height: 64,
            }),
            ..Default::default()
        };
        state.preview(PathBuf::from("/tmp/b.png"));
        assert!(state.inpaint.is_none());
        state.inpaint = Some(PendingInpaint {
            source: PathBuf::from("/tmp/b.png"),
            mask: PathBuf::from("/tmp/mask.png"),
            width: 64,
            height: 64,
        });
        state.load(PathBuf::from("/tmp/c.png"));
        assert!(state.inpaint.is_none());
    }

    #[test]
    fn undo_restores_crops_in_reverse_order() {
        let mut stack = UndoStack::default();
        let first = AppState {
            display: Some(PathBuf::from("/tmp/a.png")),
            references: Vec::new(),
            inpaint: None,
        };
        let second = AppState {
            display: Some(PathBuf::from("/tmp/b.png")),
            references: vec![PathBuf::from("/tmp/b.png")],
            inpaint: None,
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
