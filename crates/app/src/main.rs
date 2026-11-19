mod adjust;
mod backends;
mod effects;
mod history;
mod history_view;
mod main_view;
mod preferences;
mod prompt_ai;
mod queue;
mod queue_view;
mod rounded_clip;
mod rounded_paintable;
mod sidebar;
mod state;
mod upscale_dialog;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use flux_backend_sdcpp::{SdServer, SdServerConfig};
use flux_core::preset::{ModelPreset, PRESETS, default_preset, preset};
use flux_core::style::STYLES;
use flux_core::upscaler::{self, UpscaleRequest, UpscalerPreset};
use flux_core::{Backend, Config, GenerateRequest, Progress, paths, size};
use gtk::{gdk, gio, glib};

use history::{Generation, History, NewGeneration};
use state::{AppState, UndoStack};

const HISTORY_LIMIT: usize = 100;

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
    pub reference_strip: gtk::Box,
    pub reference_thumbs: gtk::Box,
    pub reference_add: gtk::Button,
    /// Scaled 48×48 textures for the reference strip, keyed by source path.
    pub reference_thumbnails: RefCell<HashMap<PathBuf, gdk::Texture>>,
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
    pub classifying: Cell<bool>,
    pub edit_button: gtk::Button,
    pub upscale_button: gtk::Button,
    pub adjust_button: gtk::Button,
    /// View zoom factor, 1.0 being fit-to-window.
    pub zoom: Cell<f64>,
    /// Running zoom animation, if any.
    pub zoom_animation: RefCell<Option<glib::SourceId>>,
    pub zoom_scroller: gtk::ScrolledWindow,
    pub zoom_controls: gtk::Box,
    pub zoom_out_button: gtk::Button,
    pub zoom_label: gtk::Label,
    pub zoom_in_button: gtk::Button,
    pub zoom_fit_button: gtk::Button,
    pub effects_button: gtk::MenuButton,
    pub effects_popover: gtk::Popover,
    pub save_effect_button: gtk::Button,
    pub custom_effects_section: gtk::Box,
    pub custom_effects_box: gtk::Box,
    pub colorize_item: gtk::Button,
    pub recover_item: gtk::Button,
    pub enhance_item: gtk::Button,
    pub denoise_item: gtk::Button,
    pub photoreal_item: gtk::Button,
    pub background_item: gtk::Button,
    pub auto_lighting_item: gtk::Button,
    pub daylight_item: gtk::Button,
    pub night_item: gtk::Button,
    pub expand_item: gtk::Button,
    pub back_button: gtk::Button,
    pub nav_up_button: gtk::Button,
    pub nav_down_button: gtk::Button,
    pub crop_button: gtk::ToggleButton,
    pub undo_button: gtk::Button,
    pub reference_button: gtk::Button,
    pub improve_button: gtk::Button,
    pub improving: Cell<bool>,
    pub backend: Arc<dyn Backend>,
    pub worker: GenerationWorker,
    pub models_dir: PathBuf,
    pub config: RefCell<Config>,
    pub history: RefCell<History>,
    pub history_list: gtk::ListBox,
    /// Jobs waiting behind the running one.
    pub queue: RefCell<queue::JobQueue>,
    pub queue_box: gtk::Box,
    pub queue_list: gtk::ListBox,
    pub queue_waiting: gtk::Label,
    pub queue_clear: gtk::Button,
    pub queue_running_row: RefCell<Option<adw::ActionRow>>,
    pub queue_tick: RefCell<Option<glib::SourceId>>,
    /// Pending count next to the progress bar.
    pub queue_status: gtk::Label,
    /// Set once the user confirmed quitting with jobs still queued.
    pub quitting: Cell<bool>,
    /// Returns from a history image to the running job's live preview.
    pub live_button: gtk::Button,
    /// Whether the canvas follows the running job's step previews. Browsing
    /// History turns this off; the queue row or the live button turns it on.
    pub live_view: Cell<bool>,
    /// Latest preview frame, kept while the canvas shows something else.
    pub live_preview: RefCell<Option<gdk::Texture>>,
    pub new_button: gtk::Button,
    pub entries: RefCell<Vec<Generation>>,
    /// Chain roots whose descendants are hidden in the sidebar.
    pub collapsed_groups: RefCell<std::collections::HashSet<i64>>,
    pub model_dropdown: gtk::DropDown,
    pub models: RefCell<Vec<&'static ModelPreset>>,
    pub rebuilding_models: Cell<bool>,
    pub model_hint: gtk::Label,
    pub style_dropdown: gtk::DropDown,
    pub mp_scale: gtk::Scale,
    pub mp_value: gtk::Label,
    pub ratio_dropdown: gtk::DropDown,
    pub ratio_model: gtk::StringList,
    pub custom_ratio: Rc<RefCell<Option<(u32, u32)>>>,
    pub ratio_area: gtk::DrawingArea,
    pub ratio_label: gtk::Label,
    pub mp_label: gtk::Label,
    pub size_hint: gtk::Label,
    pub seed_entry: gtk::Entry,
    pub random_button: gtk::Button,
    pub state: RefCell<AppState>,
    pub generating: Cell<bool>,
    pub upscaling: Cell<bool>,
    pub backend_installing: Cell<bool>,
    pub size_guard: Cell<bool>,
    pub size_cell: Rc<Cell<(u32, u32)>>,
    pub image_size: Rc<Cell<(u32, u32)>>,
    pub last_scroll: Cell<Option<Instant>>,
    pub crop_mode: Cell<bool>,
    pub crop_rect: Rc<RefCell<Option<CropRect>>>,
    pub undo: RefCell<UndoStack>,
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
             border-color: @card_bg_color;
         }
         .reference-strip {
             background-color: @card_bg_color;
             border: 1px solid alpha(@view_fg_color, 0.12);
             border-radius: 10px;
             padding: 4px 8px;
         }
         .reference-thumb {
             padding: 0;
             min-width: 0;
             min-height: 0;
             border-radius: 8px;
         }
         .reference-index {
             background-color: alpha(@window_fg_color, 0.8);
             color: @window_bg_color;
             border-radius: 6px;
             padding: 0 4px;
             font-size: 0.7em;
             font-weight: bold;
         }
         .reference-remove {
             min-width: 18px;
             min-height: 18px;
             padding: 0;
             margin: 2px;
             background-color: alpha(@window_bg_color, 0.85);
         }
         .history-guide {
             background-color: alpha(@accent_bg_color, 0.45);
             border-radius: 1px;
         }
         .history-toggle {
             background-color: alpha(@accent_bg_color, 0.22);
             color: @accent_color;
             border-radius: 8px;
             padding: 1px 5px;
             min-width: 0;
             min-height: 0;
         }
         .history-toggle:hover {
             background-color: alpha(@accent_bg_color, 0.35);
         }
         .sidebar-pane, .overlay-pane {
             background-color: @sidebar_bg_color;
         }
         popover.effects-popover > contents {
             background-color: @window_bg_color;
         }
         popover.effects-popover > arrow {
             background-color: @window_bg_color;
         }
         popover.adjust-popover > contents {
             background-color: @window_bg_color;
         }
         popover.adjust-popover > arrow {
             background-color: @window_bg_color;
         }
         .zoom-controls {
             padding: 2px 4px;
             border-radius: 12px;
         }
         button.active-backend-icon,
         button.active-backend-icon:disabled {
             color: @accent_bg_color;
             opacity: 1;
         }
         button.backend-check {
             color: @view_fg_color;
             opacity: 1;
         }
         button.backend-check:hover {
             color: @accent_bg_color;
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

    let initial_preset = preset(&config.active_preset).unwrap_or_else(default_preset);
    let initial_size =
        size::size_for_mp_step(config.mp(), config.ratio(), initial_preset.size_step);
    let size_cell = Rc::new(Cell::new(initial_size));
    let image_size = Rc::new(Cell::new((0u32, 0u32)));

    let view = main_view::build();
    view.improve_button
        .set_visible(prompt_ai::find_cli(&config).is_some());
    let side = sidebar::build(
        &config,
        size_cell.clone(),
        image_size.clone(),
        initial_preset.size_step,
    );

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
        reference_strip: view.reference_strip,
        reference_thumbs: view.reference_thumbs,
        reference_add: view.reference_add,
        reference_thumbnails: RefCell::new(HashMap::new()),
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
        classifying: Cell::new(false),
        edit_button: view.edit_button,
        upscale_button: view.upscale_button,
        adjust_button: view.adjust_button,
        zoom: Cell::new(1.0),
        zoom_animation: RefCell::new(None),
        zoom_scroller: view.zoom_scroller,
        zoom_controls: view.zoom_controls,
        zoom_out_button: view.zoom_out_button,
        zoom_label: view.zoom_label,
        zoom_in_button: view.zoom_in_button,
        zoom_fit_button: view.zoom_fit_button,
        effects_button: view.effects_button,
        effects_popover: view.effects_popover,
        save_effect_button: view.save_effect_button,
        custom_effects_section: view.custom_effects_section,
        custom_effects_box: view.custom_effects_box,
        colorize_item: view.colorize_item,
        recover_item: view.recover_item,
        enhance_item: view.enhance_item,
        denoise_item: view.denoise_item,
        photoreal_item: view.photoreal_item,
        background_item: view.background_item,
        auto_lighting_item: view.auto_lighting_item,
        daylight_item: view.daylight_item,
        night_item: view.night_item,
        expand_item: view.expand_item,
        back_button: view.back_button,
        nav_up_button: view.nav_up_button,
        nav_down_button: view.nav_down_button,
        crop_button: view.crop_button,
        undo_button: view.undo_button,
        reference_button: view.reference_button,
        improve_button: view.improve_button,
        improving: Cell::new(false),
        backend,
        worker,
        models_dir,
        config: RefCell::new(config),
        history: RefCell::new(history),
        history_list: side.history_list,
        queue: RefCell::new(queue::JobQueue::default()),
        queue_box: side.queue_box,
        queue_list: side.queue_list,
        queue_waiting: side.queue_waiting,
        queue_clear: side.queue_clear,
        queue_running_row: RefCell::new(None),
        queue_tick: RefCell::new(None),
        queue_status: view.queue_status,
        quitting: Cell::new(false),
        live_button: view.live_button,
        live_view: Cell::new(true),
        live_preview: RefCell::new(None),
        new_button: side.new_button,
        entries: RefCell::new(Vec::new()),
        collapsed_groups: RefCell::new(std::collections::HashSet::new()),
        model_dropdown: side.model_dropdown,
        models: RefCell::new(Vec::new()),
        rebuilding_models: Cell::new(false),
        model_hint: side.model_hint,
        style_dropdown: side.style_dropdown,
        mp_scale: side.mp_scale,
        mp_value: side.mp_value,
        ratio_dropdown: side.ratio_dropdown,
        ratio_model: side.ratio_model,
        custom_ratio: side.custom_ratio,
        ratio_area: side.ratio_area,
        ratio_label: side.ratio_label,
        mp_label: side.mp_label,
        size_hint: side.size_hint,
        seed_entry: side.seed_entry,
        random_button: side.random_button,
        state: RefCell::new(AppState::default()),
        generating: Cell::new(false),
        upscaling: Cell::new(false),
        backend_installing: Cell::new(false),
        size_guard: Cell::new(false),
        size_cell,
        image_size: image_size.clone(),
        last_scroll: Cell::new(None),
        crop_mode: Cell::new(false),
        crop_rect,
        undo: RefCell::new(UndoStack::default()),
    });

    sidebar_toggle
        .bind_property("active", &split, "show-sidebar")
        .bidirectional()
        .build();

    wire_events(&ui, &preferences_button);
    effects::rebuild_menu(&ui);
    setup_crop_overlay(&ui);
    setup_zoom_overlay(&ui);

    // Notification clicks land here: focus the window and show that result.
    {
        let action = gio::SimpleAction::new("show-generation", Some(glib::VariantTy::INT64));
        let ui = ui.clone();
        action.connect_activate(move |_, parameter| {
            let Some(id) = parameter.and_then(|value| value.get::<i64>()) else {
                return;
            };
            ui.window.present();
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
        app.add_action(&action);
    }

    // Reference copies belong to queued jobs only; nothing survives a restart.
    let _ = std::fs::remove_dir_all(paths::cache_dir().join("queue"));

    if let Ok(entries) = ui.history.borrow().recent(HISTORY_LIMIT) {
        *ui.entries.borrow_mut() = entries;
        history_view::collapse_all_groups(&ui);
        history_view::rebuild(&ui);
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
            if ui.upscaling.get() {
                if let Err(error) = ui.backend.cancel_upscale() {
                    ui.toast
                        .add_toast(adw::Toast::new(&format!("Cancel failed: {error}")));
                }
                ui.status.set_text("Cancelling…");
            } else {
                queue_view::cancel_running(&ui);
            }
        });
    }
    {
        let ui = ui.clone();
        ui.queue_clear
            .clone()
            .connect_clicked(move |_| queue_view::clear_pending(&ui));
    }
    {
        let ui = ui.clone();
        ui.live_button
            .clone()
            .connect_clicked(move |_| show_live(&ui));
    }
    {
        let ui = ui.clone();
        let button = ui.reference_button.clone();
        button.connect_clicked(move |_| choose_references(&ui));
    }
    {
        let ui = ui.clone();
        let button = ui.reference_add.clone();
        button.connect_clicked(move |_| choose_references(&ui));
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
            // The new preset can require a coarser pixel grid.
            update_size_ui(&ui);
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
        let scale = ui.mp_scale.clone();
        scale.connect_value_changed(move |_| update_size_ui(&ui));
    }
    {
        let ui = ui.clone();
        let dropdown = ui.ratio_dropdown.clone();
        dropdown.connect_selected_notify(move |dropdown| {
            if ui.size_guard.get() {
                return;
            }
            // The custom entry is only a temporary mirror of the current
            // reference/history ratio; drop it when a preset is picked.
            let custom_position = size::RATIOS.len() as u32;
            if (dropdown.selected() as usize) < size::RATIOS.len()
                && ui.ratio_model.n_items() > custom_position
            {
                ui.size_guard.set(true);
                ui.ratio_model.remove(custom_position);
                ui.custom_ratio.replace(None);
                ui.size_guard.set(false);
            }
            update_size_ui(&ui);
        });
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
        let list = ui.history_list.clone();
        list.connect_row_activated(move |_, row| {
            if ui.upscaling.get() || ui.classifying.get() {
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
        let button = ui.adjust_button.clone();
        button.connect_clicked(move |_| adjust::show(&ui));
    }
    {
        let ui = ui.clone();
        let button = ui.save_effect_button.clone();
        button.connect_clicked(move |_| effects::show_save_dialog(&ui));
    }
    {
        let ui = ui.clone();
        let button = ui.colorize_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            colorize_image(&ui);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.recover_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            recover_image(&ui);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.enhance_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            enhance_image(&ui);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.denoise_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            denoise_image(&ui);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.photoreal_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            photoreal_image(&ui);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.background_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            replace_background_image(&ui);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.auto_lighting_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            auto_lighting(&ui);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.daylight_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            convert_lighting(&ui, false);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.night_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            convert_lighting(&ui, true);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.expand_item.clone();
        button.connect_clicked(move |_| {
            ui.effects_popover.popdown();
            expand_image(&ui);
        });
    }
    {
        let ui = ui.clone();
        let button = ui.back_button.clone();
        button.connect_clicked(move |_| back_to_view(&ui));
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
        let button = ui.zoom_in_button.clone();
        button.connect_clicked(move |_| zoom_by(&ui, ZOOM_STEP));
    }
    {
        let ui = ui.clone();
        let button = ui.zoom_out_button.clone();
        button.connect_clicked(move |_| zoom_by(&ui, 1.0 / ZOOM_STEP));
    }
    {
        let ui = ui.clone();
        let button = ui.zoom_fit_button.clone();
        button.connect_clicked(move |_| reset_zoom(&ui));
    }
    for property in ["width", "height"] {
        let ui = ui.clone();
        let scroller = ui.zoom_scroller.clone();
        scroller.connect_notify_local(Some(property), move |_, _| {
            apply_zoom(&ui);
            update_zoom_controls(&ui);
        });
    }
    {
        let ui = ui.clone();
        let gesture = gtk::GestureDrag::new();
        gesture.set_button(gdk::BUTTON_PRIMARY);
        let start = Rc::new(Cell::new((0.0, 0.0)));
        {
            let ui = ui.clone();
            let start = start.clone();
            gesture.connect_drag_begin(move |_, _, _| {
                if ui.zoom.get() <= 1.0 {
                    return;
                }
                // A running animation would fight the pointer.
                stop_zoom_animation(&ui);
                start.set((
                    ui.zoom_scroller.hadjustment().value(),
                    ui.zoom_scroller.vadjustment().value(),
                ));
                ui.stack.set_cursor_from_name(Some("grabbing"));
            });
        }
        {
            let ui = ui.clone();
            gesture.connect_drag_update(move |_, dx, dy| {
                if ui.zoom.get() <= 1.0 {
                    return;
                }
                let (start_x, start_y) = start.get();
                let horizontal = ui.zoom_scroller.hadjustment();
                let vertical = ui.zoom_scroller.vadjustment();
                let max_x = (horizontal.upper() - horizontal.page_size()).max(horizontal.lower());
                let max_y = (vertical.upper() - vertical.page_size()).max(vertical.lower());
                horizontal.set_value((start_x - dx).clamp(horizontal.lower(), max_x));
                vertical.set_value((start_y - dy).clamp(vertical.lower(), max_y));
            });
        }
        {
            let ui = ui.clone();
            gesture.connect_drag_end(move |_, _, _| {
                if ui.zoom.get() > 1.0 {
                    ui.stack.set_cursor_from_name(Some("grab"));
                }
            });
        }
        // The gesture lives on the scroller: the stack itself moves while
        // panning, and a moving widget turns drag offsets into feedback.
        ui.zoom_scroller.add_controller(gesture);
    }
    {
        let ui = ui.clone();
        let stack = ui.stack.clone();
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _dx, dy| {
            if dy == 0.0
                || ui.generating.get()
                || ui.upscaling.get()
                || ui.zooming.get()
                || ui.classifying.get()
            {
                return glib::Propagation::Proceed;
            }
            // While zoomed the wheel pans the viewport instead of walking history.
            if ui.zoom.get() > 1.0 {
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
        window.connect_close_request(move |window| {
            let _ = ui.config.borrow().save();
            if ui.quitting.get() {
                return glib::Propagation::Proceed;
            }
            let (running, waiting) = {
                let queue = ui.queue.borrow();
                (queue.running().is_some(), queue.pending_len())
            };
            if !running && waiting == 0 {
                return glib::Propagation::Proceed;
            }
            let body = match (running, waiting) {
                (true, 0) => "A job is still running.".to_string(),
                (true, 1) => "A job is still running and 1 more is waiting.".to_string(),
                (true, count) => format!("A job is still running and {count} more are waiting."),
                (false, 1) => "1 job is still waiting.".to_string(),
                (false, count) => format!("{count} jobs are still waiting."),
            };
            let dialog = adw::AlertDialog::builder()
                .heading("Quit Flux Studio?")
                .body(format!("{body} Quitting cancels the queue."))
                .close_response("cancel")
                .default_response("cancel")
                .build();
            dialog.add_response("cancel", "Keep running");
            dialog.add_response("quit", "Quit");
            dialog.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
            let ui = ui.clone();
            dialog.connect_response(Some("quit"), move |_, _| {
                ui.quitting.set(true);
                let _ = ui.backend.cancel();
                ui.window.destroy();
            });
            dialog.present(Some(window));
            glib::Propagation::Stop
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
            if key == gdk::Key::Escape && ui.zoom.get() > 1.0 {
                reset_zoom(&ui);
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
                gdk::Key::plus | gdk::Key::equal => {
                    zoom_by(&ui, ZOOM_STEP);
                    glib::Propagation::Stop
                }
                gdk::Key::minus => {
                    zoom_by(&ui, 1.0 / ZOOM_STEP);
                    glib::Propagation::Stop
                }
                gdk::Key::_0 => {
                    reset_zoom(&ui);
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
    ui.rebuilding_models.set(true);
    ui.model_dropdown.set_model(Some(&list));

    let is_empty = ui.models.borrow().is_empty();
    ui.model_dropdown.set_sensitive(!is_empty);

    if is_empty {
        ui.model_dropdown.set_selected(gtk::INVALID_LIST_POSITION);
        ui.rebuilding_models.set(false);
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
    ui.rebuilding_models.set(false);

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
    let prompt_helper = prompt_ai::find_cli(&ui.config.borrow());
    if ui.models.borrow().is_empty() {
        ui.model_dropdown
            .set_tooltip_text(Some("No models installed"));
        ui.model_hint
            .set_text("No models installed — download in Preferences");
        ui.model_hint.add_css_class("error");
        ui.generate.set_sensitive(false);
        ui.reference_button.set_visible(false);
        render_reference_strip(ui);
        if prompt_helper.is_some() {
            ui.improve_button
                .set_tooltip_text(Some("No model installed"));
            ui.improve_button.set_sensitive(false);
        }
        return;
    }

    let model = active_preset(ui);
    let missing = model.missing_required(&ui.models_dir);

    ui.model_dropdown.set_tooltip_text(Some(model.description));
    ui.reference_button.set_visible(model.supports_edit);
    render_reference_strip(ui);

    if missing.is_empty() {
        ui.model_hint.set_text(&format!(
            "{} · {:.1} GB · installed",
            model.label,
            model.total_bytes() as f64 / 1e9
        ));
        ui.model_hint.remove_css_class("error");
        ui.generate.set_sensitive(true);
    } else {
        let names: Vec<&str> = missing.iter().map(|file| file.rel_path).collect();
        ui.model_hint.set_text(&format!(
            "Missing {} — download in Preferences",
            names.join(", ")
        ));
        ui.model_hint.add_css_class("error");
        ui.generate.set_sensitive(false);
    }

    if prompt_helper.is_some() {
        let encoder = model
            .llm_encoder()
            .map(|file| ui.models_dir.join(file.rel_path));
        let encoder_ready = encoder.as_ref().map(|path| path.exists()).unwrap_or(false);
        if !ui.improving.get() {
            ui.improve_button.set_sensitive(encoder_ready);
        }
        let tooltip = match encoder {
            Some(path) if path.exists() => format!(
                "Improve the prompt with {} (llama.cpp)",
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            ),
            Some(_) => "Text encoder not installed".to_string(),
            None => "This model has no language text encoder".to_string(),
        };
        ui.improve_button.set_tooltip_text(Some(&tooltip));
    }
}

/// Refresh the prompt-improvement button after the active llama.cpp changes.
pub(crate) fn refresh_prompt_helper(ui: &Rc<Ui>) {
    let found = prompt_ai::find_cli(&ui.config.borrow());
    ui.improve_button.set_visible(found.is_some());
    refresh_model(ui);
}

/// The ratio currently selected in the sidebar.
pub(crate) fn selected_ratio(ui: &Rc<Ui>) -> (u32, u32) {
    let index = ui.ratio_dropdown.selected() as usize;
    if let Some(&ratio) = size::RATIOS.get(index) {
        ratio
    } else {
        ui.custom_ratio.borrow().unwrap_or((1, 1))
    }
}

/// Point the sidebar at a preset ratio, or add/update its custom entry.
fn set_ratio_selection(ui: &Rc<Ui>, ratio: (u32, u32)) {
    ui.size_guard.set(true);
    match size::ratio_for_size(ratio.0, ratio.1) {
        size::RatioMatch::Preset(index) => {
            let custom_position = size::RATIOS.len() as u32;
            if ui.ratio_model.n_items() > custom_position {
                ui.ratio_model.remove(custom_position);
            }
            ui.custom_ratio.replace(None);
            ui.ratio_dropdown.set_selected(index as u32);
        }
        size::RatioMatch::Custom(custom) => {
            let custom_position = size::RATIOS.len() as u32;
            if ui.ratio_model.n_items() > custom_position {
                ui.ratio_model.remove(custom_position);
            }
            ui.ratio_model.append(&size::ratio_label(custom));
            ui.custom_ratio.replace(Some(custom));
            ui.ratio_dropdown.set_selected(custom_position);
        }
    }
    ui.size_guard.set(false);
}

fn set_target(ui: &Rc<Ui>, mp: f64, ratio: (u32, u32)) {
    set_ratio_selection(ui, ratio);
    ui.size_guard.set(true);
    ui.mp_scale.set_value(mp.clamp(size::MP_MIN, size::MP_MAX));
    ui.size_guard.set(false);
    update_size_ui(ui);
}

fn current_mp(ui: &Rc<Ui>) -> f64 {
    ((ui.mp_scale.value().clamp(size::MP_MIN, size::MP_MAX) * 100.0).round()) / 100.0
}

fn target_size(ui: &Rc<Ui>) -> (u32, u32) {
    size::size_for_mp_step(
        current_mp(ui),
        selected_ratio(ui),
        active_preset(ui).size_step,
    )
}

pub(crate) fn update_size_ui(ui: &Rc<Ui>) {
    if ui.size_guard.get() {
        return;
    }
    let mp = current_mp(ui);
    let ratio = selected_ratio(ui);
    let (width, height) = size::size_for_mp_step(mp, ratio, active_preset(ui).size_step);
    let megapixels = size::megapixels(width, height);

    ui.size_guard.set(true);
    if (ui.mp_scale.value() - mp).abs() > 1e-9 {
        ui.mp_scale.set_value(mp);
    }
    ui.size_guard.set(false);

    ui.mp_value.set_text(&format!("{mp:.2} MP"));
    ui.ratio_label.set_text(&size::ratio_label(ratio));
    ui.mp_label
        .set_text(&format!("{megapixels:.2} MP - {width} × {height} px"));

    let mut hint = String::new();
    if size::is_large(width, height) {
        hint.push_str("Large size — slower and may exceed 8 GB VRAM");
    }
    if (width == size::MAX_SIZE || height == size::MAX_SIZE) && megapixels + 0.005 < mp {
        if !hint.is_empty() {
            hint.push('\n');
        }
        hint.push_str(&format!(
            "Capped at {} px — max {:.2} MP at this ratio",
            size::MAX_SIZE,
            megapixels
        ));
    } else if (width == size::MIN_SIZE || height == size::MIN_SIZE) && megapixels > mp + 0.005 {
        if !hint.is_empty() {
            hint.push('\n');
        }
        hint.push_str(&format!(
            "Minimum side is {} px — minimum {:.2} MP at this ratio",
            size::MIN_SIZE,
            megapixels
        ));
    }

    if hint.is_empty() {
        ui.size_hint.set_text("");
        ui.size_hint.remove_css_class("warning");
    } else {
        ui.size_hint.set_text(&hint);
        ui.size_hint.add_css_class("warning");
    }

    ui.size_cell.set((width, height));
    ui.ratio_area.queue_draw();

    let mut config = ui.config.borrow_mut();
    config.default_mp = Some(mp);
    config.default_ratio_width = Some(ratio.0);
    config.default_ratio_height = Some(ratio.1);
    config.default_width = width;
    config.default_height = height;
    let _ = config.save();
}

pub(crate) fn set_size(ui: &Rc<Ui>, width: u32, height: u32) {
    let width = width.max(1);
    let height = height.max(1);
    if matches_current_target(ui, width, height) {
        set_target(ui, current_mp(ui), selected_ratio(ui));
        return;
    }
    let mp = size::mp_for_size(width, height);
    set_target(ui, mp, (width, height));
}

/// Whether `(width, height)` is just the current target rounded to the grid.
///
/// A coarse grid cannot reproduce every aspect exactly: 3:4 at 0.5 MP becomes
/// 608 x 832 on a 32 px grid, which reduces to 19:26. Treating that as a custom
/// ratio would change the sidebar for images that the app itself produced.
fn matches_current_target(ui: &Rc<Ui>, width: u32, height: u32) -> bool {
    let step = active_preset(ui).size_step;
    let (target_width, target_height) =
        size::size_for_mp_step(current_mp(ui), selected_ratio(ui), step);
    let tolerance = i64::from(step) / 2;
    (i64::from(target_width) - i64::from(width)).abs() <= tolerance
        && (i64::from(target_height) - i64::from(height)).abs() <= tolerance
}

/// Match the target ratio to the reference image, keeping the current MP.
///
/// Only applies to a single reference: with several inputs the sidebar target
/// is the source of truth.
fn fit_reference_size(ui: &Rc<Ui>) {
    if ui.state.borrow().references.len() != 1 {
        return;
    }
    let (width, height) = ui.image_size.get();
    if width == 0 || height == 0 {
        return;
    }
    let mp = current_mp(ui);
    if matches_current_target(ui, width, height) {
        set_target(ui, mp, selected_ratio(ui));
    } else {
        set_target(ui, mp, (width, height));
    }
}

/// Keep the sidebar selection in sync with the image on the canvas.
fn sync_history_selection(ui: &Rc<Ui>) {
    match ui.state.borrow().display.clone() {
        Some(path) => history_view::highlight(ui, &path),
        None => ui.history_list.unselect_all(),
    }
}

const ZOOM_STEP: f64 = 1.25;
const MAX_ZOOM_SCALE: f64 = 8.0;
const MAX_ZOOM_EDGE: f64 = 16384.0;

/// Screen pixels per image pixel when the image is fitted to the canvas.
fn fit_scale_for(view: (f64, f64), image: (u32, u32)) -> Option<f64> {
    let (view_w, view_h) = view;
    let (image_w, image_h) = image;
    if view_w <= 0.0 || view_h <= 0.0 || image_w == 0 || image_h == 0 {
        return None;
    }
    Some((view_w / f64::from(image_w)).min(view_h / f64::from(image_h)))
}

fn fit_scale(ui: &Ui) -> Option<f64> {
    fit_scale_for(
        (
            f64::from(ui.zoom_scroller.width()),
            f64::from(ui.zoom_scroller.height()),
        ),
        ui.image_size.get(),
    )
}

/// Largest zoom multiplier that keeps the image at 800% of its own pixels.
fn max_zoom_for(fit: f64, image: (u32, u32)) -> f64 {
    let longest = f64::from(image.0.max(image.1));
    let by_scale = MAX_ZOOM_SCALE / fit;
    let by_edge = MAX_ZOOM_EDGE / (longest * fit).max(1.0);
    by_scale.min(by_edge).max(1.0)
}

fn zoom_percent(fit: f64, zoom: f64) -> String {
    format!("{:.0}%", fit * zoom * 100.0)
}

/// Resize the picture stack inside the scroller for the current zoom factor.
fn apply_zoom(ui: &Ui) {
    let Some(fit) = fit_scale(ui) else {
        ui.stack.set_size_request(-1, -1);
        return;
    };
    let image = ui.image_size.get();
    let zoom = ui.zoom.get().clamp(1.0, max_zoom_for(fit, image));
    ui.zoom.set(zoom);
    // Pad the axis that still fits to the viewport so the image stays centred;
    // the other axis keeps the exact zoomed size and scrolls.
    let view_w = f64::from(ui.zoom_scroller.width()).max(1.0);
    let view_h = f64::from(ui.zoom_scroller.height()).max(1.0);
    let scaled_w = (f64::from(image.0) * fit * zoom).ceil().max(1.0);
    let scaled_h = (f64::from(image.1) * fit * zoom).ceil().max(1.0);
    ui.stack
        .set_size_request(scaled_w.max(view_w) as i32, scaled_h.max(view_h) as i32);
    if zoom <= 1.0 {
        ui.zoom_scroller.hadjustment().set_value(0.0);
        ui.zoom_scroller.vadjustment().set_value(0.0);
    }
    ui.stack
        .set_cursor_from_name(if zoom > 1.0 { Some("grab") } else { None });
}

fn update_zoom_controls(ui: &Ui) {
    let has_image = ui.state.borrow().display.is_some();
    let busy =
        ui.generating.get() || ui.upscaling.get() || ui.zooming.get() || ui.classifying.get();
    let visible = has_image && !busy && !ui.crop_mode.get();
    ui.zoom_controls.set_visible(visible);
    // Hiding the box is not enough: GTK keeps focusable children in the tab
    // chain when only an ancestor is invisible.
    ui.zoom_out_button.set_visible(visible);
    ui.zoom_label.set_visible(visible);
    ui.zoom_in_button.set_visible(visible);
    ui.zoom_fit_button.set_visible(visible);
    if !visible {
        return;
    }
    let zoom = ui.zoom.get();
    match fit_scale(ui) {
        Some(fit) => {
            ui.zoom_label.set_text(&zoom_percent(fit, zoom));
            ui.zoom_in_button
                .set_sensitive(zoom < max_zoom_for(fit, ui.image_size.get()) - 1e-6);
        }
        None => {
            ui.zoom_label.set_text("100%");
            ui.zoom_in_button.set_sensitive(false);
        }
    }
    ui.zoom_out_button.set_sensitive(zoom > 1.0 + 1e-6);
}

fn stop_zoom_animation(ui: &Ui) {
    if let Some(source) = ui.zoom_animation.borrow_mut().take() {
        source.remove();
    }
}

/// Ease the view to `target` instead of snapping.
fn animate_zoom(ui: &Rc<Ui>, target: f64) {
    stop_zoom_animation(ui);
    let Some(fit) = fit_scale(ui) else {
        return;
    };
    let target = target.clamp(1.0, max_zoom_for(fit, ui.image_size.get()));
    let start = ui.zoom.get();
    if (target - start).abs() < 1e-4 {
        ui.zoom.set(target);
        apply_zoom(ui);
        update_zoom_controls(ui);
        return;
    }

    let started = Instant::now();
    let duration = 0.18;
    let ui_callback = ui.clone();
    let source = glib::timeout_add_local(Duration::from_millis(16), move || {
        let progress = (started.elapsed().as_secs_f64() / duration).min(1.0);
        ui_callback
            .zoom
            .set(start + (target - start) * ease(progress));
        apply_zoom(&ui_callback);
        update_zoom_controls(&ui_callback);
        if progress >= 1.0 {
            ui_callback.zoom.set(target);
            apply_zoom(&ui_callback);
            update_zoom_controls(&ui_callback);
            *ui_callback.zoom_animation.borrow_mut() = None;
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    *ui.zoom_animation.borrow_mut() = Some(source);
}

fn zoom_by(ui: &Rc<Ui>, factor: f64) {
    if ui.state.borrow().display.is_none() {
        return;
    }
    let Some(fit) = fit_scale(ui) else {
        return;
    };
    let max = max_zoom_for(fit, ui.image_size.get());
    let next = (ui.zoom.get() * factor).clamp(1.0, max);
    animate_zoom(ui, if next < 1.0 + 1e-6 { 1.0 } else { next });
}

/// Return to fit-to-window without touching the image.
fn reset_zoom(ui: &Ui) {
    stop_zoom_animation(ui);
    ui.zoom.set(1.0);
    apply_zoom(ui);
    update_zoom_controls(ui);
}

/// Paint `texture` on the canvas, crossfading from whichever frame is visible.
///
/// The stack has two picture children so consecutive updates animate instead
/// of snapping, which makes step previews and history navigation feel smooth.
fn crossfade_to_texture(ui: &Rc<Ui>, texture: &gdk::Texture) {
    let rounded = rounded_paintable::RoundedPaintable::new(texture);
    let (target, picture) = match ui.stack.visible_child_name().as_deref() {
        Some("image") => ("back", &ui.picture_back),
        Some("back") => ("image", &ui.picture),
        _ => ("image", &ui.picture),
    };
    picture.set_paintable(Some(&rounded));
    ui.stack.set_visible_child_name(target);
}

pub(crate) fn update_image_view(ui: &Rc<Ui>) {
    let display = ui.state.borrow().display.clone();

    match display {
        Some(path) => match gdk::Texture::from_filename(&path) {
            Ok(texture) => {
                ui.image_size
                    .set((texture.width() as u32, texture.height() as u32));
                crossfade_to_texture(ui, &texture);
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

    render_reference_strip(ui);

    ui.ratio_area.queue_draw();
    sync_history_selection(ui);
    reset_zoom(ui);
    update_image_actions(ui);
}

/// Rebuild the reference thumbnails above the canvas.
fn render_reference_strip(ui: &Rc<Ui>) {
    while let Some(child) = ui.reference_thumbs.first_child() {
        ui.reference_thumbs.remove(&child);
    }

    let references = ui.state.borrow().references.clone();
    ui.reference_strip.set_visible(!references.is_empty());

    let max = active_preset(ui).max_references as usize;
    for (index, path) in references.iter().enumerate() {
        let number = index + 1;
        let picture = gtk::Picture::builder()
            .width_request(48)
            .height_request(48)
            .content_fit(gtk::ContentFit::Cover)
            .build();
        if let Some(texture) = reference_thumbnail(ui, path) {
            picture.set_paintable(Some(&texture));
        }

        let image = gtk::Button::builder()
            .child(&picture)
            .tooltip_text(format!(
                "Image {number} · refer to it as <image_{number}>\n{}",
                path.display()
            ))
            .css_classes(["flat", "reference-thumb"])
            .build();
        {
            let ui = ui.clone();
            let path = path.clone();
            image.connect_clicked(move |_| {
                if references_busy(&ui) {
                    return;
                }
                pause_live_view(&ui);
                ui.state.borrow_mut().select_display(path.clone());
                update_image_view(&ui);
                ui.status.set_text(&format!(
                    "Reference {number} of {count}",
                    count = ui.state.borrow().references.len()
                ));
            });
        }

        let badge = gtk::Label::builder()
            .label(number.to_string())
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start(3)
            .margin_top(3)
            .css_classes(["reference-index"])
            .build();

        let remove = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Remove this reference")
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .css_classes(["flat", "circular", "reference-remove"])
            .build();
        {
            let ui = ui.clone();
            let path = path.clone();
            remove.connect_clicked(move |_| remove_reference(&ui, &path));
        }

        let tile = gtk::Overlay::new();
        tile.set_child(Some(&image));
        tile.add_overlay(&badge);
        tile.add_overlay(&remove);
        ui.reference_thumbs.append(&tile);
    }

    let at_max = references.len() >= max;
    ui.reference_add.set_sensitive(!at_max);
    ui.reference_add.set_tooltip_text(Some(if at_max {
        "Maximum number of references reached"
    } else {
        "Add reference images"
    }));
}

pub(crate) fn show_composer(ui: &Rc<Ui>, show: bool) {
    if show {
        ui.crop_button.set_active(false);
        set_crop_mode(ui, false);
        ui.effects_popover.popdown();
    }
    ui.composer.set_reveal_child(show);
    update_image_actions(ui);
    if show {
        ui.prompt.grab_focus();
    }
}

pub(crate) fn update_image_actions(ui: &Rc<Ui>) {
    let (has_image, editing) = {
        let state = ui.state.borrow();
        (state.display.is_some(), state.is_editing())
    };
    let composing = ui.composer.reveals_child();
    let generating = ui.generating.get();
    let upscaling = ui.upscaling.get();
    let zooming = ui.zooming.get();
    let classifying = ui.classifying.get();
    let foreground = upscaling || zooming || classifying;
    let position = history_position(ui);
    let actions = has_image && !foreground;
    let editable = actions && active_preset(ui).supports_edit;
    let navigation = actions && !editing && !generating;
    let vision_ready = flux_core::vision::VISION_HELPER.is_installed(&ui.models_dir);

    ui.nav_up_button
        .set_visible(navigation && position.is_some_and(|(index, _)| index > 0));
    ui.nav_down_button
        .set_visible(navigation && position.is_some_and(|(index, count)| index + 1 < count));
    ui.effects_button.set_visible(editable);
    ui.save_effect_button
        .set_visible(actions && !generating && effects::current_savable_prompt(ui).is_some());
    ui.auto_lighting_item.set_visible(vision_ready);
    ui.daylight_item.set_visible(!vision_ready);
    ui.night_item.set_visible(!vision_ready);
    ui.upscale_button.set_visible(actions && !generating);
    ui.adjust_button.set_visible(actions && !generating);
    ui.live_button
        .set_visible(generating && !ui.live_view.get());
    ui.edit_button.set_visible(editable && !composing);
    ui.crop_button.set_visible(editable && !generating);
    ui.back_button.set_visible(editing && !foreground);
    ui.undo_button
        .set_visible(editable && !generating && ui.undo.borrow().can_undo());
    update_zoom_controls(ui);

    if composing || !editable {
        ui.effects_popover.popdown();
    }
    if !editable {
        ui.crop_button.set_active(false);
    }
}

fn edit_current_image(ui: &Rc<Ui>) {
    if ui.upscaling.get() || ui.classifying.get() {
        return;
    }
    pause_live_view(ui);
    if !active_preset(ui).supports_edit {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} cannot edit images.",
            active_preset(ui).label
        )));
        return;
    }
    let Some(path) = ui.state.borrow().display.clone() else {
        return;
    };
    let added = {
        let mut state = ui.state.borrow_mut();
        if state.references.is_empty() {
            state.references.push(path);
            true
        } else {
            false
        }
    };
    update_image_view(ui);
    if added {
        fit_reference_size(ui);
    }
    show_composer(ui, true);
    let count = ui.state.borrow().references.len();
    ui.status.set_text(&if count == 1 {
        "Editing reference".to_string()
    } else {
        format!("Editing with {count} references")
    });
}

fn set_crop_mode(ui: &Rc<Ui>, active: bool) {
    ui.crop_mode.set(active);
    ui.crop_overlay.set_visible(active);
    *ui.crop_rect.borrow_mut() = None;
    if active {
        // Selection coordinates assume a letterboxed fit.
        reset_zoom(ui);
        ui.crop_overlay.set_cursor_from_name(Some("crosshair"));
        ui.crop_overlay.grab_focus();
        ui.status.set_text("Drag to select an area · Esc to cancel");
    } else {
        ui.crop_overlay.set_cursor_from_name(None);
        ui.crop_overlay.queue_draw();
    }
    update_image_actions(ui);
}

fn undo_crop(ui: &Rc<Ui>) {
    if ui.generating.get() || ui.classifying.get() {
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
    fit_reference_size(ui);
    show_composer(ui, false);
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
            fit_reference_size(ui);
            start_zoom(
                ui,
                surface,
                (0.0, 0.0, image_w as f64, image_h as f64),
                (x as f64, y as f64, width as f64, height as f64),
                false,
            );
            show_composer(ui, false);
            ui.status.set_text(&format!(
                "Cropped to {width} × {height} · press Edit to prompt"
            ));
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

pub(crate) fn new_image(ui: &Rc<Ui>) {
    if ui.upscaling.get() || ui.classifying.get() {
        return;
    }
    pause_live_view(ui);
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
    ui.mp_scale.set_sensitive(enabled);
    ui.ratio_dropdown.set_sensitive(enabled);
    ui.seed_entry.set_sensitive(enabled);
    ui.random_button.set_sensitive(enabled);
    ui.new_button.set_sensitive(enabled);
    ui.history_list.set_sensitive(enabled);
    ui.reference_button.set_sensitive(enabled);
    ui.improve_button
        .set_sensitive(enabled && !ui.improving.get());
    ui.generate.set_sensitive(enabled);
}

fn choose_references(ui: &Rc<Ui>) {
    if references_busy(ui) {
        return;
    }
    if !active_preset(ui).supports_edit {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} cannot edit images.",
            active_preset(ui).label
        )));
        return;
    }
    let dialog = gtk::FileDialog::builder()
        .title("Choose reference images")
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
    dialog.open_multiple(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
        let Ok(files) = result else {
            return;
        };
        let mut paths = Vec::new();
        for index in 0..files.n_items() {
            if let Some(path) = files
                .item(index)
                .and_downcast::<gtk::gio::File>()
                .and_then(|file| file.path())
            {
                paths.push(path);
            }
        }
        add_references(&ui, paths);
    });
}

/// Append picked images to the edit reference set.
fn add_references(ui: &Rc<Ui>, paths: Vec<PathBuf>) {
    if references_busy(ui) {
        return;
    }
    pause_live_view(ui);
    let max = active_preset(ui).max_references as usize;
    if max == 0 {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} cannot edit images.",
            active_preset(ui).label
        )));
        return;
    }

    let mut added = 0usize;
    let mut capped = false;
    let (was_empty, count) = {
        let mut state = ui.state.borrow_mut();
        let was_empty = state.references.is_empty();
        let mut first_added = None;
        for path in paths {
            if state.references.len() >= max {
                capped = true;
                break;
            }
            if state
                .references
                .iter()
                .any(|existing| same_file(existing, &path))
            {
                continue;
            }
            if first_added.is_none() {
                first_added = Some(path.clone());
            }
            state.references.push(path);
            added += 1;
        }
        if was_empty && let Some(path) = first_added {
            state.display = Some(path);
        }
        (was_empty, state.references.len())
    };

    if added == 0 {
        return;
    }

    ui.undo.borrow_mut().clear();
    update_image_view(ui);
    if was_empty && count == 1 {
        fit_reference_size(ui);
    }
    show_composer(ui, false);
    let label = if count == 1 {
        "Reference ready".to_string()
    } else {
        format!("{count} references ready")
    };
    ui.status
        .set_text(&format!("{label} · press Edit to prompt"));

    if capped {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "Using {count} references (the maximum for this model)."
        )));
    }
}

fn remove_reference(ui: &Rc<Ui>, path: &Path) {
    if references_busy(ui) {
        return;
    }
    pause_live_view(ui);
    let (removed, count) = {
        let mut state = ui.state.borrow_mut();
        let before = state.references.len();
        state.references.retain(|reference| reference != path);
        (before != state.references.len(), state.references.len())
    };
    if !removed {
        return;
    }

    {
        let mut state = ui.state.borrow_mut();
        let showing_removed = state.display.as_deref() == Some(path);
        if showing_removed && !state.references.is_empty() {
            let next = state.references.first().cloned();
            state.display = next;
        }
    }
    update_image_view(ui);
    if count == 0 {
        ui.status.set_text("Viewing image");
    } else {
        ui.status.set_text(&format!("{count} reference(s) ready"));
    }
}

fn references_busy(ui: &Rc<Ui>) -> bool {
    ui.upscaling.get() || ui.zooming.get() || ui.classifying.get()
}

/// A 48×48 cover-scaled texture for a reference thumbnail.
fn reference_thumbnail(ui: &Rc<Ui>, path: &Path) -> Option<gdk::Texture> {
    if let Some(texture) = ui.reference_thumbnails.borrow().get(path).cloned() {
        return Some(texture);
    }
    let image = image::open(path).ok()?;
    let thumb = image.resize_to_fill(48, 48, image::imageops::FilterType::Triangle);
    let mut png = Vec::new();
    thumb
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    let bytes = glib::Bytes::from(&png);
    let texture = gdk::Texture::from_bytes(&bytes).ok()?;
    ui.reference_thumbnails
        .borrow_mut()
        .insert(path.to_path_buf(), texture.clone());
    Some(texture)
}

fn same_file(left: &Path, right: &Path) -> bool {
    left == right
        || matches!(
            (left.canonicalize(), right.canonicalize()),
            (Ok(left), Ok(right)) if left == right
        )
}

fn improve_prompt(ui: &Rc<Ui>) {
    if ui.improving.get() {
        return;
    }

    let original = current_prompt(ui).trim().to_string();
    if original.is_empty() {
        ui.toast.add_toast(adw::Toast::new("Write a prompt first."));
        return;
    }

    let Some(cli) = prompt_ai::find_cli(&ui.config.borrow()) else {
        return;
    };
    let encoder = ui.config.borrow().prompt_helper_model.clone().or_else(|| {
        active_preset(ui)
            .llm_encoder()
            .map(|file| ui.models_dir.join(file.rel_path))
    });
    let Some(encoder) = encoder else {
        ui.toast
            .add_toast(adw::Toast::new("This model has no language text encoder."));
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

const COLORIZE_PROMPT: &str = "photorealistic colorization of this vintage black and white photo, restoring natural skin tones and authentic colors while maintaining the original details";
const RECOVER_PROMPT: &str = "restore this old damaged photograph, remove scratches, stains, creases, dust and tears, repair missing areas, recover natural detail and texture, photorealistic restoration, keep the same composition, subjects, and expression";
const ENHANCE_PROMPT: &str = "enhance this image, sharpen fine details, increase clarity and micro-contrast, reduce blur and compression artifacts, natural textures, photorealistic, keep the same composition, subjects, colors, and lighting";
const DENOISE_PROMPT: &str = "remove noise, grain, compression artifacts and banding from this image, smooth flat areas while preserving edges and fine detail, keep the same composition, subjects, colors, and lighting";
const PHOTOREAL_PROMPT: &str = "transform this drawing or cartoon into a photorealistic photograph, realistic skin, hair, fabric and material textures, natural lighting and shadows, accurate anatomy and perspective, keep the same composition, subjects, pose, and details";
const BACKGROUND_PROMPT: &str = "replace the background with a clean neutral studio backdrop, soft even lighting, keep the subject, pose, expression, hair and clothing unchanged, realistic shadows and edges";
const NIGHT_PROMPT: &str = "transform this daytime scene into night, dark blue sky, moonlight, glowing windows and street lights, deep shadows, cinematic night photography, keep the same composition, subjects, and details";
const DAY_PROMPT: &str = "transform this night scene into bright daylight, clear blue sky, warm sunlight, natural colors, soft shadows, keep the same composition, subjects, and details";
const EXPAND_PROMPT: &str = "expand the image to fill the new canvas, extending the scene naturally beyond the original borders, keep the same composition, subjects, lighting, and details";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenerationMode {
    Generate,
    Edit,
    Custom,
    Colorize,
    Recover,
    Enhance,
    Denoise,
    Photoreal,
    Background,
    Night,
    Day,
    Expand,
}

impl GenerationMode {
    pub(crate) fn queue_label(self) -> &'static str {
        match self {
            Self::Generate => "Generate",
            Self::Edit => "Edit",
            _ => "Effect",
        }
    }

    fn keeps_reference(self) -> bool {
        self == Self::Edit
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::Colorize => "colorize",
            Self::Recover => "recover",
            Self::Enhance => "enhance",
            Self::Denoise => "denoise",
            Self::Photoreal => "photoreal",
            Self::Background => "background",
            Self::Night => "night",
            Self::Day => "day",
            Self::Expand => "expand",
            Self::Generate | Self::Edit => "generate",
        }
    }

    fn done_message(self) -> &'static str {
        match self {
            Self::Custom => "Effect applied.",
            Self::Colorize => "Image colorized.",
            Self::Recover => "Image recovered.",
            Self::Enhance => "Image enhanced.",
            Self::Denoise => "Image denoised.",
            Self::Photoreal => "Image made photorealistic.",
            Self::Background => "Background replaced.",
            Self::Night => "Converted to night.",
            Self::Day => "Converted to day.",
            Self::Expand => "Image expanded.",
            Self::Generate | Self::Edit => "Image generated.",
        }
    }
}

fn current_style_id(ui: &Rc<Ui>) -> &'static str {
    STYLES
        .get(ui.style_dropdown.selected() as usize)
        .map(|style| style.id)
        .unwrap_or("none")
}

