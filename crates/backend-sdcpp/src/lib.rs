//! Backends that drive stable-diffusion.cpp.
//!
//! `SdCpp` spawns the one-shot `sd-cli` per generation (reloads weights each
//! time). `SdServer` keeps the persistent `sd-server` context alive instead and
//! is what the desktop app uses.

pub mod server;

pub use server::{SdServer, SdServerConfig};

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_channel::Sender;

use flux_core::preset::{ModelPreset, ModelRole, preset};
use flux_core::{Backend, BackendInfo, GenerateRequest, Progress};

/// How many trailing stderr lines to keep for error reporting.
const STDERR_TAIL_LINES: usize = 6;

#[derive(Debug, Clone)]
pub struct SdCppConfig {
    pub cli: PathBuf,
    /// Root directory containing `diffusion/`, `vae/` and `text_encoders/`.
    pub models_dir: PathBuf,
}

pub struct SdCpp {
    config: SdCppConfig,
    child: Arc<Mutex<Option<Child>>>,
}

impl SdCpp {
    pub fn new(config: SdCppConfig) -> Self {
        Self {
            config,
            child: Arc::new(Mutex::new(None)),
        }
    }

    pub fn config(&self) -> &SdCppConfig {
        &self.config
    }

    fn require_file(&self, preset: &ModelPreset, role: ModelRole) -> Result<PathBuf> {
        let path = preset
            .path(role, &self.config.models_dir)
            .ok_or_else(|| anyhow!("{} has no {:?} file", preset.id, role))?;
        if !path.exists() {
            bail!(
                "{} for {} not found at {}",
                role_label(role),
                preset.label,
                path.display()
            );
        }
        Ok(path)
    }
}

fn role_label(role: ModelRole) -> &'static str {
    match role {
        ModelRole::Diffusion => "diffusion model",
        ModelRole::Vae => "VAE",
        ModelRole::TextEncoder => "text encoder",
        ModelRole::VisionEncoder => "vision encoder",
    }
}

