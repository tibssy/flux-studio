use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use flux_core::preset::{ModelPreset, PRESETS};
use gtk::glib;

use crate::{Ui, refresh_model};

struct PresetRow {
    preset: &'static ModelPreset,
    row: adw::ActionRow,
    button: gtk::Button,
}

fn format_bytes(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

fn preset_status(preset: &ModelPreset, models_dir: &Path) -> String {
    let missing = preset.missing(models_dir);
    if missing.is_empty() {
        format!(
            "Installed · {} steps · {}",
            preset.steps,
            format_bytes(preset.total_bytes())
        )
    } else {
        format!(
            "Missing {} file(s) · {} · {} steps",
            missing.len(),
            format_bytes(preset.total_bytes()),
            preset.steps
        )
    }
}

pub(crate) fn show_preferences(ui: &Rc<Ui>, focus_models: bool) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Preferences")
        .search_enabled(false)
        .build();

    let general = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("emblem-system-symbolic")
        .build();

    let output_group = adw::PreferencesGroup::builder().title("Output").build();
    let output_row = adw::ActionRow::builder()
        .title("Output folder")
        .subtitle(ui.config.borrow().output_dir.display().to_string())
        .build();
    let output_button = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    output_row.add_suffix(&output_button);
    output_group.add(&output_row);
    general.add(&output_group);
    {
        let ui = ui.clone();
        let row = output_row.clone();
        output_button.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Choose output folder")
                .modal(true)
                .build();
            let window = ui.window.clone();
            let ui = ui.clone();
            let row = row.clone();
            dialog.select_folder(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
                let Ok(folder) = result else {
                    return;
                };
                let Some(path) = folder.path() else {
                    return;
                };
                {
                    let mut config = ui.config.borrow_mut();
                    config.output_dir = path.clone();
                    let _ = config.save();
                }
                row.set_subtitle(&path.display().to_string());
            });
        });
    }

    let performance_group = adw::PreferencesGroup::builder()
        .title("Performance")
        .build();
    let offload_row = adw::SwitchRow::builder()
        .title("Offload weights to CPU")
        .subtitle("Keeps VRAM under ~5 GB at the cost of speed")
        .build();
    offload_row.set_active(ui.config.borrow().offload);
    {
        let ui = ui.clone();
        offload_row.connect_active_notify(move |row| {
            let active = row.is_active();
            {
                let mut config = ui.config.borrow_mut();
                if config.offload != active {
                    config.offload = active;
                    let _ = config.save();
                }
            }
            if ui.offload_switch.is_active() != active {
                ui.offload_switch.set_active(active);
            }
        });
    }
    performance_group.add(&offload_row);
    general.add(&performance_group);
    dialog.add(&general);

    let models_page = adw::PreferencesPage::builder()
        .title("Models")
        .icon_name("folder-download-symbolic")
        .build();

    let location_group = adw::PreferencesGroup::builder().title("Location").build();
    let location_row = adw::ActionRow::builder()
        .title("Models folder")
        .subtitle(ui.models_dir.display().to_string())
        .build();
    location_group.add(&location_row);
    models_page.add(&location_group);

    let presets_group = adw::PreferencesGroup::builder()
        .title("Model presets")
        .description("Each preset downloads into the models folder above.")
        .build();
    let rows: Rc<RefCell<Vec<PresetRow>>> = Rc::new(RefCell::new(Vec::new()));
    let progress = gtk::ProgressBar::builder()
        .show_text(true)
        .visible(false)
        .build();
    let download_status = gtk::Label::builder()
        .label("")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();

    for preset in PRESETS {
        let row = adw::ActionRow::builder()
            .title(preset.label)
            .subtitle(preset_status(preset, &ui.models_dir))
            .build();
        let button = gtk::Button::builder()
            .label("Download")
            .valign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        button.set_sensitive(!preset.is_installed(&ui.models_dir));
        row.add_suffix(&button);
        presets_group.add(&row);
        rows.borrow_mut().push(PresetRow {
            preset,
            row,
            button,
        });
    }
    models_page.add(&presets_group);

    let download_group = adw::PreferencesGroup::builder().title("Downloads").build();
    let download_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    download_box.append(&progress);
    download_box.append(&download_status);
    download_group.add(&download_box);
    models_page.add(&download_group);

    let refresh: Rc<dyn Fn()> = {
        let rows = rows.clone();
        let models_dir = ui.models_dir.clone();
        Rc::new(move || {
            for entry in rows.borrow().iter() {
                entry
                    .row
                    .set_subtitle(&preset_status(entry.preset, &models_dir));
                let installed = entry.preset.is_installed(&models_dir);
                entry
                    .button
                    .set_label(if installed { "Installed" } else { "Download" });
                entry.button.set_sensitive(!installed);
            }
        })
    };

    let buttons: Vec<gtk::Button> = rows
        .borrow()
        .iter()
        .map(|entry| entry.button.clone())
        .collect();
    for entry in rows.borrow().iter() {
        let ui = ui.clone();
        let preset = entry.preset;
        let progress = progress.clone();
        let download_status = download_status.clone();
        let buttons = buttons.clone();
        let refresh = refresh.clone();
        entry.button.connect_clicked(move |_| {
            start_download(
                &ui,
                preset,
                &progress,
                &download_status,
                &buttons,
                refresh.clone(),
            );
        });
    }

    dialog.add(&models_page);

    let about = adw::PreferencesPage::builder()
        .title("About")
        .icon_name("help-about-symbolic")
        .build();
    let about_group = adw::PreferencesGroup::new();
    about_group.add(
        &adw::ActionRow::builder()
            .title(flux_core::APP_NAME)
            .subtitle(concat!("Version ", env!("CARGO_PKG_VERSION")))
            .build(),
    );
    about_group.add(
        &adw::ActionRow::builder()
            .title("Backend")
            .subtitle("stable-diffusion.cpp · CUDA")
            .build(),
    );
    about_group.add(
        &adw::ActionRow::builder()
            .title("License")
            .subtitle("GPL-3.0-or-later")
            .build(),
    );
    about.add(&about_group);
    dialog.add(&about);

    if focus_models {
        dialog.set_visible_page(&models_page);
    }
    dialog.present(Some(&ui.window));
}