fn start_generation(ui: &Rc<Ui>) {
    let editing = ui.state.borrow().is_editing();
    if editing && !active_preset(ui).supports_edit {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} cannot edit images. Remove the references or pick another model.",
            active_preset(ui).label
        )));
        return;
    }
    let mode = if editing {
        GenerationMode::Edit
    } else {
        GenerationMode::Generate
    };
    generate(
        ui,
        mode,
        current_prompt(ui),
        current_style_id(ui),
        true,
        None,
    );
}

fn generate(
    ui: &Rc<Ui>,
    mode: GenerationMode,
    base_prompt: String,
    style_id: &str,
    require_composer: bool,
    source_path: Option<String>,
) {
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("The backend is still installing."));
        return;
    }
    if ui.upscaling.get()
        || ui.zooming.get()
        || ui.classifying.get()
        || !ui.generate.is_sensitive()
        || (require_composer && !ui.composer.reveals_child())
    {
        return;
    }
    if ui.queue.borrow().is_full() {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "The queue is full ({} jobs). Wait for one to finish.",
            queue::LIMIT
        )));
        return;
    }

    let base_prompt = base_prompt.trim().to_string();
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

    let effective_prompt = flux_core::style::compose_prompt(&base_prompt, style_id);

    let (width, height) = target_size(ui);
    let references = ui.state.borrow().references.clone();
    let output_dir = ui.config.borrow().output_dir.clone();
    let (offload, previews) = {
        let config = ui.config.borrow();
        (config.offload, config.show_previews)
    };

    let output = output_dir.join(format!("{}-{}.png", slugify(&base_prompt), now_millis()));

    let mut request = GenerateRequest::new(effective_prompt, output);
    request.preset_id = model.id.to_string();
    request.width = width;
    request.height = height;
    request.steps = model.steps;
    request.cfg = model.cfg;
    request.sigmas = model.sigmas.to_vec();
    request.seed = seed;
    request.sampler = model.sampler;
    request.offload = offload;
    request.previews = previews;

    // The queue keeps its own copies: a crop in the cache or a history entry
    // can disappear before this job starts. The originals stay in the job for
    // History's parent lookup.
    let id = ui.queue.borrow_mut().next_id();
    let (copies, reference_dir) = match copy_references(id, &references) {
        Ok(prepared) => prepared,
        Err(error) => {
            ui.toast.add_toast(adw::Toast::new(&format!(
                "Could not prepare the references: {error}"
            )));
            return;
        }
    };
    request.reference_images = copies;

    let job = queue::Job {
        id,
        request,
        mode,
        prompt: base_prompt,
        style: style_id.to_string(),
        model,
        width,
        height,
        seed,
        steps: model.steps,
        reference_paths: references,
        source_path,
        seed_text,
        reference_dir,
    };

    let was_busy = ui.queue.borrow().running().is_some();
    if !was_busy {
        ui.live_view.set(true);
    }
    ui.queue.borrow_mut().push(job);
    queue_view::rebuild(ui);
    if was_busy {
        ui.status.set_text("Added to the queue");
    }
    pump_queue(ui);
}

