mod history;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use adw::prelude::*;
use flux_backend_sdcpp::{SdCpp, SdCppConfig};
use flux_core::preset::{ModelPreset, PRESETS, default_preset, preset};
use flux_core::{Backend, Config, GenerateRequest, Progress, paths};
use gtk::{gdk, glib};

use history::{Generation, History, NewGeneration};

const HISTORY_LIMIT: usize = 30;

const RESOLUTIONS: &[(&str, u32, u32)] = &[
    ("768 × 768", 768, 768),
    ("512 × 512", 512, 512),
    ("896 × 896", 896, 896),
    ("1024 × 1024", 1024, 1024),
    ("1024 × 768", 1024, 768),
    ("768 × 1024", 768, 1024),
    ("1152 × 768", 1152, 768),
    ("768 × 1152", 768, 1152),
    ("1024 × 576", 1024, 576),
    ("576 × 1024", 576, 1024),
];

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "flux_studio=info,flux_backend_sdcpp=info".into()),
        )
        .init();

    let app = adw::Application::builder()
        .application_id(flux_core::APP_ID)
        .build();
    app.connect_activate(build_ui);
    app.run()
}

struct Ui {
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    picture: gtk::Picture,
    prompt: gtk::TextView,
    generate: gtk::Button,
    cancel: gtk::Button,
    spinner: gtk::Spinner,
    progress: gtk::ProgressBar,
    status: gtk::Label,
    toast: adw::ToastOverlay,
    backend: Arc<dyn Backend>,
    models_dir: PathBuf,
    resolution: Cell<(u32, u32)>,
    config: RefCell<Config>,
    history: RefCell<History>,
    history_list: gtk::ListBox,
    entries: RefCell<Vec<Generation>>,
    model_dropdown: gtk::DropDown,
    resolution_dropdown: gtk::DropDown,
    seed_entry: gtk::Entry,
    reference: RefCell<Option<PathBuf>>,
    reference_button: gtk::Button,
    reference_picture: gtk::Picture,
    reference_name: gtk::Label,
    reference_clear: gtk::Button,
    generating: Cell<bool>,
}

