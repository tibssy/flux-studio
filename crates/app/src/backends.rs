use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use flux_core::runtime::{self, BackendEntry, BackendManifest, HardwareProfile};
use gtk::{gdk, glib};

use crate::Ui;

const BACKEND_ICON: &str = "application-x-executable-symbolic";

type Refresh = Rc<dyn Fn()>;
type RefreshSlot = Rc<RefCell<Option<Refresh>>>;

/// Install the CPU backend automatically when no usable server exists.
pub(crate) fn auto_install_cpu(ui: &Rc<Ui>) {
    if ui.backend_installing.get() {
        return;
    }
    let manifest = runtime::bundled_manifest();
    let Some(entry) = manifest.cpu_entry().cloned() else {
        return;
    };
    if let Some(reason) = HardwareProfile::detect().compatibility(&entry) {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "The CPU backend is not compatible: {reason}"
        )));
        return;
    }
    ui.status
        .set_text("No backend found; downloading the CPU backend…");
    ui.toast.add_toast(adw::Toast::new(
        "No backend found. Downloading the CPU backend (29 MB)…",
    ));

    let ui_for_progress = ui.clone();
    let on_progress: Rc<dyn Fn(u64, u64)> = Rc::new(move |downloaded, total| {
        let percent = if total > 0 {
            downloaded as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        ui_for_progress
            .status
            .set_text(&format!("Downloading CPU backend… {percent:.0}%"));
    });
    let ui_for_done = ui.clone();
    let entry_for_done = entry.clone();
    let on_done: Rc<dyn Fn(Result<PathBuf, String>)> = Rc::new(move |result| match result {
        Ok(server) => {
            let backend = entry_for_done.backend.clone();
            {
                let mut config = ui_for_done.config.borrow_mut();
                config.sd_server = Some(server.clone());
                config.backend_id = Some(entry_for_done.id.clone());
                config.backend = backend.clone();
                let _ = config.save();
            }
            ui_for_done.backend.set_server(server, backend);
            ui_for_done.backend.unload();
            ui_for_done.status.set_text("Ready · CPU backend installed");
            ui_for_done
                .toast
                .add_toast(adw::Toast::new("CPU backend installed."));
        }
        Err(error) => {
            ui_for_done.status.set_text("Backend installation failed");
            ui_for_done.toast.add_toast(adw::Toast::new(&format!(
                "Could not install the CPU backend: {error}"
            )));
        }
    });

    install_entry(ui, &entry, on_progress, on_done);
}

