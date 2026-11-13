mod history;
mod history_view;
mod main_view;
mod preferences;
mod prompt_ai;
mod sidebar;
mod state;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use adw::prelude::*;
use flux_backend_sdcpp::{SdServer, SdServerConfig};
use flux_core::preset::{ModelPreset, ModelRole, PRESETS, default_preset, preset};
use flux_core::style::STYLES;
use flux_core::{Backend, Config, GenerateRequest, Progress, paths, size};
use gtk::{gdk, glib};

use history::{Generation, History, NewGeneration};
use state::AppState;

const HISTORY_LIMIT: usize = 100;

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

pub(crate) struct Ui {
    pub window: adw::ApplicationWindow,
    pub split: adw::OverlaySplitView,
    pub toast: adw::ToastOverlay,
    pub stack: gtk::Stack,
    pub picture: gtk::Picture,
    pub reference_bar: gtk::Box,
    pub reference_name: gtk::Label,
    pub prompt: gtk::TextView,
    pub generate: gtk::Button,
    pub cancel: gtk::Button,
    pub progress: gtk::ProgressBar,
    pub status: gtk::Label,
    pub reference_button: gtk::Button,
    pub improve_button: gtk::Button,
    pub improving: Cell<bool>,
    pub prompt_helper: Option<PathBuf>,
    pub backend: Arc<dyn Backend>,
    pub worker: GenerationWorker,
    pub models_dir: PathBuf,
    pub config: RefCell<Config>,
    pub history: RefCell<History>,
    pub history_list: gtk::ListBox,
    pub new_button: gtk::Button,
    pub entries: RefCell<Vec<Generation>>,
    pub model_dropdown: gtk::DropDown,
    pub model_hint: gtk::Label,
    pub style_dropdown: gtk::DropDown,
    pub width_scale: gtk::Scale,
    pub height_scale: gtk::Scale,
    pub width_value: gtk::Label,
    pub height_value: gtk::Label,
    pub ratio_area: gtk::DrawingArea,
    pub ratio_label: gtk::Label,
    pub mp_label: gtk::Label,
    pub size_hint: gtk::Label,
    pub seed_entry: gtk::Entry,
    pub random_button: gtk::Button,
    pub offload_switch: gtk::Switch,
    pub advanced_summary: gtk::Label,
    pub state: RefCell<AppState>,
    pub generating: Cell<bool>,
    pub size_guard: Cell<bool>,
    pub size_cell: Rc<Cell<(u32, u32)>>,
}

/// Runs generations on one long-lived thread.
///
/// The persistent `sd-server` backend sets `PR_SET_PDEATHSIG` on the server
/// process, which follows the thread that spawned it. Keeping a single worker
/// alive for the app lifetime lets the server survive across generations while
/// still dying with the app.
struct GenerationWorker {
    jobs: async_channel::Sender<(GenerateRequest, async_channel::Sender<Progress>)>,
}

impl GenerationWorker {
    fn new(backend: Arc<dyn Backend>) -> Self {
        let (jobs, queue) =
            async_channel::unbounded::<(GenerateRequest, async_channel::Sender<Progress>)>();
        std::thread::spawn(move || {
            while let Ok((request, progress)) = queue.recv_blocking() {
                if let Err(error) = backend.generate(&request, progress.clone()) {
                    let _ = progress.send_blocking(Progress::Failed(error.to_string()));
                }
            }
        });
        Self { jobs }
    }
}

