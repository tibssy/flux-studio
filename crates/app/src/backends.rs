use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use imagiflume_core::Config;
use imagiflume_core::runtime::{self, BackendEntry, BackendManifest, HardwareProfile};

use crate::Ui;

const BACKEND_ICON: &str = "application-x-executable-symbolic";

type Refresh = Rc<dyn Fn()>;
type RefreshSlot = Rc<RefCell<Option<Refresh>>>;

/// Custom binaries link against host libraries that a Flatpak sandbox does
/// not expose, so the chooser is only useful in native builds.
fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists()
}

/// Drop custom backend selections that cannot work inside the sandbox.
pub(crate) fn sanitize_config(config: &mut Config) {
    sanitize_config_for(config, in_flatpak());
}

fn sanitize_config_for(config: &mut Config, flatpak: bool) {
    if !flatpak {
        return;
    }
    if let Some(custom) = config.custom_server.take()
        && config.sd_server.as_ref() == Some(&custom)
    {
        config.sd_server = None;
        config.backend_id = None;
    }
    if let Some(custom) = config.llama_custom_cli.take()
        && config.prompt_helper_cli.as_ref() == Some(&custom)
    {
        config.prompt_helper_cli = None;
        config.llama_backend_id = None;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    StableDiffusion,
    LlamaCpp,
}

impl Section {
    const ALL: [Section; 2] = [Section::StableDiffusion, Section::LlamaCpp];

    fn label(self) -> &'static str {
        match self {
            Section::StableDiffusion => "stable-diffusion",
            Section::LlamaCpp => "llama.cpp",
        }
    }

    fn root(self) -> PathBuf {
        match self {
            Section::StableDiffusion => runtime::backends_dir(),
            Section::LlamaCpp => runtime::llama_backends_dir(),
        }
    }

    fn bundled_manifest(self) -> BackendManifest {
        match self {
            Section::StableDiffusion => runtime::bundled_manifest(),
            Section::LlamaCpp => runtime::bundled_llama_manifest(),
        }
    }

    fn manifest_url(self, config: &Config) -> String {
        match self {
            Section::StableDiffusion => config
                .backend_manifest_url
                .clone()
                .unwrap_or_else(|| runtime::DEFAULT_MANIFEST_URL.to_string()),
            Section::LlamaCpp => config
                .llama_manifest_url
                .clone()
                .unwrap_or_else(|| runtime::DEFAULT_LLAMA_MANIFEST_URL.to_string()),
        }
    }

    fn system_path(self) -> Option<PathBuf> {
        match self {
            Section::StableDiffusion => runtime::find_system_server(),
            Section::LlamaCpp => runtime::find_system_llama_cli(),
        }
    }

    fn group_title(self) -> &'static str {
        match self {
            Section::StableDiffusion => "Stable Diffusion backends",
            Section::LlamaCpp => "llama.cpp backends",
        }
    }

    fn group_description(self) -> &'static str {
        match self {
            Section::StableDiffusion => {
                "Compatible builds for this system. The refresh button checks the pinned manifest."
            }
            Section::LlamaCpp => {
                "Compatible llama-cli builds used for prompt improvement. The refresh button checks the pinned manifest."
            }
        }
    }

    fn system_title(self) -> &'static str {
        match self {
            Section::StableDiffusion => "System backend",
            Section::LlamaCpp => "System llama.cpp",
        }
    }

    fn system_tooltip(self) -> &'static str {
        match self {
            Section::StableDiffusion => "sd-server installed by the system",
            Section::LlamaCpp => "llama-cli installed by the system",
        }
    }

    fn use_system_tooltip(self) -> &'static str {
        match self {
            Section::StableDiffusion => "Use the system backend",
            Section::LlamaCpp => "Use the system llama.cpp",
        }
    }

    fn custom_title(self) -> &'static str {
        match self {
            Section::StableDiffusion => "Custom backend",
            Section::LlamaCpp => "Custom llama.cpp",
        }
    }

    fn custom_placeholder(self) -> &'static str {
        match self {
            Section::StableDiffusion => "Choose an sd-server binary",
            Section::LlamaCpp => "Choose a llama-cli binary",
        }
    }

    fn choose_title(self) -> &'static str {
        match self {
            Section::StableDiffusion => "Choose sd-server",
            Section::LlamaCpp => "Choose llama-cli",
        }
    }

    fn choose_tooltip(self) -> &'static str {
        match self {
            Section::StableDiffusion => "Choose a custom sd-server binary",
            Section::LlamaCpp => "Choose a custom llama-cli binary",
        }
    }

    fn active_id(self, config: &Config) -> Option<String> {
        match self {
            Section::StableDiffusion => config.backend_id.clone(),
            Section::LlamaCpp => config.llama_backend_id.clone(),
        }
    }

    fn active_path(self, config: &Config) -> Option<PathBuf> {
        match self {
            Section::StableDiffusion => config.sd_server.clone(),
            Section::LlamaCpp => config.prompt_helper_cli.clone(),
        }
    }

    fn custom(self, config: &Config) -> Option<PathBuf> {
        match self {
            Section::StableDiffusion => config.custom_server.clone(),
            Section::LlamaCpp => config.llama_custom_cli.clone(),
        }
    }

    fn set_custom(self, config: &mut Config, path: Option<PathBuf>) {
        match self {
            Section::StableDiffusion => config.custom_server = path,
            Section::LlamaCpp => config.llama_custom_cli = path,
        }
    }

    fn busy(self, ui: &Ui) -> bool {
        match self {
            Section::StableDiffusion => ui.generating.get() || ui.upscaling.get(),
            Section::LlamaCpp => ui.improving.get(),
        }
    }

    fn busy_message(self) -> &'static str {
        match self {
            Section::StableDiffusion => "Wait for the current image to finish.",
            Section::LlamaCpp => "Wait for the prompt improvement to finish.",
        }
    }

    fn set_active(
        self,
        ui: &Rc<Ui>,
        path: PathBuf,
        backend: Option<String>,
        backend_id: Option<String>,
    ) {
        if !path.is_file() {
            ui.toast.add_toast(adw::Toast::new(&format!(
                "{} does not exist.",
                path.display()
            )));
            return;
        }
        if self.busy(ui) {
            ui.toast.add_toast(adw::Toast::new(self.busy_message()));
            return;
        }
        {
            let mut config = ui.config.borrow_mut();
            match self {
                Section::StableDiffusion => {
                    config.sd_server = Some(path.clone());
                    config.backend = backend.clone();
                    config.backend_id = backend_id;
                }
                Section::LlamaCpp => {
                    config.prompt_helper_cli = Some(path.clone());
                    config.llama_backend_id = backend_id;
                }
            }
            let _ = config.save();
        }
        match self {
            Section::StableDiffusion => {
                ui.backend.set_server(path.clone(), backend);
                ui.backend.unload();
            }
            Section::LlamaCpp => crate::refresh_prompt_helper(ui),
        }
        let message = match self {
            Section::StableDiffusion => format!("Active backend: {}", path.display()),
            Section::LlamaCpp => format!("Active llama.cpp: {}", path.display()),
        };
        ui.toast.add_toast(adw::Toast::new(&message));
    }

    fn refresh_message(self, outcome: &imagiflume_models::ManifestRefresh) -> String {
        let mut message = match (outcome.curated_ok, outcome.upstream_ok) {
            (true, true) => match self {
                Section::StableDiffusion => "Backend manifest updated.".to_string(),
                Section::LlamaCpp => "llama.cpp backend list updated.".to_string(),
            },
            (false, true) => match self {
                Section::StableDiffusion => {
                    "Backend list updated from the latest upstream release.".to_string()
                }
                Section::LlamaCpp => {
                    "llama.cpp backend list updated from the latest upstream release.".to_string()
                }
            },
            _ => match self {
                Section::StableDiffusion => {
                    "Backend manifest updated; upstream release check unavailable.".to_string()
                }
                Section::LlamaCpp => {
                    "llama.cpp backend list updated; upstream release check unavailable."
                        .to_string()
                }
            },
        };
        if !outcome.errors.is_empty() {
            message.push_str(&format!(
                " Skipped {} invalid backend entr{}.",
                outcome.errors.len(),
                if outcome.errors.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            ));
        }
        message
    }

    fn custom_selected_message(self, path: &std::path::Path) -> String {
        match self {
            Section::StableDiffusion => format!(
                "Custom backend selected: {} · use the check button to activate it.",
                path.display()
            ),
            Section::LlamaCpp => format!(
                "Custom llama.cpp selected: {} · use the check button to activate it.",
                path.display()
            ),
        }
    }
}

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

    install_entry(ui, Section::StableDiffusion, &entry, on_progress, on_done);
}

