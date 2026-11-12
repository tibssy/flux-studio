//! Resolution limits shared by the UI and the backend.

/// Smallest side the UI offers.
pub const MIN_SIZE: u32 = 256;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_the_grid() {
        assert_eq!(snap(0), MIN_SIZE);
        assert_eq!(snap(250), MIN_SIZE);
        assert_eq!(snap(320), 320);
        assert_eq!(snap(830), 832);
        assert_eq!(snap(2000), MAX_SIZE);
    }

    #[test]
    fn validates_range_and_step() {
        assert!(is_valid(256));
        assert!(is_valid(1280));
        assert!(!is_valid(250));
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
}