fn install_styles() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".prompt-field {
             background-color: @card_bg_color;
             border: 1px solid alpha(@view_fg_color, 0.15);
             border-radius: 9px;
         }
         .prompt-field textview {
             background-color: transparent;
         }
         .prompt-field scrolledwindow {
             background-color: transparent;
         }
         .prompt-field:focus-within {
             border-color: @accent_bg_color;
         }
         .reference-bar {
             background-color: @accent_bg_color;
             color: @accent_fg_color;
             border-radius: 6px;
             padding: 4px 10px;
         }
         .sidebar-pane, .overlay-pane {
             background-color: @sidebar_bg_color;
         }",
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn build_ui(app: &adw::Application) {
    install_styles();

    let config = Config::load();
    let models_dir = resolve_models_dir(&config);
    let sd_server = resolve_sd_server(&config);

    let backend: Arc<dyn Backend> = Arc::new(SdServer::new(SdServerConfig {
        cli: sd_server,
        models_dir: models_dir.clone(),
    }));

    let history = match History::open(&paths::history_db()) {
        Ok(history) => history,
        Err(error) => {
            tracing::error!(%error, "failed to open history database; using memory");
            History::open_in_memory().expect("in-memory history")
        }
    };

    let size_cell = Rc::new(Cell::new((
        size::snap(config.default_width),
        size::snap(config.default_height),
    )));

    let prompt_helper = prompt_ai::find_cli(&config);
    let view = main_view::build();
    view.improve_button.set_visible(prompt_helper.is_some());
    let side = sidebar::build(&config, size_cell.clone());

    let header = adw::HeaderBar::new();
    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Toggle sidebar (Ctrl+B)")
        .active(config.sidebar_visible)
        .build();
    header.pack_start(&sidebar_toggle);

    let preferences_button = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Preferences")
        .build();
    header.pack_end(&preferences_button);

    // The toggle and window controls live in the content pane's header, so the
    // sidebar can occupy the full window height.
    let content_view = adw::ToolbarView::builder().content(&view.root).build();
    content_view.add_top_bar(&header);

    let split = adw::OverlaySplitView::builder()
        .sidebar(&side.root)
        .content(&content_view)
        .min_sidebar_width(340.0)
        .max_sidebar_width(420.0)
        .show_sidebar(config.sidebar_visible)
        .build();

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&split));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(flux_core::APP_NAME)
        .default_width(1240)
        .default_height(860)
        .build();
    window.set_content(Some(&toast));

    let worker = GenerationWorker::new(backend.clone());

    let ui = Rc::new(Ui {
        window,
        split: split.clone(),
        toast,
        stack: view.stack,
        picture: view.picture,
        reference_bar: view.reference_bar,
        reference_name: view.reference_name,
        prompt: view.prompt,
        generate: view.generate,
        cancel: view.cancel,
        progress: view.progress,
        status: view.status,
        reference_button: view.reference_button,
        improve_button: view.improve_button,
        improving: Cell::new(false),
        prompt_helper,
        backend,
        worker,
        models_dir,
        config: RefCell::new(config),
        history: RefCell::new(history),
        history_list: side.history_list,
        new_button: side.new_button,
        entries: RefCell::new(Vec::new()),
        model_dropdown: side.model_dropdown,
        model_hint: side.model_hint,
        style_dropdown: side.style_dropdown,
        width_scale: side.width_scale,
        height_scale: side.height_scale,
        width_value: side.width_value,
        height_value: side.height_value,
        ratio_area: side.ratio_area,
        ratio_label: side.ratio_label,
        mp_label: side.mp_label,
        size_hint: side.size_hint,
        seed_entry: side.seed_entry,
        random_button: side.random_button,
        offload_switch: side.offload_switch,
        advanced_summary: side.advanced_summary,
        state: RefCell::new(AppState::default()),
        generating: Cell::new(false),
        size_guard: Cell::new(false),
        size_cell,
    });

    sidebar_toggle
        .bind_property("active", &split, "show-sidebar")
        .bidirectional()
        .build();

    wire_events(&ui, &preferences_button);

    if let Ok(entries) = ui.history.borrow().recent(HISTORY_LIMIT) {
        for entry in &entries {
            history_view::add_row(&ui, entry, false);
        }
        *ui.entries.borrow_mut() = entries;
    }

    if let Err(error) = ui.config.borrow().save() {
        tracing::warn!(%error, "failed to write config");
    }

    refresh_model(&ui);
    update_size_ui(&ui);
    update_image_view(&ui);

    ui.window.present();
    ui.prompt.grab_focus();

    if !active_preset(&ui).is_installed(&ui.models_dir) {
        preferences::show_preferences(&ui, true);
    }
}

