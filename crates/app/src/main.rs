mod backends;
mod history;
mod history_view;
mod main_view;
mod preferences;
mod prompt_ai;
mod sidebar;
mod state;
mod upscale_dialog;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use flux_backend_sdcpp::{SdServer, SdServerConfig};
use flux_core::preset::{ModelPreset, ModelRole, PRESETS, default_preset, preset};
use flux_core::style::STYLES;
use flux_core::upscaler::{self, UpscaleRequest, UpscalerPreset};
use flux_core::{Backend, Config, GenerateRequest, Progress, paths, size};
use gtk::{gdk, glib};

use history::{Generation, History, NewGeneration};
use state::{AppState, UndoStack};

const HISTORY_LIMIT: usize = 100;
pub(crate) const AUTO_HIDE_SECONDS: u32 = 3;

type CropRect = (f64, f64, f64, f64);

struct ZoomState {
    surface: gtk::cairo::ImageSurface,
    start: CropRect,
    end: CropRect,
    progress: f64,
}

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
    pub picture_back: gtk::Picture,
    pub reference_bar: gtk::Box,
    pub reference_name: gtk::Label,
    pub prompt: gtk::TextView,
    pub generate: gtk::Button,
    pub cancel: gtk::Button,
    pub progress: gtk::ProgressBar,
    pub progress_row: gtk::Box,
    pub status: gtk::Label,
    pub composer: gtk::Revealer,
    pub crop_overlay: gtk::DrawingArea,
    pub zoom_overlay: gtk::DrawingArea,
    pub zoom_state: Rc<RefCell<Option<ZoomState>>>,
    pub zoom_source: RefCell<Option<glib::SourceId>>,
    pub zooming: Cell<bool>,
    pub edit_button: gtk::Button,
    pub upscale_button: gtk::Button,
    pub nav_up_button: gtk::Button,
    pub nav_down_button: gtk::Button,
    pub crop_button: gtk::ToggleButton,
    pub undo_button: gtk::Button,
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
    pub models: RefCell<Vec<&'static ModelPreset>>,
    pub rebuilding_models: Cell<bool>,
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
    pub upscaling: Cell<bool>,
    pub backend_installing: Cell<bool>,
    pub size_guard: Cell<bool>,
    pub size_cell: Rc<Cell<(u32, u32)>>,
    pub image_size: Cell<(u32, u32)>,
    pub last_scroll: Cell<Option<Instant>>,
    pub crop_mode: Cell<bool>,
    pub crop_rect: Rc<RefCell<Option<CropRect>>>,
    pub undo: RefCell<UndoStack>,
    pub sidebar_root: gtk::Box,
    pub pointer_in_sidebar: Cell<bool>,
    pub auto_hiding: Cell<bool>,
    pub auto_hide_source: RefCell<Option<glib::SourceId>>,
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
         }
         .active-backend-label {
             color: @accent_bg_color;
             font-weight: 600;
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
    let needs_backend = config.sd_server.is_none()
        && config.backend_id.is_none()
        && std::env::var_os("FLUX_SD_SERVER").is_none()
        && !server_starts(&sd_server);

    let backend: Arc<dyn Backend> = Arc::new(SdServer::new(SdServerConfig {
        cli: sd_server,
        models_dir: models_dir.clone(),
        backend: config.backend.clone(),
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
        .icon_name("view-more-symbolic")
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
    let crop_rect = Rc::new(RefCell::new(None));
    let zoom_state = Rc::new(RefCell::new(None));

    let ui = Rc::new(Ui {
        window,
        split: split.clone(),
        toast,
        stack: view.stack,
        picture: view.picture,
        picture_back: view.picture_back,
        reference_bar: view.reference_bar,
        reference_name: view.reference_name,
        prompt: view.prompt,
        generate: view.generate,
        cancel: view.cancel,
        progress: view.progress,
        progress_row: view.progress_row,
        status: view.status,
        composer: view.composer,
        crop_overlay: view.crop_overlay,
        zoom_overlay: view.zoom_overlay,
        zoom_state,
        zoom_source: RefCell::new(None),
        zooming: Cell::new(false),
        edit_button: view.edit_button,
        upscale_button: view.upscale_button,
        nav_up_button: view.nav_up_button,
        nav_down_button: view.nav_down_button,
        crop_button: view.crop_button,
        undo_button: view.undo_button,
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
        models: RefCell::new(Vec::new()),
        rebuilding_models: Cell::new(false),
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
        upscaling: Cell::new(false),
        backend_installing: Cell::new(false),
        size_guard: Cell::new(false),
        size_cell,
        image_size: Cell::new((0, 0)),
        last_scroll: Cell::new(None),
        crop_mode: Cell::new(false),
        crop_rect,
        undo: RefCell::new(UndoStack::default()),
        sidebar_root: side.root.clone(),
        pointer_in_sidebar: Cell::new(false),
        auto_hiding: Cell::new(false),
        auto_hide_source: RefCell::new(None),
    });

    sidebar_toggle
        .bind_property("active", &split, "show-sidebar")
        .bidirectional()
        .build();

    wire_events(&ui, &preferences_button);
    setup_crop_overlay(&ui);
    setup_zoom_overlay(&ui);
    setup_auto_hide(&ui);

    if let Ok(entries) = ui.history.borrow().recent(HISTORY_LIMIT) {
        for entry in &entries {
            history_view::add_row(&ui, entry, false);
        }
        *ui.entries.borrow_mut() = entries;
    }

    if let Err(error) = ui.config.borrow().save() {
        tracing::warn!(%error, "failed to write config");
    }

    refresh_model_list(&ui);
    update_size_ui(&ui);
    update_image_view(&ui);

    ui.window.present();
    show_composer(&ui, true);

    if ui.models.borrow().is_empty() {
        preferences::show_preferences(&ui, true);
    }
    if needs_backend {
        backends::auto_install_cpu(&ui);
    }

    refresh_auto_hide(&ui);
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
            let upscaling = ui.upscaling.get();
            let result = if upscaling {
                ui.backend.cancel_upscale()
            } else {
                ui.backend.cancel()
            };
            if let Err(error) = result {
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
            if ui.rebuilding_models.get() {
                return;
            }
            let index = dropdown.selected() as usize;
            let Some(model) = ui.models.borrow().get(index).copied() else {
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
            if ui.generating.get() || ui.upscaling.get() {
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
            if !visible {
                cancel_auto_hide(&ui);
            } else {
                refresh_auto_hide(&ui);
            }
            if ui.auto_hiding.get() {
                return;
            }
            let mut config = ui.config.borrow_mut();
            if config.sidebar_visible != visible {
                config.sidebar_visible = visible;
                let _ = config.save();
            }
        });
    }
    {
        let ui = ui.clone();
        let button = ui.edit_button.clone();
        button.connect_clicked(move |_| edit_current_image(&ui));
    }
    {
        let ui = ui.clone();
        let button = ui.upscale_button.clone();
        button.connect_clicked(move |_| upscale_dialog::show(&ui));
    }
    {
        let ui = ui.clone();
        let button = ui.nav_up_button.clone();
        button.connect_clicked(move |_| navigate_history(&ui, -1));
    }
    {
        let ui = ui.clone();
        let button = ui.nav_down_button.clone();
        button.connect_clicked(move |_| navigate_history(&ui, 1));
    }
    {
        let ui = ui.clone();
        let stack = ui.stack.clone();
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _dx, dy| {
            if dy == 0.0 || ui.generating.get() || ui.upscaling.get() || ui.zooming.get() {
                return glib::Propagation::Proceed;
            }
            if ui
                .last_scroll
                .get()
                .is_some_and(|last| last.elapsed() < Duration::from_millis(180))
            {
                return glib::Propagation::Stop;
            }
            ui.last_scroll.set(Some(Instant::now()));
            if dy < 0.0 {
                navigate_history(&ui, -1);
            } else {
                navigate_history(&ui, 1);
            }
            glib::Propagation::Stop
        });
        stack.add_controller(scroll);
    }
    {
        let ui = ui.clone();
        let button = ui.crop_button.clone();
        button.connect_toggled(move |button| set_crop_mode(&ui, button.is_active()));
    }
    {
        let ui = ui.clone();
        let button = ui.undo_button.clone();
        button.connect_clicked(move |_| undo_crop(&ui));
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
            if key == gdk::Key::Escape && ui.crop_mode.get() {
                ui.crop_button.set_active(false);
                return glib::Propagation::Stop;
            }
            if !state.contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Return => {
                    if ui.composer.reveals_child() {
                        start_generation(&ui);
                    }
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
                gdk::Key::u | gdk::Key::U => {
                    upscale_dialog::show(&ui);
                    glib::Propagation::Stop
                }
                gdk::Key::comma => {
                    preferences::show_preferences(&ui, false);
                    glib::Propagation::Stop
                }
                gdk::Key::Up => {
                    navigate_history(&ui, -1);
                    glib::Propagation::Stop
                }
                gdk::Key::Down => {
                    navigate_history(&ui, 1);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
    }
    ui.window.add_controller(key_controller);
}

pub(crate) fn refresh_model_list(ui: &Rc<Ui>) {
    let installed: Vec<&'static ModelPreset> = PRESETS
        .iter()
        .filter(|preset| preset.is_installed(&ui.models_dir))
        .collect();
    let labels: Vec<&str> = installed.iter().map(|preset| preset.label).collect();
    let list = gtk::StringList::new(&labels);
    *ui.models.borrow_mut() = installed;
    ui.model_dropdown.set_model(Some(&list));

    let is_empty = ui.models.borrow().is_empty();
    ui.model_dropdown
        .set_sensitive(!is_empty && !ui.generating.get());

    if is_empty {
        ui.model_dropdown.set_selected(gtk::INVALID_LIST_POSITION);
        refresh_model(ui);
        return;
    }

    let active = ui.config.borrow().active_preset.clone();
    let index = ui
        .models
        .borrow()
        .iter()
        .position(|preset| preset.id == active)
        .unwrap_or(0);
    ui.model_dropdown.set_selected(index as u32);

    let selected_id = ui.models.borrow()[index].id.to_string();
    {
        let mut config = ui.config.borrow_mut();
        if config.active_preset != selected_id {
            config.active_preset = selected_id;
            let _ = config.save();
        }
    }
    refresh_model(ui);
}

pub(crate) fn active_preset(ui: &Rc<Ui>) -> &'static ModelPreset {
    let index = ui.model_dropdown.selected() as usize;
    if let Some(model) = ui.models.borrow().get(index).copied() {
        return model;
    }
    preset(&ui.config.borrow().active_preset).unwrap_or_else(default_preset)
}

pub(crate) fn refresh_model(ui: &Rc<Ui>) {
    if ui.models.borrow().is_empty() {
        ui.model_dropdown
            .set_tooltip_text(Some("No models installed"));
        ui.model_hint
            .set_text("No models installed — download in Preferences");
        ui.model_hint.add_css_class("error");
        ui.advanced_summary
            .set_text("Download a model in Preferences");
        ui.generate.set_sensitive(false);
        if ui.prompt_helper.is_some() {
            ui.improve_button
                .set_tooltip_text(Some("No model installed"));
            ui.improve_button.set_sensitive(false);
        }
        return;
    }

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
                ui.image_size
                    .set((texture.width() as u32, texture.height() as u32));
                ui.picture.set_paintable(Some(&texture));
                ui.stack.set_visible_child_name("image");
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "failed to load image");
                ui.image_size.set((0, 0));
                ui.picture.set_paintable(gdk::Paintable::NONE);
                ui.stack.set_visible_child_name("empty");
            }
        },
        None => {
            ui.image_size.set((0, 0));
            ui.picture.set_paintable(gdk::Paintable::NONE);
            ui.picture_back.set_paintable(gdk::Paintable::NONE);
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

    update_image_actions(ui);
}

pub(crate) fn show_composer(ui: &Rc<Ui>, show: bool) {
    ui.composer.set_reveal_child(show);
    update_image_actions(ui);
    refresh_auto_hide(ui);
    if show {
        ui.prompt.grab_focus();
    }
}

pub(crate) fn update_image_actions(ui: &Rc<Ui>) {
    let has_image = ui.state.borrow().display.is_some();
    let composing = ui.composer.reveals_child();
    let generating = ui.generating.get();
    let upscaling = ui.upscaling.get();
    let zooming = ui.zooming.get();
    let position = history_position(ui);
    let navigation = has_image && !generating && !upscaling && !zooming;

    ui.nav_up_button
        .set_visible(navigation && position.is_some_and(|(index, count)| index + 1 < count));
    ui.nav_down_button
        .set_visible(navigation && position.is_some_and(|(index, _)| index > 0));
    ui.upscale_button
        .set_visible(has_image && !composing && !generating && !upscaling && !zooming);
    ui.edit_button
        .set_visible(has_image && !composing && !generating && !upscaling && !zooming);
    ui.crop_button
        .set_visible(has_image && !generating && !upscaling && !zooming);
    ui.undo_button.set_visible(
        has_image && !generating && !upscaling && !zooming && ui.undo.borrow().can_undo(),
    );

    if !has_image || generating || upscaling || zooming {
        ui.crop_button.set_active(false);
    }
}

fn edit_current_image(ui: &Rc<Ui>) {
    if ui.generating.get() || ui.upscaling.get() {
        return;
    }
    let Some(path) = ui.state.borrow().display.clone() else {
        return;
    };
    ui.state.borrow_mut().reference = Some(path);
    update_image_view(ui);
    show_composer(ui, true);
    ui.status.set_text("Editing reference");
}

fn set_crop_mode(ui: &Rc<Ui>, active: bool) {
    ui.crop_mode.set(active);
    ui.crop_overlay.set_visible(active);
    *ui.crop_rect.borrow_mut() = None;
    if active {
        ui.crop_overlay.set_cursor_from_name(Some("crosshair"));
        ui.crop_overlay.grab_focus();
        ui.status.set_text("Drag to select an area · Esc to cancel");
    } else {
        ui.crop_overlay.set_cursor_from_name(None);
        ui.crop_overlay.queue_draw();
    }
}

fn undo_crop(ui: &Rc<Ui>) {
    if ui.generating.get() {
        return;
    }
    cancel_zoom(ui);
    // End the mutable borrow before update_image_view() re-borrows the stack.
    let entry = ui.undo.borrow_mut().undo();
    let Some(entry) = entry else {
        return;
    };

    let previous = entry.state;
    let Some(path) = previous.display.clone() else {
        *ui.state.borrow_mut() = previous;
        update_image_view(ui);
        return;
    };

    *ui.state.borrow_mut() = previous;
    update_image_view(ui);
    ui.status.set_text("Crop undone");

    match load_zoom_surface(&path) {
        Ok((surface, size)) => start_zoom(
            ui,
            surface,
            entry.selection,
            (0.0, 0.0, size.0 as f64, size.1 as f64),
            false,
        ),
        Err(error) => {
            // The restored image is already visible; only the effect is lost.
            tracing::warn!(%error, "failed to load the undo zoom image");
        }
    }
}

fn load_zoom_surface(path: &Path) -> anyhow::Result<(gtk::cairo::ImageSurface, (u32, u32))> {
    let image = image::open(path)?;
    let size = (image.width(), image.height());
    let surface = image_surface(&image)?;
    Ok((surface, size))
}

fn setup_crop_overlay(ui: &Rc<Ui>) {
    let rect = ui.crop_rect.clone();
    ui.crop_overlay
        .set_draw_func(move |_area, cr, width, height| {
            let Some((x0, y0, x1, y1)) = *rect.borrow() else {
                return;
            };
            let left = x0.min(x1);
            let top = y0.min(y1);
            let right = x0.max(x1);
            let bottom = y0.max(y1);

            cr.set_source_rgba(0.0, 0.0, 0.0, 0.4);
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.fill();

            if right - left > 0.0 && bottom - top > 0.0 {
                cr.set_operator(gtk::cairo::Operator::Clear);
                cr.rectangle(left, top, right - left, bottom - top);
                let _ = cr.fill();
                cr.set_operator(gtk::cairo::Operator::Over);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
                cr.set_line_width(2.0);
                cr.rectangle(left, top, right - left, bottom - top);
                let _ = cr.stroke();
            }
        });

    let gesture = gtk::GestureDrag::new();
    {
        let ui = ui.clone();
        gesture.connect_drag_begin(move |_, x, y| {
            *ui.crop_rect.borrow_mut() = Some((x, y, x, y));
            ui.crop_overlay.queue_draw();
        });
    }
    {
        let ui = ui.clone();
        gesture.connect_drag_update(move |gesture, dx, dy| {
            let (sx, sy) = gesture.start_point().unwrap_or((0.0, 0.0));
            *ui.crop_rect.borrow_mut() = Some((sx, sy, sx + dx, sy + dy));
            ui.crop_overlay.queue_draw();
        });
    }
    {
        let ui = ui.clone();
        gesture.connect_drag_end(move |gesture, dx, dy| {
            let (sx, sy) = gesture.start_point().unwrap_or((0.0, 0.0));
            let (ex, ey) = (sx + dx, sy + dy);
            *ui.crop_rect.borrow_mut() = Some((sx, sy, ex, ey));
            ui.crop_overlay.queue_draw();
            apply_crop(&ui, (sx, sy, ex, ey));
        });
    }
    ui.crop_overlay.add_controller(gesture);
}

fn apply_crop(ui: &Rc<Ui>, selection: CropRect) {
    let area_w = ui.crop_overlay.width() as f64;
    let area_h = ui.crop_overlay.height() as f64;
    let (image_w, image_h) = ui.image_size.get();
    let Some((x, y, width, height)) =
        crop_rect_from_drag((area_w, area_h), (image_w, image_h), selection)
    else {
        *ui.crop_rect.borrow_mut() = None;
        ui.crop_overlay.queue_draw();
        ui.status.set_text("Select a larger area");
        return;
    };

    let Some(source) = ui.state.borrow().display.clone() else {
        ui.crop_button.set_active(false);
        return;
    };

    let result = (|| -> anyhow::Result<(PathBuf, gtk::cairo::ImageSurface)> {
        let image = image::open(&source)?;
        let surface = image_surface(&image)?;
        let cropped = image.crop_imm(x, y, width, height);
        let directory = paths::cache_dir().join("crops");
        std::fs::create_dir_all(&directory)?;
        let destination = directory.join(format!("crop-{}.png", now_millis()));
        cropped.save(&destination)?;
        Ok((destination, surface))
    })();

    match result {
        Ok((destination, surface)) => {
            let previous = ui.state.borrow().clone();
            ui.undo
                .borrow_mut()
                .push(&previous, (x as f64, y as f64, width as f64, height as f64));
            ui.state.borrow_mut().load(destination);
            update_image_view(ui);
            start_zoom(
                ui,
                surface,
                (0.0, 0.0, image_w as f64, image_h as f64),
                (x as f64, y as f64, width as f64, height as f64),
                true,
            );
            ui.status
                .set_text(&format!("Cropped to {width} × {height}"));
            ui.toast
                .add_toast(adw::Toast::new(&format!("Cropped to {width} × {height}.")));
            ui.crop_button.set_active(false);
        }
        Err(error) => {
            tracing::warn!(%error, "failed to crop image");
            ui.toast
                .add_toast(adw::Toast::new(&format!("Could not crop: {error}")));
            ui.crop_button.set_active(false);
        }
    }
}

/// Map a drag in widget coordinates to image pixels, honoring letterboxing.
pub(crate) fn crop_rect_from_drag(
    area: (f64, f64),
    image: (u32, u32),
    selection: CropRect,
) -> Option<(u32, u32, u32, u32)> {
    let (area_width, area_height) = area;
    let (image_width, image_height) = image;
    if area_width <= 0.0 || area_height <= 0.0 || image_width == 0 || image_height == 0 {
        return None;
    }
    let image_w = image_width as f64;
    let image_h = image_height as f64;
    let scale = (area_width / image_w).min(area_height / image_h);
    if scale <= 0.0 {
        return None;
    }
    let rendered_w = image_w * scale;
    let rendered_h = image_h * scale;
    let offset_x = (area_width - rendered_w) / 2.0;
    let offset_y = (area_height - rendered_h) / 2.0;
    let (x0, y0, x1, y1) = selection;

    let left = x0.min(x1).clamp(offset_x, offset_x + rendered_w);
    let right = x0.max(x1).clamp(offset_x, offset_x + rendered_w);
    let top = y0.min(y1).clamp(offset_y, offset_y + rendered_h);
    let bottom = y0.max(y1).clamp(offset_y, offset_y + rendered_h);

    let px = (((left - offset_x) / scale).round() as i64).clamp(0, image_width as i64) as u32;
    let py = (((top - offset_y) / scale).round() as i64).clamp(0, image_height as i64) as u32;
    let px_end = (((right - offset_x) / scale).round() as i64).clamp(0, image_width as i64) as u32;
    let py_end =
        (((bottom - offset_y) / scale).round() as i64).clamp(0, image_height as i64) as u32;

    let width = px_end.saturating_sub(px);
    let height = py_end.saturating_sub(py);
    if width < 16 || height < 16 {
        return None;
    }
    Some((px, py, width, height))
}

fn image_surface(image: &image::DynamicImage) -> anyhow::Result<gtk::cairo::ImageSurface> {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut surface =
        gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width as i32, height as i32)?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().map_err(|error| anyhow::anyhow!("{error}"))?;
        for (index, pixel) in rgba.pixels().enumerate() {
            let x = index % width as usize;
            let y = index / width as usize;
            let offset = y * stride + x * 4;
            let alpha = pixel.0[3] as u16;
            data[offset] = (pixel.0[2] as u16 * alpha / 255) as u8;
            data[offset + 1] = (pixel.0[1] as u16 * alpha / 255) as u8;
            data[offset + 2] = (pixel.0[0] as u16 * alpha / 255) as u8;
            data[offset + 3] = pixel.0[3];
        }
    }
    surface.mark_dirty();
    Ok(surface)
}

