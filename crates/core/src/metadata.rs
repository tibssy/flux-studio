//! Prompt and generation metadata embedded in PNG text chunks.
//!
//! The history database is the app's source of truth, but a PNG that carries
//! its own prompt and settings stays useful outside the app: it can be shared,
//! archived or dropped back into another tool without losing how it was made.
//!
//! Two chunks are written:
//!
//! - `imagiflume`: a JSON [`ImageMetadata`] blob (UTF-8, `iTXt`).
//! - `parameters`: an A1111-style text blob for interoperability with other
//!   stable-diffusion front ends.
//!
//! Images written before the rename used the `flux-studio` keyword. Those are
//! still read, and rewriting one replaces the legacy chunk.
//!
//! Chunks are inserted right after `IHDR` without recompressing the image, so
//! writing metadata is cheap even for large upscales.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Keyword of the structured metadata chunk.
pub const METADATA_KEYWORD: &str = "imagiflume";
/// Keyword used by releases before the Imagiflume rename.
pub const LEGACY_METADATA_KEYWORD: &str = "flux-studio";
/// Keyword of the A1111-compatible text chunk.
pub const PARAMETERS_KEYWORD: &str = "parameters";

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Everything worth keeping with a generated image.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageMetadata {
    #[serde(default)]
    pub app_version: String,
    /// `generate`, `upscale`, `adjust`, or an effect id.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub seed: String,
    #[serde(default)]
    pub steps: u32,
    #[serde(default)]
    pub cfg: f32,
    #[serde(default)]
    pub sampler: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub loras: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub duration_ms: i64,
}

/// Insert (or replace) the Imagiflume text chunks in `path`.
pub fn write_png(path: &Path, metadata: &ImageMetadata) -> Result<()> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let updated = inject(&bytes, metadata)?;

    // Replace the file atomically so an interrupted write cannot truncate the
    // image that was just generated.
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image.png".into());
    let temporary = directory.join(format!(".{name}.metadata"));
    std::fs::write(&temporary, updated)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

/// Read the Imagiflume chunk, if the PNG has one.
pub fn read_png(path: &Path) -> Result<Option<ImageMetadata>> {
    let bytes = std::fs::read(path)?;
    for chunk in chunks(&bytes)? {
        let Some(text) = decode_text_chunk(chunk.kind, chunk.data)? else {
            continue;
        };
        if text.0 != METADATA_KEYWORD && text.0 != LEGACY_METADATA_KEYWORD {
            continue;
        }
        let metadata = serde_json::from_str(&text.1)
            .with_context(|| format!("invalid metadata in {}", path.display()))?;
        return Ok(Some(metadata));
    }
    Ok(None)
}

fn inject(bytes: &[u8], metadata: &ImageMetadata) -> Result<Vec<u8>> {
    if !bytes.starts_with(&PNG_SIGNATURE) {
        bail!("not a PNG file");
    }
    let parsed = chunks(bytes)?;
    let json = serde_json::to_string(metadata)?;

    let mut output = Vec::with_capacity(bytes.len() + json.len() + 256);
    output.extend_from_slice(&PNG_SIGNATURE);
    let mut inserted = false;
    for chunk in parsed {
        if chunk.kind == b"IHDR" {
            write_chunk(&mut output, b"IHDR", chunk.data);
            write_chunk(&mut output, b"iTXt", &text_payload(METADATA_KEYWORD, &json));
            write_chunk(
                &mut output,
                b"iTXt",
                &text_payload(PARAMETERS_KEYWORD, &parameters(metadata)),
            );
            inserted = true;
            continue;
        }
        if is_our_text_chunk(chunk.kind, chunk.data) {
            continue;
        }
        write_chunk(&mut output, chunk.kind, chunk.data);
    }
    if !inserted {
        bail!("PNG has no IHDR chunk");
    }
    Ok(output)
}

