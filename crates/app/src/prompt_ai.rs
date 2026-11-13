//! Prompt rewriting through llama.cpp using the active model's text encoder.
//!
//! The stable-diffusion.cpp server keeps the encoder for conditioning only and
//! has no text-generation API, so this spawns `llama-cli` with the same GGUF
//! file for a few seconds. No extra model download and no external service.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use flux_core::Config;

const SYSTEM_PROMPT: &str = "\
Rewrite the user's image-generation prompt to be more specific and visually \
descriptive while preserving their intent. Add details about lighting, \
composition, materials and atmosphere, but do not add new subjects or change \
the scene. Keep it under 60 words. Return only the improved prompt wrapped in \
<prompt> and </prompt> tags.";

const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TOKENS: u32 = 220;

/// Locate `llama-cli` from config, the environment, or `PATH`.
pub(crate) fn find_cli(config: &Config) -> Option<PathBuf> {
    if let Some(path) = &config.prompt_helper_cli
        && is_executable(path)
    {
        return Some(path.clone());
    }
    if let Ok(path) = std::env::var("FLUX_LLAMA_CLI") {
        let path = PathBuf::from(path);
        if is_executable(&path) {
            return Some(path);
        }
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

    let mut child = build_command(cli, encoder, prompt)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {}", cli.display()))?;

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || read_all(stdout));
    let stderr_reader = std::thread::spawn(move || read_all(stderr));

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("prompt helper timed out after {}s", TIMEOUT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    if !status.success() {
        bail!("llama-cli exited with {status}{}", stderr_tail(&stderr));
    }

    extract(&stdout).ok_or_else(|| anyhow!("the model did not return a prompt"))
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
}
