//! Manual brightness, contrast and saturation adjustments for a finished image.
//!
//! The popover previews the result on the canvas without touching the file;
//! only "Save copy" writes a new PNG and adds it to the history chain.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::gdk;
use gtk::glib;
use imagiflume_core::adjust::Adjustments;

use crate::history::{Generation, NewGeneration};
use crate::rounded_paintable::RoundedPaintable;
use crate::{
    Ui, active_preset, annotate_image, fade_to_image, history_view, now_millis, reload_history,
    slugify, unix_seconds, write_thumbnail,
};

/// Decoded source pixels plus the texture that was on screen before previewing.
struct Preview {
    base: image::RgbaImage,
    source: PathBuf,
    texture: gdk::Texture,
    revision: Cell<u64>,
    saved: Cell<bool>,
}

pub(crate) fn show(ui: &Rc<Ui>) {
    if (ui.generating.get() && !ui.upscaling.get()) || ui.zooming.get() || ui.classifying.get() {
        return;
    }
    let Some(source) = ui.state.borrow().display.clone() else {
        ui.toast.add_toast(adw::Toast::new("No image to adjust."));
        return;
    };
    let base = match image::open(&source) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            tracing::warn!(%error, path = %source.display(), "failed to open image for adjustment");
            ui.toast
                .add_toast(adw::Toast::new("Could not read this image."));
            return;
        }
    };
    let texture = match gdk::Texture::from_filename(&source) {
        Ok(texture) => texture,
        Err(error) => {
            tracing::warn!(%error, path = %source.display(), "failed to load texture");
            ui.toast
                .add_toast(adw::Toast::new("Could not read this image."));
            return;
        }
    };
    let (width, height) = base.dimensions();
    let preview = Rc::new(Preview {
        base,
        source,
        texture,
        revision: Cell::new(0),
        saved: Cell::new(false),
    });

    let popover = gtk::Popover::new();
    popover.add_css_class("adjust-popover");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_size_request(340, -1);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(14);
    content.set_margin_end(14);

    let heading = gtk::Label::builder()
        .label(format!("Adjust colors · {width} × {height}"))
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["heading"])
        .build();
    let reset = gtk::Button::with_label("Reset");
    reset.add_css_class("flat");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.append(&heading);
    header.append(&reset);
    content.append(&header);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let (brightness_row, brightness_scale, brightness_value) =
        slider_row("Brightness", -100.0, 100.0, 0.0, format_signed);
    let (contrast_row, contrast_scale, contrast_value) =
        slider_row("Contrast", 50.0, 150.0, 100.0, format_percent);
    let (saturation_row, saturation_scale, saturation_value) =
        slider_row("Saturation", 0.0, 200.0, 100.0, format_percent);
    content.append(&brightness_row);
    content.append(&contrast_row);
    content.append(&saturation_row);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::builder()
        .label("Save copy")
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.set_halign(gtk::Align::End);
    footer.append(&cancel);
    footer.append(&save);
    content.append(&footer);

    popover.set_child(Some(&content));

    let read = {
        let brightness = brightness_scale.clone();
        let contrast = contrast_scale.clone();
        let saturation = saturation_scale.clone();
        move || Adjustments {
            brightness: brightness.value().round() as i32,
            contrast: contrast.value().round() as u32,
            saturation: saturation.value().round() as u32,
        }
    };

    let refresh: Rc<dyn Fn()> = {
        let ui = ui.clone();
        let preview = preview.clone();
        let read = read.clone();
        let brightness_value = brightness_value.clone();
        let contrast_value = contrast_value.clone();
        let saturation_value = saturation_value.clone();
        let brightness = brightness_scale.clone();
        let contrast = contrast_scale.clone();
        let saturation = saturation_scale.clone();
        let save = save.clone();
        Rc::new(move || {
            let adjustments = read();
            brightness_value.set_text(&format_signed(brightness.value()));
            contrast_value.set_text(&format_percent(contrast.value()));
            saturation_value.set_text(&format_percent(saturation.value()));
            save.set_sensitive(!adjustments.is_neutral());

            let revision = preview.revision.get().wrapping_add(1);
            preview.revision.set(revision);
            let ui = ui.clone();
            let preview = preview.clone();
            // Coalesce drag events: only the newest revision is rendered.
            glib::timeout_add_local_once(Duration::from_millis(30), move || {
                if preview.revision.get() == revision {
                    render(&ui, &preview, adjustments);
                }
            });
        })
    };

    for scale in [&brightness_scale, &contrast_scale, &saturation_scale] {
        let refresh = refresh.clone();
        scale.connect_value_changed(move |_| refresh());
    }

    {
        let refresh = refresh.clone();
        let brightness = brightness_scale.clone();
        let contrast = contrast_scale.clone();
        let saturation = saturation_scale.clone();
        reset.connect_clicked(move |_| {
            brightness.set_value(0.0);
            contrast.set_value(100.0);
            saturation.set_value(100.0);
            refresh();
        });
    }
    {
        let popover = popover.clone();
        cancel.connect_clicked(move |_| popover.popdown());
    }
    {
        let ui = ui.clone();
        let popover = popover.clone();
        let preview = preview.clone();
        let read = read.clone();
        save.connect_clicked(move |_| {
            save_adjusted(&ui, &preview, read(), &popover);
        });
    }
    {
        let ui = ui.clone();
        let preview = preview.clone();
        popover.connect_closed(move |popover| {
            if !preview.saved.get() {
                restore(&ui, &preview);
            }
            popover.unparent();
        });
    }

    popover.set_parent(&ui.adjust_button);
    popover.popup();
}

