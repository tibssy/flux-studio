use std::path::{Path, PathBuf};

use crate::request::Sampler;

/// The role a model file plays in a preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRole {
    Diffusion,
    Vae,
    /// LLM text encoder (`--llm`) for FLUX.2/3, or T5-XXL (`--t5xxl`) for FLUX.1.
    TextEncoder,
    /// CLIP-L text encoder (`--clip_l`), used by FLUX.1 alongside T5.
    ClipEncoder,
    VisionEncoder,
    Upscaler,
    /// Tiny AutoEncoder used only for live step previews (`--taesd`).
    ///
    /// Optional: generation works without it, falling back to a rough latent
    /// projection for previews.
    Preview,
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

/// Licensing information for a preset's weights.
#[derive(Debug, Clone, Copy)]
pub struct ModelLicense {
    /// Short license name, e.g. "Apache-2.0".
    pub name: &'static str,
    /// Copyright holder.
    pub holder: &'static str,
    /// Link to the license text or model card.
    pub url: &'static str,
    /// One-sentence summary shown next to the download button.
    pub summary: &'static str,
    /// Require an explicit in-app confirmation before downloading.
    pub requires_acceptance: bool,
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
    /// Largest number of reference images the model accepts (0 without edit).
    pub max_references: u32,
    /// Licensing for the weights, shown in Preferences and Credits.
    pub license: Option<ModelLicense>,
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

    /// Missing files needed to run the model, ignoring optional preview weights.
    pub fn missing_required(&self, models_dir: &Path) -> Vec<&'static ModelFileSpec> {
        self.files
            .iter()
            .filter(|file| file.role != ModelRole::Preview)
            .filter(|file| !models_dir.join(file.rel_path).exists())
            .collect()
    }

    /// Whether the model can generate. Preview weights are optional.
    pub fn is_installed(&self, models_dir: &Path) -> bool {
        self.missing_required(models_dir).is_empty()
    }

    /// The optional Tiny AutoEncoder used for live step previews.
    pub fn preview_weights(&self, models_dir: &Path) -> Option<PathBuf> {
        self.path(ModelRole::Preview, models_dir)
            .filter(|path| path.exists())
    }

    /// The text encoder as a causal LLM, when the preset uses one.
    ///
    /// FLUX.2/FLUX.3 and MOSS feed a Qwen/Mistral model through `--llm`, which
    /// llama.cpp can also reuse for prompt improvement. FLUX.1 instead pairs
    /// T5-XXL with CLIP-L, neither of which is a chat model.
    pub fn llm_encoder(&self) -> Option<&'static ModelFileSpec> {
        if self.file(ModelRole::ClipEncoder).is_some() {
            None
        } else {
            self.file(ModelRole::TextEncoder)
        }
    }

    /// Whether the text encoders are T5-XXL + CLIP-L (`--t5xxl`/`--clip_l`).
    pub fn uses_clip_encoders(&self) -> bool {
        self.file(ModelRole::ClipEncoder).is_some()
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
            url: "https://huggingface.co/Comfy-Org/Flux-2-Klein-4B/resolve/main/split_files/vae/flux2-vae.safetensors",
            approx_bytes: 336_211_292,
        },
        ModelFileSpec {
            role: ModelRole::TextEncoder,
            rel_path: "text_encoders/Qwen3-4B-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf",
            approx_bytes: 2_497_281_312,
        },
        ModelFileSpec {
            role: ModelRole::Preview,
            rel_path: "vae/taef2.safetensors",
            url: "https://huggingface.co/madebyollin/taef2/resolve/main/taef2.safetensors",
            approx_bytes: 10_700_000,
        },
    ],
    steps: 4,
    cfg: 1.0,
    sampler: Sampler::Euler,
    default_width: 768,
    default_height: 768,
    supports_edit: true,
    max_references: 10,
    license: Some(ModelLicense {
        name: "Apache-2.0",
        holder: "Black Forest Labs",
        url: "https://huggingface.co/black-forest-labs/FLUX.2-klein-4B",
        summary: "Apache-2.0; commercial use allowed.",
        requires_acceptance: false,
    }),
    extra_args: &[],
};