/// Copy the selected references into a per-job folder.
fn copy_references(
    id: u64,
    references: &[PathBuf],
) -> Result<(Vec<PathBuf>, Option<PathBuf>), String> {
    if references.is_empty() {
        return Ok((Vec::new(), None));
    }
    let dir = paths::cache_dir().join("queue").join(id.to_string());
    if let Err(error) = std::fs::create_dir_all(&dir) {
        return Err(format!("could not create {}: {error}", dir.display()));
    }

    let mut copies = Vec::with_capacity(references.len());
    for (index, source) in references.iter().enumerate() {
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png");
        let target = dir.join(format!("ref-{}.{extension}", index + 1));
        if let Err(error) = std::fs::copy(source, &target) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("could not copy {}: {error}", source.display()));
        }
        copies.push(target);
    }
    Ok((copies, Some(dir)))
}

/// Start the next waiting job when the worker is idle.
///
/// Returns whether a job is now running.
fn pump_queue(ui: &Rc<Ui>) -> bool {
    if ui.queue.borrow().running().is_some() {
        return true;
    }
    let Some(job) = ui.queue.borrow_mut().take_next() else {
        return false;
    };
    ui.queue.borrow_mut().start(job.clone());

    ui.generating.set(true);
    ui.live_preview.borrow_mut().take();
    ui.generate
        .set_tooltip_text(Some("Add to queue (Ctrl+Enter)"));
    ui.cancel.set_sensitive(true);
    ui.progress.set_visible(true);
    ui.progress.set_fraction(0.0);
    ui.progress.set_text(Some("Loading models…"));
    if ui.live_view.get() {
        ui.status.set_text("Loading models…");
    }
    sync_live_controls(ui);
    update_image_actions(ui);

    let (sender, receiver) = async_channel::unbounded();
    if ui
        .worker
        .jobs
        .send_blocking((job.request.clone(), sender))
        .is_err()
    {
        tracing::error!("generation worker is unavailable");
        complete_job(
            ui,
            &job,
            Outcome::Failed("The generation worker is unavailable.".into()),
            0,
        );
        ui.queue.borrow_mut().clear_running();
        return pump_queue(ui);
    }

    queue_view::rebuild(ui);
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let started = Instant::now();
        let mut finished = false;
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
                    if ui.live_view.get() {
                        ui.status.set_text(&format!("Step {current}/{total}"));
                    }
                    queue_view::set_step(&ui, current, total);
                }
                Progress::Preview { step, total, png } => {
                    let bytes = glib::Bytes::from(&png);
                    match gdk::Texture::from_bytes(&bytes) {
                        Ok(texture) => {
                            ui.live_preview.replace(Some(texture.clone()));
                            if ui.live_view.get() {
                                crossfade_to_texture(&ui, &texture);
                            }
                        }
                        Err(error) => tracing::warn!(%error, "invalid preview frame"),
                    }
                    ui.progress.set_fraction(step as f64 / total.max(1) as f64);
                    ui.progress.set_text(Some(&format!("Step {step}/{total}")));
                    if ui.live_view.get() {
                        ui.status.set_text(&format!("Step {step}/{total}"));
                    }
                    queue_view::set_step(&ui, step, total);
                }
                Progress::Log(line) => tracing::debug!(target: "sd-cli", "{line}"),
                Progress::Cancelled => {
                    let duration_ms = started.elapsed().as_millis() as i64;
                    complete_job(&ui, &job, Outcome::Cancelled, duration_ms);
                    finished = true;
                    break;
                }
                Progress::Done(path) => {
                    let duration_ms = started.elapsed().as_millis() as i64;
                    complete_job(&ui, &job, Outcome::Done(path), duration_ms);
                    finished = true;
                    break;
                }
                Progress::Failed(error) => {
                    let duration_ms = started.elapsed().as_millis() as i64;
                    complete_job(&ui, &job, Outcome::Failed(error), duration_ms);
                    finished = true;
                    break;
                }
            }
        }
        if !finished {
            let duration_ms = started.elapsed().as_millis() as i64;
            complete_job(
                &ui,
                &job,
                Outcome::Failed("The generation worker stopped unexpectedly.".into()),
                duration_ms,
            );
        }
        finish_job(&ui);
    });
    true
}

