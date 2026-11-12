use std::path::PathBuf;

use anyhow::Result;
use async_channel::Sender;

use crate::request::{GenerateRequest, Progress};

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

    /// Release cached model weights, if the backend keeps any.
    ///
    /// The default is a no-op for backends that load models per request.
    fn unload(&self) {}
}