/// FLUX.2 klein 9B: the larger sibling of klein 4B, with the same 4-step
/// distillation and multi-reference editing.
///
/// Weights are under the **FLUX Non-Commercial License**, so the download sits
/// behind the in-app license confirmation.
pub const FLUX2_KLEIN_9B: ModelPreset = ModelPreset {
    id: "flux2-klein-9b",
    label: "FLUX.2 klein 9B",
    description: "4-step distilled · 9B · needs ~10 GB of RAM",
    files: &[
        ModelFileSpec {
            role: ModelRole::Diffusion,
            rel_path: "diffusion/flux-2-klein-9b-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/FLUX.2-klein-9B-GGUF/resolve/main/flux-2-klein-9b-Q4_K_M.gguf",
            approx_bytes: 5_673_900_320,
        },
        ModelFileSpec {
            role: ModelRole::Vae,
            rel_path: "vae/flux2-vae.safetensors",
            url: "https://huggingface.co/Comfy-Org/Flux-2-Klein-4B/resolve/main/split_files/vae/flux2-vae.safetensors",
            approx_bytes: 336_211_292,
        },
        ModelFileSpec {
            role: ModelRole::TextEncoder,
            rel_path: "text_encoders/Qwen3-8B-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf",
            approx_bytes: 5_027_784_512,
        },
        ModelFileSpec {
            role: ModelRole::Preview,
            rel_path: "vae/taef2.safetensors",
            url: "https://huggingface.co/madebyollin/taef2/resolve/main/taef2.safetensors",
            approx_bytes: 10_700_000,
        },
    ],
    steps: 4,
    cfg: 1.0,
    sampler: Sampler::Euler,
    default_width: 768,
    default_height: 768,
    supports_edit: true,
    max_references: 10,
    license: Some(ModelLicense {
        name: "FLUX Non-Commercial License",
        holder: "Black Forest Labs",
        url: "https://huggingface.co/black-forest-labs/FLUX.2-klein-9B/blob/main/LICENSE.txt",
        summary: "Non-commercial use only. Commercial use requires a license from Black Forest Labs.",
        requires_acceptance: true,
    }),
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
            url: "https://huggingface.co/Comfy-Org/Flux-2-Klein-4B/resolve/main/split_files/vae/flux2-vae.safetensors",
            approx_bytes: 336_211_292,
        },
        ModelFileSpec {
            role: ModelRole::TextEncoder,
            rel_path: "text_encoders/Qwen3VL-4B-Instruct-Q4_K_M.gguf",
            url: "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3VL-4B-Instruct-Q4_K_M.gguf",
            approx_bytes: 2_497_281_664,
        },
        ModelFileSpec {
            role: ModelRole::Preview,
            rel_path: "vae/taef2.safetensors",
            url: "https://huggingface.co/madebyollin/taef2/resolve/main/taef2.safetensors",
            approx_bytes: 10_700_000,
        },
    ],
    steps: 16,
    cfg: 1.0,
    sampler: Sampler::Euler,
    default_width: 768,
    default_height: 768,
    supports_edit: true,
    max_references: 10,
    license: Some(ModelLicense {
        name: "FLUX Community License",
        holder: "Black Forest Labs",
        url: "https://huggingface.co/black-forest-labs/flux-3-image-klein",
        summary: "Commercial use allowed below the license revenue threshold.",
        requires_acceptance: false,
    }),
    extra_args: &[],
};