enum Outcome {
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

/// Record a finished job in History, show it, and notify if asked.
fn complete_job(ui: &Rc<Ui>, job: &queue::Job, outcome: Outcome, duration_ms: i64) {
    if let Some(dir) = &job.reference_dir
        && let Err(error) = std::fs::remove_dir_all(dir)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %dir.display(), "failed to remove queued references");
    }

    match outcome {
        Outcome::Done(path) => {
            let image_path = path.to_string_lossy().into_owned();
            let follow_live = ui.live_view.get();

            // Only touch the references when the user has not changed them
            // since queueing; otherwise they belong to the next composition.
            let references = references_after_result(
                &ui.state.borrow().references,
                &job.reference_paths,
                job.mode,
                &path,
            );
            ui.undo.borrow_mut().clear();
            {
                let mut state = ui.state.borrow_mut();
                if follow_live {
                    match references {
                        Some(_) => state.on_result(path.clone(), job.mode.keeps_reference()),
                        None => state.display = Some(path.clone()),
                    }
                } else if let Some(references) = references {
                    state.references = references;
                }
            }
            if follow_live {
                update_image_view(ui);
            } else {
                render_reference_strip(ui);
                update_image_actions(ui);
            }

            let history_id = record_history(ui, job, &image_path, duration_ms, follow_live);
            // Do not overwrite the seed field while more jobs are waiting: it
            // belongs to the next composition, not to this result.
            if ui.queue.borrow().pending_len() == 0 && ui.seed_entry.text().trim() == job.seed_text
            {
                ui.seed_entry.set_text(&job.seed.to_string());
            }

            ui.toast.add_toast(adw::Toast::new(job.mode.done_message()));
            if follow_live {
                ui.status.set_text(&format!(
                    "{} · {} × {} · seed {}",
                    job.model.label, job.width, job.height, job.seed
                ));
            } else {
                ui.status.set_text(&format!(
                    "Result added to History · {} × {}",
                    job.width, job.height
                ));
            }
            notify_finished(ui, job, history_id, duration_ms, None);
        }
        Outcome::Failed(error) => {
            let short = error.lines().next().unwrap_or(&error).to_string();
            tracing::error!(%error, "generation failed");
            if ui.live_view.get() {
                update_image_view(ui);
                ui.status.set_text("Generation failed");
            }
            ui.toast
                .add_toast(adw::Toast::new(&format!("Generation failed: {short}")));
            notify_finished(ui, job, None, duration_ms, Some(&short));
        }
        Outcome::Cancelled => {
            if ui.live_view.get() {
                update_image_view(ui);
                ui.status.set_text("Cancelled");
            }
            ui.toast.add_toast(adw::Toast::new("Generation cancelled."));
        }
    }
}

