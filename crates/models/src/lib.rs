//! Resumable model downloader driven by the preset registry in `imagiflume-core`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use async_channel::Sender;
use futures_util::StreamExt;
use imagiflume_core::preset::{ModelFileSpec, ModelPreset};
use imagiflume_core::runtime::{ArchiveFormat, BackendEntry, InstalledBackend};
use imagiflume_core::upscaler::UpscalerPreset;
use imagiflume_core::vision::VisionHelper;
use reqwest::StatusCode;
use reqwest::header::RANGE;
use sha2::{Digest, Sha256};

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
        .user_agent(concat!("imagiflume/", env!("CARGO_PKG_VERSION")))
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

/// Download the optional vision helper model and its projector.
pub async fn download_vision_helper(
    helper: &VisionHelper,
    models_dir: &Path,
    progress: Sender<DownloadProgress>,
) -> Result<Vec<PathBuf>> {
    let missing = helper.missing(models_dir);
    let file_count = missing.len();
    let mut paths = Vec::with_capacity(file_count);

    for (index, file) in missing.into_iter().enumerate() {
        paths.push(download(file, models_dir, index, file_count, progress.clone()).await?);
    }
    Ok(paths)
}

#[derive(Debug, Clone)]
pub struct BackendProgress {
    pub downloaded: u64,
    pub total: u64,
}

/// Download, verify and install one prebuilt backend.
///
/// The archive is extracted into `<backends_dir>/<id>` and the bundled
/// `sd-server` is executed once to catch missing shared libraries before the
/// backend is offered as active.
pub async fn download_backend(
    entry: &BackendEntry,
    backends_dir: &Path,
    progress: Sender<BackendProgress>,
) -> Result<PathBuf> {
    if !entry.is_available() {
        bail!("{} is not published yet", entry.label);
    }
    if entry.is_installed(backends_dir) {
        return Ok(entry.binary_path(backends_dir));
    }

    let downloads = backends_dir.join(".downloads");
    std::fs::create_dir_all(&downloads)?;
    let extension = match entry.archive_format() {
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::TarGz => "tar.gz",
    };
    let archive = downloads.join(format!("{}.{extension}", entry.id));
    let part = downloads.join(format!("{}.{extension}.part", entry.id));

    if archive.exists() && sha256_file(&archive)? != entry.sha256 {
        std::fs::remove_file(&archive)?;
    }
    if !archive.exists() {
        download_backend_archive(entry, &part, &archive, progress).await?;
    }

    let actual = sha256_file(&archive)?;
    if actual != entry.sha256 {
        bail!(
            "checksum mismatch for {}: expected {}, got {}",
            entry.label,
            entry.sha256,
            actual
        );
    }

    let staging =
        backends_dir
            .join(".staging")
            .join(format!("{}-{}", entry.id, std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    let result = extract_backend(&archive, entry, &staging);
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_file(&archive);
        return Err(error);
    }

    let binary = staging.join(entry.binary_name());
    if !binary.is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        bail!("{} does not contain {}", entry.label, entry.binary_name());
    }
    let output = std::process::Command::new(&binary)
        .arg("--version")
        .output()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        let _ = std::fs::remove_dir_all(&staging);
        bail!(
            "{} cannot run on this system: {}",
            entry.label,
            detail.lines().next().unwrap_or("unknown error")
        );
    }

    let installed = InstalledBackend {
        id: entry.id.clone(),
        version: entry.version.clone(),
        kind: entry.kind,
        size: entry.size,
        installed_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0),
    };
    std::fs::write(
        staging.join("installed.json"),
        serde_json::to_vec_pretty(&installed)?,
    )?;

    let destination = entry.install_dir(backends_dir);
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(&staging, &destination)?;
    let _ = std::fs::remove_file(&archive);
    Ok(destination.join(entry.binary_name()))
}

