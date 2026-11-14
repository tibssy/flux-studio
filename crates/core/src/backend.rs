use std::path::PathBuf;

use anyhow::{Result, bail};
use async_channel::Sender;

use crate::request::{GenerateRequest, Progress};
use crate::upscaler::{UpscaleRequest, UpscaledImage};

#[derive(Debug, Clone)]
pub struct BackendInfo {
    pub name: String,
    pub device: String,
    pub version: Option<String>,
}

/// An image-generation backend.
///
/// Implementations are expected to be blocking: callers run `generate` on a
/// worker thread and consume `progress` from the UI thread.
pub trait Backend: Send + Sync + 'static {
    fn info(&self) -> BackendInfo;

    fn generate(&self, request: &GenerateRequest, progress: Sender<Progress>) -> Result<PathBuf>;

    /// Ask the running generation to stop. Must be safe to call when idle.
    fn cancel(&self) -> Result<()>;

    /// Upscale one image with a registered upscaler preset.
    ///
    /// Backends that cannot upscale return an error; the default keeps the
    /// trait usable for generation-only implementations.
    fn upscale(&self, _request: &UpscaleRequest) -> Result<UpscaledImage> {
        bail!("upscaling is not supported by this backend")
    }

    /// Ask the running upscale to stop. Must be safe to call when idle.
    fn cancel_upscale(&self) -> Result<()> {
        Ok(())
    }

    /// Release cached model weights, if the backend keeps any.
    ///
    /// The default is a no-op for backends that load models per request.
    fn unload(&self) {}

    /// Point the backend at another server binary and/or `--backend` device.
    ///
    /// The next generation restarts the server with the new settings.
    fn set_server(&self, _cli: PathBuf, _backend: Option<String>) {}
}