fn start_download(
    ui: &Rc<Ui>,
    preset: &'static ModelPreset,
    progress: &gtk::ProgressBar,
    status: &gtk::Label,
    buttons: &[gtk::Button],
    refresh: Rc<dyn Fn()>,
) {
    let models_dir = ui.models_dir.clone();
    let missing: Vec<_> = preset
        .files
        .iter()
        .filter(|file| !models_dir.join(file.rel_path).exists())
        .collect();
    if missing.is_empty() {
        return;
    }
    let sizes: Vec<u64> = missing.iter().map(|file| file.approx_bytes).collect();
    let total_bytes: u64 = sizes.iter().sum();

    for button in buttons {
        button.set_sensitive(false);
    }
    progress.set_visible(true);
    progress.set_fraction(0.0);
    progress.set_text(Some("Starting…"));
    status.set_text(&format!("Downloading {}…", preset.label));

    let (progress_tx, progress_rx) = async_channel::unbounded::<flux_models::DownloadProgress>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);
    let download_dir = models_dir.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(flux_models::download_preset(
                preset,
                &download_dir,
                progress_tx,
            ))?;
            Ok(())
        })();
        let _ = done_tx.send_blocking(result.map_err(|error| error.to_string()));
    });

    {
        let progress = progress.clone();
        let status = status.clone();
        let sizes = sizes.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = progress_rx.recv().await {
                let before: u64 = sizes
                    .get(..event.file_index.min(sizes.len()))
                    .map(|slice| slice.iter().sum())
                    .unwrap_or(0);
                let fraction = if total_bytes > 0 {
                    (before + event.downloaded) as f64 / total_bytes as f64
                } else {
                    0.0
                };
                progress.set_fraction(fraction.min(1.0));
                progress.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
                status.set_text(&format!(
                    "{} · {:.1} / {:.1} GB (file {} of {})",
                    event.file,
                    event.downloaded as f64 / 1e9,
                    event.total as f64 / 1e9,
                    event.file_index + 1,
                    event.file_count
                ));
            }
        });
    }

    {
        let ui = ui.clone();
        let progress = progress.clone();
        let status = status.clone();
        glib::spawn_future_local(async move {
            let result = done_rx.recv().await;
            progress.set_visible(false);
            match result {
                Ok(Ok(())) => {
                    status.set_text(&format!("{} is ready.", preset.label));
                    ui.toast.add_toast(adw::Toast::new(&format!(
                        "{} models downloaded.",
                        preset.label
                    )));
                }
                Ok(Err(error)) => {
                    status.set_text(&format!("Download failed: {error}"));
                    ui.toast
                        .add_toast(adw::Toast::new(&format!("Download failed: {error}")));
                }
                Err(_) => {}
            }
            refresh();
            refresh_model(&ui);
        });
    }
}
