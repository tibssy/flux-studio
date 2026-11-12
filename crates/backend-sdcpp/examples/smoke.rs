//! Headless smoke test for the persistent server backend.
//!
//! Run from the repository root:
//!   cargo run -p flux-backend-sdcpp --example smoke
//!
//! It generates the same image twice (the second run should reuse the loaded
//! model) and then performs a reference edit.

use std::path::PathBuf;
use std::time::Instant;

use flux_backend_sdcpp::{SdServer, SdServerConfig};
use flux_core::{Backend, GenerateRequest, Progress, Sampler};

fn request(prompt: &str, output: &str, references: Vec<PathBuf>) -> GenerateRequest {
    GenerateRequest {
        preset_id: "flux2-klein-4b".into(),
        prompt: prompt.into(),
        width: 256,
        height: 256,
        steps: 4,
        cfg: 1.0,
        seed: 42,
        sampler: Sampler::Euler,
        offload: true,
        reference_images: references,
        output: PathBuf::from(output),
    }
}

fn run(backend: &SdServer, request: &GenerateRequest) -> anyhow::Result<()> {
    let (sender, receiver) = async_channel::unbounded();
    let consumer = std::thread::spawn(move || {
        while let Ok(progress) = receiver.recv_blocking() {
            match progress {
                Progress::Step { current, total } => print!(" {current}/{total}"),
                Progress::Loading => print!(" loading"),
                Progress::Failed(error) => print!(" failed: {error}"),
                Progress::Cancelled => print!(" cancelled"),
                Progress::Done(_) => {}
                Progress::Log(_) => {}
            }
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    });

    let started = Instant::now();
    let path = backend.generate(request, sender)?;
    consumer.join().ok();
    println!(
        "\n  -> {} in {:.1}s",
        path.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let models_dir = std::env::var("FLUX_MODELS_DIR").unwrap_or_else(|_| "models".into());
    let cli = std::env::var("FLUX_SD_SERVER")
        .unwrap_or_else(|_| "vendor/stable-diffusion.cpp/build/bin/sd-server".into());
    let backend = SdServer::new(SdServerConfig {
        cli: PathBuf::from(cli),
        models_dir: PathBuf::from(models_dir),
    });

    println!("first generation (loads the model)");
    run(
        &backend,
        &request(
            "a red fox, golden hour",
            "/tmp/opencode/rust-server-1.png",
            vec![],
        ),
    )?;

    println!("second generation (should reuse the model)");
    run(
        &backend,
        &request(
            "a blue fox, winter light",
            "/tmp/opencode/rust-server-2.png",
            vec![],
        ),
    )?;

    println!("reference edit");
    run(
        &backend,
        &request(
            "make it snow, pale blue light",
            "/tmp/opencode/rust-server-3.png",
            vec![PathBuf::from("/tmp/opencode/rust-server-1.png")],
        ),
    )?;

    Ok(())
}
