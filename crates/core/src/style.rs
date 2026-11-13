//! Built-in style presets.
//!
//! A style is a prompt suffix, not a separate model: it lets users pick a
//! consistent look without having to remember phrasing. The registry is the
//! source of truth for the UI dropdown, history and prompt composition.

/// One selectable style.
#[derive(Debug, Clone, Copy)]
pub struct StylePreset {
    pub id: &'static str,
    pub label: &'static str,
    /// Appended to the prompt. `None` means "leave the prompt untouched".
    pub suffix: Option<&'static str>,
}

pub const NONE: StylePreset = StylePreset {
    id: "none",
    label: "None",
    suffix: None,
};

pub const STYLES: &[StylePreset] = &[
    NONE,
    StylePreset {
        id: "cinematic",
        label: "Cinematic",
        suffix: Some("cinematic film still, dramatic lighting, shallow depth of field, 35mm"),
    },
    StylePreset {
        id: "dynamic",
        label: "Dynamic",
        suffix: Some("dynamic composition, sense of motion, dramatic perspective"),
    },
    StylePreset {
        id: "2d",
        label: "2D design",
        suffix: Some("flat vector illustration, bold shapes, clean lines, minimal shading"),
    },
    StylePreset {
        id: "3d",
        label: "3D design",
        suffix: Some("3D render, soft studio lighting, subsurface scattering, high detail"),
    },
    StylePreset {
        id: "photographic",
        label: "Photographic",
        suffix: Some("photograph, natural light, sharp focus, 50mm lens"),
    },
    StylePreset {
        id: "anime",
        label: "Anime",
        suffix: Some("anime key visual, cel shading, detailed line art"),
    },
    StylePreset {
        id: "disney-pixar",
        label: "Disney/Pixar",
        suffix: Some(
            "Disney Pixar style 3D animation, expressive characters, soft global illumination, vibrant colors",
        ),
    },
    StylePreset {
        id: "sketch",
        label: "Sketch",
        suffix: Some("pencil sketch, hand-drawn, cross-hatching, graphite on paper, monochrome"),
    },
    StylePreset {
        id: "vector",
        label: "Vector graphic",
        suffix: Some("flat vector graphic, clean geometric shapes, solid colors, sharp edges"),
    },
    StylePreset {
        id: "watercolor",
        label: "Watercolor",
        suffix: Some("watercolor painting, soft washes, paper texture, loose brushstrokes"),
    },
    StylePreset {
        id: "oil-painting",
        label: "Oil painting",
        suffix: Some("oil painting, impasto, rich brushwork, canvas texture"),
    },
    StylePreset {
        id: "comic",
        label: "Comic book",
        suffix: Some("comic book art, bold ink lines, halftone shading, dynamic composition"),
    },
    StylePreset {
        id: "cyberpunk",
        label: "Cyberpunk",
        suffix: Some("cyberpunk, neon lights, futuristic city, high contrast, cinematic"),
    },
    StylePreset {
        id: "fantasy",
        label: "Fantasy art",
        suffix: Some("fantasy concept art, epic scale, detailed environment, dramatic lighting"),
    },
    StylePreset {
        id: "minimalist",
        label: "Minimalist",
        suffix: Some("minimalist, clean composition, negative space, simple shapes"),
    },
    StylePreset {
        id: "pixel-art",
        label: "Pixel art",
        suffix: Some("pixel art, 16-bit, limited palette, crisp pixels"),
    },
];

pub fn style(id: &str) -> Option<&'static StylePreset> {
    STYLES.iter().find(|style| style.id == id)
}

pub fn default_style() -> &'static StylePreset {
    &NONE
}

pub fn label(id: &str) -> &str {
    style(id).map(|style| style.label).unwrap_or(id)
}

/// Compose the prompt sent to the backend from the user's text and a style.
pub fn compose_prompt(base: &str, style_id: &str) -> String {
    let base = base.trim();
    match style(style_id).and_then(|style| style.suffix) {
        Some(suffix) if !base.is_empty() => format!("{base}, {suffix}"),
        _ => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        for (index, style) in STYLES.iter().enumerate() {
            assert!(
                STYLES[index + 1..].iter().all(|other| other.id != style.id),
                "duplicate style id {}",
                style.id
            );
        }
    }

    #[test]
    fn none_leaves_prompt_untouched() {
        assert_eq!(compose_prompt("a cat", "none"), "a cat");
        assert_eq!(compose_prompt("  a cat  ", "none"), "a cat");
    }

    #[test]
    fn appends_suffix_once() {
        let prompt = compose_prompt("a cat", "cinematic");
        assert!(prompt.starts_with("a cat, "));
        assert!(prompt.contains("cinematic film still"));
        assert_eq!(prompt.matches("cinematic film still").count(), 1);
    }

    #[test]
    fn empty_prompt_stays_empty() {
        assert_eq!(compose_prompt("   ", "cinematic"), "");
        assert_eq!(compose_prompt("", "anime"), "");
    }

    #[test]
    fn unknown_style_is_ignored() {
        assert_eq!(compose_prompt("a cat", "does-not-exist"), "a cat");
    }
}