impl Backend for SdCpp {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "stable-diffusion.cpp".into(),
            device: std::env::var("FLUX_DEVICE").unwrap_or_else(|_| "cuda".into()),
            version: None,
        }
    }

    fn generate(&self, request: &GenerateRequest, progress: Sender<Progress>) -> Result<PathBuf> {
        if !self.config.cli.exists() {
            bail!(
                "sd-cli not found at {} (set FLUX_SD_CLI or build vendor/stable-diffusion.cpp)",
                self.config.cli.display()
            );
        }
        let preset = preset(&request.preset_id)
            .ok_or_else(|| anyhow!("unknown model preset {:?}", request.preset_id))?;

        for role in [ModelRole::Diffusion, ModelRole::Vae, ModelRole::TextEncoder] {
            self.require_file(preset, role)?;
        }
        if preset.file(ModelRole::VisionEncoder).is_some() {
            self.require_file(preset, ModelRole::VisionEncoder)?;
        }
        for reference in &request.reference_images {
            if !reference.exists() {
                bail!("reference image not found at {}", reference.display());
            }
        }
        if !request.reference_images.is_empty() && !preset.supports_edit {
            bail!("{} does not support reference images", preset.label);
        }
        if let Some(parent) = request.output.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let _ = progress.send_blocking(Progress::Loading);

        let mut child = Command::new(&self.config.cli)
            .args(build_args(&self.config.models_dir, preset, request))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn {}", self.config.cli.display()))?;

        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        *self.child.lock().expect("child mutex") = Some(child);

        let expected_steps = request.steps;
        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
        let stdout_thread = {
            let tx = progress.clone();
            std::thread::spawn(move || read_progress_stream(stdout, tx, expected_steps))
        };
        let stderr_thread = {
            let tx = progress.clone();
            let tail = stderr_tail.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    {
                        let mut tail = tail.lock().expect("stderr tail");
                        if tail.len() == STDERR_TAIL_LINES {
                            tail.pop_front();
                        }
                        tail.push_back(line.clone());
                    }
                    let _ = tx.send_blocking(Progress::Log(line));
                }
            })
        };

        let status = loop {
            let maybe_status = {
                let mut guard = self.child.lock().expect("child mutex");
                match guard.as_mut() {
                    Some(child) => child.try_wait()?,
                    None => {
                        drop(guard);
                        let _ = progress.send_blocking(Progress::Cancelled);
                        let _ = stdout_thread.join();
                        let _ = stderr_thread.join();
                        return Err(anyhow!("generation cancelled"));
                    }
                }
            };
            match maybe_status {
                Some(status) => break status,
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        *self.child.lock().expect("child mutex") = None;

        let _ = stdout_thread.join();
        let _ = stderr_thread.join();

        if !status.success() {
            let tail = stderr_tail.lock().expect("stderr tail");
            let detail = tail
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            drop(tail);
            tracing::error!(target: "sd-cli", "exited with {status}\n{detail}");
            let message = if detail.is_empty() {
                format!("sd-cli exited with {status}")
            } else {
                format!("sd-cli exited with {status}: {detail}")
            };
            let _ = progress.send_blocking(Progress::Failed(message.clone()));
            bail!(message);
        }
        if !request.output.exists() {
            let message = format!(
                "sd-cli finished but {} was not created",
                request.output.display()
            );
            let _ = progress.send_blocking(Progress::Failed(message.clone()));
            bail!(message);
        }

        let _ = progress.send_blocking(Progress::Done(request.output.clone()));
        Ok(request.output.clone())
    }

    fn cancel(&self) -> Result<()> {
        let child = self.child.lock().expect("child mutex").take();
        if let Some(mut child) = child {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

/// Build the `sd-cli` argument list for one request.
pub fn build_args(
    models_dir: &Path,
    preset: &ModelPreset,
    request: &GenerateRequest,
) -> Vec<OsString> {
    let path = |role| {
        preset
            .path(role, models_dir)
            .expect("preset has required model file")
            .into_os_string()
    };

    let mut args: Vec<OsString> = vec![
        "--diffusion-model".into(),
        path(ModelRole::Diffusion),
        "--vae".into(),
        path(ModelRole::Vae),
        "--llm".into(),
        path(ModelRole::TextEncoder),
    ];

    if preset.file(ModelRole::VisionEncoder).is_some() {
        args.push("--llm_vision".into());
        args.push(path(ModelRole::VisionEncoder));
    }

    args.extend([
        "-p".into(),
        request.prompt.clone().into(),
        "-W".into(),
        request.width.to_string().into(),
        "-H".into(),
        request.height.to_string().into(),
        "--steps".into(),
        request.steps.to_string().into(),
        "--cfg-scale".into(),
        format!("{:.2}", request.cfg).into(),
        "--sampling-method".into(),
        request.sampler.as_str().into(),
        "--seed".into(),
        request.seed.to_string().into(),
        "--vae-tiling".into(),
    ]);

    for arg in preset.extra_args {
        args.push((*arg).into());
    }

    for reference in &request.reference_images {
        args.push("-r".into());
        args.push(reference.clone().into_os_string());
    }

    args.extend(["-o".into(), request.output.clone().into_os_string(), "-v".into()]);

    if request.offload {
        args.push("--offload-to-cpu".into());
    }

    args
}

fn read_progress_stream<R: Read>(mut stream: R, tx: Sender<Progress>, expected_steps: u32) {
    let mut buffer = [0u8; 2048];
    let mut line = String::new();
    let mut escape = false;

    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &byte in &buffer[..read] {
            if escape {
                // Skip the remainder of an ANSI CSI sequence (ends on a letter).
                if byte.is_ascii_alphabetic() || byte == b'~' {
                    escape = false;
                }
                continue;
            }
            match byte {
                b'\x1b' => escape = true,
                b'\r' | b'\n' => {
                    if let Some(step) = parse_progress_line(&line, expected_steps) {
                        let _ = tx.send_blocking(step);
                    }
                    line.clear();
                }
                _ if line.len() < 4096 => line.push(byte as char),
                _ => {}
            }
        }
    }

    if let Some(step) = parse_progress_line(&line, expected_steps) {
        let _ = tx.send_blocking(step);
    }
}

/// Parse a sampling line like `  |=====>    |  2/4 - 3.62s/it - ETA 00:07`.
///
/// Only lines whose total equals the requested step count and whose speed unit
/// is `s/it` are accepted. This filters out VAE tiling (`3/25 - 1.30it/s`) and
/// tensor loading (`5/140 - 0.00MB/s`), which use the same bar format.
pub fn parse_progress_line(line: &str, expected_steps: u32) -> Option<Progress> {
    if expected_steps == 0 || !line.contains("s/it") {
        return None;
    }
    for token in line.split_whitespace() {
        let token = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '/');
        let Some((current, total)) = token.split_once('/') else {
            continue;
        };
        let (Ok(current), Ok(total)) = (current.parse::<u32>(), total.parse::<u32>()) else {
            continue;
        };
        if total == expected_steps && current <= total {
            return Some(Progress::Step { current, total });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_core::preset::FLUX2_KLEIN_4B;

    #[test]
    fn parses_progress_bar() {
        let parsed = parse_progress_line("  |=====>        |  2/4 -  3.62s/it - ETA 00:07", 4);
        assert!(matches!(
            parsed,
            Some(Progress::Step {
                current: 2,
                total: 4
            })
        ));
    }

    #[test]
    fn ignores_other_totals_and_units() {
        assert!(parse_progress_line("size 1024/1024", 4).is_none());
        assert!(parse_progress_line("  |==>  |  5/140 - 0.00MB/s", 4).is_none());
        assert!(parse_progress_line("  |==>  |  3/25 - 1.30it/s", 4).is_none());
        // A 25-step model must not accept VAE tile progress.
        assert!(parse_progress_line("  |==>  |  3/25 - 1.30it/s", 25).is_none());
        assert!(parse_progress_line("  |==>  |  3/25 - 2.10s/it", 25).is_some());
    }

    #[test]
    fn builds_expected_args() {
        let preset = &FLUX2_KLEIN_4B;
        let request = GenerateRequest {
            preset_id: preset.id.into(),
            prompt: "a cat".into(),
            width: 768,
            height: 768,
            steps: 4,
            cfg: 1.0,
            seed: 42,
            sampler: flux_core::Sampler::Euler,
            offload: true,
            reference_images: vec!["/tmp/ref.png".into()],
            output: "/tmp/out.png".into(),
        };
        let args = build_args(Path::new("/models"), preset, &request);
        let args: Vec<String> = args
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|w| w == ["--steps", "4"]));
        assert!(args.windows(2).any(|w| w == ["--cfg-scale", "1.00"]));
        assert!(args.contains(&"--offload-to-cpu".to_string()));
        assert!(args.windows(2).any(|w| w == ["-r", "/tmp/ref.png"]));
        assert!(args.windows(2).any(|w| w == ["-o", "/tmp/out.png"]));
        assert!(args.windows(2).any(|w| w == [
            "--diffusion-model",
            "/models/diffusion/flux-2-klein-4b-Q4_K_M.gguf"
        ]));
    }

    #[test]
    fn flux3_uses_sixteen_steps_and_vision_encoder() {
        let preset = &flux_core::preset::FLUX3_KLEIN;
        let request = GenerateRequest {
            preset_id: preset.id.into(),
            prompt: "a cat".into(),
            width: 768,
            height: 768,
            steps: preset.steps,
            cfg: preset.cfg,
            seed: 7,
            sampler: preset.sampler,
            offload: true,
            reference_images: Vec::new(),
            output: "/tmp/out.png".into(),
        };
        let args: Vec<String> = build_args(Path::new("/models"), preset, &request)
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|w| w == ["--steps", "16"]));
        assert!(args.windows(2).any(|w| w == ["--seed", "7"]));
        assert!(args.iter().any(|a| a.ends_with("Qwen3VL-4B-Instruct-Q4_K_M.gguf")));
        assert!(args.iter().any(|a| a.ends_with("flux3-image-klein-Q4_K.gguf")));
    }
}
