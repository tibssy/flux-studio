//! LoRA selection dialog.
//!
//! Weights live in a folder next to the models (`models/loras` by default).
//! The server resolves LoRA paths relative to the folder it was started with,
//! so the dialog only stores relative paths and weights in the config.

use std::cell::Cell;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use imagiflume_core::config::LoraSelection;

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

    let mut disabled = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(folder)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let validation = validate(folder, &relative);
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
        let subtitle = match &validation {
            Ok(()) => format!("{} · {}", relative, size_label(size)),
            Err(reason) => format!("{} · {} · {reason}", relative, size_label(size)),
        };
        let row = adw::ActionRow::builder()
            .title(&file_name)
            .subtitle(subtitle)
            .build();
        if let Err(reason) = &validation {
            let warning = gtk::Image::from_icon_name("dialog-warning-symbolic");
            warning.set_tooltip_text(Some(reason));
            row.add_prefix(&warning);
        }

        // A weight that cannot load must not stay active: the server would
        // silently generate without it.
        let usable = validation.is_ok();
        if !usable && active {
            set_lora(ui, &relative, None);
            disabled.push(file_name.clone());
        }
        let active = active && usable;

        let weight = gtk::SpinButton::with_range(0.0, 2.0, 0.05);
        weight.set_digits(2);
        weight.set_width_chars(5);
        weight.set_value(weight_value as f64);
        weight.set_sensitive(active);
        weight.set_tooltip_text(Some("LoRA weight"));

        let switch = gtk::Switch::new();
        switch.set_active(active);
        switch.set_valign(gtk::Align::Center);
        switch.set_sensitive(usable);
        switch.set_tooltip_text(Some(if !usable {
            "Fix or replace this file before enabling it"
        } else if active {
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

    if !disabled.is_empty() {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "Disabled {}: the file cannot load. Replace it and rescan.",
            disabled.join(", ")
        )));
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

/// Check that a weight can plausibly load before sending it to the server.
///
/// stable-diffusion.cpp treats a LoRA it cannot register as a warning and
/// keeps generating without it, so a broken file otherwise looks like a
/// LoRA that simply had no effect. Safetensors headers are cheap to inspect:
/// they declare every tensor's byte range, which catches truncated downloads.
pub(crate) fn validate(folder: &Path, relative: &str) -> Result<(), String> {
    let path = folder.join(relative);
    if !path.is_file() {
        return Err("File not found".into());
    }
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "safetensors" | "sft" => validate_safetensors(&path),
        "gguf" => validate_gguf(&path),
        // Torch checkpoints are opaque without a torch reader; let the
        // server be the judge instead of rejecting every one of them.
        _ => Ok(()),
    }
}

fn validate_safetensors(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| format!("Cannot read file: {error}"))?;
    let file_len = file
        .metadata()
        .map_err(|error| format!("Cannot read file: {error}"))?
        .len();
    let mut length = [0u8; 8];
    file.read_exact(&mut length)
        .map_err(|_| "Not a valid safetensors file".to_string())?;
    let header_len = u64::from_le_bytes(length);
    const MAX_HEADER: u64 = 64 * 1024 * 1024;
    if header_len == 0 || header_len > MAX_HEADER || header_len.saturating_add(8) > file_len {
        return Err("Not a valid safetensors file".into());
    }
    let mut header = vec![0u8; header_len as usize];
    file.read_exact(&mut header)
        .map_err(|_| "Not a valid safetensors file".to_string())?;
    let parsed: serde_json::Value =
        serde_json::from_slice(&header).map_err(|_| "Not a valid safetensors file".to_string())?;
    let Some(tensors) = parsed.as_object() else {
        return Err("Not a valid safetensors file".into());
    };

    let data_len = file_len - 8 - header_len;
    let mut declared = 0u64;
    let mut looks_like_lora = false;
    for (name, tensor) in tensors {
        if name == "__metadata__" {
            continue;
        }
        if name.to_ascii_lowercase().contains("lora") {
            looks_like_lora = true;
        }
        let offsets = tensor
            .get("data_offsets")
            .and_then(|value| value.as_array())
            .filter(|offsets| offsets.len() == 2)
            .and_then(|offsets| Some((offsets[0].as_u64()?, offsets[1].as_u64()?)));
        let Some((start, end)) = offsets else {
            return Err("Not a valid safetensors file".into());
        };
        if start > end {
            return Err("Not a valid safetensors file".into());
        }
        declared = declared.max(end);
    }
    if !looks_like_lora {
        return Err("No LoRA tensors found".into());
    }
    if declared > data_len {
        let expected = file_len - data_len + declared;
        return Err(format!(
            "Incomplete file ({} of {})",
            size_label(file_len),
            size_label(expected)
        ));
    }
    Ok(())
}

fn validate_gguf(path: &Path) -> Result<(), String> {
    let mut magic = [0u8; 4];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .map_err(|_| "Not a valid GGUF file".to_string())?;
    if &magic == b"GGUF" {
        Ok(())
    } else {
        Err("Not a valid GGUF file".into())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "flux-lora-validation-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn lora_header() -> serde_json::Value {
        serde_json::json!({
            "lora_unet_double_blocks_0.lora_A.weight": {
                "dtype": "F32", "shape": [1, 4], "data_offsets": [0, 4]
            },
            "lora_unet_double_blocks_0.lora_B.weight": {
                "dtype": "F32", "shape": [4, 1], "data_offsets": [4, 8]
            }
        })
    }

    fn write_safetensors(path: &Path, header: &serde_json::Value, data: &[u8]) {
        let header = serde_json::to_vec(header).unwrap();
        let mut file = File::create(path).unwrap();
        file.write_all(&(header.len() as u64).to_le_bytes())
            .unwrap();
        file.write_all(&header).unwrap();
        file.write_all(data).unwrap();
    }

    #[test]
    fn accepts_a_complete_lora() {
        let dir = test_dir("complete");
        write_safetensors(&dir.join("ok.safetensors"), &lora_header(), &[0u8; 8]);
        assert_eq!(validate(&dir, "ok.safetensors"), Ok(()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_a_truncated_lora() {
        let dir = test_dir("truncated");
        write_safetensors(&dir.join("short.safetensors"), &lora_header(), &[0u8; 4]);
        let error = validate(&dir, "short.safetensors").unwrap_err();
        assert!(error.starts_with("Incomplete file"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_a_file_without_lora_tensors() {
        let dir = test_dir("plain");
        let header = serde_json::json!({
            "model.weight": {"dtype": "F32", "shape": [1], "data_offsets": [0, 4]}
        });
        write_safetensors(&dir.join("plain.safetensors"), &header, &[0u8; 4]);
        assert_eq!(
            validate(&dir, "plain.safetensors"),
            Err("No LoRA tensors found".into())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_a_missing_file() {
        let dir = test_dir("missing");
        assert_eq!(
            validate(&dir, "nope.safetensors"),
            Err("File not found".into())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn checks_the_gguf_magic() {
        let dir = test_dir("gguf");
        std::fs::write(dir.join("good.gguf"), b"GGUF\x03\x00\x00\x00").unwrap();
        std::fs::write(dir.join("bad.gguf"), b"NOPE").unwrap();
        assert_eq!(validate(&dir, "good.gguf"), Ok(()));
        assert_eq!(
            validate(&dir, "bad.gguf"),
            Err("Not a valid GGUF file".into())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn accepts_opaque_torch_checkpoints() {
        let dir = test_dir("checkpoint");
        std::fs::write(dir.join("weights.pt"), b"not really a torch file").unwrap();
        assert_eq!(validate(&dir, "weights.pt"), Ok(()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