fn is_our_text_chunk(kind: &[u8; 4], data: &[u8]) -> bool {
    if !matches!(kind, b"iTXt" | b"tEXt" | b"zTXt") {
        return false;
    }
    let keyword = data
        .iter()
        .position(|byte| *byte == 0)
        .map(|end| &data[..end])
        .unwrap_or(data);
    keyword == METADATA_KEYWORD.as_bytes()
        || keyword == LEGACY_METADATA_KEYWORD.as_bytes()
        || keyword == PARAMETERS_KEYWORD.as_bytes()
}

struct Chunk<'a> {
    kind: &'a [u8; 4],
    data: &'a [u8],
}

fn chunks(bytes: &[u8]) -> Result<Vec<Chunk<'_>>> {
    if !bytes.starts_with(&PNG_SIGNATURE) {
        bail!("not a PNG file");
    }
    let mut chunks = Vec::new();
    let mut offset = PNG_SIGNATURE.len();
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("four length bytes"),
        ) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .context("PNG chunk length overflow")?;
        if end > bytes.len() {
            bail!("truncated PNG chunk");
        }
        let kind: &[u8; 4] = bytes[offset + 4..offset + 8]
            .try_into()
            .expect("four type bytes");
        let data = &bytes[offset + 8..offset + 8 + length];
        chunks.push(Chunk { kind, data });
        offset = end;
        if kind == b"IEND" {
            break;
        }
    }
    Ok(chunks)
}

fn write_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    output.extend_from_slice(&hasher.finalize().to_be_bytes());
}

/// `iTXt` payload with an uncompressed UTF-8 body.
fn text_payload(keyword: &str, text: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(keyword.len() + text.len() + 6);
    payload.extend_from_slice(keyword.as_bytes());
    payload.push(0);
    payload.push(0); // compression flag: uncompressed
    payload.push(0); // compression method
    payload.push(0); // empty language tag
    payload.push(0); // empty translated keyword
    payload.extend_from_slice(text.as_bytes());
    payload
}

/// Decode an `iTXt` or `tEXt` chunk into `(keyword, text)`.
fn decode_text_chunk(kind: &[u8; 4], data: &[u8]) -> Result<Option<(String, String)>> {
    let keyword_end = data.iter().position(|byte| *byte == 0);
    let Some(keyword_end) = keyword_end else {
        return Ok(None);
    };
    let keyword = String::from_utf8_lossy(&data[..keyword_end]).into_owned();
    let rest = &data[keyword_end + 1..];
    match kind {
        b"tEXt" => Ok(Some((keyword, String::from_utf8_lossy(rest).into_owned()))),
        b"iTXt" => {
            if rest.len() < 2 || rest[0] != 0 {
                // Compressed iTXt chunks are not written by this app.
                return Ok(None);
            }
            let mut cursor = 2;
            for _ in 0..2 {
                let Some(end) = rest[cursor..].iter().position(|byte| *byte == 0) else {
                    return Ok(None);
                };
                cursor += end + 1;
            }
            Ok(Some((
                keyword,
                String::from_utf8_lossy(&rest[cursor..]).into_owned(),
            )))
        }
        _ => Ok(None),
    }
}