fn wire_events(ui: &Rc<Ui>, preferences_button: &gtk::Button) {
    {
        let ui = ui.clone();
        let button = ui.generate.clone();
        button.connect_clicked(move |_| start_generation(&ui));
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
        let button = ui.improve_button.clone();
        button.connect_clicked(move |_| improve_prompt(&ui));
    }
    {
        let ui = ui.clone();
        let button = ui.new_button.clone();
        button.connect_clicked(move |_| new_image(&ui));
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
            // Free the previous model right away instead of waiting for the
            // idle timeout.
            ui.backend.unload();
            refresh_model(&ui);
        });
    }
    {
        let ui = ui.clone();
        let dropdown = ui.style_dropdown.clone();
        dropdown.connect_selected_notify(move |dropdown| {
            let index = dropdown.selected() as usize;
            let Some(style) = STYLES.get(index) else {
                return;
            };
            let mut config = ui.config.borrow_mut();
            config.style_id = style.id.to_string();
            let _ = config.save();
        });
    }
    {
        let ui = ui.clone();
        let scale = ui.width_scale.clone();
        scale.connect_value_changed(move |_| update_size_ui(&ui));
    }
    {
        let ui = ui.clone();
        let scale = ui.height_scale.clone();
        scale.connect_value_changed(move |_| update_size_ui(&ui));
    }
    {
        let entry = ui.seed_entry.clone();
        let button = ui.random_button.clone();
        button.connect_clicked(move |_| {
            entry.set_text(&GenerateRequest::random_seed().to_string());
        });
    }
    {
        let ui = ui.clone();
        let switch = ui.offload_switch.clone();
        switch.connect_active_notify(move |switch| {
            let active = switch.is_active();
            let mut config = ui.config.borrow_mut();
            if config.offload != active {
                config.offload = active;
                let _ = config.save();
            }
            drop(config);
            // Offload is a server start-up flag, so release the context.
            ui.backend.unload();
        });
    }
    {
        let ui = ui.clone();
        let list = ui.history_list.clone();
        list.connect_row_activated(move |_, row| {
            if ui.generating.get() {
                return;
            }
            let Ok(id) = row.widget_name().parse::<i64>() else {
                return;
            };
            let entry = ui
                .entries
                .borrow()
                .iter()
                .find(|entry| entry.id == id)
                .cloned();
            if let Some(entry) = entry {
                history_view::load_entry(&ui, &entry);
            }
        });
    }
    {
        let ui = ui.clone();
        let button = preferences_button.clone();
        button.connect_clicked(move |_| preferences::show_preferences(&ui, false));
    }
    {
        let ui = ui.clone();
        let split = ui.split.clone();
        split.connect_show_sidebar_notify(move |split| {
            let visible = split.shows_sidebar();
            let mut config = ui.config.borrow_mut();
            if config.sidebar_visible != visible {
                config.sidebar_visible = visible;
                let _ = config.save();
            }
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
    // Capture phase: the focused TextView would otherwise consume Return.
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let ui = ui.clone();
        let split = ui.split.clone();
        key_controller.connect_key_pressed(move |_, key, _, state| {
            if !state.contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Return => {
                    start_generation(&ui);
                    glib::Propagation::Stop
                }
                gdk::Key::b | gdk::Key::B => {
                    split.set_show_sidebar(!split.shows_sidebar());
                    glib::Propagation::Stop
                }
                gdk::Key::n | gdk::Key::N => {
                    new_image(&ui);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
    }
    ui.window.add_controller(key_controller);
}

pub(crate) fn active_preset(ui: &Rc<Ui>) -> &'static ModelPreset {
    preset(&ui.config.borrow().active_preset).unwrap_or_else(default_preset)
}

pub(crate) fn refresh_model(ui: &Rc<Ui>) {
    let model = active_preset(ui);
    let missing = model.missing(&ui.models_dir);

    ui.model_dropdown.set_tooltip_text(Some(model.description));
    ui.advanced_summary.set_text(&format!(
        "{} steps · CFG {:.1} · {} (fixed by preset)",
        model.steps,
        model.cfg,
        model.sampler.as_str()
    ));

    if missing.is_empty() {
        ui.model_hint.set_text(&format!(
            "{} · {:.1} GB · installed",
            model.label,
            model.total_bytes() as f64 / 1e9
        ));
        ui.model_hint.remove_css_class("error");
        if !ui.generating.get() {
            ui.generate.set_sensitive(true);
        }
    } else {
        let names: Vec<&str> = missing.iter().map(|file| file.rel_path).collect();
        ui.model_hint.set_text(&format!(
            "Missing {} — download in Preferences",
            names.join(", ")
        ));
        ui.model_hint.add_css_class("error");
        ui.generate.set_sensitive(false);
    }

    if ui.prompt_helper.is_some() {
        let encoder = model.path(ModelRole::TextEncoder, &ui.models_dir);
        let encoder_ready = encoder.as_ref().map(|path| path.exists()).unwrap_or(false);
        if !ui.generating.get() && !ui.improving.get() {
            ui.improve_button.set_sensitive(encoder_ready);
        }
        let tooltip = match encoder {
            Some(path) if path.exists() => format!(
                "Improve the prompt with {} (llama.cpp)",
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            ),
            _ => "Text encoder not installed".to_string(),
        };
        ui.improve_button.set_tooltip_text(Some(&tooltip));
    }
}

pub(crate) fn update_size_ui(ui: &Rc<Ui>) {
    if ui.size_guard.get() {
        return;
    }
    let width = size::snap(ui.width_scale.value() as u32);
    let height = size::snap(ui.height_scale.value() as u32);

    ui.size_guard.set(true);
    if (ui.width_scale.value() as u32) != width {
        ui.width_scale.set_value(width as f64);
    }
    if (ui.height_scale.value() as u32) != height {
        ui.height_scale.set_value(height as f64);
    }
    ui.size_guard.set(false);

    ui.width_value.set_text(&width.to_string());
    ui.height_value.set_text(&height.to_string());
    ui.ratio_label.set_text(&size::aspect_label(width, height));
    let megapixels = size::megapixels(width, height);
    ui.mp_label.set_text(&format!("{megapixels:.2} MP"));
    if size::is_large(width, height) {
        ui.size_hint
            .set_text("Large size — slower and may exceed 8 GB VRAM");
        ui.size_hint.add_css_class("warning");
    } else {
        ui.size_hint.set_text("");
        ui.size_hint.remove_css_class("warning");
    }
    ui.size_cell.set((width, height));
    ui.ratio_area.queue_draw();

    let mut config = ui.config.borrow_mut();
    config.default_width = width;
    config.default_height = height;
    let _ = config.save();
}

pub(crate) fn set_size(ui: &Rc<Ui>, width: u32, height: u32) {
    ui.size_guard.set(true);
    ui.width_scale.set_value(size::snap(width) as f64);
    ui.height_scale.set_value(size::snap(height) as f64);
    ui.size_guard.set(false);
    update_size_ui(ui);
}

pub(crate) fn update_image_view(ui: &Rc<Ui>) {
    let (display, reference) = {
        let state = ui.state.borrow();
        (state.display.clone(), state.reference.clone())
    };

    match display {
        Some(path) => match gdk::Texture::from_filename(&path) {
            Ok(texture) => {
                ui.picture.set_paintable(Some(&texture));
                ui.stack.set_visible_child_name("image");
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "failed to load image");
                ui.picture.set_paintable(gdk::Paintable::NONE);
                ui.stack.set_visible_child_name("empty");
            }
        },
        None => {
            ui.picture.set_paintable(gdk::Paintable::NONE);
            ui.stack.set_visible_child_name("empty");
        }
    }

    match reference {
        Some(path) => {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            ui.reference_name.set_text(&name);
            ui.reference_name
                .set_tooltip_text(Some(&path.display().to_string()));
            ui.reference_bar.set_visible(true);
        }
        None => {
            ui.reference_name.set_text("");
            ui.reference_name.set_tooltip_text(None);
            ui.reference_bar.set_visible(false);
        }
    }
}

pub(crate) fn new_image(ui: &Rc<Ui>) {
    if ui.generating.get() {
        return;
    }
    ui.state.borrow_mut().new_image();
    update_image_view(ui);
    ui.status.set_text("New image");
    ui.prompt.grab_focus();
}

fn set_controls_enabled(ui: &Rc<Ui>, enabled: bool) {
    ui.model_dropdown.set_sensitive(enabled);
    ui.style_dropdown.set_sensitive(enabled);
    ui.width_scale.set_sensitive(enabled);
    ui.height_scale.set_sensitive(enabled);
    ui.seed_entry.set_sensitive(enabled);
    ui.random_button.set_sensitive(enabled);
    ui.offload_switch.set_sensitive(enabled);
    ui.new_button.set_sensitive(enabled);
    ui.history_list.set_sensitive(enabled);
    ui.reference_button.set_sensitive(enabled);
    ui.improve_button
        .set_sensitive(enabled && !ui.improving.get());
    ui.generate.set_visible(enabled);
    ui.cancel.set_visible(!enabled);
    ui.cancel.set_sensitive(!enabled);
}

fn choose_reference(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title("Choose a reference image")
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
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        ui.state.borrow_mut().load(path);
        update_image_view(&ui);
        ui.status.set_text(&format!("Editing reference · {name}"));
        ui.prompt.grab_focus();
    });
}