fn build_ui(app: &adw::Application) {
    let config = Config::load();
    let models_dir = resolve_models_dir(&config);
    let sd_cli = resolve_sd_cli(&config);

    let backend: Arc<dyn Backend> = Arc::new(SdCpp::new(SdCppConfig {
        cli: sd_cli,
        models_dir: models_dir.clone(),
    }));

    let history = match History::open(&paths::history_db()) {
        Ok(history) => history,
        Err(error) => {
            tracing::error!(%error, "failed to open history database; using memory");
            History::open_in_memory().expect("in-memory history")
        }
    };

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(flux_core::APP_NAME)
        .default_width(1180)
        .default_height(860)
        .build();

    let header = adw::HeaderBar::new();
    let preferences_button = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Preferences")
        .build();
    header.pack_end(&preferences_button);

    let empty = adw::StatusPage::builder()
        .icon_name("image-x-generic-symbolic")
        .title("No image yet")
        .description("Describe an image below and press Generate.")
        .build();

    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .hexpand(true)
        .vexpand(true)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&picture, Some("image"));
    stack.set_visible_child_name("empty");

    let content = gtk::ScrolledWindow::builder()
        .child(&stack)
        .hexpand(true)
        .vexpand(true)
        .build();

    let prompt = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(10)
        .bottom_margin(10)
        .left_margin(10)
        .right_margin(10)
        .build();

    let prompt_scroller = gtk::ScrolledWindow::builder()
        .child(&prompt)
        .min_content_height(72)
        .max_content_height(150)
        .propagate_natural_height(true)
        .build();
    prompt_scroller.add_css_class("card");

    let model_labels: Vec<&str> = PRESETS.iter().map(|item| item.label).collect();
    let model_dropdown = gtk::DropDown::from_strings(&model_labels);
    let active_model_index = PRESETS
        .iter()
        .position(|item| item.id == config.active_preset)
        .unwrap_or(0);
    model_dropdown.set_selected(active_model_index as u32);
    model_dropdown.set_hexpand(true);
    model_dropdown.set_tooltip_text(Some(PRESETS[active_model_index].description));

    let model_caption = gtk::Label::builder()
        .label("Model")
        .css_classes(["dim-label"])
        .build();
    let model_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    model_row.append(&model_caption);
    model_row.append(&model_dropdown);

    let labels: Vec<&str> = RESOLUTIONS.iter().map(|(label, _, _)| *label).collect();
    let resolution_dropdown = gtk::DropDown::from_strings(&labels);
    let initial_index = RESOLUTIONS
        .iter()
        .position(|(_, width, height)| {
            *width == config.default_width && *height == config.default_height
        })
        .unwrap_or(0);
    resolution_dropdown.set_selected(initial_index as u32);

    let size_label = gtk::Label::builder()
        .label("Size")
        .css_classes(["dim-label"])
        .build();
    let seed_label = gtk::Label::builder()
        .label("Seed")
        .css_classes(["dim-label"])
        .build();
    let seed_entry = gtk::Entry::builder()
        .placeholder_text("random")
        .width_chars(18)
        .max_width_chars(22)
        .input_purpose(gtk::InputPurpose::Digits)
        .build();
    let random_button = gtk::Button::builder()
        .icon_name("media-playlist-shuffle-symbolic")
        .tooltip_text("New random seed")
        .css_classes(["flat"])
        .build();
    {
        let seed_entry = seed_entry.clone();
        random_button.connect_clicked(move |_| {
            seed_entry.set_text(&GenerateRequest::random_seed().to_string());
        });
    }

    let settings_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    settings_row.append(&size_label);
    settings_row.append(&resolution_dropdown);
    settings_row.append(&seed_label);
    settings_row.append(&seed_entry);
    settings_row.append(&random_button);

    let reference_button = gtk::Button::builder()
        .label("Reference image…")
        .tooltip_text("Use an image as the starting point (edit mode)")
        .build();
    let reference_picture = gtk::Picture::builder()
        .width_request(40)
        .height_request(40)
        .content_fit(gtk::ContentFit::Cover)
        .visible(false)
        .build();
    reference_picture.add_css_class("card");
    let reference_name = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .max_width_chars(40)
        .css_classes(["dim-label"])
        .visible(false)
        .build();
    let reference_clear = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text("Remove reference image")
        .css_classes(["flat"])
        .visible(false)
        .build();
    let reference_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    reference_row.append(&reference_button);
    reference_row.append(&reference_picture);
    reference_row.append(&reference_name);
    reference_row.append(&reference_clear);

    let generate = gtk::Button::builder()
        .label("Generate")
        .css_classes(["suggested-action", "pill"])
        .build();
    let cancel = gtk::Button::builder()
        .label("Cancel")
        .css_classes(["pill"])
        .sensitive(false)
        .build();
    let spinner = gtk::Spinner::builder().visible(false).build();
    let progress = gtk::ProgressBar::builder()
        .hexpand(true)
        .show_text(true)
        .visible(false)
        .build();
    let status = gtk::Label::builder()
        .label("Ready")
        .css_classes(["dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.append(&spinner);
    controls.append(&progress);
    controls.append(&cancel);
    controls.append(&generate);

    let composer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    composer.set_margin_top(12);
    composer.set_margin_bottom(12);
    composer.set_margin_start(12);
    composer.set_margin_end(12);
    composer.append(&prompt_scroller);
    composer.append(&model_row);
    composer.append(&reference_row);
    composer.append(&settings_row);
    composer.append(&status);
    composer.append(&controls);

    // The composer must not be an AdwToolbarView bottom bar: libadwaita
    // wraps every bottom bar in a GtkWindowHandle, which makes the whole
    // prompt area drag the window and double-click maximize it. Keep the
    // composer in the normal content area and only the header as a toolbar.
    let page_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page_content.set_vexpand(true);
    page_content.append(&content);
    page_content.append(&composer);

    let toolbar_view = adw::ToolbarView::builder().content(&page_content).build();
    toolbar_view.add_top_bar(&header);

    let history_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .build();
    let history_scroller = gtk::ScrolledWindow::builder()
        .child(&history_list)
        .vexpand(true)
        .build();
    let sidebar_title = gtk::Label::builder()
        .label("History")
        .css_classes(["title-4"])
        .build();
    let sidebar_header = adw::HeaderBar::new();
    sidebar_header.set_show_start_title_buttons(false);
    sidebar_header.set_show_end_title_buttons(false);
    sidebar_header.set_title_widget(Some(&sidebar_title));
    let sidebar = adw::ToolbarView::builder().content(&history_scroller).build();
    sidebar.add_top_bar(&sidebar_header);

    let sidebar_page = adw::NavigationPage::new(&sidebar, "History");
    let content_page = adw::NavigationPage::new(&toolbar_view, flux_core::APP_NAME);
    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar_page)
        .content(&content_page)
        .max_sidebar_width(320.0)
        .build();

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&split));
    window.set_content(Some(&toast));

    let ui = Rc::new(Ui {
        window,
        stack,
        picture,
        prompt,
        generate,
        cancel,
        spinner,
        progress,
        status,
        toast,
        backend,
        models_dir,
        resolution: Cell::new((config.default_width, config.default_height)),
        config: RefCell::new(config),
        history: RefCell::new(history),
        history_list,
        entries: RefCell::new(Vec::new()),
        model_dropdown,
        resolution_dropdown,
        seed_entry,
        reference: RefCell::new(None),
        reference_button,
        reference_picture,
        reference_name,
        reference_clear,
        generating: Cell::new(false),
    });

    {
        let ui = ui.clone();
        let button = ui.generate.clone();
        button.connect_clicked(move |_| start_generation(&ui));
    }
    {
        let ui = ui.clone();
        preferences_button.connect_clicked(move |_| show_preferences(&ui, false));
    }
    {
        let ui = ui.clone();
        let button = ui.cancel.clone();
        button.connect_clicked(move |_| {
            if let Err(error) = ui.backend.cancel() {
                ui.toast
                    .add_toast(adw::Toast::new(&format!("Cancel failed: {error}")));
            }
            ui.status.set_text("Cancelling…");
        });
    }
    {
        let ui = ui.clone();
        let button = ui.reference_button.clone();
        button.connect_clicked(move |_| choose_reference(&ui));
    }
    {
        let ui = ui.clone();
        let button = ui.reference_clear.clone();
        button.connect_clicked(move |_| set_reference(&ui, None));
    }
    {
        let ui = ui.clone();
        let dropdown = ui.model_dropdown.clone();
        dropdown.connect_selected_notify(move |dropdown| {
            let index = dropdown.selected() as usize;
            let Some(model) = PRESETS.get(index) else {
                return;
            };
            dropdown.set_tooltip_text(Some(model.description));
            {
                let mut config = ui.config.borrow_mut();
                config.active_preset = model.id.to_string();
                let _ = config.save();
            }
            update_model_state(&ui);
            if !model.is_installed(&ui.models_dir) {
                ui.toast.add_toast(adw::Toast::new(&format!(
                    "{} needs {} model file(s) in {}",
                    model.label,
                    model.missing(&ui.models_dir).len(),
                    ui.models_dir.display()
                )));
            }
        });
    }
    {
        let ui = ui.clone();
        let dropdown = ui.resolution_dropdown.clone();
        dropdown.connect_selected_notify(move |dropdown| {
            let index = dropdown.selected() as usize;
            if let Some((_, width, height)) = RESOLUTIONS.get(index) {
                ui.resolution.set((*width, *height));
                ui.status.set_text(&format!("{width} × {height}"));
                let mut config = ui.config.borrow_mut();
                config.default_width = *width;
                config.default_height = *height;
                let _ = config.save();
            }
        });
    }
    {
        let ui = ui.clone();
        let list = ui.history_list.clone();
        list.connect_row_activated(move |_, row| {
            // Resolve by stored id: row position and entry order must never be
            // assumed to match.
            let Ok(id) = row.widget_name().parse::<i64>() else {
                return;
            };
            let entry = ui.entries.borrow().iter().find(|e| e.id == id).cloned();
            let Some(entry) = entry else {
                return;
            };

            if let Ok(texture) = gdk::Texture::from_filename(Path::new(&entry.image_path)) {
                ui.picture.set_paintable(Some(&texture));
                ui.stack.set_visible_child_name("image");
            }
            ui.prompt.buffer().set_text(&entry.prompt);
            ui.seed_entry.set_text(&entry.seed);
            if let Some(index) = RESOLUTIONS
                .iter()
                .position(|(_, width, height)| *width == entry.width && *height == entry.height)
            {
                ui.resolution_dropdown.set_selected(index as u32);
            }
            if let Some(index) = PRESETS.iter().position(|model| model.id == entry.model) {
                ui.model_dropdown.set_selected(index as u32);
            }
            set_reference(&ui, entry.reference_path.as_ref().map(PathBuf::from));
            ui.status.set_text(&format!(
                "{} · {} × {} · seed {}",
                model_label(&entry.model),
                entry.width,
                entry.height,
                entry.seed
            ));
        });
    }
    {
        let ui = ui.clone();
        let window = ui.window.clone();
        window.connect_close_request(move |_| {
            let _ = ui.config.borrow().save();
            glib::Propagation::Proceed
        });
    }

    let key_controller = gtk::EventControllerKey::new();
    // Capture phase: the focused TextView would otherwise consume Return
    // before the window-level shortcut sees it.
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let ui = ui.clone();
        key_controller.connect_key_pressed(move |_, key, _, state| {
            if key == gdk::Key::Return && state.contains(gdk::ModifierType::CONTROL_MASK) {
                start_generation(&ui);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    ui.window.add_controller(key_controller);

    if let Ok(entries) = ui.history.borrow().recent(HISTORY_LIMIT) {
        for entry in &entries {
            add_history_row(&ui, entry, false);
        }
        *ui.entries.borrow_mut() = entries;
    }

    // Materialize the config on first run so later edits have a base file.
    if let Err(error) = ui.config.borrow().save() {
        tracing::warn!(%error, "failed to write config");
    }

    update_model_state(&ui);
    ui.window.present();
    ui.prompt.grab_focus();

    if !active_preset(&ui).is_installed(&ui.models_dir) {
        show_preferences(&ui, true);
    }
}

fn active_preset(ui: &Ui) -> &'static ModelPreset {
    preset(&ui.config.borrow().active_preset).unwrap_or_else(default_preset)
}

fn update_model_state(ui: &Rc<Ui>) {
    if ui.generating.get() {
        return;
    }
    let model = active_preset(ui);
    let missing = model.missing(&ui.models_dir);

    ui.reference_button.set_sensitive(model.supports_edit);
    ui.reference_button.set_tooltip_text(Some(if model.supports_edit {
        "Use an image as the starting point (edit mode)"
    } else {
        "This model does not support reference images"
    }));
    if !model.supports_edit {
        set_reference(ui, None);
    }

    if missing.is_empty() {
        ui.generate.set_sensitive(true);
        ui.status.set_text(&format!(
            "{} · {} steps · {:.1} GB",
            model.label,
            model.steps,
            model.total_bytes() as f64 / 1e9
        ));
    } else {
        ui.generate.set_sensitive(false);
        let names: Vec<&str> = missing.iter().map(|file| file.rel_path).collect();
        ui.status.set_text(&format!(
            "{}: missing {} in {}",
            model.label,
            names.join(", "),
            ui.models_dir.display()
        ));
    }
}

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

fn show_preferences(ui: &Rc<Ui>, focus_models: bool) {
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
            let mut config = ui.config.borrow_mut();
            config.offload = row.is_active();
            let _ = config.save();
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

    let download_group = adw::PreferencesGroup::builder()
        .title("Downloads")
        .build();
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

    let (progress_tx, progress_rx) =
        async_channel::unbounded::<flux_models::DownloadProgress>();
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
            update_model_state(&ui);
        });
    }
}