/// A1111-compatible `parameters` text.
fn parameters(metadata: &ImageMetadata) -> String {
    let mut text = metadata.prompt.trim().to_string();
    text.push_str("\nNegative prompt: ");
    let mut settings = vec![
        format!("Steps: {}", metadata.steps),
        format!("Sampler: {}", metadata.sampler),
        format!("CFG scale: {:.1}", metadata.cfg),
        format!("Seed: {}", metadata.seed),
        format!("Size: {}x{}", metadata.width, metadata.height),
        format!("Model: {}", metadata.model),
        format!(
            "Version: Imagiflume {}",
            if metadata.app_version.is_empty() {
                "unknown"
            } else {
                metadata.app_version.as_str()
            }
        ),
    ];
    if !metadata.references.is_empty() {
        settings.push(format!("Reference: {}", metadata.references.join(", ")));
    }
    text.push('\n');
    text.push_str(&settings.join(", "));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smallest valid PNG: a 1×1 transparent image.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    fn metadata() -> ImageMetadata {
        ImageMetadata {
            app_version: "0.1.0".into(),
            kind: "generate".into(),
            prompt: "a lighthouse at dusk, ünïcode".into(),
            model: "flux2-klein-4b".into(),
            style: "cinematic".into(),
            seed: "42".into(),
            steps: 4,
            cfg: 1.0,
            sampler: "euler".into(),
            width: 608,
            height: 816,
            references: vec!["/tmp/ref.png".into()],
            loras: vec![],
            source: None,
            created_at: 1_765_000_000,
            duration_ms: 12_500,
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "imagiflume-metadata-{}-{name}.png",
            std::process::id()
        ))
    }

    #[test]
    fn round_trips_metadata() {
        let path = temp_path("round-trip");
        std::fs::write(&path, TINY_PNG).unwrap();

        write_png(&path, &metadata()).unwrap();
        let read = read_png(&path).unwrap().unwrap();
        assert_eq!(read, metadata());

        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(&PNG_SIGNATURE));
        assert!(
            chunks(&bytes)
                .unwrap()
                .iter()
                .any(|chunk| chunk.kind == b"IDAT")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn replaces_older_metadata() {
        let path = temp_path("replace");
        std::fs::write(&path, TINY_PNG).unwrap();

        let mut first = metadata();
        first.prompt = "first".into();
        write_png(&path, &first).unwrap();
        write_png(&path, &metadata()).unwrap();

        assert_eq!(read_png(&path).unwrap().unwrap(), metadata());
        let bytes = std::fs::read(&path).unwrap();
        let count = chunks(&bytes)
            .unwrap()
            .iter()
            .filter(|chunk| chunk.kind == b"iTXt" && is_our_text_chunk(chunk.kind, chunk.data))
            .count();
        assert_eq!(count, 2, "structured and A1111 chunks only");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reads_metadata_written_before_the_rename() {
        let path = temp_path("legacy");
        let mut output = Vec::new();
        output.extend_from_slice(&PNG_SIGNATURE);
        for chunk in chunks(TINY_PNG).unwrap() {
            if chunk.kind == b"IHDR" {
                write_chunk(&mut output, b"IHDR", chunk.data);
                let json = serde_json::to_string(&metadata()).unwrap();
                write_chunk(
                    &mut output,
                    b"iTXt",
                    &text_payload(LEGACY_METADATA_KEYWORD, &json),
                );
            } else {
                write_chunk(&mut output, chunk.kind, chunk.data);
            }
        }
        std::fs::write(&path, output).unwrap();

        assert_eq!(read_png(&path).unwrap().unwrap(), metadata());

        // Rewriting replaces the legacy keyword instead of leaving two chunks.
        write_png(&path, &metadata()).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let legacy = chunks(&bytes)
            .unwrap()
            .iter()
            .filter_map(|chunk| decode_text_chunk(chunk.kind, chunk.data).ok().flatten())
            .any(|(keyword, _)| keyword == LEGACY_METADATA_KEYWORD);
        assert!(!legacy);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn ignores_images_without_metadata() {
        let path = temp_path("plain");
        std::fs::write(&path, TINY_PNG).unwrap();
        assert!(read_png(&path).unwrap().is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_non_png_files() {
        assert!(inject(b"not a png", &metadata()).is_err());
    }

    #[test]
    fn writes_a1111_parameters() {
        let text = parameters(&metadata());
        assert!(text.starts_with("a lighthouse at dusk, ünïcode\nNegative prompt: "));
        assert!(text.contains("Steps: 4"));
        assert!(text.contains("Seed: 42"));
        assert!(text.contains("Size: 608x816"));
    }
}
