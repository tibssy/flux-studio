use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{paths, size};

/// A prompt saved from an edit result and shown in the effects menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomEffect {
    pub name: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Optional override for the `sd-cli` binary.
    pub sd_cli: Option<PathBuf>,
    pub models_dir: PathBuf,
    pub output_dir: PathBuf,
    pub default_width: u32,
    pub default_height: u32,
    /// Target megapixels selected in the sidebar. Optional for older configs.
    pub default_mp: Option<f64>,
    /// Aspect ratio selected in the sidebar. Optional for older configs.
    pub default_ratio_width: Option<u32>,
    pub default_ratio_height: Option<u32>,
    pub offload: bool,
    /// Id of the preset selected in the model dropdown.
    pub active_preset: String,
    /// Id of the upscaler selected in the upscale dialog.
    pub upscaler_id: String,
    /// Requested output scale for upscaling (2 or 4).
    pub upscale_factor: u32,
    /// Id of the style selected in the sidebar.
    pub style_id: String,
    /// Whether the left sidebar is shown.
    pub sidebar_visible: bool,
    /// Optional override for the `sd-server` binary.
    pub sd_server: Option<PathBuf>,
    /// Id of the installed backend selected in Preferences.
    pub backend_id: Option<String>,
    /// Explicit `--backend` argument for `sd-server` (cpu, vulkan0, cuda0, ...).
    pub backend: Option<String>,
    /// Optional override for the backend manifest URL.
    pub backend_manifest_url: Option<String>,
    /// Custom `sd-server` selected in Preferences but not necessarily active.
    pub custom_server: Option<PathBuf>,
    /// Optional override for the `llama-cli` binary used to improve prompts.
    pub prompt_helper_cli: Option<PathBuf>,
    /// Optional GGUF used for prompt improvement; defaults to the active
    /// preset's text encoder.
    pub prompt_helper_model: Option<PathBuf>,
    /// Id of the installed llama.cpp backend selected in Preferences.
    pub llama_backend_id: Option<String>,
    /// Optional override for the llama.cpp backend manifest URL.
    pub llama_manifest_url: Option<String>,
    /// Custom `llama-cli` selected in Preferences but not necessarily active.
    pub llama_custom_cli: Option<PathBuf>,
    /// Preset ids whose license the user explicitly accepted in Preferences.
    pub accepted_licenses: Vec<String>,
    /// Show live step previews on the canvas while sampling.
    pub show_previews: bool,
    /// Show a desktop notification when a job finishes while the window is in
    /// the background.
    pub notify_on_finish: bool,
    /// Put finished images on the system clipboard.
    pub copy_result_to_clipboard: bool,
    /// Images queued per Generate press (1–4).
    pub batch_count: u32,
    /// Recent prompts, newest last.
    pub prompt_history: Vec<String>,
    /// Restore the last displayed image and draft prompt on launch.
    pub session_restore: bool,
    pub last_image: Option<PathBuf>,
    pub draft_prompt: String,
    /// User-saved prompts shown at the bottom of the effects menu.
    pub custom_effects: Vec<CustomEffect>,
}

impl Default for Config {
    fn default() -> Self {
        let preset = crate::preset::default_preset();
        Self {
            sd_cli: None,
            models_dir: paths::default_models_dir(),
            output_dir: paths::default_output_dir(),
            default_width: preset.default_width,
            default_height: preset.default_height,
            default_mp: None,
            default_ratio_width: None,
            default_ratio_height: None,
            offload: true,
            active_preset: preset.id.to_string(),
            upscaler_id: crate::upscaler::default_upscaler().id.to_string(),
            upscale_factor: crate::upscaler::NATIVE_SCALE,
            style_id: crate::style::default_style().id.to_string(),
            sidebar_visible: true,
            sd_server: None,
            backend_id: None,
            backend: None,
            backend_manifest_url: None,
            custom_server: None,
            prompt_helper_cli: None,
            prompt_helper_model: None,
            llama_backend_id: None,
            llama_manifest_url: None,
            llama_custom_cli: None,
            accepted_licenses: Vec::new(),
            show_previews: true,
            notify_on_finish: true,
            copy_result_to_clipboard: false,
            batch_count: 1,
            prompt_history: Vec::new(),
            session_restore: true,
            last_image: None,
            draft_prompt: String::new(),
            custom_effects: Vec::new(),
        }
    }
}

impl Config {
    /// Target megapixels, deriving it from the legacy width/height when the
    /// newer field is missing.
    pub fn mp(&self) -> f64 {
        let derived = size::mp_for_size(self.default_width.max(1), self.default_height.max(1));
        let mp = self
            .default_mp
            .filter(|mp| mp.is_finite())
            .unwrap_or(derived);
        mp.clamp(size::MP_MIN, size::MP_MAX)
    }

    /// Selected aspect ratio, reduced to lowest terms.
    pub fn ratio(&self) -> (u32, u32) {
        match (self.default_ratio_width, self.default_ratio_height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => {
                size::reduce_ratio(width, height)
            }
            _ => size::reduce_ratio(self.default_width.max(1), self.default_height.max(1)),
        }
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(paths::config_file()) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = paths::config_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_configs_get_new_defaults() {
        let config: Config = toml::from_str(
            r#"
            models_dir = "/models"
            output_dir = "/out"
            default_width = 768
            default_height = 768
            offload = true
            active_preset = "flux2-klein-4b"
            sidebar_auto_hide = true
            "#,
        )
        .unwrap();
        assert_eq!(config.style_id, "none");
        assert!(config.sidebar_visible);
        assert!(config.sd_server.is_none());
        assert!(config.backend_id.is_none());
        assert!(config.backend.is_none());
        assert!(config.backend_manifest_url.is_none());
        assert!(config.custom_server.is_none());
        assert!(config.prompt_helper_cli.is_none());
        assert!(config.prompt_helper_model.is_none());
        assert!(config.llama_backend_id.is_none());
        assert!(config.llama_manifest_url.is_none());
        assert!(config.llama_custom_cli.is_none());
        assert!(config.show_previews);
        assert!(config.custom_effects.is_empty());
        assert_eq!(config.batch_count, 1);
        assert!(config.prompt_history.is_empty());
        assert!(config.session_restore);
        assert!(config.last_image.is_none());
        assert!(config.draft_prompt.is_empty());
        assert_eq!(config.upscaler_id, crate::upscaler::default_upscaler().id);
        assert_eq!(config.upscale_factor, crate::upscaler::NATIVE_SCALE);
        assert!((config.mp() - 0.589824).abs() < 1e-6);
        assert_eq!(config.ratio(), (1, 1));
    }

    #[test]
    fn reads_explicit_mp_and_ratio() {
        let config: Config = toml::from_str(
            r#"
            models_dir = "/models"
            output_dir = "/out"
            default_width = 1344
            default_height = 752
            default_mp = 1.0
            default_ratio_width = 16
            default_ratio_height = 9
            "#,
        )
        .unwrap();
        assert_eq!(config.mp(), 1.0);
        assert_eq!(config.ratio(), (16, 9));
    }

    #[test]
    fn derives_custom_ratio_from_legacy_size() {
        let config: Config = toml::from_str(
            r#"
            models_dir = "/models"
            output_dir = "/out"
            default_width = 1216
            default_height = 832
            "#,
        )
        .unwrap();
        assert_eq!(config.ratio(), (19, 13));
    }

    #[test]
    fn round_trips_custom_effects() {
        let mut config = Config::default();
        config.custom_effects.push(CustomEffect {
            name: "Restore".into(),
            prompt: "repair this photo".into(),
        });
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.custom_effects, config.custom_effects);
    }
}
