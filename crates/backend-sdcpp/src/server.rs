//! Persistent `sd-server` backend.
//!
//! Starting `sd-cli` for every image reloads several gigabytes of weights each
//! time. `sd-server` keeps the context loaded, so the app starts it on first
//! use, reuses it for subsequent generations, and stops it after 60 seconds of
//! inactivity (or when the user switches models).

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use async_channel::Sender;
use base64::Engine;
use flux_core::preset::{ModelPreset, ModelRole, preset};
use flux_core::{Backend, BackendInfo, GenerateRequest, Progress};
use serde::Deserialize;
use serde_json::json;

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const START_TIMEOUT: Duration = Duration::from_secs(180);
const JOB_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub struct SdServerConfig {
    pub cli: PathBuf,
    pub models_dir: PathBuf,
}

pub struct SdServer {
    inner: Arc<Mutex<Inner>>,
    job: Arc<Mutex<Option<String>>>,
    cancelling: Arc<AtomicBool>,
}

struct Inner {
    config: SdServerConfig,
    client: reqwest::blocking::Client,
    child: Option<Child>,
    port: u16,
    loaded: Option<&'static str>,
    offload: bool,
    busy: bool,
    last_used: Instant,
    log: PathBuf,
}

impl SdServer {
    pub fn new(config: SdServerConfig) -> Self {
        let log = flux_core::paths::cache_dir().join("sd-server.log");
        let inner = Arc::new(Mutex::new(Inner {
            config,
            client: reqwest::blocking::Client::builder()
                .build()
                .expect("HTTP client"),
            child: None,
            port: 0,
            loaded: None,
            offload: true,
            busy: false,
            last_used: Instant::now(),
            log,
        }));

        // Stop the server (freeing VRAM/RAM) after a minute of inactivity.
        let monitor = inner.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let mut inner = monitor.lock().expect("server mutex");
                if inner.busy || inner.child.is_none() {
                    continue;
                }
                if inner.last_used.elapsed() >= IDLE_TIMEOUT {
                    tracing::info!(
                        "model idle for {}s, stopping sd-server",
                        IDLE_TIMEOUT.as_secs()
                    );
                    inner.stop();
                }
            }
        });

        Self {
            inner,
            job: Arc::new(Mutex::new(None)),
            cancelling: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Backend for SdServer {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "stable-diffusion.cpp (server)".into(),
            device: std::env::var("FLUX_DEVICE").unwrap_or_else(|_| "cuda".into()),
            version: None,
        }
    }

    fn generate(&self, request: &GenerateRequest, progress: Sender<Progress>) -> Result<PathBuf> {
        self.cancelling.store(false, Ordering::SeqCst);
        let result = self.run(request, &progress);

        *self.job.lock().expect("job mutex") = None;
        self.cancelling.store(false, Ordering::SeqCst);
        let mut inner = self.inner.lock().expect("server mutex");
        inner.busy = false;
        inner.last_used = Instant::now();
        drop(inner);

        result
    }

    fn cancel(&self) -> Result<()> {
        self.cancelling.store(true, Ordering::SeqCst);
        let job = self.job.lock().expect("job mutex").clone();
        let Some(job) = job else {
            return Ok(());
        };
        let inner = self.inner.lock().expect("server mutex");
        if inner.child.is_none() {
            return Ok(());
        }
        let url = format!("http://127.0.0.1:{}/sdcpp/v1/jobs/{job}/cancel", inner.port);
        let _ = inner
            .client
            .post(url)
            .timeout(Duration::from_secs(5))
            .send();
        Ok(())
    }

    fn unload(&self) {
        let mut inner = self.inner.lock().expect("server mutex");
        if !inner.busy {
            inner.stop();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop();
    }
}