/// Fibo 2.0 Edit Turbo, distilled for reference-image editing in 4 steps.
///
/// Weights are **CC BY-NC 4.0** (non-commercial), © BRIA AI. The preset gates
/// the download behind an in-app license confirmation.
pub const FIBO2_EDIT_TURBO: ModelPreset = ModelPreset {
    id: "fibo2-edit-turbo",
    label: "Fibo 2.0 Edit Turbo",
    description: "4-step edit · 8B · non-commercial licence",
    files: &[
        ModelFileSpec {
            role: ModelRole::Diffusion,
            rel_path: "diffusion/fibo2_edit_turbo-Q4_K_M.gguf",
            url: "https://huggingface.co/molbal/fibo-2.0-GGUF/resolve/main/fibo2-edit-turbo-Q4_K_M.gguf",
            approx_bytes: 5_540_000_000,
        },
        ModelFileSpec {
            role: ModelRole::Vae,
            rel_path: "vae/flux2-vae.safetensors",
            url: "https://huggingface.co/Comfy-Org/Flux-2-Klein-4B/resolve/main/split_files/vae/flux2-vae.safetensors",
            approx_bytes: 336_211_292,
        },
        ModelFileSpec {
            role: ModelRole::TextEncoder,
            rel_path: "text_encoders/Qwen3VL-4B-Instruct-Q4_K_M.gguf",
            url: "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3VL-4B-Instruct-Q4_K_M.gguf",
            approx_bytes: 2_497_281_664,
        },
        ModelFileSpec {
            role: ModelRole::VisionEncoder,
            rel_path: "text_encoders/mmproj-Qwen3VL-4B-Instruct-F16.gguf",
            url: "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/main/mmproj-Qwen3VL-4B-Instruct-F16.gguf",
            approx_bytes: 840_000_000,
        },
        ModelFileSpec {
            role: ModelRole::Preview,
            rel_path: "vae/taef2.safetensors",
            url: "https://huggingface.co/madebyollin/taef2/resolve/main/taef2.safetensors",
            approx_bytes: 10_700_000,
        },
    ],
    steps: 4,
    cfg: 1.0,
    sampler: Sampler::Euler,
    default_width: 768,
    default_height: 768,
    supports_edit: true,
    max_references: 10,
    license: Some(ModelLicense {
        name: "CC BY-NC 4.0",
        holder: "BRIA AI",
        url: "https://creativecommons.org/licenses/by-nc/4.0/",
        summary: "Non-commercial use only. Commercial use requires an agreement with BRIA.",
        requires_acceptance: true,
    }),
    extra_args: &[],
};

/// FLUX.1 schnell: 4-step 12B text-to-image. Uses T5-XXL + CLIP-L instead of
/// an LLM encoder, and has no reference-image support.
pub const FLUX1_SCHNELL: ModelPreset = ModelPreset {
    id: "flux1-schnell",
    label: "FLUX.1 schnell",
    description: "4-step · 12B · text-to-image only",
    files: &[
        ModelFileSpec {
            role: ModelRole::Diffusion,
            rel_path: "diffusion/flux1-schnell-Q4_K_S.gguf",
            url: "https://huggingface.co/unsloth/FLUX.1-schnell-GGUF/resolve/main/flux1-schnell-Q4_K_S.gguf",
            approx_bytes: 6_780_000_000,
        },
        ModelFileSpec {
            role: ModelRole::Vae,
            rel_path: "vae/flux1-vae-bf16.safetensors",
            url: "https://huggingface.co/meituan-longcat/LongCat-Image-Edit-Turbo/resolve/main/vae/diffusion_pytorch_model.safetensors",
            approx_bytes: 167_666_902,
        },
        ModelFileSpec {
            role: ModelRole::TextEncoder,
            rel_path: "text_encoders/t5-v1_1-xxl-encoder-Q4_K_M.gguf",
            url: "https://huggingface.co/city96/t5-v1_1-xxl-encoder-gguf/resolve/main/t5-v1_1-xxl-encoder-Q4_K_M.gguf",
            approx_bytes: 2_900_000_000,
        },
        ModelFileSpec {
            role: ModelRole::ClipEncoder,
            rel_path: "text_encoders/clip_l.safetensors",
            url: "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/main/clip_l.safetensors",
            approx_bytes: 246_000_000,
        },
        ModelFileSpec {
            role: ModelRole::Preview,
            rel_path: "vae/taef1.safetensors",
            url: "https://huggingface.co/madebyollin/taef1/resolve/main/diffusion_pytorch_model.safetensors",
            approx_bytes: 9_800_000,
        },
    ],
    steps: 4,
    cfg: 1.0,
    sampler: Sampler::Euler,
    default_width: 768,
    default_height: 768,
    supports_edit: false,
    max_references: 0,
    license: Some(ModelLicense {
        name: "Apache-2.0",
        holder: "Black Forest Labs",
        url: "https://huggingface.co/black-forest-labs/FLUX.1-schnell",
        summary: "Apache-2.0; commercial use allowed.",
        requires_acceptance: false,
    }),
    extra_args: &["--clip-on-cpu"],
};