/// Build the Backends preferences page.
pub(crate) fn build_page(ui: &Rc<Ui>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Backends")
        .icon_name(BACKEND_ICON)
        .build();

    let hardware = Rc::new(HardwareProfile::detect());
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

    let refresh_buttons: Vec<gtk::Button> = Section::ALL
        .into_iter()
        .map(|section| build_section(ui, &page, section, &hardware))
        .collect();

    {
        let buttons = refresh_buttons;
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        key.connect_key_pressed(move |_, key, _, state| {
            if state.contains(gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gdk::Key::r | gdk::Key::R)
            {
                for button in &buttons {
                    if button.is_sensitive() {
                        button.emit_clicked();
                    }
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        page.add_controller(key);
    }

    page
}

fn build_section(
    ui: &Rc<Ui>,
    page: &adw::PreferencesPage,
    section: Section,
    hardware: &Rc<HardwareProfile>,
) -> gtk::Button {
    let manifest = Rc::new(RefCell::new(section.bundled_manifest()));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(format!(
            "Check the {} backend manifest (Ctrl+R)",
            section.label()
        ))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let group = adw::PreferencesGroup::builder()
        .title(section.group_title())
        .description(section.group_description())
        .header_suffix(&refresh_button)
        .build();
    group.add(&list);
    page.add(&group);

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
        let refresh = refresh.clone();
        Rc::new(move || {
            rebuild(&ui, &list, &manifest, &hardware, section, refresh.clone());
        })
    };
    *refresh_slot.borrow_mut() = Some(populate.clone());
    populate();

    {
        let ui = ui.clone();
        let manifest = manifest.clone();
        let refresh = refresh.clone();
        refresh_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            let url = section.manifest_url(&ui.config.borrow());
            let base = manifest.borrow().clone();
            let (tx, rx) = async_channel::bounded::<Result<imagiflume_models::ManifestRefresh, String>>(1);
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<imagiflume_models::ManifestRefresh> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    match section {
                        Section::StableDiffusion => {
                            runtime.block_on(imagiflume_models::refresh_manifest(&base, &url))
                        }
                        Section::LlamaCpp => {
                            runtime.block_on(imagiflume_models::refresh_llama_manifest(&base, &url))
                        }
                    }
                })();
                let _ = tx.send_blocking(result.map_err(|error| error.to_string()));
            });
            let button = button.clone();
            let ui = ui.clone();
            let refresh = refresh.clone();
            let manifest = manifest.clone();
            glib::spawn_future_local(async move {
                let result = rx.recv().await;
                button.set_sensitive(true);
                match result {
                    Ok(Ok(outcome)) => {
                        let message = section.refresh_message(&outcome);
                        *manifest.borrow_mut() = outcome.manifest;
                        ui.toast.add_toast(adw::Toast::new(&message));
                        refresh();
                    }
                    Ok(Err(error)) => {
                        ui.toast.add_toast(adw::Toast::new(&format!(
                            "Could not fetch the {} backend manifest; keeping the pinned list ({error})",
                            section.label()
                        )));
                        refresh();
                    }
                    Err(_) => {
                        ui.toast.add_toast(adw::Toast::new(&format!(
                            "{} backend manifest request stopped; keeping the current list.",
                            section.label()
                        )));
                        refresh();
                    }
                }
            });
        });
    }

    refresh_button
}