/// Build the Backends preferences page.
pub(crate) fn build_page(ui: &Rc<Ui>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Backends")
        .icon_name(BACKEND_ICON)
        .build();

    let hardware = Rc::new(HardwareProfile::detect());
    let manifest = Rc::new(RefCell::new(runtime::bundled_manifest()));
    let progress = gtk::ProgressBar::builder()
        .show_text(true)
        .visible(false)
        .build();
    let status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption", "dim-label"])
        .build();

    let hardware_group = adw::PreferencesGroup::builder()
        .title("Detected hardware")
        .description("Used to decide which downloadable backends are compatible.")
        .build();
    hardware_group.add(
        &adw::ActionRow::builder()
            .title("Devices")
            .subtitle(hardware.summary())
            .build(),
    );
    page.add(&hardware_group);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Check the backend manifest (Ctrl+R)")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    {
        let refresh_button = refresh_button.clone();
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        key.connect_key_pressed(move |_, key, _, state| {
            if state.contains(gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gdk::Key::r | gdk::Key::R)
            {
                refresh_button.emit_clicked();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        page.add_controller(key);
    }
    let backends_group = adw::PreferencesGroup::builder()
        .title("Available backends")
        .description(
            "Compatible builds for this system. The refresh button checks the pinned manifest.",
        )
        .header_suffix(&refresh_button)
        .build();
    backends_group.add(&list);
    page.add(&backends_group);

    let download_group = adw::PreferencesGroup::builder().title("Downloads").build();
    let download_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    download_box.append(&progress);
    download_box.append(&status);
    download_group.add(&download_box);
    page.add(&download_group);

    let refresh_slot: RefreshSlot = Rc::new(RefCell::new(None));
    let refresh: Refresh = {
        let slot = refresh_slot.clone();
        Rc::new(move || {
            if let Some(refresh) = slot.borrow().as_ref() {
                refresh();
            }
        })
    };

    let populate: Rc<dyn Fn()> = {
        let ui = ui.clone();
        let manifest = manifest.clone();
        let hardware = hardware.clone();
        let list = list.clone();
        let progress = progress.clone();
        let status = status.clone();
        let refresh = refresh.clone();
        Rc::new(move || {
            rebuild(
                &ui,
                &list,
                &manifest,
                &hardware,
                &progress,
                &status,
                refresh.clone(),
            );
        })
    };
    *refresh_slot.borrow_mut() = Some(populate.clone());
    populate();

    {
        let ui = ui.clone();
        let status = status.clone();
        let manifest = manifest.clone();
        let refresh = refresh.clone();
        refresh_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            status.set_text("Checking the backend manifest…");
            let url = ui
                .config
                .borrow()
                .backend_manifest_url
                .clone()
                .unwrap_or_else(|| runtime::DEFAULT_MANIFEST_URL.to_string());
            let base = manifest.borrow().clone();
            let (tx, rx) = async_channel::bounded::<Result<flux_models::ManifestRefresh, String>>(1);
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<flux_models::ManifestRefresh> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(flux_models::refresh_manifest(&base, &url))
                })();
                let _ = tx.send_blocking(result.map_err(|error| error.to_string()));
            });
            let button = button.clone();
            let ui = ui.clone();
            let status = status.clone();
            let refresh = refresh.clone();
            let manifest = manifest.clone();
            glib::spawn_future_local(async move {
                let result = rx.recv().await;
                button.set_sensitive(true);
                match result {
                    Ok(Ok(outcome)) => {
                        *manifest.borrow_mut() = outcome.manifest;
                        let mut message = if outcome.curated_ok && outcome.upstream_ok {
                            "Backend manifest updated.".to_string()
                        } else if outcome.upstream_ok {
                            "Backend list updated from the latest upstream release.".to_string()
                        } else {
                            "Backend manifest updated; upstream release check unavailable."
                                .to_string()
                        };
                        if !outcome.errors.is_empty() {
                            message.push_str(&format!(
                                " Skipped {} invalid backend entr{}.",
                                outcome.errors.len(),
                                if outcome.errors.len() == 1 { "y" } else { "ies" }
                            ));
                        }
                        status.set_text(&message);
                        ui.toast.add_toast(adw::Toast::new(&message));
                        refresh();
                    }
                    Ok(Err(error)) => {
                        status.set_text(
                            "Remote manifest unavailable; keeping the current backend list.",
                        );
                        ui.toast.add_toast(adw::Toast::new(&format!(
                            "Could not fetch the backend manifest; keeping the pinned list ({error})"
                        )));
                        refresh();
                    }
                    Err(_) => {
                        status.set_text(
                            "Manifest request stopped; keeping the current backend list.",
                        );
                        refresh();
                    }
                }
            });
        });
    }

    page
}