pub const PRESETS: &[ModelPreset] = &[
    FLUX1_SCHNELL,
    FLUX2_KLEIN_4B,
    FLUX2_KLEIN_9B,
    FLUX3_KLEIN,
    FIBO2_EDIT_TURBO,
];

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
            assert!(
                preset.file(ModelRole::TextEncoder).is_some(),
                "{}",
                preset.id
            );
            assert!(preset.total_bytes() > 0);
        }
    }

    #[test]
    fn preview_weights_are_optional() {
        let dir = std::env::temp_dir().join(format!("flux-preset-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let preset = &FLUX2_KLEIN_4B;
        assert!(preset.file(ModelRole::Preview).is_some());
        assert!(!preset.is_installed(&dir));
        assert!(preset.preview_weights(&dir).is_none());

        // Pretend the required weights are present but the preview weights are not.
        for file in preset
            .files
            .iter()
            .filter(|file| file.role != ModelRole::Preview)
        {
            let path = dir.join(file.rel_path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
        }
        assert!(preset.is_installed(&dir));
        assert_eq!(preset.missing(&dir).len(), 1);
        assert!(preset.missing_required(&dir).is_empty());

        let preview = preset.file(ModelRole::Preview).unwrap();
        let path = dir.join(preview.rel_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();
        assert_eq!(preset.preview_weights(&dir), Some(path));
        assert!(preset.missing(&dir).is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_llm_presets_expose_a_prompt_encoder() {
        assert!(FLUX2_KLEIN_4B.llm_encoder().is_some());
        assert!(FLUX2_KLEIN_9B.llm_encoder().is_some());
        assert!(FLUX3_KLEIN.llm_encoder().is_some());
        assert!(FIBO2_EDIT_TURBO.llm_encoder().is_some());
        assert!(FLUX1_SCHNELL.llm_encoder().is_none());
        assert!(FLUX1_SCHNELL.file(ModelRole::ClipEncoder).is_some());
    }

    #[test]
    fn only_restricted_licenses_require_acceptance() {
        let fibo = FIBO2_EDIT_TURBO.license.expect("fibo has a license");
        assert!(fibo.requires_acceptance);
        assert_eq!(fibo.name, "CC BY-NC 4.0");
        let klein9b = FLUX2_KLEIN_9B.license.expect("klein 9B has a license");
        assert!(klein9b.requires_acceptance);
        assert_eq!(klein9b.name, "FLUX Non-Commercial License");
        assert!(
            !FLUX2_KLEIN_4B
                .license
                .expect("klein has a license")
                .requires_acceptance
        );
        assert!(
            !FLUX1_SCHNELL
                .license
                .expect("schnell has a license")
                .requires_acceptance
        );
    }

    #[test]
    fn edit_presets_declare_a_reference_limit() {
        for preset in PRESETS {
            if preset.supports_edit {
                assert!(
                    preset.max_references > 0,
                    "{} supports editing but rejects references",
                    preset.id
                );
            } else {
                assert_eq!(
                    preset.max_references, 0,
                    "{} cannot edit but declares a reference limit",
                    preset.id
                );
            }
        }
        assert_eq!(FLUX3_KLEIN.max_references, 10);
        assert_eq!(FLUX2_KLEIN_9B.max_references, 10);
        assert_eq!(FLUX1_SCHNELL.max_references, 0);
    }

    #[test]
    fn resolves_paths_under_models_dir() {
        let preset = &FLUX2_KLEIN_4B;
        let path = preset
            .path(ModelRole::Diffusion, Path::new("/models"))
            .unwrap();
        assert_eq!(
            path,
            PathBuf::from("/models/diffusion/flux-2-klein-4b-Q4_K_M.gguf")
        );
    }
}