fn choose_reference(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .modal(true)
        .build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    for mime in ["image/png", "image/jpeg", "image/webp"] {
        filter.add_mime_type(mime);
    }
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    let ui = ui.clone();
    let window = ui.window.clone();
    dialog.open(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            return;
        };
        set_reference(&ui, Some(path));
    });
}

fn set_reference(ui: &Rc<Ui>, path: Option<PathBuf>) {
    match path {
        Some(path) => {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            ui.reference_name.set_text(&name);
            ui.reference_name
                .set_tooltip_text(Some(&path.display().to_string()));
            ui.reference_name.set_visible(true);
            ui.reference_clear.set_visible(true);
            if let Ok(texture) = gdk::Texture::from_filename(&path) {
                ui.reference_picture.set_paintable(Some(&texture));
                ui.reference_picture.set_visible(true);
            }
            if !ui.generating.get() {
                ui.status
                    .set_text("Edit mode: describe the change in the prompt");
            }
            *ui.reference.borrow_mut() = Some(path);
        }
        None => {
            ui.reference_name.set_visible(false);
            ui.reference_clear.set_visible(false);
            ui.reference_picture.set_visible(false);
            ui.reference_picture.set_paintable(gdk::Paintable::NONE);
            *ui.reference.borrow_mut() = None;
        }
    }
}