fn rebuild(
    ui: &Rc<Ui>,
    list: &gtk::ListBox,
    manifest: &Rc<RefCell<BackendManifest>>,
    hardware: &HardwareProfile,
    progress: &gtk::ProgressBar,
    status: &gtk::Label,
    refresh: Rc<dyn Fn()>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let system_path = runtime::find_system_server();
    let (active_id, server, custom, custom_active) = {
        let config = ui.config.borrow();
        let mut custom = config.custom_server.clone();
        if custom.is_none()
            && config.backend_id.is_none()
            && config.sd_server.as_deref() != system_path.as_deref()
        {
            custom = config.sd_server.clone();
        }
        let custom_active = custom.as_deref().is_some_and(|path| {
            config.backend_id.is_none() && config.sd_server.as_deref() == Some(path)
        });
        (
            config.backend_id.clone(),
            config.sd_server.clone(),
            custom,
            custom_active,
        )
    };
    if custom.is_some() && ui.config.borrow().custom_server.is_none() {
        let mut config = ui.config.borrow_mut();
        config.custom_server = custom.clone();
        let _ = config.save();
    }

    if let Some(path) = &system_path
        && runtime::server_works(path)
    {
        let active =
            server.as_deref() == Some(path.as_path()) || (server.is_none() && active_id.is_none());
        let subtitle = path.display().to_string();
        let row = adw::ActionRow::builder()
            .title("System backend")
            .subtitle(subtitle)
            .tooltip_text("sd-server installed by the system")
            .build();
        if active {
            row.add_suffix(&active_label());
        } else {
            let button = gtk::Button::builder()
                .icon_name("object-select-symbolic")
                .tooltip_text("Use the system backend")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let ui = ui.clone();
            let status = status.clone();
            let refresh = refresh.clone();
            let path = path.clone();
            button.connect_clicked(move |_| {
                set_custom_server(&ui, path.clone(), None, None, &status);
                refresh();
            });
            row.add_suffix(&button);
        }
        list.append(&row);
    }

    let entries = manifest.borrow().backends.clone();
    for entry in entries {
        let installed = entry.is_installed(&runtime::backends_dir());
        let installed_version = entry
            .installed_version(&runtime::backends_dir())
            .unwrap_or_else(|| entry.version.clone());
        let active = active_id.as_deref() == Some(entry.id.as_str());
        let compatibility = hardware.compatibility(&entry);
        let available = entry.is_available();
        if !installed && (!available || compatibility.is_some()) {
            continue;
        }

        let installed_info = entry.installed_info(&runtime::backends_dir());
        let installed_size = installed_info
            .as_ref()
            .map(|marker| marker.size)
            .filter(|size| *size > 0)
            .unwrap_or(entry.size);
        let update_available = installed && installed_version != entry.version && available;
        let previous = previous_info(&entry);
        let previous_available = previous_dir(&entry).join("sd-server").is_file();

        let mut subtitle = if installed {
            if update_available {
                format!(
                    "Installed {} · {} | Available {} · {}",
                    short_version(&installed_version),
                    format_size(installed_size),
                    short_version(&entry.version),
                    format_size(entry.size)
                )
            } else {
                format!(
                    "Installed {} · {} · {}",
                    short_version(&installed_version),
                    format_size(installed_size),
                    entry.kind.label()
                )
            }
        } else if available {
            format!(
                "{} · {} · {}",
                format_size(entry.size),
                entry.kind.label(),
                compatibility
                    .clone()
                    .unwrap_or_else(|| "ready to download".into())
            )
        } else {
            format!("{} · not published yet", entry.kind.label())
        };
        if let Some(previous) = &previous {
            subtitle.push_str(&format!(
                " · rollback: {}",
                short_version(&previous.version)
            ));
        }
        if installed && let Some(reason) = &compatibility {
            subtitle.push_str(&format!(" · {reason}"));
        }

        let row = adw::ActionRow::builder()
            .title(&entry.label)
            .subtitle(subtitle)
            .build();
        if let Some(notes) = &entry.notes {
            row.set_tooltip_text(Some(notes));
        }

        let mut buttons = Vec::new();
        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove this backend")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(installed && !active)
            .build();
        let rollback = gtk::Button::builder()
            .icon_name("edit-undo-symbolic")
            .tooltip_text(
                previous
                    .as_ref()
                    .map(|previous| {
                        format!(
                            "Restore the previous version {}",
                            short_version(&previous.version)
                        )
                    })
                    .unwrap_or_else(|| "Restore the previous backend".into()),
            )
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(previous_available)
            .build();
        let set_active = gtk::Button::builder()
            .icon_name("object-select-symbolic")
            .tooltip_text("Use this backend")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(installed && !active && compatibility.is_none())
            .build();
        let update = gtk::Button::builder()
            .icon_name("software-update-available-symbolic")
            .tooltip_text(format!("Update to {}", short_version(&entry.version)))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(update_available && compatibility.is_none())
            .build();
        let download = gtk::Button::builder()
            .icon_name("document-save-symbolic")
            .tooltip_text("Download this backend")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(!installed && available && compatibility.is_none())
            .build();
        buttons.push(download.clone());
        buttons.push(update.clone());
        buttons.push(rollback.clone());
        buttons.push(set_active.clone());
        buttons.push(delete.clone());
        row.add_suffix(&delete);
        row.add_suffix(&set_active);
        row.add_suffix(&download);
        row.add_suffix(&rollback);
        row.add_suffix(&update);
        if active {
            row.add_suffix(&active_label());
        }
        list.append(&row);

        {
            let ui = ui.clone();
            let entry = entry.clone();
            let status = status.clone();
            let refresh = refresh.clone();
            set_active.connect_clicked(move |_| {
                activate_entry(&ui, &entry, &status);
                refresh();
            });
        }
        {
            let ui = ui.clone();
            let entry = entry.clone();
            let refresh = refresh.clone();
            delete.connect_clicked(move |_| {
                confirm_remove(&ui, &entry, refresh.clone());
            });
        }
        {
            let ui = ui.clone();
            let entry = entry.clone();
            let status = status.clone();
            let refresh = refresh.clone();
            rollback.connect_clicked(move |_| {
                rollback_entry(&ui, &entry, &status);
                refresh();
            });
        }
        for button in [&download, &update] {
            let ui = ui.clone();
            let entry = entry.clone();
            let progress = progress.clone();
            let status = status.clone();
            let refresh = refresh.clone();
            let buttons = buttons.clone();
            let auto_activate = false;
            button.connect_clicked(move |_| {
                start_install(
                    &ui,
                    &entry,
                    &progress,
                    &status,
                    refresh.clone(),
                    buttons.clone(),
                    auto_activate,
                );
            });
        }
    }

    let custom_row = adw::ActionRow::builder()
        .title("Custom backend")
        .subtitle(match &custom {
            Some(path) => {
                let mut subtitle = path.display().to_string();
                if !path.is_file() {
                    subtitle.push_str(" · missing");
                }
                subtitle
            }
            None => "Choose an sd-server binary".into(),
        })
        .build();
    let choose = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text("Choose a custom sd-server binary")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    {
        let ui = ui.clone();
        let status = status.clone();
        let refresh = refresh.clone();
        choose.connect_clicked(move |_| {
            let refresh = refresh.clone();
            choose_server(&ui, &status, move || refresh());
        });
    }
    custom_row.add_suffix(&choose);
    if let Some(path) = custom {
        if custom_active {
            custom_row.add_suffix(&active_label());
        } else if path.is_file() {
            let button = gtk::Button::builder()
                .icon_name("object-select-symbolic")
                .tooltip_text("Use this custom backend")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let ui = ui.clone();
            let status = status.clone();
            let refresh = refresh.clone();
            button.connect_clicked(move |_| {
                {
                    let mut config = ui.config.borrow_mut();
                    config.custom_server = Some(path.clone());
                    let _ = config.save();
                }
                set_custom_server(&ui, path.clone(), None, None, &status);
                refresh();
            });
            custom_row.add_suffix(&button);
        }
    }
    list.append(&custom_row);
}