fn rebuild(
    ui: &Rc<Ui>,
    list: &gtk::ListBox,
    manifest: &Rc<RefCell<BackendManifest>>,
    hardware: &HardwareProfile,
    section: Section,
    refresh: Rc<dyn Fn()>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let root = section.root();
    let system_path = section.system_path();
    let custom_allowed = !in_flatpak();
    let (active_id, active_path, custom, custom_active) = {
        let config = ui.config.borrow();
        let mut custom = if custom_allowed {
            section.custom(&config)
        } else {
            None
        };
        if custom_allowed
            && custom.is_none()
            && section.active_id(&config).is_none()
            && section.active_path(&config).as_deref() != system_path.as_deref()
        {
            custom = section.active_path(&config);
        }
        let custom_active = custom.as_deref().is_some_and(|path| {
            section.active_id(&config).is_none()
                && section.active_path(&config).as_deref() == Some(path)
        });
        (
            section.active_id(&config),
            section.active_path(&config),
            custom,
            custom_active,
        )
    };
    if custom_allowed && custom.is_some() && section.custom(&ui.config.borrow()).is_none() {
        let mut config = ui.config.borrow_mut();
        section.set_custom(&mut config, custom.clone());
        let _ = config.save();
    }

    if let Some(path) = &system_path
        && runtime::binary_works(path)
    {
        let active = active_path.as_deref() == Some(path.as_path())
            || (active_path.is_none() && active_id.is_none());
        let subtitle = path.display().to_string();
        let row = adw::ActionRow::builder()
            .title(section.system_title())
            .subtitle(subtitle)
            .tooltip_text(section.system_tooltip())
            .build();
        let check = check_button(active, section.use_system_tooltip());
        if !active {
            let ui = ui.clone();
            let refresh = refresh.clone();
            let path = path.clone();
            check.connect_clicked(move |_| {
                section.set_active(&ui, path.clone(), None, None);
                refresh();
            });
        }
        row.add_suffix(&check_slot(Some(check.upcast::<gtk::Widget>())));
        list.append(&row);
    }

    let entries = manifest.borrow().backends.clone();
    for entry in entries {
        let installed = entry.is_installed(&root);
        let installed_version = entry
            .installed_version(&root)
            .unwrap_or_else(|| entry.version.clone());
        let active = active_id.as_deref() == Some(entry.id.as_str());
        let compatibility = hardware.compatibility(&entry);
        let available = entry.is_available();
        if !installed && (!available || compatibility.is_some()) {
            continue;
        }

        let installed_info = entry.installed_info(&root);
        let installed_size = installed_info
            .as_ref()
            .map(|marker| marker.size)
            .filter(|size| *size > 0)
            .unwrap_or(entry.size);
        let previous = previous_info(&entry, &root);
        let previous_available = previous_dir(&entry, &root)
            .join(entry.binary_name())
            .is_file();
        let update_available = update_available(
            installed,
            &installed_version,
            &entry.version,
            available,
            previous.as_ref().map(|previous| previous.version.as_str()),
        );

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
        let check = check_button(active, "Use this backend");
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
        buttons.push(check.clone());
        buttons.push(delete.clone());
        row.add_suffix(&delete);
        row.add_suffix(&download);
        row.add_suffix(&rollback);
        row.add_suffix(&update);
        if installed {
            if active || compatibility.is_none() {
                row.add_suffix(&check_slot(Some(check.clone().upcast::<gtk::Widget>())));
            } else {
                row.add_suffix(&check_slot(None));
            }
        }
        list.append(&row);

        if !active && compatibility.is_none() {
            let ui = ui.clone();
            let entry = entry.clone();
            let refresh = refresh.clone();
            check.connect_clicked(move |_| {
                activate_entry(&ui, section, &entry);
                refresh();
            });
        }
        {
            let ui = ui.clone();
            let entry = entry.clone();
            let refresh = refresh.clone();
            delete.connect_clicked(move |_| {
                confirm_remove(&ui, section, &entry, refresh.clone());
            });
        }
        {
            let ui = ui.clone();
            let entry = entry.clone();
            let refresh = refresh.clone();
            rollback.connect_clicked(move |_| {
                rollback_entry(&ui, section, &entry);
                refresh();
            });
        }
        for button in [&download, &update] {
            let ui = ui.clone();
            let entry = entry.clone();
            let row = row.clone();
            let refresh = refresh.clone();
            let buttons = buttons.clone();
            let auto_activate = false;
            button.connect_clicked(move |_| {
                start_install(
                    &ui,
                    section,
                    &entry,
                    &row,
                    refresh.clone(),
                    buttons.clone(),
                    auto_activate,
                );
            });
        }
    }

    if !in_flatpak() {
        let custom_row = adw::ActionRow::builder()
            .title(section.custom_title())
            .subtitle(match &custom {
                Some(path) => {
                    let mut subtitle = path.display().to_string();
                    if !path.is_file() {
                        subtitle.push_str(" · missing");
                    }
                    subtitle
                }
                None => section.custom_placeholder().into(),
            })
            .build();
        let choose = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(section.choose_tooltip())
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        {
            let ui = ui.clone();
            let refresh = refresh.clone();
            choose.connect_clicked(move |_| {
                let refresh = refresh.clone();
                choose_binary(&ui, section, move || refresh());
            });
        }
        let check = match custom {
            Some(_) if custom_active => Some(check_button(true, "Active custom backend")),
            Some(path) if path.is_file() => {
                let button = check_button(false, "Use this custom backend");
                let ui = ui.clone();
                let refresh = refresh.clone();
                button.connect_clicked(move |_| {
                    {
                        let mut config = ui.config.borrow_mut();
                        section.set_custom(&mut config, Some(path.clone()));
                        let _ = config.save();
                    }
                    section.set_active(&ui, path.clone(), None, None);
                    refresh();
                });
                Some(button)
            }
            Some(_) => {
                let button = check_button(false, "Custom backend is missing");
                button.set_sensitive(false);
                Some(button)
            }
            None => None,
        };
        if let Some(button) = check {
            custom_row.add_suffix(&choose);
            custom_row.add_suffix(&check_slot(Some(button.upcast::<gtk::Widget>())));
        } else {
            // Keep the chooser in the same column as the activate buttons of the
            // other rows instead of leaving an empty slot to its right.
            custom_row.add_suffix(&check_slot(Some(choose.clone().upcast::<gtk::Widget>())));
        }
        list.append(&custom_row);
    }
}

