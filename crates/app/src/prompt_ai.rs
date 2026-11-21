//! Prompt rewriting and small vision decisions through llama.cpp.
//!
//! The stable-diffusion.cpp server keeps the encoder for conditioning only and
//! has no text-generation API, so this spawns `llama-cli` with the same GGUF
//! file for a few seconds. The optional day/night helper uses the small
//! LFM2.5-VL model plus its multimodal projector instead.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use imagiflume_core::Config;

const SYSTEM_PROMPT: &str = "\
Rewrite the user's image-generation prompt to be more specific and visually \
descriptive while preserving their intent. Add details about lighting, \
composition, materials and atmosphere, but do not add new subjects or change \
the scene. Keep it under 60 words. Return only the improved prompt wrapped in \
<prompt> and </prompt> tags.";

const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TOKENS: u32 = 220;
const VISION_PROMPT: &str =
    "Is this a night scene or a daylight scene? Answer with one word: night or daylight.";
const VISION_TIMEOUT: Duration = Duration::from_secs(60);

/// Locate `llama-cli` from config, the environment, or `PATH`.
pub(crate) fn find_cli(config: &Config) -> Option<PathBuf> {
    if let Some(path) = &config.prompt_helper_cli
        && is_executable(path)
    {
        return Some(path.clone());
    }
    if let Ok(path) = std::env::var("IMAGIFLUME_LLAMA_CLI") {
        let path = PathBuf::from(path);
        if is_executable(&path) {
            return Some(path);
        }
    }
    // Bundled and system-wide installs live next to the app or under
    // /usr/lib/imagiflume, neither of which is necessarily on PATH.
    if let Some(path) = imagiflume_core::runtime::find_system_llama_cli()
        && is_executable(&path)
    {
        return Some(path);
    }
    let path_var = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path_var) {
        let candidate = directory.join("llama-cli");
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Rewrite `prompt` with `encoder` using `cli`.
pub(crate) fn improve(prompt: &str, encoder: &Path, cli: &Path) -> Result<String> {
    if !encoder.exists() {
        bail!("text encoder not found at {}", encoder.display());
    }

    let stdout = run_command(build_command(cli, encoder, prompt), TIMEOUT)?;
    extract(&stdout).ok_or_else(|| anyhow!("the model did not return a prompt"))
}

/// Ask the vision helper whether `image` is a night scene.
pub(crate) fn classify_lighting(
    image: &Path,
    model: &Path,
    mmproj: &Path,
    cli: &Path,
) -> Result<bool> {
    if !model.exists() {
        bail!("vision model not found at {}", model.display());
    }
    if !mmproj.exists() {
        bail!("vision projector not found at {}", mmproj.display());
    }
    if !image.exists() {
        bail!("image not found at {}", image.display());
    }

    let stdout = run_command(
        build_vision_command(cli, model, mmproj, image),
        VISION_TIMEOUT,
    )?;
    parse_lighting(&stdout)
        .ok_or_else(|| anyhow!("the vision model did not return night or daylight"))
}

fn run_command(mut command: Command, timeout: Duration) -> Result<String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || read_all(stdout));
    let stderr_reader = std::thread::spawn(move || read_all(stderr));

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("llama-cli timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    if !status.success() {
        bail!("llama-cli exited with {status}{}", stderr_tail(&stderr));
    }

    Ok(stdout)
}

fn build_command(cli: &Path, encoder: &Path, prompt: &str) -> Command {
    let mut command = Command::new(cli);
    command
        .arg("-m")
        .arg(encoder)
        .arg("-sys")
        .arg(SYSTEM_PROMPT)
        .arg("-p")
        .arg(prompt)
        .arg("-st")
        .arg("--no-display-prompt")
        .arg("--color")
        .arg("off")
        // Qwen3 base spends the whole budget on chain-of-thought otherwise.
        .arg("-rea")
        .arg("off")
        .arg("-c")
        .arg("4096")
        .arg("--no-warmup")
        .arg("-n")
        .arg(MAX_TOKENS.to_string())
        .arg("--temp")
        .arg("0.4")
        .arg("-t")
        .arg(threads().to_string())
        .arg("--simple-io");
    command
}