/// References to apply after a result.
///
/// `None` means the user changed the strip after queueing, so the result must
/// not replace it. Otherwise edits promote the result to the next reference
/// and everything else clears the strip, as in a single-job run.
fn references_after_result(
    current: &[PathBuf],
    queued: &[PathBuf],
    mode: GenerationMode,
    result: &Path,
) -> Option<Vec<PathBuf>> {
    if current != queued {
        return None;
    }
    Some(if mode.keeps_reference() {
        vec![result.to_path_buf()]
    } else {
        Vec::new()
    })
}

fn record_history(
    ui: &Rc<Ui>,
    job: &queue::Job,
    image_path: &str,
    duration_ms: i64,
    follow_live: bool,
) -> Option<i64> {
    let seed = job.seed.to_string();
    let reference_paths = job.reference_strings();
    let reference_path = reference_paths.first().cloned();
    let created_at = unix_seconds();

    let inserted = ui.history.borrow().insert(&NewGeneration {
        prompt: &job.prompt,
        model: job.model.id,
        style: &job.style,
        width: job.width,
        height: job.height,
        steps: job.steps,
        seed: &seed,
        created_at,
        duration_ms,
        image_path,
        reference_path: reference_path.as_deref(),
        reference_paths: &reference_paths,
        kind: job.mode.kind(),
        source_path: job.source_path.as_deref(),
    });

    match inserted {
        Ok((id, parent_id)) => {
            if let Err(error) = write_thumbnail(Path::new(image_path), id) {
                tracing::warn!(%error, "failed to write thumbnail");
            }
            let entry = Generation {
                id,
                prompt: job.prompt.clone(),
                model: job.model.id.to_string(),
                style: job.style.clone(),
                width: job.width,
                height: job.height,
                steps: job.steps,
                seed,
                created_at,
                duration_ms,
                image_path: image_path.to_string(),
                reference_path,
                reference_paths,
                kind: job.mode.kind().into(),
                source_path: job.source_path.clone(),
                parent_id,
            };
            ui.entries.borrow_mut().insert(0, entry.clone());
            trim_history(ui);
            if follow_live {
                history_view::rebuild(ui);
                history_view::highlight(ui, Path::new(&entry.image_path));
            } else {
                // Keep the user's place in History while they browse.
                history_view::rebuild_preserving_scroll(ui);
                if let Some(display) = ui.state.borrow().display.clone() {
                    history_view::highlight(ui, &display);
                }
            }
            update_image_actions(ui);
            Some(id)
        }
        Err(error) => {
            tracing::error!(%error, "failed to record history");
            None
        }
    }
}