impl SdServer {
    fn run(&self, request: &GenerateRequest, progress: &Sender<Progress>) -> Result<PathBuf> {
        let model = preset(&request.preset_id)
            .ok_or_else(|| anyhow!("unknown model preset {:?}", request.preset_id))?;

        let port = {
            let mut inner = self.inner.lock().expect("server mutex");
            inner.busy = true;
            inner.last_used = Instant::now();
            inner.ensure(model, request.offload, progress)?;
            if self.cancelling.load(Ordering::SeqCst) {
                drop(inner);
                let _ = progress.send_blocking(Progress::Cancelled);
                bail!("generation cancelled");
            }
            inner.port
        };

        for reference in &request.reference_images {
            if !reference.exists() {
                bail!("reference image not found at {}", reference.display());
            }
        }
        if !request.reference_images.is_empty() && !model.supports_edit {
            bail!("{} does not support reference images", model.label);
        }
        if let Some(parent) = request.output.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut payload = json!({
            "prompt": request.prompt,
            "width": request.width,
            "height": request.height,
            "seed": request.seed,
            "batch_count": 1,
            "output_format": "png",
            "preview": "none",
            "sample_params": {
                "sample_method": request.sampler.as_str(),
                "sample_steps": request.steps,
                "guidance": { "txt_cfg": request.cfg },
            },
            "vae_tiling_params": { "enabled": true },
        });
        if !request.reference_images.is_empty() {
            let images = request
                .reference_images
                .iter()
                .map(|path| reference_data_url(path))
                .collect::<Result<Vec<_>>>()?;
            payload["ref_images"] = json!(images);
        }

        let client = self.inner.lock().expect("server mutex").client.clone();
        let submit_url = format!("http://127.0.0.1:{port}/sdcpp/v1/img_gen");
        let response = client
            .post(&submit_url)
            .json(&payload)
            .timeout(Duration::from_secs(60))
            .send()
            .with_context(|| format!("POST {submit_url}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            let message = format!("sd-server rejected the job (HTTP {status}): {body}");
            let _ = progress.send_blocking(Progress::Failed(message.clone()));
            bail!(message);
        }
        let submitted: JobSubmitted = response.json().context("invalid job response")?;
        *self.job.lock().expect("job mutex") = Some(submitted.id.clone());

        if self.cancelling.load(Ordering::SeqCst) {
            let url = format!(
                "http://127.0.0.1:{port}/sdcpp/v1/jobs/{}/cancel",
                submitted.id
            );
            let _ = client.post(url).timeout(Duration::from_secs(5)).send();
        }

        let started = Instant::now();
        let mut last_step = None;
        loop {
            if started.elapsed() > JOB_TIMEOUT {
                let message = "generation timed out".to_string();
                let _ = progress.send_blocking(Progress::Failed(message.clone()));
                bail!(message);
            }

            let url = format!("http://127.0.0.1:{port}/sdcpp/v1/jobs/{}", submitted.id);
            let response = client
                .get(&url)
                .timeout(Duration::from_secs(15))
                .send()
                .with_context(|| format!("GET {url}"))?;
            if !response.status().is_success() {
                bail!("job disappeared (HTTP {})", response.status());
            }
            let job: Job = response.json().context("invalid job status")?;

            if let Some(step) = job.step_progress()
                && last_step != Some(step)
            {
                last_step = Some(step);
                let _ = progress.send_blocking(Progress::Step {
                    current: step.0,
                    total: step.1,
                });
            }

            match job.status.as_str() {
                "completed" => {
                    let image = job
                        .result
                        .and_then(|result| result.images.into_iter().next())
                        .ok_or_else(|| anyhow!("sd-server returned no image"))?;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(image.b64_json)
                        .context("invalid image payload")?;
                    std::fs::write(&request.output, bytes)?;
                    let _ = progress.send_blocking(Progress::Done(request.output.clone()));
                    return Ok(request.output.clone());
                }
                "failed" => {
                    let message = job
                        .error
                        .map(|error| error.message)
                        .unwrap_or_else(|| "generation failed".into());
                    let _ = progress.send_blocking(Progress::Failed(message.clone()));
                    bail!(message);
                }
                "cancelled" => {
                    let _ = progress.send_blocking(Progress::Cancelled);
                    bail!("generation cancelled");
                }
                _ => {}
            }

            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Inner {
    fn ensure(
        &mut self,
        model: &'static ModelPreset,
        offload: bool,
        progress: &Sender<Progress>,
    ) -> Result<()> {
        if let Some(child) = self.child.as_mut()
            && child.try_wait()?.is_some()
        {
            self.child = None;
            self.loaded = None;
        }

        let restart =
            self.child.is_none() || self.loaded != Some(model.id) || self.offload != offload;
        if restart {
            self.stop();
            let _ = progress.send_blocking(Progress::Loading);
            self.start(model, offload)?;
        }
        self.last_used = Instant::now();
        Ok(())
    }

    fn start(&mut self, model: &'static ModelPreset, offload: bool) -> Result<()> {
        let port = free_port()?;
        if let Some(parent) = self.log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let log = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.log)
            .with_context(|| format!("failed to open {}", self.log.display()))?;
        let log_err = log.try_clone()?;

        let path = |role| {
            model
                .path(role, &self.config.models_dir)
                .ok_or_else(|| anyhow!("{} has no {:?} file", model.id, role))
        };
        let diffusion = path(ModelRole::Diffusion)?;
        let vae = path(ModelRole::Vae)?;
        let llm = path(ModelRole::TextEncoder)?;
        for file in [&diffusion, &vae, &llm] {
            if !file.exists() {
                bail!("model file not found at {}", file.display());
            }
        }

        let mut command = Command::new(&self.config.cli);
        command
            .arg("--diffusion-model")
            .arg(diffusion)
            .arg("--vae")
            .arg(vae)
            .arg("--llm")
            .arg(llm);
        if let Some(vision) = model.path(ModelRole::VisionEncoder, &self.config.models_dir) {
            if !vision.exists() {
                bail!("model file not found at {}", vision.display());
            }
            command.arg("--llm_vision").arg(vision);
        }
        if offload {
            command.arg("--offload-to-cpu");
        }
        command
            .arg("--listen-ip")
            .arg("127.0.0.1")
            .arg("--listen-port")
            .arg(port.to_string())
            .arg("-v")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));

        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                // Do not leave an orphaned server if the app is killed.
                let _ = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }

        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn {}", self.config.cli.display()))?;
        self.child = Some(child);
        self.port = port;
        self.offload = offload;
        self.loaded = Some(model.id);
        self.wait_ready(model)
    }