fn start_install(
    ui: &Rc<Ui>,
    section: Section,
    entry: &BackendEntry,
    row: &adw::ActionRow,
    refresh: Rc<dyn Fn()>,
    buttons: Vec<gtk::Button>,
    auto_activate: bool,
) {
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("A backend download is already running."));
        return;
    }
    if section.busy(ui) {
        ui.toast.add_toast(adw::Toast::new(section.busy_message()));
        return;
    }
    let root = section.root();
    let install_dir = entry.install_dir(&root);
    let backup = previous_dir(entry, &root);
    let is_update = entry.is_installed(&root)
        && entry.installed_version(&root).as_deref() != Some(entry.version.as_str());
    if is_update {
        if section == Section::StableDiffusion {
            ui.backend.unload();
        }
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
    if section == Section::StableDiffusion
        && ui.config.borrow().backend_id.as_deref() == Some(entry.id.as_str())
    {
        ui.backend.unload();
    }
    row.set_subtitle("Downloading…");

    let row_progress = row.clone();
    let on_progress: Rc<dyn Fn(u64, u64)> = Rc::new(move |downloaded, total| {
        let percent = if total > 0 {
            downloaded as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        row_progress.set_subtitle(&format!(
            "Downloading · {percent:.0}% · {} / {}",
            format_size(downloaded),
            format_size(total)
        ));
    });
    let ui_done = ui.clone();
    let entry_done = entry.clone();
    let row_done = row.clone();
    let refresh_done = refresh.clone();
    let install_dir_done = install_dir.clone();
    let backup_done = backup.clone();
    let on_done: Rc<dyn Fn(Result<PathBuf, String>)> = Rc::new(move |result| {
        match result {
            Ok(_) => {
                ui_done
                    .toast
                    .add_toast(adw::Toast::new(&format!("{} installed.", entry_done.label)));
                if auto_activate {
                    activate_entry(&ui_done, section, &entry_done);
                }
            }
            Err(error) => {
                if is_update {
                    let _ = std::fs::remove_dir_all(&install_dir_done);
                    if backup_done.is_dir() {
                        let _ = std::fs::rename(&backup_done, &install_dir_done);
                    }
                }
                row_done.set_subtitle("Install failed");
                ui_done
                    .toast
                    .add_toast(adw::Toast::new(&format!("Install failed: {error}")));
            }
        }
        refresh_done();
    });

    install_entry(ui, section, entry, on_progress, on_done);
}

fn install_entry(
    ui: &Rc<Ui>,
    section: Section,
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

    let (progress_tx, progress_rx) =
        async_channel::unbounded::<imagiflume_models::BackendProgress>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<PathBuf, String>>(1);
    let root = section.root();
    let entry = entry.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<PathBuf> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(imagiflume_models::download_backend(
                &entry,
                &root,
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

fn activate_entry(ui: &Rc<Ui>, section: Section, entry: &BackendEntry) {
    let path = entry.binary_path(&section.root());
    if !path.is_file() {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} is not installed.",
            entry.label
        )));
        return;
    }
    section.set_active(ui, path, entry.backend.clone(), Some(entry.id.clone()));
}

