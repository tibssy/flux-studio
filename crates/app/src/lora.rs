//! LoRA selection dialog.
//!
//! Weights live in a folder next to the models (`models/loras` by default).
//! The server resolves LoRA paths relative to the folder it was started with,
//! so the dialog only stores relative paths and weights in the config.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use flux_core::config::LoraSelection;

use crate::Ui;

/// Update the sidebar summary after the selection changes.
pub(crate) fn update_summary(ui: &Rc<Ui>) {
    let count = ui.config.borrow().loras.len();
    ui.lora_summary.set_text(&match count {
        0 => "None active".to_string(),
        1 => "1 active".to_string(),
        count => format!("{count} active"),
    });
    ui.lora_button.set_tooltip_text(Some(&match count {
        0 => "Apply LoRA weights to generations".to_string(),
        1 => "1 LoRA active".to_string(),
        count => format!("{count} LoRAs active"),
    }));
}

pub(crate) fn show_dialog(ui: &Rc<Ui>) {
    let dialog = adw::Dialog::builder()
        .title("LoRAs")
        .content_width(620)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "LoRAs",
        "Applied to every generation and effect",
    )));
    toolbar.add_top_bar(&header);

    let page = adw::PreferencesPage::new();

    let folder = ui.lora_dir.clone();
    if let Err(error) = std::fs::create_dir_all(&folder) {
        tracing::warn!(%error, path = %folder.display(), "failed to create the LoRA folder");
    }

    let folder_group = adw::PreferencesGroup::builder().title("Folder").build();
    let folder_row = adw::ActionRow::builder()
        .title("LoRA folder")
        .subtitle(folder.display().to_string())
        .build();
    let open = gtk::Button::builder()
        .label("Open")
        .valign(gtk::Align::Center)
        .tooltip_text("Open the LoRA folder")
        .build();
    {
        let folder = folder.clone();
        open.connect_clicked(move |_| open_folder(&folder));
    }
    folder_row.add_suffix(&open);
    folder_group.add(&folder_row);
    page.add(&folder_group);

    let weights = adw::PreferencesGroup::builder()
        .title("Weights")
        .description("LoRAs are model-specific; use weights trained for the active model family.")
        .build();
    page.add(&weights);
    let rows: Rc<std::cell::RefCell<Vec<gtk::Widget>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));

    let rescan = gtk::Button::builder()
        .label("Rescan folder")
        .halign(gtk::Align::Start)
        .css_classes(["flat"])
        .build();
    {
        let ui = ui.clone();
        let weights = weights.clone();
        let folder = folder.clone();
        let rows = rows.clone();
        rescan.connect_clicked(move |_| populate(&ui, &weights, &folder, &rows));
    }
    folder_row.add_suffix(&rescan);

    populate(ui, &weights, &folder, &rows);

    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(&ui.window));
}

fn populate(
    ui: &Rc<Ui>,
    group: &adw::PreferencesGroup,
    folder: &Path,
    rows: &Rc<std::cell::RefCell<Vec<gtk::Widget>>>,
) {
    for row in rows.borrow_mut().drain(..) {
        group.remove(&row);
    }

    let mut files = Vec::new();
    collect(folder, &mut files);
    files.sort();

    if files.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No LoRAs found")
            .subtitle(format!(
                "Put .safetensors (or .ckpt/.gguf) files in {} and press Rescan.",
                folder.display()
            ))
            .build();
        group.add(&row);
        rows.borrow_mut().push(row.upcast());
        update_summary(ui);
        return;
    }

    for path in files {
        let relative = path
            .strip_prefix(folder)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let selected = ui
            .config
            .borrow()
            .loras
            .iter()
            .find(|lora| lora.path == relative)
            .cloned();
        let active = selected.is_some();
        let weight_value = selected.map(|lora| lora.weight).unwrap_or(1.0);

        let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| relative.clone());
        let row = adw::ActionRow::builder()
            .title(&file_name)
            .subtitle(format!("{} · {}", relative, size_label(size)))
            .build();

        let weight = gtk::SpinButton::with_range(0.0, 2.0, 0.05);
        weight.set_digits(2);
        weight.set_width_chars(5);
        weight.set_value(weight_value as f64);
        weight.set_sensitive(active);
        weight.set_tooltip_text(Some("LoRA weight"));

        let switch = gtk::Switch::new();
        switch.set_active(active);
        switch.set_valign(gtk::Align::Center);
        switch.set_tooltip_text(Some(if active {
            "Disable this LoRA"
        } else {
            "Enable this LoRA"
        }));

        let suffix = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        suffix.set_valign(gtk::Align::Center);
        suffix.append(&weight);
        suffix.append(&switch);
        row.add_suffix(&suffix);
        group.add(&row);
        rows.borrow_mut().push(row.clone().upcast());

        let guard = Rc::new(Cell::new(false));

        {
            let ui = ui.clone();
            let relative = relative.clone();
            let weight = weight.clone();
            let guard = guard.clone();
            switch.connect_active_notify(move |switch| {
                if guard.get() {
                    return;
                }
                let active = switch.is_active();
                weight.set_sensitive(active);
                let enabled_weight = weight.value() as f32;
                let saved = if active {
                    set_lora(&ui, &relative, Some(enabled_weight))
                } else {
                    set_lora(&ui, &relative, None)
                };
                update_summary(&ui);
                if !saved {
                    guard.set(true);
                    switch.set_active(!active);
                    guard.set(false);
                }
            });
        }

        {
            let ui = ui.clone();
            let relative = relative.clone();
            let switch = switch.clone();
            weight.connect_value_changed(move |weight| {
                if !switch.is_active() {
                    return;
                }
                if set_lora(&ui, &relative, Some(weight.value() as f32)) {
                    update_summary(&ui);
                }
            });
        }
    }

    update_summary(ui);
}

fn set_lora(ui: &Rc<Ui>, path: &str, weight: Option<f32>) -> bool {
    let mut config = ui.config.borrow_mut();
    let previous = config.loras.clone();
    config.loras.retain(|lora| lora.path != path);
    if let Some(weight) = weight {
        config.loras.push(LoraSelection {
            path: path.to_string(),
            weight: weight.clamp(0.0, 2.0),
        });
    }
    match config.save() {
        Ok(()) => true,
        Err(error) => {
            config.loras = previous;
            tracing::warn!(%error, "failed to save the LoRA selection");
            ui.toast
                .add_toast(adw::Toast::new("Could not save the LoRA selection."));
            false
        }
    }
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(
            extension.as_str(),
            "safetensors" | "ckpt" | "sft" | "gguf" | "pt" | "pth"
        ) {
            files.push(path);
        }
    }
}

fn open_folder(folder: &Path) {
    if let Err(error) = std::fs::create_dir_all(folder) {
        tracing::warn!(%error, path = %folder.display(), "failed to create the LoRA folder");
        return;
    }
    let file = gtk::gio::File::for_path(folder);
    if let Err(error) =
        gtk::gio::AppInfo::launch_default_for_uri(&file.uri(), None::<&gtk::gio::AppLaunchContext>)
    {
        tracing::warn!(%error, path = %folder.display(), "failed to open the LoRA folder");
    }
}

fn size_label(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}