/// Drop the finished job and start the next one, or return to idle.
fn finish_job(ui: &Rc<Ui>) {
    ui.queue.borrow_mut().clear_running();
    if !pump_queue(ui) {
        ui.generating.set(false);
        ui.live_preview.borrow_mut().take();
        ui.generate.set_tooltip_text(Some("Generate (Ctrl+Enter)"));
        sync_live_controls(ui);
        ui.cancel.set_sensitive(false);
        ui.progress.set_visible(false);
        ui.progress.set_fraction(0.0);
    }
    refresh_model(ui);
    queue_view::rebuild(ui);
    update_image_actions(ui);
}

/// Keep the progress row and the "show live" button in sync with the view.
fn sync_live_controls(ui: &Ui) {
    if ui.upscaling.get() {
        return;
    }
    let running = ui.generating.get();
    let live = ui.live_view.get();
    ui.progress_row.set_visible(running && live);
    ui.live_button.set_visible(running && !live);
}

/// Follow the running job's previews again.
///
/// The latest frame is kept while the user browses History, so returning is
/// instant even between sampling steps.
pub(crate) fn show_live(ui: &Rc<Ui>) {
    ui.live_view.set(true);
    if let Some(texture) = ui.live_preview.borrow().clone() {
        crossfade_to_texture(ui, &texture);
    }
    if ui.generating.get() {
        ui.status.set_text("Live preview");
    }
    sync_live_controls(ui);
    update_image_actions(ui);
}