fn confirm_remove(ui: &Rc<Ui>, section: Section, entry: &BackendEntry, refresh: Rc<dyn Fn()>) {
    let path = entry.install_dir(&section.root());
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
            if section.busy(&ui) {
                ui.toast.add_toast(adw::Toast::new(section.busy_message()));
                return;
            }
            if section.active_id(&ui.config.borrow()).as_deref() == Some(entry.id.as_str()) {
                ui.toast.add_toast(adw::Toast::new(
                    "Switch to another backend before removing this one.",
                ));
                return;
            }
            match std::fs::remove_dir_all(entry.install_dir(&section.root())) {
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

fn choose_binary(ui: &Rc<Ui>, section: Section, on_done: impl Fn() + 'static) {
    let dialog = gtk::FileDialog::builder()
        .title(section.choose_title())
        .modal(true)
        .build();
    let ui = ui.clone();
    let window = ui.window.clone();
    dialog.open(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            return;
        };
        if !path.is_file() {
            ui.toast.add_toast(adw::Toast::new(&format!(
                "{} does not exist.",
                path.display()
            )));
            return;
        }
        {
            let mut config = ui.config.borrow_mut();
            section.set_custom(&mut config, Some(path.clone()));
            let _ = config.save();
        }
        ui.toast
            .add_toast(adw::Toast::new(&section.custom_selected_message(&path)));
        on_done();
    });
}

