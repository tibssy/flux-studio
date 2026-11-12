use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use flux_core::preset::{PRESETS, preset};
use flux_core::style::STYLES;
use gtk::{gdk, glib};

use crate::history::Generation;
use crate::{Ui, refresh_model, set_size, thumb_path, update_image_view};

pub(crate) fn add_row(ui: &Rc<Ui>, entry: &Generation, prepend: bool) {
    let row = gtk::ListBoxRow::new();
    // Row activation resolves entries by id, never by position.
    row.set_widget_name(&entry.id.to_string());

    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row_box.set_margin_top(6);
    row_box.set_margin_bottom(6);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);

    let thumbnail = gtk::Picture::builder()
        .width_request(56)
        .height_request(56)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Cover)
        .build();
    thumbnail.add_css_class("card");
    let thumb = thumb_path(entry.id);
    if thumb.exists()
        && let Ok(texture) = gdk::Texture::from_filename(&thumb)
    {
        thumbnail.set_paintable(Some(&texture));
    }

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let prompt_label = gtk::Label::builder()
        .label(&entry.prompt)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(18)
        .build();
    let meta_label = gtk::Label::builder()
        .label(format!(
            "{} · {}×{}",
            model_label(&entry.model),
            entry.width,
            entry.height
        ))
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(22)
        .css_classes(["caption", "dim-label"])
        .build();
    text.append(&prompt_label);
    text.append(&meta_label);

    let actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
    actions.set_valign(gtk::Align::Center);
    let info_button = gtk::Button::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text("Details")
        .css_classes(["flat"])
        .build();
    let delete_button = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Delete image")
        .css_classes(["flat"])
        .build();
    actions.append(&info_button);
    actions.append(&delete_button);

    row_box.append(&thumbnail);
    row_box.append(&text);
    row_box.append(&actions);
    row.set_child(Some(&row_box));

    row.set_tooltip_text(Some(&format!(
        "{}\n{} · {} × {} · {} steps · {:.1}s · seed {}\nstyle: {}{}",
        entry.prompt,
        model_label(&entry.model),
        entry.width,
        entry.height,
        entry.steps,
        entry.duration_ms as f64 / 1000.0,
        entry.seed,
        style_label(&entry.style),
        entry
            .reference_path
            .as_deref()
            .map(|path| format!("\nreference: {path}"))
            .unwrap_or_default()
    )));

    {
        let ui = ui.clone();
        let entry = entry.clone();
        info_button.connect_clicked(move |_| show_info(&ui, &entry));
    }
    {
        let ui = ui.clone();
        let entry = entry.clone();
        delete_button.connect_clicked(move |_| confirm_delete(&ui, &entry));
    }

    if prepend {
        ui.history_list.prepend(&row);
    } else {
        ui.history_list.append(&row);
    }
}

/// Load a history entry as the active image and restore its settings.
pub(crate) fn load_entry(ui: &Rc<Ui>, entry: &Generation) {
    if ui.generating.get() {
        return;
    }

    ui.prompt.buffer().set_text(&entry.prompt);
    ui.seed_entry.set_text(&entry.seed);

    if let Some(index) = PRESETS.iter().position(|preset| preset.id == entry.model) {
        ui.model_dropdown.set_selected(index as u32);
    }
    if let Some(index) = STYLES.iter().position(|style| style.id == entry.style) {
        ui.style_dropdown.set_selected(index as u32);
    }

    set_size(ui, entry.width, entry.height);

    ui.state.borrow_mut().load(PathBuf::from(&entry.image_path));
    update_image_view(ui);
    refresh_model(ui);

    let name = Path::new(&entry.image_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.image_path.clone());
    ui.status.set_text(&format!("Editing reference · {name}"));
}

