//! Resumable model downloader driven by the preset registry in `flux-core`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use async_channel::Sender;
use flux_core::preset::{ModelFileSpec, ModelPreset};
use flux_core::upscaler::UpscalerPreset;
use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::RANGE;

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub file: &'static str,
    pub downloaded: u64,
    pub total: u64,
    pub file_index: usize,
    pub file_count: usize,
}

/// Download one model file, resuming a partial `.part` file when present.
pub async fn download(
    file: &ModelFileSpec,
    models_dir: &Path,
    file_index: usize,
    file_count: usize,
    progress: Sender<DownloadProgress>,
) -> Result<PathBuf> {
    let dest = models_dir.join(file.rel_path);
    if dest.exists() {
        return Ok(dest);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let filename = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.rel_path.to_string());
    let part = dest.with_file_name(format!("{filename}.part"));
    let existing = std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);

    let client = reqwest::Client::builder()
        .user_agent(concat!("flux-studio/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut request = client.get(file.url);
    if existing > 0 {
        request = request.header(RANGE, format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("GET {}", file.url))?;
    let status = response.status();

    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        std::fs::rename(&part, &dest)?;
        return Ok(dest);
    }
    if !status.is_success() {
        bail!("download failed: HTTP {status} for {}", file.url);
    }

    let resumed = status == StatusCode::PARTIAL_CONTENT;
    let total = response
        .content_length()
        .unwrap_or(file.approx_bytes)
        .saturating_add(if resumed { existing } else { 0 });

    let mut out = if resumed {
        OpenOptions::new().append(true).open(&part)?
    } else {
        File::create(&part)?
    };
    let mut downloaded = if resumed { existing } else { 0 };

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        out.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        let _ = progress
            .send(DownloadProgress {
                file: file.rel_path,
                downloaded,
                total,
                file_index,
                file_count,
            })
            .await;
    }
    out.flush()?;
    drop(out);

    std::fs::rename(&part, &dest)?;
    tracing::info!(file = file.rel_path, bytes = downloaded, "model downloaded");
    Ok(dest)
}

/// Download every missing file for one preset, sequentially.
pub async fn download_preset(
    preset: &ModelPreset,
    models_dir: &Path,
    progress: Sender<DownloadProgress>,
) -> Result<Vec<PathBuf>> {
    let missing: Vec<_> = preset
        .files
        .iter()
        .filter(|file| !models_dir.join(file.rel_path).exists())
        .collect();
    let file_count = missing.len();
    let mut paths = Vec::with_capacity(file_count);

    for (index, file) in missing.into_iter().enumerate() {
        paths.push(download(file, models_dir, index, file_count, progress.clone()).await?);
    }
    Ok(paths)
}

/// Download one upscaler weight, resuming a partial `.part` file when present.
pub async fn download_upscaler(
    preset: &UpscalerPreset,
    models_dir: &Path,
    progress: Sender<DownloadProgress>,
) -> Result<PathBuf> {
    download(&preset.file, models_dir, 0, 1, progress).await
}

#[cfg(test)]
mod tests {
    use flux_core::preset::{FLUX2_KLEIN_4B, ModelRole};

    #[test]
    fn missing_reports_absent_files() {
        let dir = std::env::temp_dir().join(format!("flux-models-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(FLUX2_KLEIN_4B.missing(&dir).len(), FLUX2_KLEIN_4B.files.len());

        let vae = FLUX2_KLEIN_4B
            .path(ModelRole::Vae, &dir)
            .expect("vae path");
        std::fs::create_dir_all(vae.parent().unwrap()).unwrap();
        std::fs::write(&vae, b"x").unwrap();
        assert_eq!(
            FLUX2_KLEIN_4B.missing(&dir).len(),
            FLUX2_KLEIN_4B.files.len() - 1
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
