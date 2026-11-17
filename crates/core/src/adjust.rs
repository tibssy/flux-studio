//! CPU color adjustments applied to a finished image.

/// Brightness, contrast and saturation values for one manual adjustment.
///
/// The sliders in the canvas popover map directly onto these fields, so the
/// struct doubles as the value saved in the history summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adjustments {
    /// Added to each channel before contrast, in 0-255 units (-100..=100).
    pub brightness: i32,
    /// Percentage around mid-gray. 100 leaves the contrast unchanged.
    pub contrast: u32,
    /// Percentage around the pixel's luma. 100 leaves the colors unchanged.
    pub saturation: u32,
}

impl Adjustments {
    pub const NEUTRAL: Self = Self {
        brightness: 0,
        contrast: 100,
        saturation: 100,
    };

    pub const fn is_neutral(&self) -> bool {
        self.brightness == 0 && self.contrast == 100 && self.saturation == 100
    }

    /// Short human-readable summary, e.g. `brightness +15 · saturation 120%`.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.brightness != 0 {
            parts.push(format!("brightness {:+}", self.brightness));
        }
        if self.contrast != 100 {
            parts.push(format!("contrast {}%", self.contrast));
        }
        if self.saturation != 100 {
            parts.push(format!("saturation {}%", self.saturation));
        }
        if parts.is_empty() {
            "no change".to_string()
        } else {
            parts.join(" · ")
        }
    }

    /// Apply the adjustments in place to tightly packed RGBA8 pixels.
    ///
    /// Brightness and contrast are folded into one 256-entry lookup table;
    /// saturation then mixes each pixel toward its Rec. 709 luma. Alpha is
    /// never touched.
    pub fn apply(&self, pixels: &mut [u8]) {
        if self.is_neutral() {
            return;
        }

        let offset = self.brightness as f32;
        let contrast = self.contrast as f32 / 100.0;
        let saturation = self.saturation as f32 / 100.0;

        let mut lut = [0u8; 256];
        for (value, entry) in lut.iter_mut().enumerate() {
            let adjusted = (value as f32 + offset - 128.0) * contrast + 128.0;
            *entry = adjusted.clamp(0.0, 255.0).round() as u8;
        }

        // `as_chunks_mut` would raise the declared MSRV.
        #[allow(clippy::chunks_exact_to_as_chunks)]
        for pixel in pixels.chunks_exact_mut(4) {
            let red = lut[pixel[0] as usize] as f32;
            let green = lut[pixel[1] as usize] as f32;
            let blue = lut[pixel[2] as usize] as f32;
            let luma = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
            pixel[0] = (luma + (red - luma) * saturation).clamp(0.0, 255.0).round() as u8;
            pixel[1] = (luma + (green - luma) * saturation)
                .clamp(0.0, 255.0)
                .round() as u8;
            pixel[2] = (luma + (blue - luma) * saturation)
                .clamp(0.0, 255.0)
                .round() as u8;
        }
    }
}

impl Default for Adjustments {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_values_are_a_no_op() {
        let mut pixels = [12, 34, 56, 78, 200, 100, 50, 255];
        let original = pixels;
        Adjustments::NEUTRAL.apply(&mut pixels);
        assert_eq!(pixels, original);
        assert!(Adjustments::NEUTRAL.is_neutral());
    }

    #[test]
    fn brightness_offsets_and_clamps() {
        let mut pixels = [128, 128, 128, 255];
        Adjustments {
            brightness: 100,
            ..Adjustments::NEUTRAL
        }
        .apply(&mut pixels);
        assert_eq!(&pixels[..3], &[228, 228, 228]);

        let mut pixels = [200, 200, 200, 255];
        Adjustments {
            brightness: 100,
            ..Adjustments::NEUTRAL
        }
        .apply(&mut pixels);
        assert_eq!(&pixels[..3], &[255, 255, 255]);

        let mut pixels = [10, 10, 10, 255];
        Adjustments {
            brightness: -100,
            ..Adjustments::NEUTRAL
        }
        .apply(&mut pixels);
        assert_eq!(&pixels[..3], &[0, 0, 0]);
    }

    #[test]
    fn contrast_expands_around_mid_gray() {
        let mut pixels = [200, 200, 200, 255];
        Adjustments {
            contrast: 150,
            ..Adjustments::NEUTRAL
        }
        .apply(&mut pixels);
        assert_eq!(pixels[0], 236);

        let mut mid = [128, 128, 128, 255];
        Adjustments {
            contrast: 150,
            ..Adjustments::NEUTRAL
        }
        .apply(&mut mid);
        assert_eq!(mid[0], 128);
    }

    #[test]
    fn zero_saturation_turns_pixels_gray() {
        let mut pixels = [200, 100, 50, 255];
        Adjustments {
            saturation: 0,
            ..Adjustments::NEUTRAL
        }
        .apply(&mut pixels);
        assert_eq!(pixels[0], pixels[1]);
        assert_eq!(pixels[1], pixels[2]);
    }

    #[test]
    fn doubled_saturation_pushes_away_from_luma() {
        let original = [200u8, 100, 50, 255];
        let luma: f64 = 0.2126 * 200.0 + 0.7152 * 100.0 + 0.0722 * 50.0;
        let mut pixels = original;
        Adjustments {
            saturation: 200,
            ..Adjustments::NEUTRAL
        }
        .apply(&mut pixels);
        let expected_red = (luma + (200.0 - luma) * 2.0).clamp(0.0, 255.0) as u8;
        assert_eq!(pixels[0], expected_red);
        assert!(pixels[0] > original[0] || original[0] == 255);
        assert!(pixels[2] < original[2]);
    }

    #[test]
    fn alpha_is_preserved() {
        let mut pixels = [10, 20, 30, 7, 200, 150, 100, 128];
        Adjustments {
            brightness: 40,
            contrast: 120,
            saturation: 160,
        }
        .apply(&mut pixels);
        assert_eq!(pixels[3], 7);
        assert_eq!(pixels[7], 128);
    }

    #[test]
    fn summary_lists_only_changed_values() {
        assert_eq!(Adjustments::NEUTRAL.summary(), "no change");
        assert_eq!(
            Adjustments {
                brightness: 15,
                ..Adjustments::NEUTRAL
            }
            .summary(),
            "brightness +15"
        );
        assert_eq!(
            Adjustments {
                brightness: -20,
                contrast: 110,
                saturation: 120,
            }
            .summary(),
            "brightness -20 · contrast 110% · saturation 120%"
        );
    }
}
