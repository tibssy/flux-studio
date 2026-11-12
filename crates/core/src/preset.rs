use std::path::{Path, PathBuf};

use crate::request::Sampler;

/// The role a model file plays in a preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRole {
    Diffusion,
    Vae,
    TextEncoder,
    VisionEncoder,
}

/// A single downloadable file belonging to a preset.
#[derive(Debug, Clone, Copy)]
pub struct ModelFileSpec {
    pub role: ModelRole,
    /// Path relative to the models directory.
    pub rel_path: &'static str,
    pub url: &'static str,
    pub approx_bytes: u64,
}

/// Everything the backend needs to run one model family.
///
/// Adding a model means adding a preset here; the UI, downloader and history
/// all read from this registry instead of hardcoding FLUX.2 klein.
#[derive(Debug, Clone, Copy)]
pub struct ModelPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub files: &'static [ModelFileSpec],
    pub steps: u32,
    pub cfg: f32,
    pub sampler: Sampler,
    pub default_width: u32,
    pub default_height: u32,
    /// Whether the model accepts `-r <image>` reference images.
    pub supports_edit: bool,
    /// Extra backend flags specific to this family.
    pub extra_args: &'static [&'static str],
}

impl ModelPreset {
    pub fn file(&self, role: ModelRole) -> Option<&'static ModelFileSpec> {
        self.files.iter().find(|file| file.role == role)
    }

    pub fn path(&self, role: ModelRole, models_dir: &Path) -> Option<PathBuf> {
        self.file(role).map(|file| models_dir.join(file.rel_path))
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.approx_bytes).sum()
    }

    /// Files that are not present yet.
    pub fn missing(&self, models_dir: &Path) -> Vec<&'static ModelFileSpec> {
        self.files
            .iter()
            .filter(|file| !models_dir.join(file.rel_path).exists())
            .collect()
    }

    pub fn is_installed(&self, models_dir: &Path) -> bool {
        self.missing(models_dir).is_empty()
    }
}

pub const FLUX2_KLEIN_4B: ModelPreset = ModelPreset {
    id: "flux2-klein-4b",
    label: "FLUX.2 klein 4B",
    description: "4-step distilled · fastest on this GPU",
    files: &[
        ModelFileSpec {
            role: ModelRole::Diffusion,
            rel_path: "diffusion/flux-2-klein-4b-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/FLUX.2-klein-4B-GGUF/resolve/main/flux-2-klein-4b-Q4_K_M.gguf",
            approx_bytes: 2_604_311_104,
        },
        ModelFileSpec {
            role: ModelRole::Vae,
            rel_path: "vae/flux2-vae.safetensors",
            url: "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/vae/flux2-vae.safetensors",
            approx_bytes: 336_213_556,
        },
        ModelFileSpec {
            role: ModelRole::TextEncoder,
            rel_path: "text_encoders/Qwen3-4B-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf",
            approx_bytes: 2_497_281_312,
        },
    ],
    steps: 4,
    cfg: 1.0,
    sampler: Sampler::Euler,
    default_width: 768,
    default_height: 768,
    supports_edit: true,
    extra_args: &[],
};

pub const FLUX3_KLEIN: ModelPreset = ModelPreset {
    id: "flux3-klein",
    label: "FLUX.3 Image klein",
    description: "16-step distilled · higher quality, ~4× slower",
    files: &[
        ModelFileSpec {
            role: ModelRole::Diffusion,
            rel_path: "diffusion/flux3-image-klein-Q4_K.gguf",
            url: "https://huggingface.co/leejet/flux-3-image-klein-GGUF/resolve/main/flux3-image-klein-Q4_K.gguf",
            approx_bytes: 3_514_023_104,
        },
        ModelFileSpec {
            role: ModelRole::Vae,
            rel_path: "vae/flux2-vae.safetensors",
            url: "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/vae/flux2-vae.safetensors",
            approx_bytes: 336_213_556,
        },
        ModelFileSpec {
            role: ModelRole::TextEncoder,
            rel_path: "text_encoders/Qwen3VL-4B-Instruct-Q4_K_M.gguf",
            url: "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3VL-4B-Instruct-Q4_K_M.gguf",
            approx_bytes: 2_497_281_664,
        },
    ],
    steps: 16,
    cfg: 1.0,
    sampler: Sampler::Euler,
    default_width: 768,
    default_height: 768,
    supports_edit: true,
    extra_args: &[],
};

pub const PRESETS: &[ModelPreset] = &[FLUX2_KLEIN_4B, FLUX3_KLEIN];

pub fn preset(id: &str) -> Option<&'static ModelPreset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

pub fn default_preset() -> &'static ModelPreset {
    &FLUX2_KLEIN_4B
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_ids_are_unique() {
        for (index, preset) in PRESETS.iter().enumerate() {
            assert!(
                PRESETS[index + 1..]
                    .iter()
                    .all(|other| other.id != preset.id),
                "duplicate preset id {}",
                preset.id
            );
        }
    }

    #[test]
    fn every_preset_has_required_files() {
        for preset in PRESETS {
            assert!(preset.file(ModelRole::Diffusion).is_some(), "{}", preset.id);
            assert!(preset.file(ModelRole::Vae).is_some(), "{}", preset.id);
            assert!(preset.file(ModelRole::TextEncoder).is_some(), "{}", preset.id);
            assert!(preset.total_bytes() > 0);
        }
    }

    #[test]
    fn resolves_paths_under_models_dir() {
        let preset = &FLUX2_KLEIN_4B;
        let path = preset.path(ModelRole::Diffusion, Path::new("/models")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/models/diffusion/flux-2-klein-4b-Q4_K_M.gguf")
        );
    }
}