    fn wait_ready(&mut self, model: &ModelPreset) -> Result<()> {
        let deadline = Instant::now() + START_TIMEOUT;
        let url = format!("http://127.0.0.1:{}/sdcpp/v1/models/status", self.port);
        loop {
            if let Some(child) = self.child.as_mut()
                && let Some(status) = child.try_wait()?
            {
                self.child = None;
                self.loaded = None;
                bail!(
                    "sd-server exited with {status} while loading {} (see {})",
                    model.label,
                    self.log.display()
                );
            }

            if let Ok(response) = self.client.get(&url).timeout(Duration::from_secs(2)).send()
                && response.status().is_success()
                && let Ok(status) = response.json::<ModelStatus>()
            {
                match status.state.as_str() {
                    "ready" => return Ok(()),
                    "error" => {
                        self.stop();
                        bail!(
                            "sd-server failed to load {}: {}",
                            model.label,
                            status.error.unwrap_or_default()
                        );
                    }
                    _ => {}
                }
            }

            if Instant::now() >= deadline {
                self.stop();
                bail!("timed out loading {}", model.label);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.loaded = None;
    }
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn reference_data_url(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read reference image {}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    let mime = match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    };
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[derive(Deserialize)]
struct JobSubmitted {
    id: String,
}

#[derive(Deserialize)]
struct Job {
    status: String,
    progress: Option<JobProgress>,
    result: Option<JobResult>,
    error: Option<JobError>,
}

impl Job {
    /// `(current, total)` for the sampling stage, if available.
    fn step_progress(&self) -> Option<(u32, u32)> {
        let progress = self.progress.as_ref()?;
        if progress.unit.as_deref() != Some("steps") {
            return None;
        }
        let total = progress.total?;
        if total == 0 {
            return None;
        }
        let completed = progress.completed.unwrap_or(0);
        Some(((completed + 1).min(total), total))
    }
}

#[derive(Deserialize)]
struct JobProgress {
    completed: Option<u32>,
    total: Option<u32>,
    unit: Option<String>,
}

#[derive(Deserialize)]
struct JobResult {
    images: Vec<JobImage>,
}

#[derive(Deserialize)]
struct JobImage {
    b64_json: String,
}

#[derive(Deserialize)]
struct JobError {
    message: String,
}

#[derive(Deserialize)]
struct ModelStatus {
    state: String,
    error: Option<String>,
}