fn show_info(ui: &Rc<Ui>, entry: &Generation) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(4);
    content.set_margin_bottom(4);
    content.set_margin_start(4);
    content.set_margin_end(4);

    content.append(&kv("Prompt", &entry.prompt));
    content.append(&kv("Model", model_label(&entry.model)));
    content.append(&kv("Style", style_label(&entry.style)));
    content.append(&kv("Effective prompt", &effective_prompt(entry)));
    content.append(&kv(
        "Size",
        &format!("{} × {} px", entry.width, entry.height),
    ));
    content.append(&kv(
        "Sampling",
        &format!(
            "{} steps · CFG {:.1} · {}",
            entry.steps,
            preset(&entry.model).map(|preset| preset.cfg).unwrap_or(1.0),
            preset(&entry.model)
                .map(|preset| preset.sampler.as_str())
                .unwrap_or("euler")
        ),
    ));
    content.append(&kv("Seed", &entry.seed));
    content.append(&kv(
        "Duration",
        &format!("{:.1} s", entry.duration_ms as f64 / 1000.0),
    ));
    content.append(&kv("Created", &created_label(entry.created_at)));
    if let Some(reference) = entry.reference_path.as_deref() {
        content.append(&kv("Reference", reference));
    }
    content.append(&kv("File", &entry.image_path));

    let scroller = gtk::ScrolledWindow::builder()
        .child(&content)
        .min_content_width(760)
        .max_content_height(380)
        .propagate_natural_height(true)
        .build();

    let dialog = adw::AlertDialog::builder()
        .heading("Image details")
        .content_width(880)
        .extra_child(&scroller)
        .build();
    dialog.add_response("close", "Close");
    dialog.add_response("copy", "Copy prompt");
    dialog.add_response("use", "Use as reference");
    dialog.set_response_appearance("use", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));

    {
        let ui = ui.clone();
        let entry = entry.clone();
        dialog.connect_response(None, move |_, response| match response {
            "copy" => {
                ui.window.clipboard().set_text(&entry.prompt);
                ui.toast.add_toast(adw::Toast::new("Prompt copied."));
            }
            "use" => load_entry(&ui, &entry),
            _ => {}
        });
    }

    dialog.present(Some(&ui.window));
}

fn confirm_delete(ui: &Rc<Ui>, entry: &Generation) {
    let dialog = adw::AlertDialog::builder()
        .heading("Delete image?")
        .body(format!(
            "This permanently removes “{}” and its history entry.",
            snippet(&entry.prompt)
        ))
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));

    {
        let ui = ui.clone();
        let entry = entry.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                delete_entry(&ui, &entry);
            }
        });
    }

    dialog.present(Some(&ui.window));
}

fn delete_entry(ui: &Rc<Ui>, entry: &Generation) {
    if ui.generating.get() {
        return;
    }
    if let Err(error) = ui.history.borrow().delete(entry.id) {
        ui.toast
            .add_toast(adw::Toast::new(&format!("Could not delete: {error}")));
        return;
    }

    if let Err(error) = std::fs::remove_file(&entry.image_path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = entry.image_path, "failed to remove image");
    }
    let _ = std::fs::remove_file(thumb_path(entry.id));

    ui.entries.borrow_mut().retain(|item| item.id != entry.id);
    if let Some(row) = row_by_id(&ui.history_list, entry.id) {
        ui.history_list.remove(&row);
    }

    let path = PathBuf::from(&entry.image_path);
    let is_active = {
        let state = ui.state.borrow();
        state.display.as_deref() == Some(path.as_path())
            || state.reference.as_deref() == Some(path.as_path())
    };
    if is_active {
        ui.state.borrow_mut().new_image();
        update_image_view(ui);
        ui.status.set_text("Image deleted");
    }

    ui.toast.add_toast(adw::Toast::new("Image deleted."));
}

fn row_by_id(list: &gtk::ListBox, id: i64) -> Option<gtk::ListBoxRow> {
    let wanted = id.to_string();
    let mut child = list.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
            && row.widget_name() == wanted
        {
            return Some(row);
        }
        child = next;
    }
    None
}

fn kv(title: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let title = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(["caption", "dim-label"])
        .build();
    let value = gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    row.append(&title);
    row.append(&value);
    row
}

fn effective_prompt(entry: &Generation) -> String {
    flux_core::style::compose_prompt(&entry.prompt, &entry.style)
}

fn created_label(created_at: i64) -> String {
    glib::DateTime::from_unix_local(created_at)
        .and_then(|date| date.format("%Y-%m-%d %H:%M"))
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn snippet(prompt: &str) -> String {
    let mut text: String = prompt.chars().take(60).collect();
    if prompt.chars().count() > 60 {
        text.push('…');
    }
    text
}

fn model_label(id: &str) -> &str {
    preset(id).map(|model| model.label).unwrap_or(id)
}

fn style_label(id: &str) -> &str {
    flux_core::style::label(id)
}
