use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sampler {
    #[default]
    Euler,
}

impl Sampler {
    pub fn as_str(self) -> &'static str {
        match self {
            Sampler::Euler => "euler",
        }
    }
}

/// A single image-generation request.
///
/// `steps`/`cfg` are copied from the selected [`crate::preset::ModelPreset`]
/// so the UI can show and override them, but the preset remains the source of
/// truth for what the model was trained for.
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    /// Id of the preset this request runs, e.g. `flux2-klein-4b`.
    pub preset_id: String,
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub seed: u64,
    pub sampler: Sampler,
    pub offload: bool,
    /// Reference images for editing (`-r`). Empty for text-to-image.
    pub reference_images: Vec<PathBuf>,
    pub output: PathBuf,
}

impl GenerateRequest {
    pub fn new(prompt: impl Into<String>, output: impl Into<PathBuf>) -> Self {
        let preset = crate::preset::default_preset();
        Self {
            preset_id: preset.id.to_string(),
            prompt: prompt.into(),
            width: preset.default_width,
            height: preset.default_height,
            steps: preset.steps,
            cfg: preset.cfg,
            seed: Self::random_seed(),
            sampler: preset.sampler,
            offload: true,
            reference_images: Vec::new(),
            output: output.into(),
        }
    }

    /// A well-mixed random seed that fits in `i64`.
    ///
    /// `RandomState` is seeded from OS entropy, and mixing the clock in keeps
    /// consecutive calls distinct even on coarse clocks. The value is masked to
    /// 63 bits because `sd-cli` parses `--seed` with `std::stoll`, which throws
    /// (and aborts the process) above `i64::MAX`.
    pub fn random_seed() -> u64 {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(nanos);
        hasher.finish() & (i64::MAX as u64)
    }

    /// Largest seed the backend accepts.
    pub const MAX_SEED: u64 = i64::MAX as u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_seeds_differ() {
        let first = GenerateRequest::random_seed();
        let second = GenerateRequest::random_seed();
        assert_ne!(first, second);
    }

    #[test]
    fn random_seeds_fit_in_i64() {
        for _ in 0..100 {
            assert!(GenerateRequest::random_seed() <= i64::MAX as u64);
        }
    }
}

#[derive(Debug, Clone)]
pub enum Progress {
    /// Weights are being loaded (no step information yet).
    Loading,
    Step { current: u32, total: u32 },
    Done(PathBuf),
    Cancelled,
    Failed(String),
    /// Raw log line from the backend. Useful for the log panel and debugging.
    Log(String),
}
