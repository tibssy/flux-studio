//! Resolution limits and MP/ratio helpers shared by the UI and the backend.

/// Smallest side the UI offers.
pub const MIN_SIZE: u32 = 128;
/// Largest side the UI offers.
pub const MAX_SIZE: u32 = 2048;
/// The backend works on multiples of 16.
pub const SIZE_STEP: u32 = 16;

/// Smallest target the MP slider offers.
pub const MP_MIN: f64 = 0.01;
/// Largest target the MP slider offers.
pub const MP_MAX: f64 = 2.0;

/// Preset aspect ratios offered in the sidebar.
pub const RATIOS: [(u32, u32); 12] = [
    (21, 9),
    (2, 1),
    (16, 9),
    (3, 2),
    (4, 3),
    (5, 4),
    (1, 1),
    (4, 5),
    (3, 4),
    (2, 3),
    (9, 16),
    (9, 21),
];

/// Ratios this close to a preset are treated as that preset. This absorbs the
/// rounding introduced by the 16 px grid.
const RATIO_TOLERANCE: f64 = 0.025;

/// Whether a size came from a preset ratio or an exact custom ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatioMatch {
    /// Index into [`RATIOS`].
    Preset(usize),
    /// Reduced width:height for a ratio outside the presets.
    Custom((u32, u32)),
}

/// Clamp and snap a dimension to the supported grid.
pub fn snap(value: u32) -> u32 {
    let clamped = value.clamp(MIN_SIZE, MAX_SIZE);
    ((clamped + SIZE_STEP / 2) / SIZE_STEP * SIZE_STEP).clamp(MIN_SIZE, MAX_SIZE)
}

pub fn is_valid(value: u32) -> bool {
    (MIN_SIZE..=MAX_SIZE).contains(&value) && value.is_multiple_of(SIZE_STEP)
}

pub fn megapixels(width: u32, height: u32) -> f64 {
    width as f64 * height as f64 / 1_000_000.0
}

/// Reduce a ratio to lowest terms, e.g. `1216:832` -> `19:13`.
pub fn reduce_ratio(width: u32, height: u32) -> (u32, u32) {
    fn gcd(mut a: u32, mut b: u32) -> u32 {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a.max(1)
    }

    let width = width.max(1);
    let height = height.max(1);
    let divisor = gcd(width, height);
    (width / divisor, height / divisor)
}

/// Aspect ratio in lowest terms, e.g. `4:3` or `13:12`.
pub fn aspect_label(width: u32, height: u32) -> String {
    let (width, height) = reduce_ratio(width, height);
    format!("{width}:{height}")
}

/// A friendly dropdown label, e.g. `16:9 · Landscape`.
pub fn ratio_label(ratio: (u32, u32)) -> String {
    let (width, height) = ratio;
    let orientation = if width > height {
        "Landscape"
    } else if width < height {
        "Portrait"
    } else {
        "Square"
    };
    format!("{width}:{height} · {orientation}")
}

/// Whether a size is big enough to warn about on an 8 GB card.
pub fn is_large(width: u32, height: u32) -> bool {
    megapixels(width, height) > 1.0
}

/// Convert an MP target and a ratio into a snapped, in-range size.
pub fn size_for_mp(mp: f64, ratio: (u32, u32)) -> (u32, u32) {
    let (ratio_width, ratio_height) = if ratio.0 == 0 || ratio.1 == 0 {
        (1, 1)
    } else {
        ratio
    };
    let mp = if mp.is_finite() {
        mp.clamp(MP_MIN, MP_MAX)
    } else {
        1.0
    };
    let area = mp * 1_000_000.0;

    let mut width = (area * ratio_width as f64 / ratio_height as f64).sqrt();
    let mut height = (area * ratio_height as f64 / ratio_width as f64).sqrt();

    // Respect the max side without changing the requested ratio.
    let shrink = (MAX_SIZE as f64 / width)
        .min(MAX_SIZE as f64 / height)
        .min(1.0);
    width *= shrink;
    height *= shrink;

    // Respect the minimum side without changing the requested ratio.
    let grow = (MIN_SIZE as f64 / width)
        .max(MIN_SIZE as f64 / height)
        .max(1.0);
    width *= grow;
    height *= grow;

    // A final proportional fit guards against extreme custom ratios.
    let shrink = (MAX_SIZE as f64 / width)
        .min(MAX_SIZE as f64 / height)
        .min(1.0);
    width *= shrink;
    height *= shrink;

    (snap_side(width), snap_side(height))
}

fn snap_side(value: f64) -> u32 {
    let snapped = (value / SIZE_STEP as f64).round() * SIZE_STEP as f64;
    (snapped.round() as u32).clamp(MIN_SIZE, MAX_SIZE)
}

pub fn mp_for_size(width: u32, height: u32) -> f64 {
    megapixels(width, height)
}