fn start_install(
    ui: &Rc<Ui>,
    entry: &BackendEntry,
    progress: &gtk::ProgressBar,
    status: &gtk::Label,
    refresh: Rc<dyn Fn()>,
    buttons: Vec<gtk::Button>,
    auto_activate: bool,
) {
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("A backend download is already running."));
        return;
    }
    let backends_dir = runtime::backends_dir();
    let install_dir = entry.install_dir(&backends_dir);
    let backup = previous_dir(entry);
    let is_update = entry.is_installed(&backends_dir)
        && entry.installed_version(&backends_dir).as_deref() != Some(entry.version.as_str());
    if is_update {
        if ui.generating.get() || ui.upscaling.get() {
            ui.toast
                .add_toast(adw::Toast::new("Wait for the current image to finish."));
            return;
        }
        ui.backend.unload();
        if backup.exists()
            && let Err(error) = std::fs::remove_dir_all(&backup)
        {
            ui.toast.add_toast(adw::Toast::new(&format!(
                "Could not replace the previous backend: {error}"
            )));
            return;
        }
        if let Err(error) = std::fs::rename(&install_dir, &backup) {
            ui.toast.add_toast(adw::Toast::new(&format!(
                "Could not keep the previous backend: {error}"
            )));
            return;
        }
    }

    for button in &buttons {
        button.set_sensitive(false);
    }
    if ui.config.borrow().backend_id.as_deref() == Some(entry.id.as_str()) {
        ui.backend.unload();
    }
    progress.set_visible(true);
    progress.set_fraction(0.0);
    progress.set_text(Some("Starting…"));
    status.set_text(&format!("Downloading {}…", entry.label));

    let progress_widget = progress.clone();
    let status_widget = status.clone();
    let label = entry.label.clone();
    let on_progress: Rc<dyn Fn(u64, u64)> = Rc::new(move |downloaded, total| {
        let fraction = if total > 0 {
            downloaded as f64 / total as f64
        } else {
            0.0
        };
        progress_widget.set_fraction(fraction.min(1.0));
        progress_widget.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
        status_widget.set_text(&format!(
            "{label} · {} / {}",
            format_size(downloaded),
            format_size(total)
        ));
    });
    let ui_done = ui.clone();
    let entry_done = entry.clone();
    let progress_done = progress.clone();
    let status_done = status.clone();
    let refresh_done = refresh.clone();
    let install_dir_done = install_dir.clone();
    let backup_done = backup.clone();
    let on_done: Rc<dyn Fn(Result<PathBuf, String>)> = Rc::new(move |result| {
        progress_done.set_visible(false);
        match result {
            Ok(_) => {
                status_done.set_text(&format!("{} installed.", entry_done.label));
                ui_done
                    .toast
                    .add_toast(adw::Toast::new(&format!("{} installed.", entry_done.label)));
                if auto_activate {
                    activate_entry(&ui_done, &entry_done, &status_done);
                }
            }
            Err(error) => {
                if is_update {
                    let _ = std::fs::remove_dir_all(&install_dir_done);
                    if backup_done.is_dir() {
                        let _ = std::fs::rename(&backup_done, &install_dir_done);
                    }
                }
                status_done.set_text(&format!("Install failed: {error}"));
                ui_done
                    .toast
                    .add_toast(adw::Toast::new(&format!("Install failed: {error}")));
            }
        }
        refresh_done();
    });

    install_entry(ui, entry, on_progress, on_done);
}