async fn download_backend_archive(
    entry: &BackendEntry,
    part: &Path,
    archive: &Path,
    progress: Sender<BackendProgress>,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("imagiflume/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client
        .get(&entry.url)
        .send()
        .await
        .with_context(|| format!("GET {}", entry.url))?;
    let status = response.status();
    if !status.is_success() {
        bail!("download failed: HTTP {status} for {}", entry.url);
    }
    let total = response.content_length().unwrap_or(entry.size);
    let mut out = File::create(part)?;
    let mut downloaded = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        out.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        let _ = progress.send(BackendProgress { downloaded, total }).await;
    }
    out.flush()?;
    drop(out);
    std::fs::rename(part, archive)?;
    Ok(())
}

fn extract_backend(archive: &Path, entry: &BackendEntry, destination: &Path) -> Result<()> {
    match entry.archive_format() {
        ArchiveFormat::Zip => extract_backend_zip(archive, entry, destination),
        ArchiveFormat::TarGz => extract_backend_tar_gz(archive, entry, destination),
    }
}

fn extract_backend_zip(archive: &Path, entry: &BackendEntry, destination: &Path) -> Result<()> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let prefix = entry
        .strip_prefix
        .clone()
        .map(PathBuf::from)
        .or_else(|| detect_zip_prefix(&mut zip, entry.binary_name()));

    for index in 0..zip.len() {
        let mut file = zip.by_index(index)?;
        let Some(path) = file.enclosed_name() else {
            bail!("archive contains an unsafe path");
        };
        let relative = match &prefix {
            Some(prefix) => match path.strip_prefix(prefix) {
                Ok(path) => path.to_path_buf(),
                Err(_) => continue,
            },
            None => path,
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(&relative);
        if file.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&target)?;
        std::io::copy(&mut file, &mut out)?;
        out.flush()?;
        drop(out);
        set_permissions(&target, &relative, entry.binary_name())?;
    }
    Ok(())
}

fn detect_zip_prefix(zip: &mut zip::ZipArchive<File>, binary: &str) -> Option<PathBuf> {
    let marker = format!("/{binary}");
    let mut best: Option<PathBuf> = None;
    for index in 0..zip.len() {
        let Ok(file) = zip.by_index(index) else {
            continue;
        };
        let name = file.name();
        if name == binary {
            return None;
        }
        if let Some(prefix) = name.strip_suffix(&marker) {
            let prefix = PathBuf::from(prefix);
            if best
                .as_ref()
                .is_none_or(|current| prefix.as_os_str().len() < current.as_os_str().len())
            {
                best = Some(prefix);
            }
        }
    }
    best
}

fn extract_backend_tar_gz(archive: &Path, entry: &BackendEntry, destination: &Path) -> Result<()> {
    let raw = destination.join(".extract");
    if raw.exists() {
        std::fs::remove_dir_all(&raw)?;
    }
    std::fs::create_dir_all(&raw)?;

    let file = File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    tar.unpack(&raw)
        .with_context(|| format!("extracting {}", archive.display()))?;

    let source = if raw.join(entry.binary_name()).is_file() {
        raw.clone()
    } else {
        find_tar_root(&raw, entry.binary_name())?
    };

    for child in std::fs::read_dir(&source)? {
        let child = child?;
        let target = destination.join(child.file_name());
        if target.exists() {
            std::fs::remove_dir_all(&target).or_else(|_| std::fs::remove_file(&target))?;
        }
        std::fs::rename(child.path(), &target)?;
    }
    std::fs::remove_dir_all(&raw)?;
    Ok(())
}

fn find_tar_root(raw: &Path, binary: &str) -> Result<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(raw)? {
        let path = entry?.path();
        if path.is_dir() && path.join(binary).is_file() {
            found.push(path);
        }
    }
    match found.len() {
        0 => bail!("archive does not contain {binary}"),
        1 => Ok(found.remove(0)),
        _ => bail!("archive contains multiple {binary} candidates"),
    }
}

