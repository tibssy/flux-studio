//! Portable effect packs.
//!
//! User effects are short prompt recipes stored in `config.toml`. Exporting
//! them to a small JSON file makes them shareable and gives users a backup for
//! the only data in the app that is not in the history database.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::config::CustomEffect;

/// Current pack format version.
pub const PACK_VERSION: u32 = 1;
const MAX_NAME_CHARS: usize = 64;
const MAX_PROMPT_CHARS: usize = 8000;

#[derive(Debug, Serialize, Deserialize)]
struct EffectPack {
    #[serde(default)]
    version: u32,
    effects: Vec<CustomEffect>,
}

/// Accepts both the versioned object and a bare array of effects.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawPack {
    Pack(EffectPack),
    List(Vec<CustomEffect>),
}

/// Serialize effects as a versioned JSON pack.
pub fn to_json(effects: &[CustomEffect]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&EffectPack {
        version: PACK_VERSION,
        effects: effects.to_vec(),
    })
}

/// Parse an imported pack.
pub fn from_json(text: &str) -> Result<Vec<CustomEffect>, String> {
    let raw: RawPack =
        serde_json::from_str(text).map_err(|_| "Not an Imagiflume effects file.".to_string())?;
    let effects = match raw {
        RawPack::Pack(pack) => {
            if pack.version > PACK_VERSION {
                return Err(format!(
                    "This effects file needs a newer Imagiflume (format {}).",
                    pack.version
                ));
            }
            pack.effects
        }
        RawPack::List(effects) => effects,
    };

    let mut valid = Vec::new();
    for effect in effects {
        let Some(effect) = normalize(effect) else {
            continue;
        };
        valid.push(effect);
    }
    if valid.is_empty() {
        return Err("The file contains no usable effects.".to_string());
    }
    Ok(valid)
}

/// Result of merging a pack into the existing effect list.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MergeReport {
    pub added: usize,
    pub renamed: usize,
    pub skipped: usize,
}

impl MergeReport {
    pub fn summary(&self) -> String {
        let mut parts = vec![format!(
            "Imported {} effect{}",
            self.added,
            if self.added == 1 { "" } else { "s" }
        )];
        if self.renamed > 0 {
            parts.push(format!("renamed {}", self.renamed));
        }
        if self.skipped > 0 {
            parts.push(format!("skipped {}", self.skipped));
        }
        format!("{}.", parts.join(", "))
    }
}

/// Merge incoming effects without ever overwriting an existing one.
///
/// Exact duplicates (same name and prompt, case-insensitive) are skipped, name
/// collisions get a numeric suffix, and empty or oversized entries are dropped.
pub fn merge(
    existing: &mut Vec<CustomEffect>,
    incoming: impl IntoIterator<Item = CustomEffect>,
) -> MergeReport {
    let mut report = MergeReport::default();
    let mut names: HashSet<String> = existing
        .iter()
        .map(|effect| effect.name.trim().to_lowercase())
        .collect();
    let mut pairs: HashSet<(String, String)> = existing
        .iter()
        .map(|effect| {
            (
                effect.name.trim().to_lowercase(),
                effect.prompt.trim().to_string(),
            )
        })
        .collect();

    for effect in incoming {
        let Some(effect) = normalize(effect) else {
            report.skipped += 1;
            continue;
        };
        let key = (effect.name.to_lowercase(), effect.prompt.clone());
        if pairs.contains(&key) {
            report.skipped += 1;
            continue;
        }

        let mut name = effect.name.clone();
        if names.contains(&name.to_lowercase()) {
            report.renamed += 1;
            let base: String = name.chars().take(MAX_NAME_CHARS - 6).collect();
            let mut suffix = 2;
            loop {
                name = format!("{base} {suffix}");
                if !names.contains(&name.to_lowercase()) {
                    break;
                }
                suffix += 1;
                if suffix > 999 {
                    name = format!("{base} {suffix}");
                    break;
                }
            }
        }

        names.insert(name.to_lowercase());
        pairs.insert((name.to_lowercase(), effect.prompt.clone()));
        existing.push(CustomEffect {
            name,
            prompt: effect.prompt,
        });
        report.added += 1;
    }
    report
}

fn normalize(effect: CustomEffect) -> Option<CustomEffect> {
    let name: String = effect.name.trim().chars().take(MAX_NAME_CHARS).collect();
    let prompt = effect.prompt.trim().to_string();
    if name.is_empty() || prompt.is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
        return None;
    }
    Some(CustomEffect { name, prompt })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(name: &str, prompt: &str) -> CustomEffect {
        CustomEffect {
            name: name.into(),
            prompt: prompt.into(),
        }
    }

    #[test]
    fn round_trips_a_pack() {
        let effects = vec![effect("Restore", "repair this photo")];
        let text = to_json(&effects).unwrap();
        assert!(text.contains("\"version\": 1"));
        assert_eq!(from_json(&text).unwrap(), effects);
    }

    #[test]
    fn accepts_a_bare_array() {
        let text = r#"[{"name":"Night","prompt":"turn this into night"}]"#;
        assert_eq!(
            from_json(text).unwrap(),
            vec![effect("Night", "turn this into night")]
        );
    }

    #[test]
    fn rejects_unknown_or_empty_files() {
        assert!(from_json("not json").is_err());
        assert!(from_json("[]").is_err());
        assert!(from_json(r#"{"version":9,"effects":[]}"#).is_err());
    }

    #[test]
    fn trims_and_drops_invalid_entries() {
        let text = r#"[
            {"name":"  Restore  ","prompt":"  fix it  "},
            {"name":"","prompt":"no name"},
            {"name":"Empty","prompt":"   "}
        ]"#;
        assert_eq!(from_json(text).unwrap(), vec![effect("Restore", "fix it")]);
    }

    #[test]
    fn merge_skips_exact_duplicates() {
        let mut existing = vec![effect("Restore", "fix it")];
        let report = merge(
            &mut existing,
            vec![effect("restore", "fix it"), effect("Night", "night")],
        );
        assert_eq!(
            report,
            MergeReport {
                added: 1,
                renamed: 0,
                skipped: 1
            }
        );
        assert_eq!(existing.len(), 2);
    }

    #[test]
    fn merge_renames_name_collisions() {
        let mut existing = vec![effect("Restore", "fix it")];
        let report = merge(&mut existing, vec![effect("Restore", "make it look new")]);
        assert_eq!(
            report,
            MergeReport {
                added: 1,
                renamed: 1,
                skipped: 0
            }
        );
        assert_eq!(existing[1].name, "Restore 2");
        assert_eq!(existing[1].prompt, "make it look new");
    }

    #[test]
    fn merge_never_overwrites() {
        let mut existing = vec![effect("A", "one"), effect("B", "two")];
        merge(&mut existing, vec![effect("A", "three")]);
        assert_eq!(existing[0], effect("A", "one"));
        assert_eq!(existing[2].name, "A 2");
    }

    #[test]
    fn summary_is_readable() {
        assert_eq!(
            MergeReport {
                added: 2,
                renamed: 1,
                skipped: 3
            }
            .summary(),
            "Imported 2 effects, renamed 1, skipped 3."
        );
        assert_eq!(
            MergeReport {
                added: 1,
                ..Default::default()
            }
            .summary(),
            "Imported 1 effect."
        );
    }
}