fn improve_prompt(ui: &Rc<Ui>) {
    if ui.generating.get() || ui.improving.get() {
        return;
    }

    let original = current_prompt(ui).trim().to_string();
    if original.is_empty() {
        ui.toast.add_toast(adw::Toast::new("Write a prompt first."));
        return;
    }

    let Some(cli) = ui.prompt_helper.clone() else {
        return;
    };
    let encoder = ui
        .config
        .borrow()
        .prompt_helper_model
        .clone()
        .or_else(|| active_preset(ui).path(ModelRole::TextEncoder, &ui.models_dir));
    let Some(encoder) = encoder else {
        ui.toast
            .add_toast(adw::Toast::new("This model has no text encoder."));
        return;
    };
    if !encoder.exists() {
        ui.toast.add_toast(adw::Toast::new(
            "The text encoder is not installed. Download the model in Preferences.",
        ));
        return;
    }

    let original_for_worker = original.clone();
    ui.improving.set(true);
    ui.improve_button.set_sensitive(false);
    ui.status.set_text("Improving prompt…");

    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = sender.send_blocking(prompt_ai::improve(&original_for_worker, &encoder, &cli));
    });

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let result = receiver.recv().await;
        ui.improving.set(false);
        refresh_model(&ui);

        match result {
            Ok(Ok(improved)) => {
                if ui.generating.get() {
                    ui.status.set_text("Improvement discarded");
                    ui.toast.add_toast(adw::Toast::new(
                        "Generation started; improvement discarded.",
                    ));
                    return;
                }
                if current_prompt(&ui).trim() != original {
                    ui.status.set_text("Prompt changed");
                    ui.toast
                        .add_toast(adw::Toast::new("Prompt changed; improvement discarded."));
                    return;
                }

                ui.prompt.buffer().set_text(&improved);
                ui.prompt.grab_focus();
                ui.status.set_text("Prompt improved");

                let toast = adw::Toast::new("Prompt improved.");
                toast.set_button_label(Some("Undo"));
                let prompt = ui.prompt.clone();
                let old = original.clone();
                let new = improved.clone();
                toast.connect_button_clicked(move |_| {
                    let buffer = prompt.buffer();
                    let current = buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), false)
                        .trim()
                        .to_string();
                    if current == new.trim() {
                        buffer.set_text(&old);
                    }
                });
                ui.toast.add_toast(toast);
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "prompt improvement failed");
                ui.status.set_text("Prompt helper unavailable");
                ui.toast.add_toast(adw::Toast::new(&format!(
                    "Could not improve prompt: {error}"
                )));
            }
            Err(_) => {
                ui.status.set_text("Prompt helper unavailable");
            }
        }
    });
}

