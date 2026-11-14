use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Optional override for the `sd-cli` binary.
    pub sd_cli: Option<PathBuf>,
    pub models_dir: PathBuf,
    pub output_dir: PathBuf,
    pub default_width: u32,
    pub default_height: u32,
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
    /// Whether the sidebar hides itself after focus and pointer leave.
    pub sidebar_auto_hide: bool,
    /// Optional override for the `sd-server` binary.
    pub sd_server: Option<PathBuf>,
    /// Id of the installed backend selected in Preferences.
    pub backend_id: Option<String>,
    /// Explicit `--backend` argument for `sd-server` (cpu, vulkan0, cuda0, ...).
    pub backend: Option<String>,
    /// Optional override for the backend manifest URL.
    pub backend_manifest_url: Option<String>,
    /// Optional override for the `llama-cli` binary used to improve prompts.
    pub prompt_helper_cli: Option<PathBuf>,
    /// Optional GGUF used for prompt improvement; defaults to the active
    /// preset's text encoder.
    pub prompt_helper_model: Option<PathBuf>,
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
            offload: true,
            active_preset: preset.id.to_string(),
            upscaler_id: crate::upscaler::default_upscaler().id.to_string(),
            upscale_factor: crate::upscaler::NATIVE_SCALE,
            style_id: crate::style::default_style().id.to_string(),
            sidebar_visible: true,
            sidebar_auto_hide: true,
            sd_server: None,
            backend_id: None,
            backend: None,
            backend_manifest_url: None,
            prompt_helper_cli: None,
            prompt_helper_model: None,
        }
    }
}

impl Config {
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
            "#,
        )
        .unwrap();
        assert_eq!(config.style_id, "none");
        assert!(config.sidebar_visible);
        assert!(config.sidebar_auto_hide);
        assert!(config.sd_server.is_none());
        assert!(config.backend_id.is_none());
        assert!(config.backend.is_none());
        assert!(config.backend_manifest_url.is_none());
        assert!(config.prompt_helper_cli.is_none());
        assert!(config.prompt_helper_model.is_none());
        assert_eq!(config.upscaler_id, crate::upscaler::default_upscaler().id);
        assert_eq!(config.upscale_factor, crate::upscaler::NATIVE_SCALE);
    }
}
