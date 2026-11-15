//! Optional vision helper used for small image decisions such as day/night.

use std::path::{Path, PathBuf};

use crate::preset::{ModelFileSpec, ModelRole};

/// A small vision-language model plus its multimodal projector.
#[derive(Debug, Clone, Copy)]
pub struct VisionHelper {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub model: ModelFileSpec,
    pub mmproj: ModelFileSpec,
    pub license: &'static str,
    pub license_url: &'static str,
    pub credits: &'static str,
}

impl VisionHelper {
    pub fn files(&self) -> [&ModelFileSpec; 2] {
        [&self.model, &self.mmproj]
    }

    pub fn model_path(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.model.rel_path)
    }

    pub fn mmproj_path(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.mmproj.rel_path)
    }

    pub fn missing(&self, models_dir: &Path) -> Vec<&ModelFileSpec> {
        self.files()
            .into_iter()
            .filter(|file| !models_dir.join(file.rel_path).exists())
            .collect()
    }

    pub fn is_installed(&self, models_dir: &Path) -> bool {
        self.missing(models_dir).is_empty()
    }

    pub fn approx_bytes(&self) -> u64 {
        self.model.approx_bytes + self.mmproj.approx_bytes
    }
}

/// LFM2.5-VL-450M is small, fast, and reliable at one-word day/night answers.
pub const VISION_HELPER: VisionHelper = VisionHelper {
    id: "lfm2.5-vl-450m",
    label: "LFM2.5-VL-450M",
    description: "Day/night image classification for the lighting button",
    model: ModelFileSpec {
        role: ModelRole::VisionEncoder,
        rel_path: "vision/LFM2.5-VL-450M-Q4_K_M.gguf",
        url: "https://huggingface.co/LiquidAI/LFM2.5-VL-450M-GGUF/resolve/main/LFM2.5-VL-450M-Q4_K_M.gguf",
        approx_bytes: 229_313_568,
    },
    mmproj: ModelFileSpec {
        role: ModelRole::VisionEncoder,
        rel_path: "vision/mmproj-LFM2.5-VL-450m-Q8_0.gguf",
        url: "https://huggingface.co/LiquidAI/LFM2.5-VL-450M-GGUF/resolve/main/mmproj-LFM2.5-VL-450m-Q8_0.gguf",
        approx_bytes: 102_815_168,
    },
    license: "LFM Open License v1.0",
    license_url: "https://huggingface.co/LiquidAI/LFM2.5-VL-450M-GGUF/blob/main/LICENSE",
    credits: "Liquid AI",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_paths_under_the_models_directory() {
        let root = Path::new("/models");
        assert_eq!(
            VISION_HELPER.model_path(root),
            root.join("vision/LFM2.5-VL-450M-Q4_K_M.gguf")
        );
        assert_eq!(
            VISION_HELPER.mmproj_path(root),
            root.join("vision/mmproj-LFM2.5-VL-450m-Q8_0.gguf")
        );
        assert_eq!(VISION_HELPER.files().len(), 2);
        assert!(VISION_HELPER.approx_bytes() > 300_000_000);
    }

    #[test]
    fn reports_missing_files() {
        let root = std::env::temp_dir().join("flux-vision-missing");
        let _ = std::fs::remove_dir_all(&root);
        assert!(!VISION_HELPER.is_installed(&root));
        assert_eq!(VISION_HELPER.missing(&root).len(), 2);
    }
}