fn current_prompt(ui: &Rc<Ui>) -> String {
    let buffer = ui.prompt.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn start_generation(ui: &Rc<Ui>) {
    if ui.generating.get() || !ui.generate.is_sensitive() {
        return;
    }

    let buffer = ui.prompt.buffer();
    let base_prompt = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .trim()
        .to_string();
    if base_prompt.is_empty() {
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
            "{} is missing model files (see Preferences).",
            model.label
        )));
        return;
    }

    let style_id = STYLES
        .get(ui.style_dropdown.selected() as usize)
        .map(|style| style.id)
        .unwrap_or("none");
    let effective_prompt = flux_core::style::compose_prompt(&base_prompt, style_id);

    let width = size::snap(ui.width_scale.value() as u32);
    let height = size::snap(ui.height_scale.value() as u32);
    let reference = ui.state.borrow().reference.clone();
    let was_edit = reference.is_some();
    let output_dir = ui.config.borrow().output_dir.clone();
    let offload = ui.config.borrow().offload;

    let output = output_dir.join(format!("{}-{}.png", slugify(&base_prompt), now_millis()));

    let mut request = GenerateRequest::new(effective_prompt, output);
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
    set_controls_enabled(ui, false);
    ui.progress.set_visible(true);
    ui.progress.set_fraction(0.0);
    ui.progress.set_text(Some("Loading models…"));
    ui.status.set_text("Loading models…");

    let (sender, receiver) = async_channel::unbounded();
    if ui
        .worker
        .jobs
        .send_blocking((request.clone(), sender))
        .is_err()
    {
        ui.toast
            .add_toast(adw::Toast::new("Generation worker is unavailable."));
        finish(ui, "Generation failed");
        return;
    }

    let ui = ui.clone();
    let prompt_for_history = base_prompt;
    let model_for_history = model.id.to_string();
    let style_for_history = style_id.to_string();
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
                    ui.progress.pulse();
                    ui.status.set_text("Loading models…");
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
                    let duration_ms = started.elapsed().as_millis() as i64;
                    let image_path = path.to_string_lossy().into_owned();
                    let created_at = unix_seconds();

                    ui.state.borrow_mut().on_result(path.clone(), was_edit);
                    update_image_view(&ui);

                    let inserted = ui.history.borrow().insert(&NewGeneration {
                        prompt: &prompt_for_history,
                        model: &model_for_history,
                        style: &style_for_history,
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
                                style: style_for_history.clone(),
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
                            history_view::add_row(&ui, &entry, true);
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
                            preset_label(&model_for_history)
                        ),
                    );
                    break;
                }
                Progress::Failed(error) => {
                    let short = error.lines().next().unwrap_or(&error);
                    tracing::error!(%error, "generation failed");
                    ui.toast
                        .add_toast(adw::Toast::new(&format!("Generation failed: {short}")));
                    finish(&ui, "Generation failed");
                    break;
                }
            }
        }
    });
}