/// Stop painting the running job's previews over a history image.
pub(crate) fn pause_live_view(ui: &Rc<Ui>) {
    if !ui.live_view.get() {
        return;
    }
    ui.live_view.set(false);
    sync_live_controls(ui);
    update_image_actions(ui);
}

/// Report a finished job through the desktop notification daemon.
///
/// Notifications are for background work: the progress bar already covers the
/// focused window. Both the setting and the focus check are cheap, so callers
/// do not need to duplicate them.
fn notify_finished(
    ui: &Rc<Ui>,
    job: &queue::Job,
    history_id: Option<i64>,
    duration_ms: i64,
    error: Option<&str>,
) {
    if !ui.config.borrow().notify_on_finish || ui.window.is_active() {
        return;
    }
    let Some(application) = ui.window.application() else {
        return;
    };
    let app: &gio::Application = application.upcast_ref();

    let title = match error {
        Some(_) => "Generation failed",
        None => "Generation finished",
    };
    let body = match error {
        Some(error) => format!("{}\n{error}", job.snippet()),
        None => format!(
            "{}\n{} × {} · {}",
            job.snippet(),
            job.width,
            job.height,
            duration_label(duration_ms)
        ),
    };

    let notification = gio::Notification::new(title);
    notification.set_body(Some(&body));
    notification.set_icon(&gio::ThemedIcon::new(flux_core::APP_ID));
    if error.is_some() {
        notification.set_priority(gio::NotificationPriority::High);
    }
    if let Some(id) = history_id {
        notification
            .set_default_action_and_target_value("app.show-generation", Some(&id.to_variant()));
    }
    app.send_notification(Some(&format!("flux-job-{}", job.id)), &notification);
}

fn duration_label(duration_ms: i64) -> String {
    let seconds = (duration_ms.max(0) as f64 / 1000.0).round() as u64;
    if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn can_start_image_action(ui: &Rc<Ui>) -> bool {
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("The backend is still installing."));
        return false;
    }
    if !ui.generate.is_sensitive() {
        ui.toast
            .add_toast(adw::Toast::new("Install a model in Preferences first."));
        return false;
    }
    true
}

pub(crate) fn start_effect(ui: &Rc<Ui>, mode: GenerationMode, prompt: &str, empty_message: &str) {
    if ui.upscaling.get() || ui.zooming.get() || ui.classifying.get() {
        return;
    }
    if !active_preset(ui).supports_edit {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} cannot edit images.",
            active_preset(ui).label
        )));
        return;
    }
    if !can_start_image_action(ui) {
        return;
    }
    let Some(source) = ui.state.borrow().display.clone() else {
        ui.toast.add_toast(adw::Toast::new(empty_message));
        return;
    };

    // Keep the source shape, but use the MP target from the sidebar.
    ui.state.borrow_mut().references = vec![source.clone()];
    update_image_view(ui);
    fit_reference_size(ui);
    ui.prompt.buffer().set_text(prompt);
    ui.seed_entry.set_text("");
    generate(
        ui,
        mode,
        prompt.to_string(),
        "none",
        false,
        Some(source.to_string_lossy().into_owned()),
    );
}

fn colorize_image(ui: &Rc<Ui>) {
    start_effect(
        ui,
        GenerationMode::Colorize,
        COLORIZE_PROMPT,
        "No image to colorize.",
    );
}

fn recover_image(ui: &Rc<Ui>) {
    start_effect(
        ui,
        GenerationMode::Recover,
        RECOVER_PROMPT,
        "No image to recover.",
    );
}

fn enhance_image(ui: &Rc<Ui>) {
    start_effect(
        ui,
        GenerationMode::Enhance,
        ENHANCE_PROMPT,
        "No image to enhance.",
    );
}

fn denoise_image(ui: &Rc<Ui>) {
    start_effect(
        ui,
        GenerationMode::Denoise,
        DENOISE_PROMPT,
        "No image to denoise.",
    );
}

fn photoreal_image(ui: &Rc<Ui>) {
    start_effect(
        ui,
        GenerationMode::Photoreal,
        PHOTOREAL_PROMPT,
        "No image to convert.",
    );
}

fn replace_background_image(ui: &Rc<Ui>) {
    start_effect(
        ui,
        GenerationMode::Background,
        BACKGROUND_PROMPT,
        "No image to change.",
    );
}

fn convert_lighting(ui: &Rc<Ui>, to_night: bool) {
    if ui.upscaling.get() || ui.zooming.get() || ui.classifying.get() {
        return;
    }
    if !can_start_image_action(ui) {
        return;
    }
    let Some(source) = ui.state.borrow().display.clone() else {
        ui.toast.add_toast(adw::Toast::new("No image to convert."));
        return;
    };
    start_lighting_conversion(ui, source, to_night);
}

