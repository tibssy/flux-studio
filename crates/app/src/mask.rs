//! Mask painting and rasterisation for inpainting.
//!
//! Strokes are stored in image coordinates so resizing the window or switching
//! zoom does not change the mask. The overlay draws them translated to screen
//! coordinates; applying the mask rasterises them into a grayscale PNG where
//! white means "repaint".

use std::path::Path;

use anyhow::{Context, Result};
use gtk::cairo;

/// One continuous brush stroke in image coordinates.
#[derive(Debug, Clone)]
pub(crate) struct MaskStroke {
    pub points: Vec<(f64, f64)>,
    pub radius: f64,
}

/// Rasterise strokes into a grayscale mask PNG.
pub(crate) fn render_mask(
    width: u32,
    height: u32,
    strokes: &[MaskStroke],
    path: &Path,
) -> Result<()> {
    if width == 0 || height == 0 {
        anyhow::bail!("the image has no size");
    }
    let mut surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, width as i32, height as i32)
            .context("failed to create the mask surface")?;
    {
        let cr = cairo::Context::new(&surface).context("failed to draw the mask")?;
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.paint().context("failed to clear the mask")?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        for stroke in strokes {
            draw_stroke(&cr, stroke);
        }
    }
    surface.flush();

    let mut mask = image::GrayImage::new(width, height);
    {
        let stride = surface.stride() as usize;
        let data = surface.data().context("failed to read the mask")?;
        for y in 0..height {
            for x in 0..width {
                let offset = y as usize * stride + x as usize * 4;
                // Cairo ARGB32 on little-endian stores B, G, R, A.
                mask.put_pixel(x, y, image::Luma([data[offset + 2]]));
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    mask.save(path)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn draw_stroke(cr: &cairo::Context, stroke: &MaskStroke) {
    let radius = stroke.radius.max(0.5);
    match stroke.points.as_slice() {
        [] => {}
        [(x, y)] => {
            cr.arc(*x, *y, radius, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        [(x, y), rest @ ..] => {
            cr.set_line_width(radius * 2.0);
            cr.move_to(*x, *y);
            for (x, y) in rest {
                cr.line_to(*x, *y);
            }
            let _ = cr.stroke();
        }
    }
}

/// Draw the strokes translated to the area that contains the image.
pub(crate) fn draw_overlay(
    cr: &cairo::Context,
    area_width: f64,
    area_height: f64,
    image_size: (u32, u32),
    strokes: &[MaskStroke],
) {
    let (x, y, width, height) = crate::contain_rect(
        (area_width, area_height),
        (image_size.0 as f64, image_size.1 as f64),
    );
    if width <= 0.0 || height <= 0.0 || image_size.0 == 0 {
        return;
    }
    let scale = width / image_size.0 as f64;
    cr.set_source_rgba(1.0, 0.15, 0.2, 0.45);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    for stroke in strokes {
        let radius = (stroke.radius * scale).max(0.5);
        match stroke.points.as_slice() {
            [] => {}
            [(px, py)] => {
                cr.arc(
                    x + px * scale,
                    y + py * scale,
                    radius,
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = cr.fill();
            }
            [(px, py), rest @ ..] => {
                cr.set_line_width(radius * 2.0);
                cr.move_to(x + px * scale, y + py * scale);
                for (px, py) in rest {
                    cr.line_to(x + px * scale, y + py * scale);
                }
                let _ = cr.stroke();
            }
        }
    }
}

/// Map a screen point inside the overlay to image coordinates.
pub(crate) fn point_to_image(
    area_width: f64,
    area_height: f64,
    image_size: (u32, u32),
    x: f64,
    y: f64,
) -> Option<(f64, f64)> {
    if image_size.0 == 0 || image_size.1 == 0 {
        return None;
    }
    let (rect_x, rect_y, rect_width, rect_height) = crate::contain_rect(
        (area_width, area_height),
        (image_size.0 as f64, image_size.1 as f64),
    );
    if rect_width <= 0.0 || rect_height <= 0.0 {
        return None;
    }
    let scale = rect_width / image_size.0 as f64;
    let image_x = (x - rect_x) / scale;
    let image_y = (y - rect_y) / scale;
    if image_x < 0.0
        || image_y < 0.0
        || image_x > image_size.0 as f64
        || image_y > image_size.1 as f64
    {
        return None;
    }
    Some((image_x, image_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterises_a_stroke_into_a_mask() {
        let path = std::env::temp_dir().join(format!("flux-mask-{}.png", std::process::id()));
        let strokes = vec![MaskStroke {
            points: vec![(20.0, 20.0), (40.0, 20.0)],
            radius: 6.0,
        }];
        render_mask(64, 64, &strokes, &path).unwrap();
        let mask = image::open(&path).unwrap().to_luma8();
        assert_eq!(mask.get_pixel(30, 20).0[0], 255);
        assert_eq!(mask.get_pixel(2, 2).0[0], 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn maps_points_through_the_letterbox() {
        // A 100x50 image in a 200x200 area: rendered at 200x100, y offset 50.
        assert_eq!(
            point_to_image(200.0, 200.0, (100, 50), 100.0, 100.0),
            Some((50.0, 25.0))
        );
        assert!(point_to_image(200.0, 200.0, (100, 50), 10.0, 10.0).is_none());
    }
}