fn slider_row(
    title: &str,
    min: f64,
    max: f64,
    initial: f64,
    format: fn(f64) -> String,
) -> (gtk::Box, gtk::Scale, gtk::Label) {
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .width_chars(9)
        .build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 1.0);
    scale.set_value(initial);
    scale.set_hexpand(true);
    scale.set_draw_value(false);
    let value = gtk::Label::builder()
        .label(format(initial))
        .xalign(1.0)
        .width_chars(5)
        .css_classes(["numeric"])
        .build();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&label);
    row.append(&scale);
    row.append(&value);
    (row, scale, value)
}

fn format_signed(value: f64) -> String {
    let value = value.round() as i32;
    if value == 0 {
        "0".to_string()
    } else {
        format!("{value:+}")
    }
}

fn format_percent(value: f64) -> String {
    format!("{}%", value.round() as i32)
}

fn active_picture(ui: &Ui) -> Option<gtk::Picture> {
    match ui.stack.visible_child_name().as_deref() {
        Some("image") => Some(ui.picture.clone()),
        Some("back") => Some(ui.picture_back.clone()),
        _ => None,
    }
}

/// Show the adjusted pixels on the canvas without writing anything.
fn render(ui: &Ui, preview: &Preview, adjustments: Adjustments) {
    let Some(picture) = active_picture(ui) else {
        return;
    };
    let mut pixels = preview.base.as_raw().clone();
    adjustments.apply(&mut pixels);
    let (width, height) = preview.base.dimensions();
    let bytes = glib::Bytes::from_owned(pixels);
    let texture: gdk::Texture = gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    )
    .upcast();
    picture.set_paintable(Some(&RoundedPaintable::new(&texture)));
}

/// Put the original texture back, but only if the same image is still shown.
fn restore(ui: &Ui, preview: &Preview) {
    let current = ui.state.borrow().display.clone();
    if current.as_deref() != Some(preview.source.as_path()) {
        return;
    }
    if let Some(picture) = active_picture(ui) {
        picture.set_paintable(Some(&RoundedPaintable::new(&preview.texture)));
    }
}

fn save_adjusted(ui: &Rc<Ui>, preview: &Preview, adjustments: Adjustments, popover: &gtk::Popover) {
    if adjustments.is_neutral() {
        return;
    }
    let started = Instant::now();
    let mut pixels = preview.base.as_raw().clone();
    adjustments.apply(&mut pixels);
    let (width, height) = preview.base.dimensions();
    let Some(image) = image::RgbaImage::from_raw(width, height, pixels) else {
        ui.toast
            .add_toast(adw::Toast::new("Could not read this image."));
        return;
    };

    let output_dir = ui.config.borrow().output_dir.clone();
    let stem = preview
        .source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(slugify)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "image".into());
    let output = output_dir.join(format!("{stem}-adjusted-{}.png", now_millis()));
    if let Err(error) = image.save(&output) {
        tracing::error!(%error, path = %output.display(), "failed to save adjustment");
        ui.toast
            .add_toast(adw::Toast::new("Could not save the copy."));
        return;
    }
    let duration_ms = started.elapsed().as_millis() as i64;

    preview.saved.set(true);
    popover.popdown();

    let source_text = preview.source.to_string_lossy().into_owned();
    let image_text = output.to_string_lossy().into_owned();
    let prompt = adjustments.summary();
    let model =
        source_model(ui, &preview.source).unwrap_or_else(|| active_preset(ui).id.to_string());
    let created_at = unix_seconds();

    let inserted = ui.history.borrow().insert(&NewGeneration {
        prompt: &prompt,
        model: &model,
        style: "none",
        width,
        height,
        steps: 0,
        seed: "",
        created_at,
        duration_ms,
        image_path: &image_text,
        reference_path: None,
        reference_paths: &[],
        kind: "adjust",
        source_path: Some(&source_text),
    });

    match inserted {
        Ok((id, parent_id)) => {
            if let Err(error) = write_thumbnail(&output, id) {
                tracing::warn!(%error, "failed to write thumbnail");
            }
            let entry = Generation {
                id,
                prompt: prompt.clone(),
                model,
                style: "none".into(),
                width,
                height,
                steps: 0,
                seed: String::new(),
                created_at,
                duration_ms,
                image_path: image_text,
                reference_path: None,
                reference_paths: Vec::new(),
                kind: "adjust".into(),
                source_path: Some(source_text),
                parent_id,
                favorite: false,
            };
            annotate_image(&output, &entry, 0.0, "", &[]);
            reload_history(ui, false);
            history_view::highlight(ui, &output);
        }
        Err(error) => tracing::error!(%error, "failed to record adjustment"),
    }

    ui.undo.borrow_mut().clear();
    fade_to_image(ui, &output);
    crate::copy_result_to_clipboard(ui, &output);
    ui.toast
        .add_toast(adw::Toast::new("Saved an adjusted copy."));
    ui.status.set_text(&format!(
        "Adjusted · {width} × {height} · {}",
        adjustments.summary()
    ));
}

fn source_model(ui: &Ui, path: &Path) -> Option<String> {
    ui.entries
        .borrow()
        .iter()
        .find(|entry| Path::new(&entry.image_path) == path)
        .map(|entry| entry.model.clone())
}