fn setup_zoom_overlay(ui: &Rc<Ui>) {
    let state = ui.zoom_state.clone();
    ui.zoom_overlay
        .set_draw_func(move |_area, cr, width, height| {
            let state = state.borrow();
            let Some(state) = state.as_ref() else {
                return;
            };
            let area = (width as f64, height as f64);
            let progress = ease(state.progress);
            let source = lerp_rect(state.start, state.end, progress);
            let destination = lerp_rect(
                contain_rect(area, (state.start.2, state.start.3)),
                contain_rect(area, (state.end.2, state.end.3)),
                progress,
            );
            if source.2 <= 0.0 || source.3 <= 0.0 || destination.2 <= 0.0 || destination.3 <= 0.0 {
                return;
            }

            let _ = cr.save();
            cr.rectangle(destination.0, destination.1, destination.2, destination.3);
            cr.clip();
            cr.translate(destination.0, destination.1);
            cr.scale(destination.2 / source.2, destination.3 / source.3);
            cr.translate(-source.0, -source.1);
            let _ = cr.set_source_surface(&state.surface, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();
        });
}

fn contain_rect(area: (f64, f64), image: (f64, f64)) -> CropRect {
    if area.0 <= 0.0 || area.1 <= 0.0 || image.0 <= 0.0 || image.1 <= 0.0 {
        return (0.0, 0.0, area.0.max(0.0), area.1.max(0.0));
    }
    let scale = (area.0 / image.0).min(area.1 / image.1);
    let width = image.0 * scale;
    let height = image.1 * scale;
    (
        (area.0 - width) / 2.0,
        (area.1 - height) / 2.0,
        width,
        height,
    )
}

fn lerp_rect(start: CropRect, end: CropRect, progress: f64) -> CropRect {
    (
        start.0 + (end.0 - start.0) * progress,
        start.1 + (end.1 - start.1) * progress,
        start.2 + (end.2 - start.2) * progress,
        start.3 + (end.3 - start.3) * progress,
    )
}

fn ease(progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    1.0 - (1.0 - progress).powi(3)
}

fn start_zoom(
    ui: &Rc<Ui>,
    surface: gtk::cairo::ImageSurface,
    start: CropRect,
    end: CropRect,
    reveal_composer: bool,
) {
    cancel_zoom(ui);
    ui.zooming.set(true);
    ui.stack.set_opacity(0.0);
    *ui.zoom_state.borrow_mut() = Some(ZoomState {
        surface,
        start,
        end,
        progress: 0.0,
    });
    ui.zoom_overlay.set_visible(true);
    ui.zoom_overlay.queue_draw();
    update_image_actions(ui);

    let started = Instant::now();
    let duration = 0.30;
    let ui_callback = ui.clone();
    let source = glib::timeout_add_local(Duration::from_millis(16), move || {
        let progress = (started.elapsed().as_secs_f64() / duration).min(1.0);
        if let Some(state) = ui_callback.zoom_state.borrow_mut().as_mut() {
            state.progress = progress;
        }
        ui_callback.zoom_overlay.queue_draw();

        if progress >= 1.0 {
            *ui_callback.zoom_state.borrow_mut() = None;
            ui_callback.zoom_overlay.set_visible(false);
            ui_callback.stack.set_opacity(1.0);
            ui_callback.zooming.set(false);
            *ui_callback.zoom_source.borrow_mut() = None;
            update_image_actions(&ui_callback);
            if reveal_composer {
                show_composer(&ui_callback, true);
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    *ui.zoom_source.borrow_mut() = Some(source);
}

pub(crate) fn cancel_zoom(ui: &Rc<Ui>) {
    if let Some(source) = ui.zoom_source.borrow_mut().take() {
        source.remove();
    }
    let had_state = ui.zoom_state.borrow_mut().take().is_some();
    if had_state || ui.zooming.get() {
        ui.zoom_overlay.set_visible(false);
        ui.stack.set_opacity(1.0);
        ui.zooming.set(false);
        update_image_actions(ui);
    }
}

fn cancel_auto_hide(ui: &Rc<Ui>) {
    if let Some(source) = ui.auto_hide_source.borrow_mut().take() {
        source.remove();
    }
}

/// Whether the pointer is physically over the sidebar.
///
/// The enter/leave flag can go stale when a dialog or another surface covers
/// the sidebar, so ask the seat for the real position when possible.
fn pointer_over_sidebar(ui: &Rc<Ui>) -> bool {
    let fallback = ui.pointer_in_sidebar.get();
    let Some(surface) = ui.window.surface() else {
        return fallback;
    };
    let display = gtk::prelude::WidgetExt::display(&ui.window);
    let Some(seat) = display.default_seat() else {
        return fallback;
    };
    let Some(pointer) = seat.pointer() else {
        return fallback;
    };
    let Some((x, y, _)) = surface.device_position(&pointer) else {
        return fallback;
    };
    let Some(bounds) = ui.sidebar_root.compute_bounds(&ui.window) else {
        return fallback;
    };
    let inside = bounds.contains_point(&gtk::graphene::Point::new(x as f32, y as f32));
    tracing::debug!(
        x,
        y,
        left = bounds.x(),
        top = bounds.y(),
        width = bounds.width(),
        height = bounds.height(),
        inside,
        "sidebar pointer position"
    );
    inside
}

fn schedule_auto_hide(ui: &Rc<Ui>) {
    if !ui.config.borrow().sidebar_auto_hide
        || !ui.split.shows_sidebar()
        || ui.generating.get()
        || ui.upscaling.get()
        || pointer_over_sidebar(ui)
    {
        return;
    }
    cancel_auto_hide(ui);
    tracing::debug!("scheduling sidebar auto-hide");
    let callback_ui = ui.clone();
    let source = glib::timeout_add_seconds_local(AUTO_HIDE_SECONDS, move || {
        let ui = &callback_ui;
        *ui.auto_hide_source.borrow_mut() = None;
        let focus = gtk::prelude::GtkWindowExt::focus(&ui.window);
        let pointer = pointer_over_sidebar(ui);
        tracing::debug!(
            active = ui.window.is_active(),
            pointer,
            focus = focus
                .as_ref()
                .map(|focus| focus.widget_name().to_string())
                .unwrap_or_default(),
            "sidebar auto-hide check"
        );
        if !ui.config.borrow().sidebar_auto_hide
            || !ui.split.shows_sidebar()
            || ui.generating.get()
            || ui.upscaling.get()
            || pointer
            || !ui.window.is_active()
        {
            return glib::ControlFlow::Break;
        }
        ui.auto_hiding.set(true);
        ui.split.set_show_sidebar(false);
        ui.auto_hiding.set(false);
        glib::ControlFlow::Break
    });
    *ui.auto_hide_source.borrow_mut() = Some(source);
}

pub(crate) fn refresh_auto_hide(ui: &Rc<Ui>) {
    if !ui.config.borrow().sidebar_auto_hide
        || !ui.split.shows_sidebar()
        || ui.generating.get()
        || ui.upscaling.get()
        || pointer_over_sidebar(ui)
    {
        cancel_auto_hide(ui);
        return;
    }
    schedule_auto_hide(ui);
}

fn setup_auto_hide(ui: &Rc<Ui>) {
    let motion = gtk::EventControllerMotion::new();
    {
        let ui = ui.clone();
        motion.connect_enter(move |_, _, _| {
            ui.pointer_in_sidebar.set(true);
            cancel_auto_hide(&ui);
        });
    }
    {
        let ui = ui.clone();
        motion.connect_leave(move |_| {
            ui.pointer_in_sidebar.set(false);
            schedule_auto_hide(&ui);
        });
    }
    ui.sidebar_root.add_controller(motion);

    {
        let ui = ui.clone();
        let window = ui.window.clone();
        window.connect_notify_local(Some("is-active"), move |window, _| {
            if window.is_active() {
                refresh_auto_hide(&ui);
            } else {
                cancel_auto_hide(&ui);
            }
        });
    }
}

pub(crate) fn new_image(ui: &Rc<Ui>) {
    if ui.generating.get() || ui.upscaling.get() {
        return;
    }
    cancel_zoom(ui);
    ui.undo.borrow_mut().clear();
    ui.state.borrow_mut().new_image();
    update_image_view(ui);
    show_composer(ui, true);
    ui.status.set_text("New image");
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
    ui.generate.set_sensitive(enabled);
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
        ui.undo.borrow_mut().clear();
        ui.state.borrow_mut().load(path);
        update_image_view(&ui);
        show_composer(&ui, true);
        ui.status.set_text(&format!("Editing reference · {name}"));
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
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("The backend is still installing."));
        return;
    }
    if ui.generating.get()
        || ui.upscaling.get()
        || ui.zooming.get()
        || !ui.generate.is_sensitive()
        || !ui.composer.reveals_child()
    {
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
    ui.composer.set_reveal_child(false);
    ui.progress_row.set_visible(true);
    ui.cancel.set_sensitive(true);
    ui.progress.set_visible(true);
    ui.progress.set_fraction(0.0);
    ui.progress.set_text(Some("Loading models…"));
    ui.status.set_text("Loading models…");
    update_image_actions(ui);

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
        show_composer(ui, true);
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
                    show_composer(&ui, true);
                    break;
                }
                Progress::Done(path) => {
                    let duration_ms = started.elapsed().as_millis() as i64;
                    let image_path = path.to_string_lossy().into_owned();
                    let created_at = unix_seconds();

                    ui.undo.borrow_mut().clear();
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
                        kind: "generate",
                        source_path: None,
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
                                kind: "generate".into(),
                                source_path: None,
                            };
                            ui.entries.borrow_mut().insert(0, entry.clone());
                            history_view::add_row(&ui, &entry, true);
                            trim_history(&ui);
                            update_image_actions(&ui);
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
                    show_composer(&ui, true);
                    break;
                }
            }
        }
    });
}

