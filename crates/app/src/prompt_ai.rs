//! Prompt rewriting through a local Ollama instance.
//!
//! This is optional: when Ollama is not running the button reports a clear
//! error instead of blocking generation. The model is auto-detected from the
//! first non-embedding model, or set explicitly with `llm_model` in the config.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use flux_core::Config;
use serde::Deserialize;
use serde_json::json;

const SYSTEM_PROMPT: &str = "\
You improve prompts for a text-to-image diffusion model. \
Rewrite the user's prompt to be more specific and visually descriptive while \
preserving their intent. Add concrete details about lighting, composition, \
materials and atmosphere, but do not add new subjects or change the scene. \
Keep it under 60 words. Return only the improved prompt, without quotes, \
labels or explanations.";

const MAX_PREDICT: u32 = 800;

pub(crate) fn improve(prompt: &str, config: &Config) -> Result<String> {
    let base = config.llm_url.trim_end_matches('/');
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;

    let model = match &config.llm_model {
        Some(model) if !model.trim().is_empty() => model.clone(),
        _ => detect_model(&client, base)?,
    };

    let response = client
        .post(format!("{base}/api/chat"))
        .json(&json!({
            "model": model,
            "stream": false,
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user", "content": prompt },
            ],
            "options": {
                "temperature": 0.4,
                "num_predict": MAX_PREDICT,
            },
        }))
        .send()
        .with_context(|| format!("could not reach Ollama at {base} (is it running?)"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        let body = body.trim();
        let detail = if body.is_empty() {
            String::new()
        } else {
            format!(": {}", body.chars().take(200).collect::<String>())
        };
        bail!("Ollama returned HTTP {status}{detail}");
    }

    let body: ChatResponse = response.json().context("invalid Ollama response")?;
    let text = body
        .message
        .map(|message| message.content)
        .unwrap_or_default();
    let cleaned = clean(&text);
    if cleaned.is_empty() {
        bail!("the model returned an empty prompt");
    }
    Ok(cleaned)
}

fn detect_model(client: &reqwest::blocking::Client, base: &str) -> Result<String> {
    let response = client
        .get(format!("{base}/api/tags"))
        .timeout(Duration::from_secs(5))
        .send()
        .with_context(|| format!("could not reach Ollama at {base} (is it running?)"))?;
    if !response.status().is_success() {
        bail!("Ollama returned HTTP {}", response.status());
    }
    let tags: Tags = response.json().context("invalid Ollama model list")?;
    select_model(&tags.models)
        .ok_or_else(|| anyhow!("no Ollama model is installed (`ollama pull <model>`)"))
}

fn select_model(models: &[TagModel]) -> Option<String> {
    // Prefer a small instruction model: a 20B model would compete with the
    // cached diffusion weights for RAM. Favor the first model in the 2B–8B
    // range, then the smallest 1B–8B one.
    if let Some(name) = models.iter().find_map(|model| {
        let size = model
            .details
            .as_ref()?
            .parameter_size
            .as_deref()
            .and_then(parameter_billions)?;
        (2.0..=8.0).contains(&size).then(|| model.name.clone())
    }) {
        return Some(name);
    }

    let mut preferred: Vec<(f64, String)> = models
        .iter()
        .filter_map(|model| {
            let size = model
                .details
                .as_ref()?
                .parameter_size
                .as_deref()
                .and_then(parameter_billions)?;
            (1.0..=8.0)
                .contains(&size)
                .then(|| (size, model.name.clone()))
        })
        .collect();
    preferred.sort_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, name)) = preferred.into_iter().next() {
        return Some(name);
    }

    models.iter().map(|model| model.name.clone()).find(|name| {
        let name = name.to_ascii_lowercase();
        !name.contains("embed") && !name.contains("minilm")
    })
}

fn parameter_billions(size: &str) -> Option<f64> {
    let size = size.trim();
    if let Some(number) = size.strip_suffix('B') {
        return number.trim().parse().ok();
    }
    if let Some(number) = size.strip_suffix('M') {
        return number
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| value / 1000.0);
    }
    None
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

#[derive(Deserialize)]
struct Tags {
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
    #[serde(default)]
    details: Option<TagDetails>,
}

#[derive(Deserialize)]
struct TagDetails {
    #[serde(default)]
    parameter_size: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: Option<ChatMessage>,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::{TagDetails, TagModel, clean, parameter_billions, select_model};

    fn model(name: &str, size: Option<&str>) -> TagModel {
        TagModel {
            name: name.into(),
            details: size.map(|size| TagDetails {
                parameter_size: Some(size.into()),
            }),
        }
    }

    #[test]
    fn strips_quotes_and_labels() {
        assert_eq!(clean("  \"a red fox\"  "), "a red fox");
        assert_eq!(clean("Improved prompt: a red fox"), "a red fox");
        assert_eq!(clean("“a red fox”"), "a red fox");
        assert_eq!(clean(""), "");
    }

    #[test]
    fn parses_parameter_counts() {
        assert_eq!(parameter_billions("2.5B"), Some(2.5));
        assert_eq!(parameter_billions("380M"), Some(0.38));
        assert_eq!(parameter_billions("unknown"), None);
    }

    #[test]
    fn selects_a_small_instruction_model() {
        let models = vec![
            model("gpt-oss-20b", Some("20.9B")),
            model("MiniCPM5-2B", Some("2.52B")),
            model("lfm3:1.7b", Some("1.7B")),
            model("all-minilm", Some("23M")),
        ];
        assert_eq!(select_model(&models).as_deref(), Some("MiniCPM5-2B"));
    }

    #[test]
    fn falls_back_past_large_and_tiny_models() {
        let models = vec![
            model("huge", Some("20B")),
            model("tiny", Some("0.5B")),
            model("small", Some("1.7B")),
        ];
        assert_eq!(select_model(&models).as_deref(), Some("small"));

        let models = vec![model("all-minilm", None), model("custom", None)];
        assert_eq!(select_model(&models).as_deref(), Some("custom"));
    }
}
