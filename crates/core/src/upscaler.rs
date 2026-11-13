use std::path::{Path, PathBuf};

use crate::preset::{ModelFileSpec, ModelRole};

/// sd.cpp only exposes four-times ESRGAN/MoSR models, so every upscale is a
/// native 4× pass. This is the server's hard output limit.
pub const MAX_UPSCALE_SIDE: u32 = 8192;
/// Largest input whose native 4× output still fits [`MAX_UPSCALE_SIDE`].
pub const MAX_INPUT_SIDE: u32 = MAX_UPSCALE_SIDE / NATIVE_SCALE;
/// Native scale factor of every supported upscaler.
pub const NATIVE_SCALE: u32 = 4;
/// Tile size that fits comfortably in 8 GB alongside the desktop.
pub const DEFAULT_TILE_SIZE: u32 = 256;
/// Retry size when the default tile does not fit in VRAM.
pub const FALLBACK_TILE_SIZE: u32 = 128;

/// A downloadable upscaler weight and everything the UI needs to describe it.
#[derive(Debug, Clone, Copy)]
pub struct UpscalerPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// File stem used by `sd-server` in upscale requests.
    pub server_name: &'static str,
    pub file: ModelFileSpec,
    pub scale: u32,
    /// Measured on the reference 8 GB GPU; used only for UI estimates.
    pub seconds_per_megapixel: f32,
    pub license: &'static str,
    pub license_url: &'static str,
    pub credits: &'static str,
}

impl UpscalerPreset {
    pub fn path(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.file.rel_path)
    }

    pub fn is_installed(&self, models_dir: &Path) -> bool {
        self.path(models_dir).exists()
    }

    pub fn approx_bytes(&self) -> u64 {
        self.file.approx_bytes
    }

    /// Rough runtime for an input of `width × height`, in seconds.
    pub fn estimated_seconds(&self, width: u32, height: u32) -> u32 {
        let megapixels = width as f32 * height as f32 / 1_000_000.0;
        (megapixels * self.seconds_per_megapixel).round().max(1.0) as u32
    }
}

/// Output dimensions for an upscale, or `None` when the native 4× pass would
/// exceed the server's 8192 px limit.
///
/// A 2× output is produced by downscaling a native 4× pass, so the 4× output
/// must fit even when the user asks for 2×.
pub fn output_size(width: u32, height: u32, factor: u32) -> Option<(u32, u32)> {
    if !matches!(factor, 2 | 4) {
        return None;
    }
    let (native_width, native_height) = scaled(width, height, NATIVE_SCALE)?;
    if native_width > MAX_UPSCALE_SIDE || native_height > MAX_UPSCALE_SIDE {
        return None;
    }
    scaled(width, height, factor)
}

fn scaled(width: u32, height: u32, factor: u32) -> Option<(u32, u32)> {
    Some((width.checked_mul(factor)?, height.checked_mul(factor)?))
}

pub const MOSR_4X: UpscalerPreset = UpscalerPreset {
    id: "mosr-4x",
    label: "Balanced 4×",
    description: "Fast and natural · best default for generated images",
    server_name: "4xNomos2_hq_mosr",
    file: ModelFileSpec {
        role: ModelRole::Upscaler,
        rel_path: "upscalers/4xNomos2_hq_mosr.pth",
        url: "https://huggingface.co/Phips/4xNomos2_hq_mosr/resolve/main/4xNomos2_hq_mosr.pth",
        approx_bytes: 17_213_494,
    },
    scale: NATIVE_SCALE,
    seconds_per_megapixel: 42.0,
    license: "CC-BY-4.0",
    license_url: "https://creativecommons.org/licenses/by/4.0/",
    credits: "Philip Hofmann (Phips)",
};

pub const REALESRGAN_X4PLUS: UpscalerPreset = UpscalerPreset {
    id: "realesrgan-x4plus",
    label: "Photo 4× (max detail)",
    description: "Sharpest photographic detail · slow",
    server_name: "RealESRGAN_x4plus",
    file: ModelFileSpec {
        role: ModelRole::Upscaler,
        rel_path: "upscalers/RealESRGAN_x4plus.pth",
        url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.1.0/RealESRGAN_x4plus.pth",
        approx_bytes: 67_040_989,
    },
    scale: NATIVE_SCALE,
    seconds_per_megapixel: 259.0,
    license: "BSD-3-Clause",
    license_url: "https://github.com/xinntao/Real-ESRGAN/blob/master/LICENSE",
    credits: "Xintao Wang and the Real-ESRGAN contributors",
};

pub const REALESRGAN_ANIME_4X: UpscalerPreset = UpscalerPreset {
    id: "realesrgan-x4plus-anime",
    label: "Anime 4×",
    description: "Clean line art and flat colours · anime and illustration",
    server_name: "RealESRGAN_x4plus_anime_6B",
    file: ModelFileSpec {
        role: ModelRole::Upscaler,
        rel_path: "upscalers/RealESRGAN_x4plus_anime_6B.pth",
        url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.2.4/RealESRGAN_x4plus_anime_6B.pth",
        approx_bytes: 17_938_799,
    },
    scale: NATIVE_SCALE,
    seconds_per_megapixel: 84.0,
    license: "BSD-3-Clause",
    license_url: "https://github.com/xinntao/Real-ESRGAN/blob/master/LICENSE",
    credits: "Xintao Wang and the Real-ESRGAN contributors",
};