fn update_available(
    installed: bool,
    installed_version: &str,
    entry_version: &str,
    available: bool,
    previous_version: Option<&str>,
) -> bool {
    installed
        && installed_version != entry_version
        && available
        && previous_version != Some(entry_version)
}

fn check_slot(content: Option<gtk::Widget>) -> gtk::Box {
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    slot.set_size_request(36, 36);
    slot.set_halign(gtk::Align::Center);
    slot.set_valign(gtk::Align::Center);
    if let Some(widget) = content {
        slot.append(&widget);
    }
    slot
}

fn check_button(active: bool, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("object-select-symbolic")
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "backend-check"])
        .build();
    if active {
        button.add_css_class("active-backend-icon");
        button.set_sensitive(false);
    }
    button
}

fn previous_dir(entry: &BackendEntry, root: &std::path::Path) -> PathBuf {
    root.join(format!("{}.previous", entry.id))
}

fn previous_info(
    entry: &BackendEntry,
    root: &std::path::Path,
) -> Option<imagiflume_core::runtime::InstalledBackend> {
    runtime::read_installed(&previous_dir(entry, root).join("installed.json"))
}

fn rollback_entry(ui: &Rc<Ui>, section: Section, entry: &BackendEntry) {
    if section.busy(ui) {
        ui.toast.add_toast(adw::Toast::new(section.busy_message()));
        return;
    }
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("A backend download is already running."));
        return;
    }
    let root = section.root();
    let install_dir = entry.install_dir(&root);
    let backup = previous_dir(entry, &root);
    if !backup.join(entry.binary_name()).is_file() {
        ui.toast
            .add_toast(adw::Toast::new("No previous backend is available."));
        return;
    }
    let version = previous_info(entry, &root)
        .map(|marker| short_version(&marker.version).to_string())
        .unwrap_or_else(|| "previous version".into());
    if section == Section::StableDiffusion {
        ui.backend.unload();
    }
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
    if section == Section::LlamaCpp {
        crate::refresh_prompt_helper(ui);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_update_when_manifest_points_at_rollback_copy() {
        assert!(!update_available(true, "new", "old", true, Some("old")));
    }

    #[test]
    fn shows_update_when_a_newer_manifest_arrives_after_an_update() {
        assert!(update_available(true, "new", "newer", true, Some("old")));
    }

    #[test]
    fn hides_update_when_installed_version_is_current() {
        assert!(!update_available(true, "same", "same", true, Some("old")));
    }

    #[test]
    fn shows_update_without_a_rollback_copy() {
        assert!(update_available(true, "old", "new", true, None));
    }

    #[test]
    fn flatpak_drops_active_custom_backends() {
        let mut config = Config::default();
        config.custom_server = Some(PathBuf::from("/run/user/1000/doc/id/sd-server"));
        config.sd_server = config.custom_server.clone();
        config.backend_id = None;
        config.llama_custom_cli = Some(PathBuf::from("/run/user/1000/doc/id/llama-cli"));
        config.prompt_helper_cli = config.llama_custom_cli.clone();
        config.llama_backend_id = None;

        sanitize_config_for(&mut config, true);

        assert!(config.custom_server.is_none());
        assert!(config.sd_server.is_none());
        assert!(config.backend_id.is_none());
        assert!(config.llama_custom_cli.is_none());
        assert!(config.prompt_helper_cli.is_none());
        assert!(config.llama_backend_id.is_none());
    }

    #[test]
    fn flatpak_keeps_a_working_installed_backend() {
        let mut config = Config {
            custom_server: Some(PathBuf::from("/run/user/1000/doc/id/sd-server")),
            sd_server: Some(PathBuf::from("/app/lib/imagiflume/sd-server")),
            backend_id: Some("upstream-cpu".into()),
            ..Default::default()
        };

        sanitize_config_for(&mut config, true);

        assert!(config.custom_server.is_none());
        assert_eq!(
            config.sd_server,
            Some(PathBuf::from("/app/lib/imagiflume/sd-server"))
        );
        assert_eq!(config.backend_id.as_deref(), Some("upstream-cpu"));
    }

    #[test]
    fn native_keeps_custom_backends() {
        let mut config = Config::default();
        config.custom_server = Some(PathBuf::from("/opt/custom/sd-server"));
        config.sd_server = config.custom_server.clone();

        sanitize_config_for(&mut config, false);

        assert_eq!(
            config.custom_server,
            Some(PathBuf::from("/opt/custom/sd-server"))
        );
        assert_eq!(config.sd_server, config.custom_server);
    }
}