fn start_generation(ui: &Rc<Ui>) {
    if ui.generating.get() || !ui.generate.is_sensitive() {
        return;
    }

    let buffer = ui.prompt.buffer();
    let prompt = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .trim()
        .to_string();
    if prompt.is_empty() {
        ui.toast.add_toast(adw::Toast::new("Enter a prompt first."));
        return;
    }

    let seed_text = ui.seed_entry.text().trim().to_string();
    let seed = if seed_text.is_empty() {
        GenerateRequest::random_seed()
    } else {
        match seed_text.parse::<u64>() {
            Ok(seed) if seed <= GenerateRequest::MAX_SEED => seed,
            Ok(_) => {
                ui.toast.add_toast(adw::Toast::new(&format!(
                    "Seed must be at most {}.",
                    GenerateRequest::MAX_SEED
                )));
                return;
            }
            Err(_) => {
                ui.toast
                    .add_toast(adw::Toast::new("Seed must be a non-negative integer."));
                return;
            }
        }
    };

    let model = active_preset(ui);
    if !model.is_installed(&ui.models_dir) {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} is missing model files (see the status line).",
            model.label
        )));
        return;
    }

    let reference = ui.reference.borrow().clone();
    if reference.is_some() && !model.supports_edit {
        ui.toast
            .add_toast(adw::Toast::new("This model does not support editing."));
        return;
    }

    let (width, height) = ui.resolution.get();
    let output_dir = ui.config.borrow().output_dir.clone();
    let offload = ui.config.borrow().offload;
    let output = output_dir.join(format!(
        "{}-{}-{}.png",
        model.id,
        now_millis(),
        seed
    ));

    let mut request = GenerateRequest::new(prompt.clone(), output);
    request.preset_id = model.id.to_string();
    request.width = width;
    request.height = height;
    request.steps = model.steps;
    request.cfg = model.cfg;
    request.seed = seed;
    request.sampler = model.sampler;
    request.offload = offload;
    request.reference_images = reference.iter().cloned().collect();

    ui.generating.set(true);
    ui.generate.set_sensitive(false);
    ui.model_dropdown.set_sensitive(false);
    ui.reference_button.set_sensitive(false);
    ui.cancel.set_sensitive(true);
    ui.spinner.set_visible(true);
    ui.spinner.start();
    ui.progress.set_visible(true);
    ui.progress.set_fraction(0.0);
    ui.progress.set_text(Some("Loading models…"));
    ui.status.set_text("Loading models…");

    let (sender, receiver) = async_channel::unbounded();
    let backend = ui.backend.clone();
    let worker_request = request.clone();
    std::thread::spawn(move || {
        if let Err(error) = backend.generate(&worker_request, sender.clone()) {
            let _ = sender.send_blocking(Progress::Failed(error.to_string()));
        }
    });

    let ui = ui.clone();
    let prompt_for_history = prompt;
    let model_for_history = model.id.to_string();
    let seed_for_history = seed.to_string();
    let steps_for_history = request.steps;
    let reference_for_history = reference
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let started = Instant::now();
    glib::spawn_future_local(async move {
        while let Ok(message) = receiver.recv().await {
            match message {
                Progress::Loading => {
                    ui.status.set_text("Loading models…");
                    ui.progress.pulse();
                }
                Progress::Step { current, total } => {
                    ui.progress.set_fraction(current as f64 / total as f64);
                    ui.progress
                        .set_text(Some(&format!("Step {current}/{total}")));
                    ui.status.set_text(&format!("Step {current}/{total}"));
                }
                Progress::Log(line) => tracing::debug!(target: "sd-cli", "{line}"),
                Progress::Cancelled => {
                    ui.toast.add_toast(adw::Toast::new("Generation cancelled."));
                    finish(&ui, "Cancelled");
                    break;
                }
                Progress::Done(path) => {
                    if let Ok(texture) = gdk::Texture::from_filename(&path) {
                        ui.picture.set_paintable(Some(&texture));
                        ui.stack.set_visible_child_name("image");
                    } else {
                        ui.toast.add_toast(adw::Toast::new(&format!(
                            "Generated {}, but could not load it.",
                            path.display()
                        )));
                    }

                    let duration_ms = started.elapsed().as_millis() as i64;
                    let image_path = path.to_string_lossy().into_owned();
                    let created_at = unix_seconds();
                    let inserted = ui.history.borrow().insert(&NewGeneration {
                        prompt: &prompt_for_history,
                        model: &model_for_history,
                        width,
                        height,
                        steps: steps_for_history,
                        seed: &seed_for_history,
                        created_at,
                        duration_ms,
                        image_path: &image_path,
                        reference_path: reference_for_history.as_deref(),
                    });

                    match inserted {
                        Ok(id) => {
                            if let Err(error) = write_thumbnail(&path, id) {
                                tracing::warn!(%error, "failed to write thumbnail");
                            }
                            let entry = Generation {
                                id,
                                prompt: prompt_for_history.clone(),
                                model: model_for_history.clone(),
                                width,
                                height,
                                steps: steps_for_history,
                                seed: seed_for_history.clone(),
                                created_at,
                                duration_ms,
                                image_path,
                                reference_path: reference_for_history.clone(),
                            };
                            ui.entries.borrow_mut().insert(0, entry.clone());
                            add_history_row(&ui, &entry, true);
                            trim_history(&ui);
                        }
                        Err(error) => {
                            tracing::error!(%error, "failed to record history");
                        }
                    }

                    ui.seed_entry.set_text(&seed_for_history);
                    ui.toast.add_toast(adw::Toast::new("Image generated."));
                    finish(
                        &ui,
                        &format!(
                            "{} · {width} × {height} · seed {seed_for_history}",
                            model_label(&model_for_history)
                        ),
                    );
                    break;
                }
                Progress::Failed(error) => {
                    ui.toast
                        .add_toast(adw::Toast::new(&format!("Generation failed: {error}")));
                    finish(&ui, "Failed");
                    break;
                }
            }
        }
    });
}