/// Show an image with a crossfade from the currently visible canvas image.
fn fade_to_image(ui: &Rc<Ui>, path: &Path) -> bool {
    let texture = match gdk::Texture::from_filename(path) {
        Ok(texture) => texture,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "failed to load image");
            return false;
        }
    };

    // Alternate between the two picture children so the stack can crossfade.
    let (target, back) = if ui.stack.visible_child_name().as_deref() == Some("image") {
        ("back", &ui.picture_back)
    } else {
        ("image", &ui.picture)
    };
    back.set_paintable(Some(&texture));
    ui.image_size
        .set((texture.width() as u32, texture.height() as u32));
    ui.state.borrow_mut().preview(path.to_path_buf());
    ui.reference_name.set_text("");
    ui.reference_name.set_tooltip_text(None);
    ui.reference_bar.set_visible(false);
    ui.stack.set_visible_child_name(target);
    update_image_actions(ui);
    true
}

/// Position of the displayed image in the history list and the list length.
fn history_position(ui: &Ui) -> Option<(usize, usize)> {
    let display = ui.state.borrow().display.clone()?;
    let entries = ui.entries.borrow();
    let index = entries
        .iter()
        .position(|entry| Path::new(&entry.image_path) == display.as_path())?;
    Some((index, entries.len()))
}