fn install_entry(
    ui: &Rc<Ui>,
    entry: &BackendEntry,
    on_progress: Rc<dyn Fn(u64, u64)>,
    on_done: Rc<dyn Fn(Result<PathBuf, String>)>,
) {
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("A backend download is already running."));
        return;
    }
    ui.backend_installing.set(true);

    let (progress_tx, progress_rx) = async_channel::unbounded::<flux_models::BackendProgress>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<PathBuf, String>>(1);
    let backends_dir = runtime::backends_dir();
    let entry = entry.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<PathBuf> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(flux_models::download_backend(
                &entry,
                &backends_dir,
                progress_tx,
            ))
        })();
        let _ = done_tx.send_blocking(result.map_err(|error| error.to_string()));
    });

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok(progress) = progress_rx.recv().await {
            on_progress(progress.downloaded, progress.total);
        }
        let result = done_rx
            .recv()
            .await
            .unwrap_or_else(|_| Err("backend worker stopped unexpectedly".into()));
        ui.backend_installing.set(false);
        on_done(result);
    });
}

fn activate_entry(ui: &Rc<Ui>, entry: &BackendEntry, status: &gtk::Label) {
    if ui.generating.get() || ui.upscaling.get() {
        ui.toast
            .add_toast(adw::Toast::new("Wait for the current image to finish."));
        return;
    }
    let path = entry.server_path(&runtime::backends_dir());
    if !path.is_file() {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} is not installed.",
            entry.label
        )));
        return;
    }
    set_custom_server(
        ui,
        path,
        entry.backend.clone(),
        Some(entry.id.clone()),
        status,
    );
    ui.toast
        .add_toast(adw::Toast::new(&format!("{} is now active.", entry.label)));
}