fn start_lighting_conversion(ui: &Rc<Ui>, source: PathBuf, to_night: bool) {
    let (mode, prompt) = if to_night {
        (GenerationMode::Night, NIGHT_PROMPT)
    } else {
        (GenerationMode::Day, DAY_PROMPT)
    };

    // Keep the source shape, but use the MP target from the sidebar.
    ui.state.borrow_mut().references = vec![source.clone()];
    update_image_view(ui);
    fit_reference_size(ui);
    ui.prompt.buffer().set_text(prompt);
    ui.seed_entry.set_text("");
    generate(
        ui,
        mode,
        prompt.to_string(),
        "none",
        false,
        Some(source.to_string_lossy().into_owned()),
    );
}

fn auto_lighting(ui: &Rc<Ui>) {
    if ui.generating.get() || ui.upscaling.get() || ui.zooming.get() || ui.classifying.get() {
        return;
    }
    if !can_start_image_action(ui) {
        return;
    }
    let Some(source) = ui.state.borrow().display.clone() else {
        ui.toast.add_toast(adw::Toast::new("No image to convert."));
        return;
    };

    let helper = &flux_core::vision::VISION_HELPER;
    if !helper.is_installed(&ui.models_dir) {
        ui.toast
            .add_toast(adw::Toast::new("Install the vision helper in Preferences."));
        return;
    }
    let Some(cli) = prompt_ai::find_cli(&ui.config.borrow()) else {
        ui.toast
            .add_toast(adw::Toast::new("Install llama.cpp to use auto day/night."));
        return;
    };

    let model = helper.model_path(&ui.models_dir);
    let mmproj = helper.mmproj_path(&ui.models_dir);

    ui.classifying.set(true);
    set_controls_enabled(ui, false);
    ui.status.set_text("Checking lighting…");
    update_image_actions(ui);

    let (sender, receiver) = async_channel::bounded(1);
    let image = source.clone();
    std::thread::spawn(move || {
        let result = prompt_ai::classify_lighting(&image, &model, &mmproj, &cli);
        let _ = sender.send_blocking(result);
    });

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let result = receiver.recv().await;
        ui.classifying.set(false);
        set_controls_enabled(&ui, true);
        match result {
            Ok(Ok(is_night)) => start_lighting_conversion(&ui, source, !is_night),
            Ok(Err(error)) => {
                tracing::warn!(%error, "lighting classification failed");
                ui.toast.add_toast(adw::Toast::new(&format!(
                    "Could not check lighting: {error}"
                )));
                ui.status.set_text("Ready");
                update_image_actions(&ui);
            }
            Err(_) => {
                ui.status.set_text("Ready");
                update_image_actions(&ui);
            }
        }
    });
}

fn expand_image(ui: &Rc<Ui>) {
    if ui.upscaling.get() || ui.zooming.get() || ui.classifying.get() {
        return;
    }
    if !can_start_image_action(ui) {
        return;
    }
    let Some(source) = ui.state.borrow().display.clone() else {
        ui.toast.add_toast(adw::Toast::new("No image to expand."));
        return;
    };
    let (source_width, source_height) = ui.image_size.get();
    let (target_width, target_height) = target_size(ui);
    let same_ratio = size::ratio_for_size(source_width.max(1), source_height.max(1))
        == size::ratio_for_size(target_width.max(1), target_height.max(1));
    let source_area = source_width as u64 * source_height as u64;
    let target_area = target_width as u64 * target_height as u64;
    if (target_width == source_width && target_height == source_height)
        || (same_ratio && target_area <= source_area)
    {
        ui.toast.add_toast(adw::Toast::new(
            "Choose a larger MP or a different ratio to expand.",
        ));
        return;
    }

    let prompt = current_prompt(ui);
    let prompt = if prompt.trim().is_empty() {
        ui.prompt.buffer().set_text(EXPAND_PROMPT);
        EXPAND_PROMPT.to_string()
    } else {
        prompt
    };
    let style_id = current_style_id(ui);
    ui.state.borrow_mut().references = vec![source.clone()];
    update_image_view(ui);
    generate(
        ui,
        GenerationMode::Expand,
        prompt,
        style_id,
        false,
        Some(source.to_string_lossy().into_owned()),
    );
}

fn back_to_view(ui: &Rc<Ui>) {
    if ui.upscaling.get() || ui.zooming.get() || ui.classifying.get() {
        return;
    }
    pause_live_view(ui);
    cancel_zoom(ui);
    set_crop_mode(ui, false);
    ui.crop_button.set_active(false);
    ui.state.borrow_mut().references.clear();
    update_image_view(ui);
    show_composer(ui, false);
    ui.status.set_text("Viewing image");
}

/// Show an image with a crossfade from the currently visible canvas image.
pub(crate) fn fade_to_image(ui: &Rc<Ui>, path: &Path) -> bool {
    let texture = match gdk::Texture::from_filename(path) {
        Ok(texture) => texture,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "failed to load image");
            return false;
        }
    };

    // Alternate between the two picture children so the stack can crossfade.
    ui.image_size
        .set((texture.width() as u32, texture.height() as u32));
    ui.state.borrow_mut().preview(path.to_path_buf());
    render_reference_strip(ui);
    crossfade_to_texture(ui, &texture);
    sync_history_selection(ui);
    reset_zoom(ui);
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
    if ui.generating.get() || ui.upscaling.get() || ui.zooming.get() || ui.classifying.get() {
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

    ui.status.set_text(&history_view::status_label(&entry));
}

/// Run one upscale on a worker thread and show the result like a generation.
pub(crate) fn start_upscale(ui: &Rc<Ui>, preset: &'static UpscalerPreset, factor: u32) {
    if ui.backend_installing.get() {
        ui.toast
            .add_toast(adw::Toast::new("The backend is still installing."));
        return;
    }
    if ui.generating.get() || ui.upscaling.get() || ui.classifying.get() {
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
                    reference_paths: &[],
                    kind: "upscale",
                    source_path: Some(&source_path),
                });

                match inserted {
                    Ok((id, parent_id)) => {
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
                            reference_paths: Vec::new(),
                            kind: "upscale".into(),
                            source_path: Some(source_path),
                            parent_id,
                        };
                        ui.entries.borrow_mut().insert(0, entry.clone());
                        trim_history(&ui);
                        history_view::rebuild(&ui);
                        history_view::highlight(&ui, Path::new(&entry.image_path));
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
    ui.status.set_text(message);
}

pub(crate) fn trim_history(ui: &Rc<Ui>) {
    let mut entries = ui.entries.borrow_mut();
    while entries.len() > HISTORY_LIMIT {
        entries.pop();
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

pub(crate) fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

pub(crate) fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn upscaler_label(id: &str) -> &str {
    flux_core::upscaler::upscaler(id)
        .map(|preset| preset.label)
        .unwrap_or(id)
}

/// Build a short, filesystem-friendly name from the prompt.
pub(crate) fn slugify(prompt: &str) -> String {
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
    use super::{
        GenerationMode, contain_rect, crop_rect_from_drag, duration_label, fit_scale_for,
        lerp_rect, max_zoom_for, references_after_result, slugify, zoom_percent,
    };
    use std::path::{Path, PathBuf};

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

    #[test]
    fn fits_images_into_the_viewport() {
        // Landscape viewport, square image: height is the limiting side.
        assert_eq!(
            fit_scale_for((800.0, 600.0), (768, 768)),
            Some(600.0 / 768.0)
        );
        // Portrait image in the same viewport is limited by height too.
        assert_eq!(
            fit_scale_for((800.0, 600.0), (600, 900)),
            Some(600.0 / 900.0)
        );
        // Wide image in a portrait viewport is limited by width.
        assert_eq!(fit_scale_for((600.0, 800.0), (1200, 600)), Some(0.5));
        assert_eq!(fit_scale_for((0.0, 600.0), (768, 768)), None);
        assert_eq!(fit_scale_for((800.0, 600.0), (0, 768)), None);
    }

    #[test]
    fn clamps_zoom_to_eight_times_the_image_pixels() {
        // A 0.5 fit can grow 16x before one image pixel covers eight screen pixels.
        assert_eq!(max_zoom_for(0.5, (768, 768)), 16.0);
        // An image already enlarged by fit grows less.
        assert_eq!(max_zoom_for(2.0, (448, 448)), 4.0);
        // Tiny images still allow fit itself.
        assert_eq!(max_zoom_for(20.0, (128, 128)), 1.0);
        // Extremely large images are capped by the maximum edge.
        assert!((max_zoom_for(0.2, (4096, 4096)) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn reports_the_real_pixel_scale() {
        assert_eq!(zoom_percent(1.0, 1.0), "100%");
        assert_eq!(zoom_percent(0.5, 1.0), "50%");
        assert_eq!(zoom_percent(0.5, 2.0), "100%");
        assert_eq!(zoom_percent(0.625, 1.0), "62%");
    }

    #[test]
    fn queued_results_only_replace_untouched_references() {
        let queued = vec![PathBuf::from("/tmp/ref.png")];
        let result = Path::new("/tmp/out.png");
        assert_eq!(
            references_after_result(&queued, &queued, GenerationMode::Edit, result),
            Some(vec![PathBuf::from("/tmp/out.png")])
        );
        assert_eq!(
            references_after_result(&queued, &queued, GenerationMode::Custom, result),
            Some(Vec::new())
        );
        assert_eq!(
            references_after_result(&queued, &queued, GenerationMode::Generate, result),
            Some(Vec::new())
        );
        // The user picked new references for the next job.
        let changed = vec![PathBuf::from("/tmp/other.png")];
        assert_eq!(
            references_after_result(&changed, &queued, GenerationMode::Edit, result),
            None
        );
        assert_eq!(
            references_after_result(&[], &[], GenerationMode::Generate, result),
            Some(Vec::new())
        );
    }

    #[test]
    fn formats_notification_durations() {
        assert_eq!(duration_label(0), "0s");
        assert_eq!(duration_label(12_400), "12s");
        assert_eq!(duration_label(65_000), "1m 05s");
        assert_eq!(duration_label(3_600_000), "60m 00s");
    }
}