/// Move `delta` entries through history: positive is older, negative newer.
fn navigate_history(ui: &Rc<Ui>, delta: isize) {
    if ui.generating.get() || ui.upscaling.get() || ui.zooming.get() {
        return;
    }
    let Some((index, count)) = history_position(ui) else {
        return;
    };
    let target = index as isize + delta;
    if target < 0 || target >= count as isize {
        return;
    }
    let entry = ui.entries.borrow()[target as usize].clone();
    let path = PathBuf::from(&entry.image_path);
    if !fade_to_image(ui, &path) {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "Could not open {}",
            path.display()
        )));
        return;
    }

    let label = history_view::entry_model_label(&entry);
    ui.status.set_text(&if entry.kind == "upscale" {
        format!("Upscaled · {label} · {} × {}", entry.width, entry.height)
    } else {
        format!("History · {label} · {} × {}", entry.width, entry.height)
    });
}

/// Run one upscale on a worker thread and show the result like a generation.
pub(crate) fn start_upscale(ui: &Rc<Ui>, preset: &'static UpscalerPreset, factor: u32) {
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("The backend is still installing."));
        return;
    }
    if ui.generating.get() || ui.upscaling.get() {
        return;
    }
    let Some(input) = ui.state.borrow().display.clone() else {
        ui.toast.add_toast(adw::Toast::new("No image to upscale."));
        return;
    };
    let (width, height) = ui.image_size.get();
    let Some((output_width, output_height)) = upscaler::output_size(width, height, factor) else {
        ui.toast.add_toast(adw::Toast::new(
            "This image is too large: the 4× pass would exceed 8192 px.",
        ));
        return;
    };
    if !preset.is_installed(&ui.models_dir) {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} is not installed (see Preferences).",
            preset.label
        )));
        return;
    }

    let output_dir = ui.config.borrow().output_dir.clone();
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(slugify)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "image".into());
    let output = output_dir.join(format!("{stem}-upscaled-{factor}x-{}.png", now_millis()));

    {
        let mut config = ui.config.borrow_mut();
        config.upscaler_id = preset.id.to_string();
        config.upscale_factor = factor;
        let _ = config.save();
    }

    let request = UpscaleRequest {
        input: input.clone(),
        output,
        upscaler_id: preset.id.to_string(),
        factor,
        tile_size: upscaler::DEFAULT_TILE_SIZE,
    };

    ui.upscaling.set(true);
    set_controls_enabled(ui, false);
    ui.composer.set_reveal_child(false);
    ui.progress_row.set_visible(true);
    ui.cancel.set_sensitive(true);
    ui.progress.set_visible(true);
    ui.progress.set_fraction(0.0);
    ui.progress.set_text(Some("Upscaling…"));
    ui.status.set_text(&format!(
        "Upscaling {width} × {height} → {output_width} × {output_height}…"
    ));
    update_image_actions(ui);

    // The generation server may still hold the diffusion model. Free it before
    // the upscale server requests VRAM.
    ui.backend.unload();

    let backend = ui.backend.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = backend.upscale(&request);
        let _ = sender.send_blocking(result);
    });

    let ui = ui.clone();
    let started = Instant::now();
    let model_id = preset.id.to_string();
    let source_path = input.to_string_lossy().into_owned();
    glib::spawn_future_local(async move {
        let ticker = glib::timeout_add_local(Duration::from_millis(500), {
            let ui = ui.clone();
            move || {
                ui.progress.pulse();
                ui.status.set_text(&format!(
                    "Upscaling {width} × {height} → {output_width} × {output_height}… {:.0}s",
                    started.elapsed().as_secs_f64()
                ));
                glib::ControlFlow::Continue
            }
        });

        let result = receiver.recv().await;
        ticker.remove();

        let result = match result {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("upscale worker stopped unexpectedly")),
        };

        match result {
            Ok(image) => {
                let duration_ms = started.elapsed().as_millis() as i64;
                let image_path = image.path.to_string_lossy().into_owned();
                let created_at = unix_seconds();
                let prompt = format!("Upscaled {factor}×");

                ui.undo.borrow_mut().clear();

                let inserted = ui.history.borrow().insert(&NewGeneration {
                    prompt: &prompt,
                    model: &model_id,
                    style: "none",
                    width: image.width,
                    height: image.height,
                    steps: 0,
                    seed: "",
                    created_at,
                    duration_ms,
                    image_path: &image_path,
                    reference_path: None,
                    kind: "upscale",
                    source_path: Some(&source_path),
                });

                match inserted {
                    Ok(id) => {
                        if let Err(error) = write_thumbnail(&image.path, id) {
                            tracing::warn!(%error, "failed to write thumbnail");
                        }
                        let entry = Generation {
                            id,
                            prompt: prompt.clone(),
                            model: model_id.clone(),
                            style: "none".into(),
                            width: image.width,
                            height: image.height,
                            steps: 0,
                            seed: String::new(),
                            created_at,
                            duration_ms,
                            image_path,
                            reference_path: None,
                            kind: "upscale".into(),
                            source_path: Some(source_path),
                        };
                        ui.entries.borrow_mut().insert(0, entry.clone());
                        history_view::add_row(&ui, &entry, true);
                        trim_history(&ui);
                    }
                    Err(error) => {
                        tracing::error!(%error, "failed to record upscale");
                    }
                }

                fade_to_image(&ui, &image.path);
                ui.toast.add_toast(adw::Toast::new(&format!(
                    "Upscaled to {} × {}.",
                    image.width, image.height
                )));
                finish(
                    &ui,
                    &format!(
                        "{} · {} × {} · {:.1}s · scroll to compare",
                        upscaler_label(&model_id),
                        image.width,
                        image.height,
                        duration_ms as f64 / 1000.0
                    ),
                );
            }
            Err(error) => {
                let text = error.to_string();
                if text.contains("cancelled") {
                    ui.toast.add_toast(adw::Toast::new("Upscaling cancelled."));
                    finish(&ui, "Cancelled");
                } else {
                    tracing::error!(%error, "upscaling failed");
                    let short = text.lines().next().unwrap_or(&text);
                    ui.toast
                        .add_toast(adw::Toast::new(&format!("Upscaling failed: {short}")));
                    finish(&ui, "Upscaling failed");
                }
            }
        }
    });
}