fn finish(ui: &Rc<Ui>, message: &str) {
    ui.generating.set(false);
    ui.cancel.set_sensitive(false);
    ui.model_dropdown.set_sensitive(true);
    ui.spinner.stop();
    ui.spinner.set_visible(false);
    ui.progress.set_visible(false);
    update_model_state(ui);
    ui.status.set_text(message);
}

fn add_history_row(ui: &Rc<Ui>, entry: &Generation, prepend: bool) {
    let row = gtk::ListBoxRow::new();
    // Keep the history id on the widget so activation never depends on the
    // row's position relative to `ui.entries`.
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
    let prompt_label = gtk::Label::builder()
        .label(&entry.prompt)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(28)
        .build();
    let edit_suffix = if entry.reference_path.is_some() {
        " · edit"
    } else {
        ""
    };
    let meta_label = gtk::Label::builder()
        .label(format!(
            "{} · {} × {}{}",
            model_label(&entry.model),
            entry.width,
            entry.height,
            edit_suffix
        ))
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(28)
        .css_classes(["caption", "dim-label"])
        .build();
    text.append(&prompt_label);
    text.append(&meta_label);

    row_box.append(&thumbnail);
    row_box.append(&text);
    row.set_child(Some(&row_box));
    row.set_tooltip_text(Some(&format!(
        "{}\n{} · {} × {} · {} steps · {:.1}s · seed {} · {}{}",
        entry.prompt,
        model_label(&entry.model),
        entry.width,
        entry.height,
        entry.steps,
        entry.duration_ms as f64 / 1000.0,
        entry.seed,
        relative_time(entry.created_at),
        entry
            .reference_path
            .as_deref()
            .map(|path| format!("\nreference: {path}"))
            .unwrap_or_default()
    )));

    if prepend {
        ui.history_list.prepend(&row);
    } else {
        ui.history_list.append(&row);
    }
}