/// Map a concrete size to a preset ratio, or to its reduced custom ratio.
pub fn ratio_for_size(width: u32, height: u32) -> RatioMatch {
    let (ratio_width, ratio_height) = reduce_ratio(width, height);

    for (index, &(preset_width, preset_height)) in RATIOS.iter().enumerate() {
        if ratio_width == preset_width && ratio_height == preset_height {
            return RatioMatch::Preset(index);
        }
    }

    let mut best: Option<(usize, f64)> = None;
    for (index, &(preset_width, preset_height)) in RATIOS.iter().enumerate() {
        let error = ((ratio_width as f64 * preset_height as f64)
            - (ratio_height as f64 * preset_width as f64))
            .abs()
            / (ratio_height as f64 * preset_width as f64).max(1.0);
        if best.is_none_or(|(_, best_error)| error < best_error) {
            best = Some((index, error));
        }
    }

    match best {
        Some((index, error)) if error <= RATIO_TOLERANCE => RatioMatch::Preset(index),
        _ => RatioMatch::Custom((ratio_width, ratio_height)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_the_grid() {
        assert_eq!(snap(0), MIN_SIZE);
        assert_eq!(snap(130), MIN_SIZE);
        assert_eq!(snap(250), 256);
        assert_eq!(snap(320), 320);
        assert_eq!(snap(830), 832);
        assert_eq!(snap(2000), 2000);
        assert_eq!(snap(3000), MAX_SIZE);
    }

    #[test]
    fn validates_range_and_step() {
        assert!(is_valid(128));
        assert!(is_valid(256));
        assert!(is_valid(2048));
        assert!(!is_valid(250));
        assert!(!is_valid(120));
        assert!(!is_valid(2064));
        assert!(!is_valid(300));
    }

    #[test]
    fn reports_common_aspects() {
        assert_eq!(aspect_label(1024, 768), "4:3");
        assert_eq!(aspect_label(768, 768), "1:1");
        assert_eq!(aspect_label(832, 768), "13:12");
        assert_eq!(aspect_label(1280, 640), "2:1");
        assert_eq!(aspect_label(1216, 832), "19:13");
    }

    #[test]
    fn computes_megapixels_and_warning() {
        assert!((megapixels(1024, 1024) - 1.048576).abs() < 1e-6);
        assert!(!is_large(768, 768));
        assert!(is_large(1280, 1024));
    }

    #[test]
    fn converts_mp_to_snapped_sizes() {
        assert_eq!(size_for_mp(0.59, (16, 9)), (1024, 576));
        assert_eq!(size_for_mp(0.60, (4, 3)), (896, 672));
        assert_eq!(size_for_mp(1.0, (4, 3)), (1152, 864));
        assert_eq!(size_for_mp(2.0, (1, 1)), (1408, 1408));
        assert_eq!(size_for_mp(2.0, (21, 9)), (2048, 880));
        assert_eq!(size_for_mp(0.01, (16, 9)), (224, 128));
    }

    #[test]
    fn mp_round_trips_within_grid_error() {
        for ratio in [(1, 1), (16, 9), (4, 3), (3, 2)] {
            for mp in [0.1, 0.25, 0.5, 1.0, 1.5] {
                let (width, height) = size_for_mp(mp, ratio);
                let actual = mp_for_size(width, height);
                assert!(
                    (actual - mp).abs() < 0.08,
                    "{ratio:?} at {mp} MP produced {width}×{height} ({actual:.3} MP)"
                );
            }
        }
    }

    #[test]
    fn produces_valid_sizes() {
        for ratio in RATIOS {
            for mp in [0.01, 0.1, 0.5, 1.0, 1.5, 2.0] {
                let (width, height) = size_for_mp(mp, ratio);
                assert!(is_valid(width), "{width} is not on the grid");
                assert!(is_valid(height), "{height} is not on the grid");
            }
        }
    }

    #[test]
    fn matches_preset_and_custom_ratios() {
        assert_eq!(ratio_for_size(1024, 768), RatioMatch::Preset(4));
        assert_eq!(ratio_for_size(1344, 752), RatioMatch::Preset(2));
        assert_eq!(ratio_for_size(2048, 880), RatioMatch::Preset(0));
        assert_eq!(ratio_for_size(1216, 832), RatioMatch::Custom((19, 13)));
        assert_eq!(ratio_for_size(176, 128), RatioMatch::Custom((11, 8)));
    }

    #[test]
    fn labels_presets_and_custom_ratios() {
        assert_eq!(ratio_label((16, 9)), "16:9 · Landscape");
        assert_eq!(ratio_label((1, 1)), "1:1 · Square");
        assert_eq!(ratio_label((9, 16)), "9:16 · Portrait");
        assert_eq!(ratio_label((19, 13)), "19:13 · Landscape");
    }
}