fn set_custom_server(
    ui: &Rc<Ui>,
    path: PathBuf,
    backend: Option<String>,
    backend_id: Option<String>,
    status: &gtk::Label,
) {
    if !path.is_file() {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} does not exist.",
            path.display()
        )));
        return;
    }
    if ui.generating.get() || ui.upscaling.get() {
        ui.toast
            .add_toast(adw::Toast::new("Wait for the current image to finish."));
        return;
    }
    {
        let mut config = ui.config.borrow_mut();
        config.sd_server = Some(path.clone());
        config.backend = backend.clone();
        config.backend_id = backend_id;
        let _ = config.save();
    }
    ui.backend.set_server(path.clone(), backend);
    ui.backend.unload();
    status.set_text(&format!("Active backend: {}", path.display()));
}

fn confirm_remove(ui: &Rc<Ui>, entry: &BackendEntry, refresh: Rc<dyn Fn()>) {
    let path = entry.install_dir(&runtime::backends_dir());
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Remove {}?", entry.label))
        .body(format!(
            "This permanently removes {} from disk.",
            path.display()
        ))
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Remove");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));
    {
        let ui = ui.clone();
        let entry = entry.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "delete" {
                return;
            }
            if ui.config.borrow().backend_id.as_deref() == Some(entry.id.as_str()) {
                ui.toast.add_toast(adw::Toast::new(
                    "Switch to another backend before removing this one.",
                ));
                return;
            }
            match std::fs::remove_dir_all(entry.install_dir(&runtime::backends_dir())) {
                Ok(()) => ui
                    .toast
                    .add_toast(adw::Toast::new(&format!("{} removed.", entry.label))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => ui.toast.add_toast(adw::Toast::new(&format!(
                    "Could not remove {}: {error}",
                    entry.label
                ))),
            }
            refresh();
        });
    }
    dialog.present(Some(&ui.window));
}

fn choose_server(ui: &Rc<Ui>, status: &gtk::Label, on_done: impl Fn() + 'static) {
    let dialog = gtk::FileDialog::builder()
        .title("Choose sd-server")
        .modal(true)
        .build();
    let ui = ui.clone();
    let status = status.clone();
    let window = ui.window.clone();
    dialog.open(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            return;
        };
        set_custom_server(&ui, path, None, None, &status);
        on_done();
    });
}

fn active_label() -> gtk::Label {
    gtk::Label::builder()
        .label("active")
        .css_classes(["active-backend-label"])
        .build()
}

fn previous_dir(entry: &BackendEntry) -> PathBuf {
    runtime::backends_dir().join(format!("{}.previous", entry.id))
}

fn previous_info(entry: &BackendEntry) -> Option<flux_core::runtime::InstalledBackend> {
    runtime::read_installed(&previous_dir(entry).join("installed.json"))
}

fn rollback_entry(ui: &Rc<Ui>, entry: &BackendEntry, status: &gtk::Label) {
    if ui.generating.get() || ui.upscaling.get() {
        ui.toast
            .add_toast(adw::Toast::new("Wait for the current image to finish."));
        return;
    }
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("A backend download is already running."));
        return;
    }
    let install_dir = entry.install_dir(&runtime::backends_dir());
    let backup = previous_dir(entry);
    if !backup.join("sd-server").is_file() {
        ui.toast
            .add_toast(adw::Toast::new("No previous backend is available."));
        return;
    }
    let version = previous_info(entry)
        .map(|marker| short_version(&marker.version).to_string())
        .unwrap_or_else(|| "previous version".into());
    ui.backend.unload();
    if install_dir.exists()
        && let Err(error) = std::fs::remove_dir_all(&install_dir)
    {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "Could not replace the current backend: {error}"
        )));
        return;
    }
    if let Err(error) = std::fs::rename(&backup, &install_dir) {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "Could not restore the previous backend: {error}"
        )));
        return;
    }
    status.set_text(&format!("{} rolled back to {version}.", entry.label));
    ui.toast.add_toast(adw::Toast::new(&format!(
        "{} rolled back to {version}.",
        entry.label
    )));
}

fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        "—".into()
    } else if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

fn short_version(version: &str) -> &str {
    version.rsplit('-').next().unwrap_or(version)
}