fn model_label(id: &str) -> &str {
    preset(id).map(|model| model.label).unwrap_or(id)
}

fn trim_history(ui: &Rc<Ui>) {
    loop {
        if ui.entries.borrow().len() <= HISTORY_LIMIT {
            break;
        }
        ui.entries.borrow_mut().pop();
        if let Some(row) = ui.history_list.last_child() {
            ui.history_list.remove(&row);
        }
    }
}

fn write_thumbnail(source: &Path, id: i64) -> anyhow::Result<()> {
    let dir = paths::thumbs_dir();
    std::fs::create_dir_all(&dir)?;
    let image = image::open(source)?;
    image.thumbnail(128, 128).save(dir.join(format!("{id}.png")))?;
    Ok(())
}

fn thumb_path(id: i64) -> PathBuf {
    paths::thumbs_dir().join(format!("{id}.png"))
}

fn resolve_models_dir(config: &Config) -> PathBuf {
    if let Ok(dir) = std::env::var("FLUX_MODELS_DIR") {
        return PathBuf::from(dir);
    }
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models");
    if local.exists() {
        return local;
    }
    config.models_dir.clone()
}

fn resolve_sd_cli(config: &Config) -> PathBuf {
    if let Ok(path) = std::env::var("FLUX_SD_CLI") {
        return PathBuf::from(path);
    }
    if let Some(path) = &config.sd_cli {
        return path.clone();
    }
    let installed = PathBuf::from("/usr/lib/flux-studio/sd-cli");
    if installed.exists() {
        return installed;
    }
    let vendored = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/stable-diffusion.cpp/build/bin/sd-cli");
    if vendored.exists() {
        return vendored;
    }
    PathBuf::from("sd-cli")
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn relative_time(created_at: i64) -> String {
    let seconds = (unix_seconds() - created_at).max(0);
    if seconds < 60 {
        "just now".to_string()
    } else if seconds < 3_600 {
        format!("{} min ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} h ago", seconds / 3_600)
    } else {
        format!("{} d ago", seconds / 86_400)
    }
}