fn build_vision_command(cli: &Path, model: &Path, mmproj: &Path, image: &Path) -> Command {
    let mut command = Command::new(cli);
    command
        .arg("-m")
        .arg(model)
        .arg("--mmproj")
        .arg(mmproj)
        .arg("--image")
        .arg(image)
        .arg("-p")
        .arg(VISION_PROMPT)
        .arg("-st")
        .arg("--no-display-prompt")
        .arg("--color")
        .arg("off")
        .arg("-c")
        .arg("2048")
        .arg("--no-warmup")
        .arg("-n")
        .arg("8")
        .arg("--temp")
        .arg("0")
        .arg("-t")
        .arg(threads().to_string())
        .arg("--simple-io");
    command
}

/// Pick the last lighting word in the model's answer. The echoed prompt is
/// removed first so its "night or daylight" wording cannot be mistaken for the
/// answer.
fn parse_lighting(stdout: &str) -> Option<bool> {
    let text = strip_ansi(stdout);
    let body = if let Some((_, rest)) = text.rsplit_once(VISION_PROMPT) {
        rest
    } else {
        text.as_str()
    };
    let body = body.split("\n[ Prompt:").next().unwrap_or(body);
    let answer = if text.contains(VISION_PROMPT) {
        body.lines().find(|line| !line.trim().is_empty())?
    } else {
        body.lines().rev().find(|line| !line.trim().is_empty())?
    };
    let answer = answer.to_ascii_lowercase();
    let night = answer.rfind("night");
    let day = answer.rfind("day");

    match (night, day) {
        (Some(night), Some(day)) => Some(night > day),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

fn threads() -> u32 {
    std::thread::available_parallelism()
        .map(|count| (count.get() / 2).clamp(1, 8) as u32)
        .unwrap_or(4)
}

fn read_all(mut reader: impl Read) -> String {
    let mut buffer = String::new();
    let _ = reader.read_to_string(&mut buffer);
    buffer
}

fn stderr_tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let start = lines.len().saturating_sub(5);
    format!(": {}", lines[start..].join(" | "))
}

/// Pull the rewritten prompt out of `llama-cli`'s chat output.
fn extract(stdout: &str) -> Option<String> {
    let text = strip_ansi(stdout);

    if let Some(start) = text.find("<prompt>") {
        let rest = &text[start + "<prompt>".len()..];
        let end = rest
            .find("</prompt>")
            .or_else(|| rest.find("\n[ Prompt:"))
            .unwrap_or(rest.len());
        let candidate = clean(&rest[..end]);
        if !candidate.is_empty() {
            return Some(candidate);
        }
    }

    // Fallback for models that ignore the tags: take the lines after the
    // echoed prompt and stop at llama-cli's timing footer.
    let (_, body) = text.rsplit_once("\n> ")?;
    let body = body.split("\n[ Prompt:").next().unwrap_or(body);
    let mut lines = body.lines();
    let _ = lines.next();
    let candidate = clean(&lines.collect::<Vec<_>>().join("\n"));
    if candidate.is_empty() || candidate.contains("available commands:") {
        None
    } else {
        Some(candidate)
    }
}

fn clean(text: &str) -> String {
    let mut text = text.trim();
    for label in [
        "Improved prompt:",
        "improved prompt:",
        "Prompt:",
        "prompt:",
        "Here is the improved prompt:",
    ] {
        if let Some(rest) = text.strip_prefix(label) {
            text = rest.trim();
        }
    }
    text.trim_matches(|character| matches!(character, '"' | '\'' | '“' | '”' | '‘' | '’'))
        .trim()
        .to_string()
}

fn strip_ansi(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            if characters.peek() == Some(&'[') {
                characters.next();
                for character in characters.by_ref() {
                    if character.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_args() -> Vec<String> {
        build_command(
            Path::new("/usr/bin/llama-cli"),
            Path::new("/models/Qwen3-4B-Q4_K_M.gguf"),
            "a red fox",
        )
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
    }

    #[test]
    fn builds_the_llama_command() {
        let args = command_args();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-m", "/models/Qwen3-4B-Q4_K_M.gguf"])
        );
        assert!(args.windows(2).any(|pair| pair == ["-p", "a red fox"]));
        assert!(args.windows(2).any(|pair| pair == ["-rea", "off"]));
        assert!(args.windows(2).any(|pair| pair == ["-c", "4096"]));
        assert!(args.windows(2).any(|pair| pair == ["-n", "220"]));
        assert!(args.contains(&"--no-warmup".to_string()));
        assert!(args.contains(&"--simple-io".to_string()));
    }

    #[test]
    fn extracts_tagged_output() {
        let output = "\n> a red fox\n<prompt>A red fox in autumn leaves, golden hour.</prompt>\n\n[ Prompt: 70 t/s ]\nExiting...\n";
        assert_eq!(
            extract(output).as_deref(),
            Some("A red fox in autumn leaves, golden hour.")
        );
    }

    #[test]
    fn extracts_unclosed_tagged_output() {
        let output = "\n> a red fox\n<prompt>A red fox in autumn leaves\n[ Prompt: 70 t/s ]\n";
        assert_eq!(
            extract(output).as_deref(),
            Some("A red fox in autumn leaves")
        );
    }

    #[test]
    fn falls_back_to_plain_output() {
        let output = "\n> a red fox\nA red fox in autumn leaves, golden hour.\n\n[ Prompt: 70 t/s ]\nExiting...\n";
        assert_eq!(
            extract(output).as_deref(),
            Some("A red fox in autumn leaves, golden hour.")
        );
    }

    #[test]
    fn ignores_cli_chatter() {
        let output = "available commands:\n  /exit\n";
        assert_eq!(extract(output), None);
    }

    #[test]
    fn strips_ansi_and_quotes() {
        let output = "\u{1b}[32m<prompt>\"a red fox\"</prompt>\u{1b}[0m";
        assert_eq!(extract(output).as_deref(), Some("a red fox"));
        assert_eq!(clean("Improved prompt: a red fox"), "a red fox");
    }

    #[test]
    fn splits_paths_for_the_cli_lookup() {
        // Sanity check that PATH splitting yields usable directories.
        if let Some(path) = std::env::var_os("PATH") {
            assert!(std::env::split_paths(&path).next().is_some());
        }
    }

    #[test]
    fn builds_the_vision_command() {
        let args: Vec<String> = build_vision_command(
            Path::new("/usr/bin/llama-cli"),
            Path::new("/models/vision/model.gguf"),
            Path::new("/models/vision/mmproj.gguf"),
            Path::new("/tmp/photo.png"),
        )
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-m", "/models/vision/model.gguf"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--mmproj", "/models/vision/mmproj.gguf"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--image", "/tmp/photo.png"])
        );
        assert!(args.windows(2).any(|pair| pair == ["-n", "8"]));
        assert!(args.windows(2).any(|pair| pair == ["--temp", "0"]));
    }

    #[test]
    fn parses_lighting_answers() {
        let night = format!("\n> {VISION_PROMPT}\nNight\n\n[ Prompt: 500 t/s ]\n");
        let day = format!("\n> {VISION_PROMPT}\nDaylight\n\n[ Prompt: 500 t/s ]\n");
        assert_eq!(parse_lighting(&night), Some(true));
        assert_eq!(parse_lighting(&day), Some(false));
        assert_eq!(
            parse_lighting("It is not night, it is daylight."),
            Some(false)
        );
        assert_eq!(parse_lighting("night"), Some(true));
        assert_eq!(parse_lighting("unclear"), None);
    }
}
