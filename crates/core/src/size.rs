//! Resolution limits shared by the UI and the backend.

/// Smallest side the UI offers.
pub const MIN_SIZE: u32 = 128;
/// Largest side the UI offers (8 GB cards start struggling above this).
pub const MAX_SIZE: u32 = 1280;
/// Sliders snap to this grid; FLUX works on multiples of 16/64.
pub const SIZE_STEP: u32 = 64;

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

/// Aspect ratio in lowest terms, e.g. `4:3` or `13:12`.
pub fn aspect_label(width: u32, height: u32) -> String {
    fn gcd(mut a: u32, mut b: u32) -> u32 {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a.max(1)
    }
    let divisor = gcd(width, height);
    format!("{}:{}", width / divisor, height / divisor)
}

/// Whether a size is big enough to warn about on an 8 GB card.
pub fn is_large(width: u32, height: u32) -> bool {
    megapixels(width, height) > 1.0
}

/// Pick the slider size whose aspect ratio best matches `width`:`height`.
///
/// Exact ratios on the grid win; ties are broken by closeness to
/// `target_area` pixels.
pub fn fit_aspect(width: u32, height: u32, target_area: f64) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (MIN_SIZE, MIN_SIZE);
    }

    let target_area = if target_area.is_finite() && target_area > 0.0 {
        target_area
    } else {
        (width as f64 * height as f64)
            .clamp((MIN_SIZE * MIN_SIZE) as f64, (MAX_SIZE * MAX_SIZE) as f64)
    };

    let mut best: Option<((u8, f64, f64), u32, u32)> = None;
    for candidate_width in (MIN_SIZE..=MAX_SIZE).step_by(SIZE_STEP as usize) {
        for candidate_height in (MIN_SIZE..=MAX_SIZE).step_by(SIZE_STEP as usize) {
            let cross_width = candidate_width as u64 * height as u64;
            let cross_height = candidate_height as u64 * width as u64;
            let exact = u8::from(cross_width != cross_height);
            let aspect_error =
                (cross_width as f64 - cross_height as f64).abs() / cross_height as f64;
            let area = candidate_width as f64 * candidate_height as f64;
            let area_error = (area - target_area).abs() / target_area;
            let score = (exact, aspect_error, area_error);

            let better = match &best {
                None => true,
                Some((best_score, _, _)) => {
                    score.0 < best_score.0
                        || (score.0 == best_score.0
                            && (score.1.total_cmp(&best_score.1).is_lt()
                                || (score.1.total_cmp(&best_score.1).is_eq()
                                    && score.2.total_cmp(&best_score.2).is_lt())))
                }
            };
            if better {
                best = Some((score, candidate_width, candidate_height));
            }
        }
    }

    best.map_or((MIN_SIZE, MIN_SIZE), |(_, width, height)| (width, height))
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
        assert_eq!(snap(2000), MAX_SIZE);
    }

    #[test]
    fn validates_range_and_step() {
        assert!(is_valid(128));
        assert!(is_valid(256));
        assert!(is_valid(1280));
        assert!(!is_valid(250));
        assert!(!is_valid(120));
        assert!(!is_valid(1300));
        assert!(!is_valid(300));
    }

    #[test]
    fn reports_common_aspects() {
        assert_eq!(aspect_label(1024, 768), "4:3");
        assert_eq!(aspect_label(768, 768), "1:1");
        assert_eq!(aspect_label(832, 768), "13:12");
        assert_eq!(aspect_label(1280, 640), "2:1");
    }

    #[test]
    fn computes_megapixels_and_warning() {
        assert!((megapixels(1024, 1024) - 1.048576).abs() < 1e-6);
        assert!(!is_large(768, 768));
        assert!(is_large(1280, 1024));
    }

    #[test]
    fn fits_reference_aspect_ratios() {
        let budget = (768 * 768) as f64;
        assert_eq!(fit_aspect(1920, 1080, budget), (1024, 576));
        assert_eq!(fit_aspect(4032, 3024, budget), (768, 576));
        assert_eq!(fit_aspect(3000, 2000, budget), (960, 640));
        assert_eq!(fit_aspect(1080, 1920, budget), (576, 1024));
        assert_eq!(fit_aspect(1024, 1024, budget), (768, 768));
        assert_eq!(fit_aspect(1216, 832, budget), (1216, 832));
    }

    #[test]
    fn fit_aspect_returns_valid_slider_sizes() {
        let budget = (768 * 768) as f64;
        for (width, height) in [
            (1000, 667),
            (1234, 567),
            (200, 150),
            (3000, 1000),
            (1, 1),
            (0, 0),
        ] {
            let (fit_width, fit_height) = fit_aspect(width, height, budget);
            assert!(is_valid(fit_width));
            assert!(is_valid(fit_height));
        }
    }
}