fn finish(ui: &Rc<Ui>, message: &str) {
    ui.generating.set(false);
    ui.upscaling.set(false);
    ui.progress_row.set_visible(false);
    ui.cancel.set_sensitive(false);
    ui.progress.set_visible(false);
    ui.progress.set_fraction(0.0);
    set_controls_enabled(ui, true);
    refresh_model(ui);
    update_image_actions(ui);
    refresh_auto_hide(ui);
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
    if let Some(server) = flux_core::runtime::find_system_server() {
        return server;
    }
    // Development checkouts keep the built server under vendor/.
    let vendored = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/stable-diffusion.cpp/build/bin/sd-server");
    if vendored.exists() {
        return vendored;
    }
    PathBuf::from("sd-server")
}

fn server_starts(path: &Path) -> bool {
    let path = if path == Path::new("sd-server") {
        match flux_core::runtime::find_in_path("sd-server") {
            Some(path) => path,
            None => return false,
        }
    } else if !path.is_file() {
        return false;
    } else {
        path.to_path_buf()
    };
    std::process::Command::new(path)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
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

fn upscaler_label(id: &str) -> &str {
    flux_core::upscaler::upscaler(id)
        .map(|preset| preset.label)
        .unwrap_or(id)
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
    use super::{contain_rect, crop_rect_from_drag, lerp_rect, slugify};

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

    #[test]
    fn maps_a_full_letterboxed_selection() {
        // 1000x1000 image in a 1000x500 area: rendered at 500x500, centered.
        assert_eq!(
            crop_rect_from_drag((1000.0, 500.0), (1000, 1000), (250.0, 0.0, 750.0, 500.0)),
            Some((0, 0, 1000, 1000))
        );
    }

    #[test]
    fn maps_a_region_inside_the_image() {
        // Same letterboxing; select the middle quarter in image coordinates.
        assert_eq!(
            crop_rect_from_drag((1000.0, 500.0), (1000, 1000), (375.0, 125.0, 625.0, 375.0)),
            Some((250, 250, 500, 500))
        );
    }

    #[test]
    fn clamps_selections_from_the_letterbox_margin() {
        assert_eq!(
            crop_rect_from_drag((1000.0, 500.0), (1000, 1000), (0.0, 0.0, 500.0, 250.0)),
            Some((0, 0, 500, 500))
        );
    }

    #[test]
    fn contains_images_with_letterboxing() {
        assert_eq!(
            contain_rect((1000.0, 500.0), (1000.0, 1000.0)),
            (250.0, 0.0, 500.0, 500.0)
        );
        assert_eq!(
            contain_rect((500.0, 1000.0), (1000.0, 1000.0)),
            (0.0, 250.0, 500.0, 500.0)
        );
    }

    #[test]
    fn interpolates_between_rects() {
        let start = (0.0, 0.0, 100.0, 100.0);
        let end = (50.0, 50.0, 100.0, 100.0);
        assert_eq!(lerp_rect(start, end, 0.0), start);
        assert_eq!(lerp_rect(start, end, 1.0), end);
        assert_eq!(lerp_rect(start, end, 0.5), (25.0, 25.0, 100.0, 100.0));
    }

    #[test]
    fn rejects_tiny_selections() {
        assert_eq!(
            crop_rect_from_drag((1000.0, 500.0), (1000, 1000), (250.0, 0.0, 256.0, 6.0)),
            None
        );
    }
}
