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
}