#[cfg(unix)]
fn set_permissions(path: &Path, relative: &Path, binary: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let executable = name == binary
        || name == "sd-cli"
        || name == "llama-server"
        || (name.starts_with("llama-") && !name.contains(".so"))
        || name.ends_with(".sh");
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _relative: &Path, _binary: &str) -> Result<()> {
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Fetch and parse the backend manifest, skipping individual corrupt entries.
pub async fn fetch_manifest(
    url: &str,
) -> Result<(imagiflume_core::runtime::BackendManifest, Vec<String>)> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("imagiflume/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !response.status().is_success() {
        bail!(
            "manifest request failed: HTTP {} for {url}",
            response.status()
        );
    }
    let text = response.text().await?;
    imagiflume_core::runtime::BackendManifest::parse_lenient(&text)
}

/// Build a manifest from the latest upstream GitHub release.
///
/// Upstream publishes Linux CPU, Vulkan and ROCm archives; CUDA remains
/// curated locally. Assets without a SHA-256 digest are skipped.
pub async fn fetch_upstream_latest()
-> Result<(imagiflume_core::runtime::BackendManifest, Vec<String>)> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("imagiflume/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client
        .get("https://api.github.com/repos/leejet/stable-diffusion.cpp/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .context("GET latest upstream release")?;
    if !response.status().is_success() {
        bail!("GitHub release request failed: HTTP {}", response.status());
    }
    let value: serde_json::Value = response.json().await?;
    let tag = value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let Some(assets) = value.get("assets").and_then(serde_json::Value::as_array) else {
        bail!("GitHub release has no assets");
    };

    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for asset in assets {
        let Some(name) = asset.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !name.contains("Linux") || !name.ends_with(".zip") {
            continue;
        }
        let kind = if name.contains("-vulkan") {
            imagiflume_core::runtime::BackendKind::Vulkan
        } else if name.contains("-rocm-") {
            imagiflume_core::runtime::BackendKind::Rocm
        } else if name.contains("-bin-Linux") {
            imagiflume_core::runtime::BackendKind::Cpu
        } else {
            continue;
        };
        let Some(url) = asset
            .get("browser_download_url")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(size) = asset.get("size").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let sha256 = asset
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .map(str::to_ascii_lowercase);
        let Some(sha256) = sha256
            .filter(|sha| sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        else {
            errors.push(format!("{name}: missing SHA-256 digest"));
            continue;
        };

        let (id, label, backend, requires) = match kind {
            imagiflume_core::runtime::BackendKind::Cpu => (
                "upstream-cpu",
                "CPU (upstream)",
                Some("cpu".to_string()),
                Vec::new(),
            ),
            imagiflume_core::runtime::BackendKind::Vulkan => (
                "upstream-vulkan",
                "Vulkan (upstream)",
                Some("vulkan0".to_string()),
                vec!["libvulkan.so.1".to_string()],
            ),
            imagiflume_core::runtime::BackendKind::Rocm => {
                ("upstream-rocm", "ROCm (upstream)", None, Vec::new())
            }
            imagiflume_core::runtime::BackendKind::Cuda => continue,
        };
        let min_glibc = if name.contains("Ubuntu-26.04") {
            Some("2.38".to_string())
        } else if name.contains("Ubuntu-24.04") {
            Some("2.39".to_string())
        } else {
            None
        };
        let rocm_major = name
            .split("-rocm-")
            .nth(1)
            .and_then(|version| version.split('.').next())
            .and_then(|major| major.parse::<u32>().ok());

        entries.push(imagiflume_core::runtime::BackendEntry {
            id: id.to_string(),
            label: label.to_string(),
            kind,
            version: tag.clone(),
            url: url.to_string(),
            sha256,
            size,
            backend,
            cuda_major: None,
            rocm_major,
            archs: Vec::new(),
            requires,
            min_glibc,
            strip_prefix: if kind == imagiflume_core::runtime::BackendKind::Rocm {
                Some("build/bin".into())
            } else {
                None
            },
            notes: Some(format!("Latest upstream release {tag}")),
            binary: None,
            archive: None,
        });
    }

    if entries.is_empty() {
        bail!("latest upstream release has no Linux CPU/Vulkan/ROCm assets");
    }
    Ok((
        imagiflume_core::runtime::BackendManifest {
            schema_version: 1,
            generated_at: tag,
            backends: entries,
        },
        errors,
    ))
}

/// Build a manifest from the latest llama.cpp release with Linux x64 assets.
///
/// `/releases/latest` can point at a release without assets (for example
/// `v1.0.0`), so the releases list is scanned for the newest usable build.
pub async fn fetch_llama_upstream_latest()
-> Result<(imagiflume_core::runtime::BackendManifest, Vec<String>)> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("imagiflume/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=20")
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .context("GET llama.cpp releases")?;
    if !response.status().is_success() {
        bail!("GitHub release request failed: HTTP {}", response.status());
    }
    let releases: Vec<serde_json::Value> = response.json().await?;
    let (tag, entries, errors) = select_llama_release(&releases)?;
    Ok((
        imagiflume_core::runtime::BackendManifest {
            schema_version: 1,
            generated_at: tag,
            backends: entries,
        },
        errors,
    ))
}

fn select_llama_release(
    releases: &[serde_json::Value],
) -> Result<(
    String,
    Vec<imagiflume_core::runtime::BackendEntry>,
    Vec<String>,
)> {
    let mut errors = Vec::new();
    for release in releases {
        match llama_entries_from_release(release) {
            Ok((tag, entries, entry_errors)) => {
                errors.extend(entry_errors);
                if !entries.is_empty() {
                    return Ok((tag, entries, errors));
                }
            }
            Err(error) => errors.push(error.to_string()),
        }
    }
    bail!("no llama.cpp release with Linux x64 assets was found")
}

fn llama_entries_from_release(
    release: &serde_json::Value,
) -> Result<(
    String,
    Vec<imagiflume_core::runtime::BackendEntry>,
    Vec<String>,
)> {
    let tag = release
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let Some(assets) = release.get("assets").and_then(serde_json::Value::as_array) else {
        return Ok((
            tag.clone(),
            Vec::new(),
            vec![format!("release {tag} has no assets")],
        ));
    };

    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for asset in assets {
        let Some(name) = asset.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !name.starts_with("llama-")
            || !name.contains("-bin-ubuntu-")
            || !name.ends_with("-x64.tar.gz")
        {
            continue;
        }
        if ["arm64", "s390x", "openvino", "sycl", "zdnn", "snapdragon"]
            .iter()
            .any(|marker| name.contains(marker))
        {
            continue;
        }

        let (kind, cuda_major, rocm_major, id, label, requires) =
            if name.ends_with("-bin-ubuntu-x64.tar.gz") {
                (
                    imagiflume_core::runtime::BackendKind::Cpu,
                    None,
                    None,
                    "llama-cpu".to_string(),
                    "CPU (llama.cpp)".to_string(),
                    Vec::new(),
                )
            } else if name.ends_with("-bin-ubuntu-vulkan-x64.tar.gz") {
                (
                    imagiflume_core::runtime::BackendKind::Vulkan,
                    None,
                    None,
                    "llama-vulkan".to_string(),
                    "Vulkan (llama.cpp)".to_string(),
                    vec!["libvulkan.so.1".to_string()],
                )
            } else if let Some(rest) = name.split("-bin-ubuntu-rocm-").nth(1) {
                let version = rest.strip_suffix("-x64.tar.gz").unwrap_or(rest);
                let Some(major) = version
                    .split('.')
                    .next()
                    .and_then(|major| major.parse::<u32>().ok())
                else {
                    errors.push(format!("{name}: missing ROCm major version"));
                    continue;
                };
                (
                    imagiflume_core::runtime::BackendKind::Rocm,
                    None,
                    Some(major),
                    format!("llama-rocm{major}"),
                    format!("ROCm {major} (llama.cpp)"),
                    vec![format!("libamdhip64.so.{major}")],
                )
            } else if let Some(rest) = name.split("-bin-ubuntu-cuda-").nth(1) {
                let version = rest.strip_suffix("-x64.tar.gz").unwrap_or(rest);
                let Some(major) = version
                    .split('.')
                    .next()
                    .and_then(|major| major.parse::<u32>().ok())
                else {
                    errors.push(format!("{name}: missing CUDA major version"));
                    continue;
                };
                (
                    imagiflume_core::runtime::BackendKind::Cuda,
                    Some(major),
                    None,
                    format!("llama-cuda{major}"),
                    format!("CUDA {major} (llama.cpp)"),
                    vec![format!("libcudart.so.{major}")],
                )
            } else {
                continue;
            };

        if entries
            .iter()
            .any(|entry: &imagiflume_core::runtime::BackendEntry| entry.id == id)
        {
            continue;
        }
        let Some(url) = asset
            .get("browser_download_url")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(size) = asset.get("size").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let sha256 = asset
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .map(str::to_ascii_lowercase);
        let Some(sha256) = sha256
            .filter(|sha| sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        else {
            errors.push(format!("{name}: missing SHA-256 digest"));
            continue;
        };

        entries.push(imagiflume_core::runtime::BackendEntry {
            id,
            label,
            kind,
            version: tag.clone(),
            url: url.to_string(),
            sha256,
            size,
            backend: None,
            cuda_major,
            rocm_major,
            archs: Vec::new(),
            requires,
            min_glibc: None,
            strip_prefix: None,
            notes: Some(format!("Latest llama.cpp release {tag}")),
            binary: Some("llama-cli".to_string()),
            archive: Some(ArchiveFormat::TarGz),
        });
    }

    Ok((tag, entries, errors))
}

/// Result of checking both the curated manifest and the latest upstream release.
pub struct ManifestRefresh {
    pub manifest: imagiflume_core::runtime::BackendManifest,
    pub errors: Vec<String>,
    pub curated_ok: bool,
    pub upstream_ok: bool,
}

/// Merge the curated manifest (if reachable) and the latest upstream release.
///
/// Upstream entries override the curated CPU/Vulkan/ROCm builds; curated CUDA
/// entries are preserved. A failure of either source does not discard the
/// other one.
pub async fn refresh_manifest(
    base: &imagiflume_core::runtime::BackendManifest,
    curated_url: &str,
) -> Result<ManifestRefresh> {
    let curated = fetch_manifest(curated_url).await;
    let upstream = fetch_upstream_latest().await;
    merge_refresh(base, curated, upstream, "manifest", "upstream")
}

/// Merge the curated llama.cpp manifest and the latest upstream release.
pub async fn refresh_llama_manifest(
    base: &imagiflume_core::runtime::BackendManifest,
    curated_url: &str,
) -> Result<ManifestRefresh> {
    let curated = fetch_manifest(curated_url).await;
    let upstream = fetch_llama_upstream_latest().await;
    merge_refresh(
        base,
        curated,
        upstream,
        "llama.cpp manifest",
        "llama.cpp upstream",
    )
}

fn merge_refresh(
    base: &imagiflume_core::runtime::BackendManifest,
    curated: Result<(imagiflume_core::runtime::BackendManifest, Vec<String>)>,
    upstream: Result<(imagiflume_core::runtime::BackendManifest, Vec<String>)>,
    curated_label: &str,
    upstream_label: &str,
) -> Result<ManifestRefresh> {
    let mut merged = base.clone();
    let mut errors = Vec::new();
    let mut failures = Vec::new();
    let mut curated_ok = false;
    let mut upstream_ok = false;

    match curated {
        Ok((manifest, entry_errors)) => {
            merged = imagiflume_core::runtime::BackendManifest::merge(&merged, &manifest);
            errors.extend(entry_errors);
            curated_ok = true;
        }
        Err(error) => failures.push(format!("{curated_label}: {error}")),
    }
    match upstream {
        Ok((manifest, entry_errors)) => {
            merged = imagiflume_core::runtime::BackendManifest::merge(&merged, &manifest);
            errors.extend(entry_errors);
            upstream_ok = true;
        }
        Err(error) => failures.push(format!("{upstream_label}: {error}")),
    }
    if !curated_ok && !upstream_ok {
        bail!("{}", failures.join("; "));
    }
    Ok(ManifestRefresh {
        manifest: merged,
        errors,
        curated_ok,
        upstream_ok,
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        download_backend, extract_backend, llama_entries_from_release, select_llama_release,
    };
    use imagiflume_core::preset::{FLUX2_KLEIN_4B, ModelRole};
    use imagiflume_core::runtime::ArchiveFormat;

    #[test]
    #[ignore = "fetches the latest upstream GitHub release"]
    fn refreshes_from_upstream_when_curated_fails() {
        use crate::refresh_manifest;
        let base = imagiflume_core::runtime::bundled_manifest();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let refreshed = runtime
            .block_on(refresh_manifest(&base, "http://127.0.0.1:9/missing.json"))
            .expect("upstream fallback");
        assert!(!refreshed.curated_ok);
        assert!(refreshed.upstream_ok);
        let cpu = refreshed.manifest.entry("upstream-cpu").expect("cpu entry");
        assert!(cpu.is_available());
        println!("upstream cpu version: {}", cpu.version);
    }

    #[test]
    #[ignore = "fetches the latest upstream GitHub release"]
    fn fetches_latest_upstream_manifest() {
        use crate::fetch_upstream_latest;
        use imagiflume_core::runtime::BackendKind;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (manifest, errors) = runtime.block_on(fetch_upstream_latest()).unwrap();
        assert!(
            manifest
                .backends
                .iter()
                .any(|entry| entry.kind == BackendKind::Cpu && entry.is_available())
        );
        assert!(manifest.entry("upstream-vulkan").is_some());
        println!("tag={} errors={errors:?}", manifest.generated_at);
        for entry in &manifest.backends {
            println!(
                "{} {} {} {}",
                entry.id, entry.version, entry.size, entry.sha256
            );
        }
    }

    #[test]
    #[ignore = "downloads the upstream CPU backend"]
    fn installs_upstream_cpu_backend() {
        use imagiflume_core::runtime::bundled_manifest;
        let dir = std::env::temp_dir().join("imagiflume-backend-install-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = bundled_manifest();
        let entry = manifest.cpu_entry().expect("cpu entry");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let server = runtime
            .block_on(download_backend(entry, &dir, tx))
            .expect("download cpu backend");
        assert!(server.is_file());
        let output = std::process::Command::new(&server)
            .arg("--version")
            .output()
            .unwrap();
        assert!(output.status.success());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "fetches the latest llama.cpp GitHub release"]
    fn fetches_latest_llama_manifest() {
        use crate::fetch_llama_upstream_latest;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (manifest, errors) = runtime.block_on(fetch_llama_upstream_latest()).unwrap();
        assert!(manifest.cpu_entry().is_some());
        assert!(manifest.entry("llama-vulkan").is_some());
        println!("tag={} errors={errors:?}", manifest.generated_at);
        for entry in &manifest.backends {
            println!(
                "{} {} {} {}",
                entry.id, entry.version, entry.size, entry.sha256
            );
        }
    }

    #[test]
    #[ignore = "downloads the upstream llama.cpp CPU backend"]
    fn installs_llama_cpu_backend() {
        use imagiflume_core::runtime::bundled_llama_manifest;
        let dir = std::env::temp_dir().join("flux-llama-backend-install-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = bundled_llama_manifest();
        let entry = manifest.cpu_entry().expect("cpu entry");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let binary = runtime
            .block_on(download_backend(entry, &dir, tx))
            .expect("download llama.cpp cpu backend");
        assert!(binary.is_file());
        let output = std::process::Command::new(&binary)
            .arg("--version")
            .output()
            .unwrap();
        assert!(output.status.success());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_reports_absent_files() {
        let dir =
            std::env::temp_dir().join(format!("imagiflume-models-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(
            FLUX2_KLEIN_4B.missing(&dir).len(),
            FLUX2_KLEIN_4B.files.len()
        );

        let vae = FLUX2_KLEIN_4B.path(ModelRole::Vae, &dir).expect("vae path");
        std::fs::create_dir_all(vae.parent().unwrap()).unwrap();
        std::fs::write(&vae, b"x").unwrap();
        assert_eq!(
            FLUX2_KLEIN_4B.missing(&dir).len(),
            FLUX2_KLEIN_4B.files.len() - 1
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_tar_gz(dir: &std::path::Path, prefix: &str) -> std::path::PathBuf {
        let path = dir.join("backend.tar.gz");
        let file = std::fs::File::create(&path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);

        let binary = b"#!/bin/sh\nexit 0\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(binary.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("{prefix}/llama-cli"), &binary[..])
            .unwrap();

        let library = b"library";
        let mut header = tar::Header::new_gnu();
        header.set_size(library.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("{prefix}/libggml.so"), &library[..])
            .unwrap();

        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
        path
    }

    #[test]
    fn extracts_tar_gz_and_flattens_the_top_level_directory() {
        let dir = std::env::temp_dir().join(format!("flux-tar-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let archive = write_tar_gz(&dir, "llama-b12383");
        let entry = imagiflume_core::runtime::bundled_llama_manifest().backends[0].clone();
        let destination = dir.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        extract_backend(&archive, &entry, &destination).unwrap();

        assert!(destination.join("llama-cli").is_file());
        assert!(destination.join("libggml.so").is_file());
        assert!(!destination.join(".extract").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(destination.join("llama-cli"))
                .unwrap()
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn maps_llama_release_assets_to_entries() {
        let sha = "a".repeat(64);
        let asset = |name: &str, url: &str| {
            serde_json::json!({
                "name": name,
                "browser_download_url": url,
                "size": 1,
                "digest": format!("sha256:{sha}"),
            })
        };
        let release = serde_json::json!({
            "tag_name": "b12383",
            "assets": [
                asset("llama-b12383-bin-ubuntu-x64.tar.gz", "https://example.com/cpu"),
                asset("llama-b12383-bin-ubuntu-vulkan-x64.tar.gz", "https://example.com/vulkan"),
                asset("llama-b12383-bin-ubuntu-rocm-10.0-x64.tar.gz", "https://example.com/rocm"),
                asset("llama-b12383-bin-ubuntu-cuda-12.8-x64.tar.gz", "https://example.com/cuda12"),
                asset("llama-b12383-bin-ubuntu-cuda-13.5-x64.tar.gz", "https://example.com/cuda13"),
                asset("cudart-llama-b12383-bin-ubuntu-cuda-12.8-x64.tar.gz", "https://example.com/cudart"),
                asset("llama-b12383-bin-ubuntu-arm64.tar.gz", "https://example.com/arm"),
                asset("llama-b12383-bin-ubuntu-openvino-2026.4.1-x64.tar.gz", "https://example.com/openvino"),
            ]
        });

        let (tag, entries, errors) = llama_entries_from_release(&release).unwrap();
        assert_eq!(tag, "b12383");
        assert!(errors.is_empty());
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "llama-cpu",
                "llama-vulkan",
                "llama-rocm10",
                "llama-cuda12",
                "llama-cuda13"
            ]
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.binary_name() == "llama-cli")
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.archive == Some(ArchiveFormat::TarGz))
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.id == "llama-cuda12")
                .unwrap()
                .cuda_major,
            Some(12)
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.id == "llama-rocm10")
                .unwrap()
                .rocm_major,
            Some(10)
        );
    }

    #[test]
    fn skips_llama_releases_without_linux_assets() {
        let sha = "b".repeat(64);
        let empty = serde_json::json!({
            "tag_name": "v1.0.0",
            "assets": []
        });
        let real = serde_json::json!({
            "tag_name": "b12383",
            "assets": [{
                "name": "llama-b12383-bin-ubuntu-x64.tar.gz",
                "browser_download_url": "https://example.com/cpu",
                "size": 1,
                "digest": format!("sha256:{sha}"),
            }]
        });

        let (tag, entries, _) = select_llama_release(&[empty, real]).unwrap();
        assert_eq!(tag, "b12383");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "llama-cpu");
    }
}