pub const NOMOS_OTF_4X: UpscalerPreset = UpscalerPreset {
    id: "nomos-otf-4x",
    label: "Compressed 4×",
    description: "Robust to JPEG, noise and blur · slow",
    server_name: "4xNomos2_otf_esrgan",
    file: ModelFileSpec {
        role: ModelRole::Upscaler,
        rel_path: "upscalers/4xNomos2_otf_esrgan.safetensors",
        url: "https://huggingface.co/Phips/4xNomos2_otf_esrgan/resolve/main/4xNomos2_otf_esrgan.safetensors",
        approx_bytes: 33_467_822,
    },
    scale: NATIVE_SCALE,
    seconds_per_megapixel: 250.0,
    license: "CC-BY-4.0",
    license_url: "https://creativecommons.org/licenses/by/4.0/",
    credits: "Philip Hofmann (Phips)",
};

pub const MOSR_FAITHFUL_4X: UpscalerPreset = UpscalerPreset {
    id: "mosr-4x-faithful",
    label: "Faithful 4× (soft)",
    description: "Preserves structure instead of inventing detail · fast",
    server_name: "4xmssim_mosr_pretrain",
    file: ModelFileSpec {
        role: ModelRole::Upscaler,
        rel_path: "upscalers/4xmssim_mosr_pretrain.pth",
        url: "https://huggingface.co/Phips/4xmssim_mosr_pretrain/resolve/main/4xmssim_mosr_pretrain.pth",
        approx_bytes: 17_213_494,
    },
    scale: NATIVE_SCALE,
    seconds_per_megapixel: 42.0,
    license: "CC-BY-4.0",
    license_url: "https://creativecommons.org/licenses/by/4.0/",
    credits: "Philip Hofmann (Phips)",
};

pub const UPSCALERS: &[UpscalerPreset] = &[
    MOSR_4X,
    REALESRGAN_X4PLUS,
    REALESRGAN_ANIME_4X,
    NOMOS_OTF_4X,
    MOSR_FAITHFUL_4X,
];

pub fn upscaler(id: &str) -> Option<&'static UpscalerPreset> {
    UPSCALERS.iter().find(|preset| preset.id == id)
}

pub fn default_upscaler() -> &'static UpscalerPreset {
    &MOSR_4X
}

#[derive(Debug, Clone)]
pub struct UpscaleRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub upscaler_id: String,
    /// Requested output scale: 2 or 4.
    pub factor: u32,
    pub tile_size: u32,
}

#[derive(Debug, Clone)]
pub struct UpscaledImage {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_server_names_are_unique() {
        for (index, preset) in UPSCALERS.iter().enumerate() {
            assert!(
                UPSCALERS[index + 1..]
                    .iter()
                    .all(|other| other.id != preset.id),
                "duplicate upscaler id {}",
                preset.id
            );
            assert!(
                UPSCALERS[index + 1..]
                    .iter()
                    .all(|other| other.server_name != preset.server_name),
                "duplicate server name {}",
                preset.server_name
            );
        }
    }

    #[test]
    fn files_live_in_the_upscalers_directory() {
        for preset in UPSCALERS {
            assert_eq!(preset.file.role, ModelRole::Upscaler);
            assert!(preset.file.rel_path.starts_with("upscalers/"));
            assert!(preset.approx_bytes() > 0);
            assert_eq!(preset.scale, NATIVE_SCALE);
        }
    }

    #[test]
    fn default_is_registered() {
        assert_eq!(default_upscaler().id, MOSR_4X.id);
    }

    #[test]
    fn converts_ids_to_paths() {
        let path = MOSR_4X.path(Path::new("/models"));
        assert_eq!(
            path,
            PathBuf::from("/models/upscalers/4xNomos2_hq_mosr.pth")
        );
    }

    #[test]
    fn computes_2x_and_4x_sizes() {
        assert_eq!(output_size(1024, 768, 4), Some((4096, 3072)));
        assert_eq!(output_size(1024, 768, 2), Some((2048, 1536)));
    }

    #[test]
    fn rejects_native_outputs_over_the_cap() {
        assert_eq!(output_size(2048, 2048, 4), Some((8192, 8192)));
        assert_eq!(output_size(2049, 1024, 4), None);
        // 2x would fit, but the native 4x pass would not.
        assert_eq!(output_size(4096, 1024, 2), None);
    }

    #[test]
    fn rejects_unknown_factors() {
        assert_eq!(output_size(512, 512, 1), None);
        assert_eq!(output_size(512, 512, 8), None);
        assert_eq!(output_size(512, 512, 3), None);
    }

    #[test]
    fn estimates_scale_with_pixels() {
        assert!(MOSR_4X.estimated_seconds(1024, 1024) >= 40);
        assert!(
            REALESRGAN_X4PLUS.estimated_seconds(1024, 1024) > MOSR_4X.estimated_seconds(1024, 1024)
        );
    }
}