fn finish(ui: &Rc<Ui>, message: &str) {
    ui.generating.set(false);
    ui.progress.set_visible(false);
    ui.progress.set_fraction(0.0);
    set_controls_enabled(ui, true);
    refresh_model(ui);
    ui.status.set_text(message);
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

pub(crate) fn write_thumbnail(source: &Path, id: i64) -> anyhow::Result<()> {
    let dir = paths::thumbs_dir();
    std::fs::create_dir_all(&dir)?;
    let image = image::open(source)?;
    image
        .thumbnail(128, 128)
        .save(dir.join(format!("{id}.png")))?;
    Ok(())
}

pub(crate) fn thumb_path(id: i64) -> PathBuf {
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

fn resolve_sd_server(config: &Config) -> PathBuf {
    if let Ok(path) = std::env::var("FLUX_SD_SERVER") {
        return PathBuf::from(path);
    }
    if let Some(path) = &config.sd_server {
        return path.clone();
    }
    // Older setups configured an sd-cli path; use the sibling server binary.
    if let Ok(path) = std::env::var("FLUX_SD_CLI")
        && let Some(server) = sibling_server(&PathBuf::from(path))
    {
        return server;
    }
    if let Some(path) = &config.sd_cli
        && let Some(server) = sibling_server(path)
    {
        return server;
    }
    let installed = PathBuf::from("/usr/lib/flux-studio/sd-server");
    if installed.exists() {
        return installed;
    }
    let vendored = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/stable-diffusion.cpp/build/bin/sd-server");
    if vendored.exists() {
        return vendored;
    }
    PathBuf::from("sd-server")
}

fn sibling_server(cli: &Path) -> Option<PathBuf> {
    let server = cli.parent()?.join("sd-server");
    server.exists().then_some(server)
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

fn preset_label(id: &str) -> &str {
    preset(id).map(|preset| preset.label).unwrap_or(id)
}

/// Build a short, filesystem-friendly name from the prompt.
fn slugify(prompt: &str) -> String {
    const MAX_WORDS: usize = 5;
    const MAX_CHARS: usize = 48;

    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in prompt.chars() {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
            if words.len() == MAX_WORDS {
                break;
            }
        }
    }
    if !current.is_empty() && words.len() < MAX_WORDS {
        words.push(current);
    }

    let mut slug = words.join("-");
    if slug.chars().count() > MAX_CHARS {
        slug = slug.chars().take(MAX_CHARS).collect();
        while slug.ends_with('-') {
            slug.pop();
        }
    }

    if slug.is_empty() {
        "image".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugifies_prompts() {
        assert_eq!(
            slugify("A brass lighthouse on a stormy cliff at dusk"),
            "a-brass-lighthouse-on-a"
        );
        assert_eq!(slugify("  Hello,   world!!  "), "hello-world");
        assert_eq!(slugify("café au lait"), "café-au-lait");
        assert_eq!(slugify(""), "image");
        assert_eq!(slugify("!!! ???"), "image");
    }

    #[test]
    fn slug_has_no_trailing_dash_and_is_short() {
        let prompt = "an extremely long prompt about a very detailed mechanical whale \
                      floating above a neon city at night, cinematic";
        let slug = slugify(prompt);
        assert!(!slug.ends_with('-'));
        assert!(slug.chars().count() <= 48, "{slug}");
        assert!(!slug.is_empty());
    }
}
