use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use imagiflume_core::preset::{ModelFileSpec, ModelPreset, PRESETS};
use imagiflume_core::upscaler::{UPSCALERS, UpscalerPreset};
use imagiflume_core::vision::{VISION_HELPER, VisionHelper};

use crate::{Ui, active_preset, refresh_model_list};

const REPO_URL: &str = "https://github.com/tibssy/imagiflume";
const ISSUES_URL: &str = "https://github.com/tibssy/imagiflume/issues";
const LICENSE_URL: &str = "https://github.com/tibssy/imagiflume/blob/main/LICENSE";
const FLUX2_KLEIN_URL: &str = "https://huggingface.co/black-forest-labs/FLUX.2-klein-4B";
const FLUX2_KLEIN_9B_URL: &str = "https://huggingface.co/black-forest-labs/FLUX.2-klein-9B";
const FLUX2_KLEIN_9B_LICENSE_URL: &str =
    "https://huggingface.co/black-forest-labs/FLUX.2-klein-9B/blob/main/LICENSE.txt";
const FLUX3_KLEIN_URL: &str = "https://huggingface.co/black-forest-labs/flux-3-image-klein";
const QWEN3_URL: &str = "https://huggingface.co/Qwen/Qwen3-4B";
const QWEN3_8B_URL: &str = "https://huggingface.co/Qwen/Qwen3-8B";
const QWEN3_VL_URL: &str = "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct";
const LFM25_VL_URL: &str = "https://huggingface.co/LiquidAI/LFM2.5-VL-450M-GGUF";
const LFM25_VL_LICENSE_URL: &str =
    "https://huggingface.co/LiquidAI/LFM2.5-VL-450M-GGUF/blob/main/LICENSE";
const SDCPP_URL: &str = "https://github.com/leejet/stable-diffusion.cpp";
const LLAMA_CPP_URL: &str = "https://github.com/ggml-org/llama.cpp";
const FLUX2_GGUF_URL: &str = "https://huggingface.co/unsloth/FLUX.2-klein-4B-GGUF";
const FLUX2_9B_GGUF_URL: &str = "https://huggingface.co/unsloth/FLUX.2-klein-9B-GGUF";
const FLUX3_GGUF_URL: &str = "https://huggingface.co/leejet/flux-3-image-klein-GGUF";
const FIBO_URL: &str = "https://huggingface.co/briaai/Fibo-2.0-edit-turbo";
const FIBO_LICENSE_URL: &str = "https://creativecommons.org/licenses/by-nc/4.0/";
const FIBO_GGUF_URL: &str = "https://huggingface.co/molbal/fibo-2.0-GGUF";
const QWEN_IMAGE_TURBO_URL: &str = "https://huggingface.co/Qwen/Qwen-Image-2.1-Turbo";
const QWEN_IMAGE_LICENSE_URL: &str = "https://huggingface.co/Qwen/Qwen-Image-2.1/blob/main/LICENSE";
const QWEN_IMAGE_GGUF_URL: &str = "https://huggingface.co/unsloth/Qwen-Image-2.1-Turbo-GGUF";
const QWEN3_VL_8B_URL: &str = "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct";
const FLUX1_SCHNELL_URL: &str = "https://huggingface.co/black-forest-labs/FLUX.1-schnell";
const SCHNELL_GGUF_URL: &str = "https://huggingface.co/unsloth/FLUX.1-schnell-GGUF";
const T5_XXL_URL: &str = "https://huggingface.co/city96/t5-v1_1-xxl-encoder-gguf";
const CLIP_L_URL: &str = "https://huggingface.co/comfyanonymous/flux_text_encoders";
const FLUX_VAE_URL: &str = "https://huggingface.co/meituan-longcat/LongCat-Image-Edit-Turbo";
const TAE_URL: &str = "https://huggingface.co/madebyollin/taef2";
const REALESRGAN_URL: &str = "https://github.com/xinntao/Real-ESRGAN";
const PHIPS_URL: &str = "https://huggingface.co/Phips";

struct PresetRow {
    preset: &'static ModelPreset,
    row: adw::ActionRow,
    button: gtk::Button,
    delete_button: gtk::Button,
}

struct UpscalerRow {
    preset: &'static UpscalerPreset,
    row: adw::ActionRow,
    button: gtk::Button,
    delete_button: gtk::Button,
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
    let license = preset
        .license
        .map(|license| format!(" · {}", license.name))
        .unwrap_or_default();
    if !preset.is_installed(models_dir) {
        format!(
            "Missing {} file(s) · {} · {} steps{license}",
            missing.len(),
            format_bytes(preset.total_bytes()),
            preset.steps
        )
    } else if !missing.is_empty() {
        // Only optional files (preview weights) are missing.
        let bytes: u64 = missing.iter().map(|file| file.approx_bytes).sum();
        format!(
            "Installed · {} steps · preview weights missing ({}){license}",
            preset.steps,
            format_bytes(bytes)
        )
    } else {
        format!(
            "Installed · {} steps · {}{license}",
            preset.steps,
            format_bytes(preset.total_bytes())
        )
    }
}

fn vision_helper_present(helper: &VisionHelper, models_dir: &Path) -> bool {
    helper.files().into_iter().any(|file| {
        let path = models_dir.join(file.rel_path);
        path.exists() || part_path(&path).exists()
    })
}

fn vision_helper_status(helper: &VisionHelper, models_dir: &Path) -> String {
    if helper.is_installed(models_dir) {
        format!(
            "Installed · {} · {}",
            format_bytes(helper.approx_bytes()),
            helper.license
        )
    } else {
        format!(
            "{} · {} · {}",
            format_bytes(helper.approx_bytes()),
            helper.license,
            helper.description
        )
    }
}

fn upscaler_status(preset: &UpscalerPreset, models_dir: &Path) -> String {
    if preset.is_installed(models_dir) {
        format!(
            "Installed · {} · {}",
            format_bytes(preset.approx_bytes()),
            preset.license
        )
    } else {
        format!(
            "{} · {} · {}",
            format_bytes(preset.approx_bytes()),
            preset.license,
            preset.description
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

    let copy_row = adw::SwitchRow::builder()
        .title("Copy results to clipboard")
        .subtitle("Makes finished generations, upscales and adjusted copies available for pasting")
        .active(ui.config.borrow().copy_result_to_clipboard)
        .build();
    {
        let ui = ui.clone();
        copy_row.connect_active_notify(move |row| {
            let active = row.is_active();
            let mut config = ui.config.borrow_mut();
            if config.copy_result_to_clipboard == active {
                return;
            }
            config.copy_result_to_clipboard = active;
            let _ = config.save();
        });
    }
    output_group.add(&copy_row);

    let session_row = adw::SwitchRow::builder()
        .title("Restore the last image on launch")
        .subtitle("Reopen the last canvas image and the draft prompt")
        .active(ui.config.borrow().session_restore)
        .build();
    {
        let ui = ui.clone();
        session_row.connect_active_notify(move |row| {
            let active = row.is_active();
            let mut config = ui.config.borrow_mut();
            if config.session_restore == active {
                return;
            }
            config.session_restore = active;
            let _ = config.save();
        });
    }
    output_group.add(&session_row);
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
        let updating = Rc::new(Cell::new(false));
        offload_row.connect_active_notify(move |row| {
            if updating.get() {
                return;
            }
            let active = row.is_active();
            if ui.generating.get() || ui.upscaling.get() || ui.zooming.get() || ui.classifying.get()
            {
                updating.set(true);
                row.set_active(ui.config.borrow().offload);
                updating.set(false);
                ui.toast
                    .add_toast(adw::Toast::new("Wait for the current image to finish."));
                return;
            }
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
    performance_group.add(&offload_row);

    let previews_row = adw::SwitchRow::builder()
        .title("Live previews")
        .subtitle("Show each sampling step on the canvas (needs ~11 MB preview weights)")
        .build();
    previews_row.set_active(ui.config.borrow().show_previews);
    {
        let ui = ui.clone();
        previews_row.connect_active_notify(move |row| {
            let active = row.is_active();
            {
                let mut config = ui.config.borrow_mut();
                if config.show_previews == active {
                    return;
                }
                config.show_previews = active;
                let _ = config.save();
            }
            if active {
                let preset = crate::active_preset(&ui);
                if preset.preview_weights(&ui.models_dir).is_none() {
                    ui.toast.add_toast(adw::Toast::new(
                        "Preview weights are not installed; previews will be blocky. Download them under Models.",
                    ));
                }
            }
        });
    }
    performance_group.add(&previews_row);
    general.add(&performance_group);

    let notifications_group = adw::PreferencesGroup::builder()
        .title("Notifications")
        .build();
    let notify_row = adw::SwitchRow::builder()
        .title("Desktop notifications")
        .subtitle("Show a notification when a job finishes while the window is in the background")
        .active(ui.config.borrow().notify_on_finish)
        .build();
    {
        let ui = ui.clone();
        notify_row.connect_active_notify(move |row| {
            let active = row.is_active();
            let mut config = ui.config.borrow_mut();
            if config.notify_on_finish == active {
                return;
            }
            config.notify_on_finish = active;
            let _ = config.save();
        });
    }
    notifications_group.add(&notify_row);
    general.add(&notifications_group);

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

    for preset in PRESETS {
        let row = adw::ActionRow::builder()
            .title(preset.label)
            .subtitle(preset_status(preset, &ui.models_dir))
            .build();
        let button = gtk::Button::builder()
            .icon_name("document-save-symbolic")
            .tooltip_text("Download model files")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        button.set_sensitive(!preset.missing(&ui.models_dir).is_empty());
        let delete_button = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete this model's files from disk")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&delete_button);
        row.add_suffix(&button);
        presets_group.add(&row);
        rows.borrow_mut().push(PresetRow {
            preset,
            row,
            button,
            delete_button,
        });
    }
    models_page.add(&presets_group);

    let upscalers_group = adw::PreferencesGroup::builder()
        .title("Upscaler models")
        .description("Optional weights for upscaling images. Download only the ones you need.")
        .build();
    let upscaler_rows: Rc<RefCell<Vec<UpscalerRow>>> = Rc::new(RefCell::new(Vec::new()));
    for preset in UPSCALERS {
        let row = adw::ActionRow::builder()
            .title(preset.label)
            .subtitle(upscaler_status(preset, &ui.models_dir))
            .build();
        let button = gtk::Button::builder()
            .icon_name("document-save-symbolic")
            .tooltip_text("Download this upscaler")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        button.set_sensitive(!preset.is_installed(&ui.models_dir));
        let delete_button = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete this upscaler from disk")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        button.set_visible(!preset.is_installed(&ui.models_dir));
        delete_button.set_visible(preset.is_installed(&ui.models_dir));
        row.add_suffix(&delete_button);
        row.add_suffix(&button);
        upscalers_group.add(&row);
        upscaler_rows.borrow_mut().push(UpscalerRow {
            preset,
            row,
            button,
            delete_button,
        });
    }
    models_page.add(&upscalers_group);

    let vision_group = adw::PreferencesGroup::builder()
        .title("Vision helper")
        .description("Optional small vision model for automatic day/night detection.")
        .build();
    let vision_row = adw::ActionRow::builder()
        .title(VISION_HELPER.label)
        .subtitle(vision_helper_status(&VISION_HELPER, &ui.models_dir))
        .build();
    let vision_button = gtk::Button::builder()
        .icon_name("document-save-symbolic")
        .tooltip_text("Download the vision helper")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let vision_delete = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Delete the vision helper from disk")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let vision_installed = VISION_HELPER.is_installed(&ui.models_dir);
    let vision_present = vision_helper_present(&VISION_HELPER, &ui.models_dir);
    vision_button.set_visible(!vision_installed);
    vision_button.set_sensitive(!vision_installed);
    vision_delete.set_visible(vision_present);
    vision_delete.set_sensitive(vision_present);
    vision_row.add_suffix(&vision_delete);
    vision_row.add_suffix(&vision_button);
    vision_group.add(&vision_row);
    models_page.add(&vision_group);

    let refresh: Rc<dyn Fn()> = {
        let rows = rows.clone();
        let models_dir = ui.models_dir.clone();
        Rc::new(move || {
            for entry in rows.borrow().iter() {
                entry
                    .row
                    .set_subtitle(&preset_status(entry.preset, &models_dir));
                let complete = entry.preset.missing(&models_dir).is_empty();
                entry.button.set_visible(!complete);
                entry.button.set_sensitive(!complete);

                let deletable = deletable_files(entry.preset, &models_dir);
                entry.delete_button.set_visible(!deletable.is_empty());
                entry.delete_button.set_sensitive(!deletable.is_empty());
            }
        })
    };
    refresh();

    let refresh_upscalers: Rc<dyn Fn()> = {
        let rows = upscaler_rows.clone();
        let models_dir = ui.models_dir.clone();
        Rc::new(move || {
            for entry in rows.borrow().iter() {
                let installed = entry.preset.is_installed(&models_dir);
                entry
                    .row
                    .set_subtitle(&upscaler_status(entry.preset, &models_dir));
                entry.button.set_visible(!installed);
                entry.button.set_sensitive(!installed);
                entry.delete_button.set_visible(installed);
                entry.delete_button.set_sensitive(installed);
            }
        })
    };
    refresh_upscalers();

    let refresh_vision: Rc<dyn Fn()> = {
        let ui = ui.clone();
        let row = vision_row.clone();
        let button = vision_button.clone();
        let delete = vision_delete.clone();
        Rc::new(move || {
            let installed = VISION_HELPER.is_installed(&ui.models_dir);
            let present = vision_helper_present(&VISION_HELPER, &ui.models_dir);
            row.set_subtitle(&vision_helper_status(&VISION_HELPER, &ui.models_dir));
            button.set_visible(!installed);
            button.set_sensitive(!installed);
            delete.set_visible(present);
            delete.set_sensitive(present);
            crate::update_image_actions(&ui);
        })
    };
    refresh_vision();

    {
        let ui = ui.clone();
        let refresh = refresh_vision.clone();
        let dialog_ref = dialog.downgrade();
        vision_delete.connect_clicked(move |_| {
            if let Some(dialog) = dialog_ref.upgrade() {
                confirm_delete_vision_helper(&ui, &dialog, refresh.clone());
            }
        });
    }
    {
        let ui = ui.clone();
        let refresh = refresh_vision.clone();
        let buttons = vec![vision_button.clone(), vision_delete.clone()];
        let row = vision_row.clone();
        vision_button.connect_clicked(move |_| {
            start_vision_download(&ui, &row, &buttons, refresh.clone());
        });
    }

    let upscaler_buttons: Vec<gtk::Button> = upscaler_rows
        .borrow()
        .iter()
        .flat_map(|entry| [entry.button.clone(), entry.delete_button.clone()])
        .collect();
    for entry in upscaler_rows.borrow().iter() {
        let ui = ui.clone();
        let preset = entry.preset;
        let refresh = refresh_upscalers.clone();
        let dialog_ref = dialog.downgrade();
        entry.delete_button.connect_clicked(move |_| {
            if let Some(dialog) = dialog_ref.upgrade() {
                confirm_delete_upscaler(&ui, preset, &dialog, refresh.clone());
            }
        });
    }
    for entry in upscaler_rows.borrow().iter() {
        let ui = ui.clone();
        let preset = entry.preset;
        let buttons = upscaler_buttons.clone();
        let refresh = refresh_upscalers.clone();
        let row = entry.row.clone();
        entry.button.connect_clicked(move |_| {
            start_upscaler_download(&ui, preset, &row, &buttons, refresh.clone());
        });
    }

    let buttons: Vec<gtk::Button> = rows
        .borrow()
        .iter()
        .flat_map(|entry| [entry.button.clone(), entry.delete_button.clone()])
        .collect();
    for entry in rows.borrow().iter() {
        let ui = ui.clone();
        let preset = entry.preset;
        let refresh = refresh.clone();
        let dialog_ref = dialog.downgrade();
        entry.delete_button.connect_clicked(move |_| {
            if let Some(dialog) = dialog_ref.upgrade() {
                confirm_delete_model(&ui, preset, &dialog, refresh.clone());
            }
        });
    }
    for entry in rows.borrow().iter() {
        let ui = ui.clone();
        let preset = entry.preset;
        let buttons = buttons.clone();
        let refresh = refresh.clone();
        let row = entry.row.clone();
        let dialog_ref = dialog.downgrade();
        entry.button.connect_clicked(move |_| {
            let start: Rc<dyn Fn()> = {
                let ui = ui.clone();
                let buttons = buttons.clone();
                let refresh = refresh.clone();
                let row = row.clone();
                Rc::new(move || {
                    start_download(&ui, preset, &row, &buttons, refresh.clone());
                })
            };

            let needs_acceptance = preset
                .license
                .is_some_and(|license| license.requires_acceptance)
                && !ui
                    .config
                    .borrow()
                    .accepted_licenses
                    .iter()
                    .any(|id| id == preset.id);
            if needs_acceptance {
                if let (Some(dialog), Some(license)) = (dialog_ref.upgrade(), preset.license) {
                    confirm_license(&ui, preset, license, &dialog, start);
                }
                return;
            }
            start();
        });
    }

    dialog.add(&models_page);
    dialog.add(&crate::backends::build_page(ui));

    let about = adw::PreferencesPage::builder()
        .title("About")
        .icon_name("help-about-symbolic")
        .build();
    let about_group = adw::PreferencesGroup::new();
    about_group.add(
        &adw::ActionRow::builder()
            .title(imagiflume_core::APP_NAME)
            .subtitle(concat!("Version ", env!("CARGO_PKG_VERSION")))
            .build(),
    );
    let (backend_label, server_path) = {
        let config = ui.config.borrow();
        (
            config.backend.clone().unwrap_or_else(|| "auto".into()),
            config
                .sd_server
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "auto-detected".into()),
        )
    };
    about_group.add(
        &adw::ActionRow::builder()
            .title("Backend")
            .subtitle(format!("stable-diffusion.cpp · {backend_label}"))
            .build(),
    );
    about_group.add(
        &adw::ActionRow::builder()
            .title("Server")
            .subtitle(server_path)
            .build(),
    );
    let prompt_helper = crate::prompt_ai::find_cli(&ui.config.borrow())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "not found".into());
    about_group.add(
        &adw::ActionRow::builder()
            .title("Prompt helper")
            .subtitle(format!("llama.cpp · {prompt_helper}"))
            .build(),
    );
    about.add(&about_group);

    let project_group = adw::PreferencesGroup::builder().title("Project").build();
    project_group.add(&link_row(
        "Website",
        "github.com/tibssy/imagiflume",
        REPO_URL,
    ));
    project_group.add(&link_row(
        "Report an Issue",
        "github.com/tibssy/imagiflume/issues",
        ISSUES_URL,
    ));
    about.add(&project_group);

    let credits_page = adw::PreferencesPage::builder()
        .title("Credits")
        .icon_name("avatar-default-symbolic")
        .build();
    let credits_group = adw::PreferencesGroup::builder()
        .title("Models and tools")
        .description("Credits for the models and projects that make this app possible.")
        .build();
    credits_group.add(&link_row(
        "FLUX.2 klein 4B",
        "Black Forest Labs · Apache-2.0",
        FLUX2_KLEIN_URL,
    ));
    credits_group.add(&link_row(
        "FLUX.2 klein 9B",
        "Black Forest Labs · FLUX Non-Commercial License",
        FLUX2_KLEIN_9B_URL,
    ));
    credits_group.add(&link_row(
        "FLUX.3 Image klein",
        "Black Forest Labs",
        FLUX3_KLEIN_URL,
    ));
    credits_group.add(&link_row(
        "Qwen3-4B text encoder",
        "Alibaba Qwen · Apache-2.0",
        QWEN3_URL,
    ));
    credits_group.add(&link_row(
        "Qwen3-8B text encoder",
        "Alibaba Qwen · Apache-2.0",
        QWEN3_8B_URL,
    ));
    credits_group.add(&link_row(
        "Qwen3-VL-4B-Instruct text encoder",
        "Alibaba Qwen · Apache-2.0",
        QWEN3_VL_URL,
    ));
    credits_group.add(&link_row(
        "FLUX.1 schnell",
        "Black Forest Labs · Apache-2.0",
        FLUX1_SCHNELL_URL,
    ));
    credits_group.add(&link_row(
        "Fibo 2.0 Edit Turbo",
        "BRIA AI · CC BY-NC 4.0 (non-commercial)",
        FIBO_URL,
    ));
    credits_group.add(&link_row(
        "Fibo 2.0 GGUF quantization",
        "molbal · follows the Fibo license",
        FIBO_GGUF_URL,
    ));
    credits_group.add(&link_row(
        "Qwen Image 2.1 Turbo",
        "Alibaba Qwen · Qwen Research License (non-commercial)",
        QWEN_IMAGE_TURBO_URL,
    ));
    credits_group.add(&link_row(
        "Qwen3-VL-8B-Instruct text encoder",
        "Alibaba Qwen · Apache-2.0",
        QWEN3_VL_8B_URL,
    ));
    credits_group.add(&link_row(
        "Qwen Image 2.1 GGUF quantization",
        "unsloth · follows the Qwen Research License",
        QWEN_IMAGE_GGUF_URL,
    ));
    credits_group.add(&link_row(
        "FLUX.1 schnell GGUF quantization",
        "unsloth · Apache-2.0",
        SCHNELL_GGUF_URL,
    ));
    credits_group.add(&link_row(
        "T5-XXL text encoder",
        "Google · Apache-2.0",
        T5_XXL_URL,
    ));
    credits_group.add(&link_row("CLIP-L text encoder", "OpenAI · MIT", CLIP_L_URL));
    credits_group.add(&link_row(
        "FLUX VAE",
        "Black Forest Labs · Apache-2.0 (repacked by Meituan LongCat)",
        FLUX_VAE_URL,
    ));
    credits_group.add(&link_row(
        "TAEF1/TAEF2 and TAEQI2.1 preview decoders",
        "madebyollin · MIT",
        TAE_URL,
    ));
    credits_group.add(&link_row(
        "LFM2.5-VL-450M vision helper",
        "Liquid AI · LFM Open License v1.0",
        LFM25_VL_URL,
    ));
    credits_group.add(&link_row(
        "stable-diffusion.cpp",
        "leejet and contributors · MIT",
        SDCPP_URL,
    ));
    credits_group.add(&link_row(
        "llama.cpp prompt helper",
        "ggml-org and contributors · MIT",
        LLAMA_CPP_URL,
    ));
    credits_group.add(&link_row(
        "FLUX.2 GGUF quantization",
        "unsloth · Apache-2.0",
        FLUX2_GGUF_URL,
    ));
    credits_group.add(&link_row(
        "FLUX.2 klein 9B GGUF quantization",
        "unsloth · follows the FLUX Non-Commercial License",
        FLUX2_9B_GGUF_URL,
    ));
    credits_group.add(&link_row(
        "FLUX.3 GGUF quantization",
        "leejet · follows the FLUX.3 license",
        FLUX3_GGUF_URL,
    ));
    credits_group.add(&link_row(
        "Real-ESRGAN upscalers",
        "Xintao Wang and contributors · BSD-3-Clause",
        REALESRGAN_URL,
    ));
    credits_group.add(&link_row(
        "Nomos and MoSR upscalers",
        "Philip Hofmann (Phips) · CC-BY-4.0",
        PHIPS_URL,
    ));
    credits_page.add(&credits_group);
    let credits_navigation = adw::NavigationPage::new(&page_toolbar(&credits_page), "Credits");

    let legal_page = adw::PreferencesPage::builder()
        .title("Legal")
        .icon_name("help-about-symbolic")
        .build();
    let legal_group = adw::PreferencesGroup::builder()
        .title("Licenses")
        .description(
            "Model weights are downloaded separately and remain under their authors' licenses. Non-commercial models are marked in Models and require confirmation before download. Imagiflume is not affiliated with Black Forest Labs.",
        )
        .build();
    legal_group.add(&link_row("Imagiflume", "GPL-3.0-or-later", LICENSE_URL));
    legal_group.add(&link_row(
        "FLUX.2 klein 4B weights",
        "Apache-2.0",
        FLUX2_KLEIN_URL,
    ));
    legal_group.add(&link_row(
        "FLUX.2 klein 9B weights",
        "FLUX Non-Commercial License · non-commercial use only",
        FLUX2_KLEIN_9B_LICENSE_URL,
    ));
    legal_group.add(&link_row(
        "FLUX.2 VAE",
        "Apache-2.0",
        "https://huggingface.co/Comfy-Org/Flux-2-Klein-4B",
    ));
    legal_group.add(&link_row(
        "LFM2.5-VL-450M vision helper",
        "LFM Open License v1.0",
        LFM25_VL_LICENSE_URL,
    ));
    legal_group.add(&link_row(
        "FLUX.3 Image klein weights",
        "Black Forest Labs license — see model card",
        FLUX3_KLEIN_URL,
    ));
    legal_group.add(&link_row(
        "Qwen3 and Qwen3-VL weights",
        "Apache-2.0",
        QWEN3_URL,
    ));
    legal_group.add(&link_row(
        "FLUX.1 schnell weights",
        "Apache-2.0",
        FLUX1_SCHNELL_URL,
    ));
    legal_group.add(&link_row(
        "Fibo 2.0 Edit Turbo weights",
        "CC BY-NC 4.0 · non-commercial, © BRIA AI",
        FIBO_LICENSE_URL,
    ));
    legal_group.add(&link_row(
        "Qwen Image 2.1 weights",
        "Qwen Research License · non-commercial use only",
        QWEN_IMAGE_LICENSE_URL,
    ));
    legal_group.add(&link_row(
        "T5-XXL and CLIP-L text encoders",
        "Apache-2.0 / MIT",
        T5_XXL_URL,
    ));
    legal_group.add(&link_row("FLUX VAE", "Apache-2.0", FLUX_VAE_URL));
    legal_group.add(&link_row(
        "TAEF1/TAEF2 and TAEQI2.1 preview decoders",
        "MIT",
        TAE_URL,
    ));
    legal_group.add(&link_row(
        "Real-ESRGAN upscaler weights",
        "BSD-3-Clause",
        REALESRGAN_URL,
    ));
    legal_group.add(&link_row(
        "Nomos and MoSR upscaler weights",
        "CC-BY-4.0 · Philip Hofmann (Phips)",
        PHIPS_URL,
    ));
    legal_group.add(&link_row("stable-diffusion.cpp", "MIT", SDCPP_URL));
    legal_group.add(&link_row("llama.cpp", "MIT", LLAMA_CPP_URL));
    legal_page.add(&legal_group);
    let legal_navigation = adw::NavigationPage::new(&page_toolbar(&legal_page), "Legal");

    let more_group = adw::PreferencesGroup::builder().title("Details").build();
    more_group.add(&navigation_row(
        "Credits",
        "Model and tool authors",
        &dialog,
        &credits_navigation,
    ));
    more_group.add(&navigation_row(
        "Legal",
        "Licenses and attribution",
        &dialog,
        &legal_navigation,
    ));
    about.add(&more_group);
    dialog.add(&about);

    if focus_models {
        dialog.set_visible_page(&models_page);
    }
    dialog.present(Some(&ui.window));
}

fn file_is_shared(rel_path: &str, preset_id: &str, models_dir: &Path) -> bool {
    PRESETS.iter().any(|other| {
        other.id != preset_id
            && other.is_installed(models_dir)
            && other.files.iter().any(|file| file.rel_path == rel_path)
    })
}

fn part_path(path: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.part", path.display()))
}

fn deletable_files(preset: &'static ModelPreset, models_dir: &Path) -> Vec<&'static ModelFileSpec> {
    preset
        .files
        .iter()
        .filter(|file| !file_is_shared(file.rel_path, preset.id, models_dir))
        .filter(|file| {
            let path = models_dir.join(file.rel_path);
            path.exists() || part_path(&path).exists()
        })
        .collect()
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        + std::fs::metadata(part_path(path))
            .map(|metadata| metadata.len())
            .unwrap_or(0)
}

fn delete_preset_files(models_dir: &Path, preset: &'static ModelPreset) -> anyhow::Result<u64> {
    let mut freed = 0;
    let mut errors = Vec::new();

    for file in deletable_files(preset, models_dir) {
        let path = models_dir.join(file.rel_path);
        for candidate in [path.clone(), part_path(&path)] {
            let size = std::fs::metadata(&candidate)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            match std::fs::remove_file(&candidate) {
                Ok(()) => freed += size,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => errors.push(format!("{}: {error}", candidate.display())),
            }
        }
    }

    if let Some(first) = errors.first() {
        anyhow::bail!("{first}");
    }
    Ok(freed)
}

fn confirm_delete_model(
    ui: &Rc<Ui>,
    preset: &'static ModelPreset,
    parent: &adw::PreferencesDialog,
    refresh: Rc<dyn Fn()>,
) {
    if ui.generating.get() {
        ui.toast
            .add_toast(adw::Toast::new("Cannot delete models while generating."));
        return;
    }

    let files = deletable_files(preset, &ui.models_dir);
    if files.is_empty() {
        ui.toast
            .add_toast(adw::Toast::new("No deletable files for this model."));
        return;
    }

    let bytes: u64 = files
        .iter()
        .map(|file| file_size(&ui.models_dir.join(file.rel_path)))
        .sum();
    let shared = preset
        .files
        .iter()
        .filter(|file| file_is_shared(file.rel_path, preset.id, &ui.models_dir))
        .filter(|file| ui.models_dir.join(file.rel_path).exists())
        .count();

    let mut body = format!(
        "This permanently removes {} file(s) ({}) from disk.",
        files.len(),
        format_bytes(bytes)
    );
    if shared > 0 {
        body.push_str(&format!(
            " {shared} shared file(s) used by other presets will be kept."
        ));
    }

    let dialog = adw::AlertDialog::builder()
        .heading(format!("Delete {}?", preset.label))
        .body(body)
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));

    {
        let ui = ui.clone();
        let refresh = refresh.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "delete" {
                return;
            }
            let result = delete_preset_files(&ui.models_dir, preset);
            if active_preset(&ui).id == preset.id {
                ui.backend.unload();
            }
            refresh();
            refresh_model_list(&ui);
            match result {
                Ok(freed) => ui.toast.add_toast(adw::Toast::new(&format!(
                    "Deleted {} file(s) · {}.",
                    files.len(),
                    format_bytes(freed)
                ))),
                Err(error) => ui.toast.add_toast(adw::Toast::new(&format!(
                    "Could not delete all files: {error}"
                ))),
            }
        });
    }

    dialog.present(Some(parent));
}

fn confirm_delete_upscaler(
    ui: &Rc<Ui>,
    preset: &'static UpscalerPreset,
    parent: &adw::PreferencesDialog,
    refresh: Rc<dyn Fn()>,
) {
    if ui.generating.get() || ui.upscaling.get() {
        ui.toast.add_toast(adw::Toast::new(
            "Cannot delete upscalers while an image is running.",
        ));
        return;
    }

    let path = preset.path(&ui.models_dir);
    if !path.exists() {
        refresh();
        return;
    }

    let dialog = adw::AlertDialog::builder()
        .heading(format!("Delete {}?", preset.label))
        .body(format!(
            "This permanently removes {} ({}) from disk.",
            path.display(),
            format_bytes(file_size(&path))
        ))
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));

    {
        let ui = ui.clone();
        let refresh = refresh.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "delete" {
                return;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    ui.toast
                        .add_toast(adw::Toast::new(&format!("{} deleted.", preset.label)));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    ui.toast
                        .add_toast(adw::Toast::new(&format!("Could not delete: {error}")));
                }
            }
            refresh();
        });
    }

    dialog.present(Some(parent));
}

fn page_toolbar(content: &impl IsA<gtk::Widget>) -> adw::ToolbarView {
    // AdwNavigationView inserts the back button and page title into this
    // header bar when the page is pushed.
    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::builder().content(content).build();
    toolbar.add_top_bar(&header);
    toolbar
}

fn navigation_row(
    title: &str,
    subtitle: &str,
    dialog: &adw::PreferencesDialog,
    target: &adw::NavigationPage,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.set_activatable(true);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    // Weak reference: the row is a descendant of the dialog, so a strong
    // reference here would form a cycle and leak the dialog after closing.
    let dialog = dialog.downgrade();
    let target = target.clone();
    row.connect_activated(move |_| {
        if let Some(dialog) = dialog.upgrade() {
            dialog.push_subpage(&target);
        }
    });
    row
}

fn link_row(title: &str, subtitle: &str, uri: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.set_activatable(true);
    row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    let target = uri.to_string();
    row.connect_activated(move |_| open_uri(&target));
    row
}

fn open_uri(uri: &str) {
    let launcher = gtk::UriLauncher::new(uri);
    let target = uri.to_string();
    launcher.launch(
        gtk::Window::NONE,
        gtk::gio::Cancellable::NONE,
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, %target, "failed to open link");
            }
        },
    );
}

fn confirm_license(
    ui: &Rc<Ui>,
    preset: &'static ModelPreset,
    license: imagiflume_core::preset::ModelLicense,
    parent: &adw::PreferencesDialog,
    on_accept: Rc<dyn Fn()>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Download {}?", preset.label))
        .body(format!(
            "{} weights are licensed under {} ({}).\n\n{}\n\nBy continuing you confirm that you accept these terms.",
            preset.label, license.name, license.holder, license.summary
        ))
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("accept", "I accept");
    dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));

    let link = gtk::LinkButton::with_label(license.url, "Read the license");
    link.set_halign(gtk::Align::Start);
    dialog.set_extra_child(Some(&link));

    let ui = ui.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "accept" {
            return;
        }
        {
            let mut config = ui.config.borrow_mut();
            if !config.accepted_licenses.iter().any(|id| id == preset.id) {
                config.accepted_licenses.push(preset.id.to_string());
            }
            if let Err(error) = config.save() {
                tracing::warn!(%error, "failed to save accepted license");
            }
        }
        on_accept();
    });
    dialog.present(Some(parent));
}

fn download_row_subtitle(done: u64, total: u64) -> String {
    let percent = if total > 0 {
        (done as f64 / total as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    format!(
        "Downloading · {percent:.0}% · {} / {}",
        format_bytes(done),
        format_bytes(total)
    )
}

fn start_download(
    ui: &Rc<Ui>,
    preset: &'static ModelPreset,
    row: &adw::ActionRow,
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
    row.set_subtitle("Downloading…");

    let (progress_tx, progress_rx) =
        async_channel::unbounded::<imagiflume_models::DownloadProgress>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);
    let download_dir = models_dir.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(imagiflume_models::download_preset(
                preset,
                &download_dir,
                progress_tx,
            ))?;
            Ok(())
        })();
        let _ = done_tx.send_blocking(result.map_err(|error| error.to_string()));
    });

    {
        let row = row.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = progress_rx.recv().await {
                let before: u64 = sizes
                    .get(..event.file_index.min(sizes.len()))
                    .map(|slice| slice.iter().sum())
                    .unwrap_or(0);
                row.set_subtitle(&download_row_subtitle(
                    before + event.downloaded,
                    total_bytes,
                ));
            }
        });
    }

    {
        let ui = ui.clone();
        let row = row.clone();
        glib::spawn_future_local(async move {
            let result = done_rx.recv().await;
            match result {
                Ok(Ok(())) => {
                    row.set_subtitle(&format!("{} · installed", preset.label));
                    ui.toast.add_toast(adw::Toast::new(&format!(
                        "{} models downloaded.",
                        preset.label
                    )));
                    // Preview weights are loaded at server start-up.
                    if crate::active_preset(&ui).id == preset.id {
                        ui.backend.unload();
                    }
                }
                Ok(Err(error)) => {
                    ui.toast
                        .add_toast(adw::Toast::new(&format!("Download failed: {error}")));
                }
                Err(_) => {}
            }
            refresh();
            refresh_model_list(&ui);
        });
    }
}

fn start_upscaler_download(
    ui: &Rc<Ui>,
    preset: &'static UpscalerPreset,
    row: &adw::ActionRow,
    buttons: &[gtk::Button],
    refresh: Rc<dyn Fn()>,
) {
    if preset.is_installed(&ui.models_dir) {
        return;
    }
    for button in buttons {
        button.set_sensitive(false);
    }
    row.set_subtitle("Downloading…");

    let (progress_tx, progress_rx) =
        async_channel::unbounded::<imagiflume_models::DownloadProgress>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);
    let models_dir = ui.models_dir.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(imagiflume_models::download_upscaler(
                preset,
                &models_dir,
                progress_tx,
            ))?;
            Ok(())
        })();
        let _ = done_tx.send_blocking(result.map_err(|error| error.to_string()));
    });

    let row = row.clone();
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = progress_rx.recv().await {
            row.set_subtitle(&download_row_subtitle(event.downloaded, event.total));
        }
        let result = done_rx.recv().await;
        match result {
            Ok(Ok(())) => {
                ui.toast
                    .add_toast(adw::Toast::new(&format!("{} downloaded.", preset.label)));
            }
            Ok(Err(error)) => {
                ui.toast
                    .add_toast(adw::Toast::new(&format!("Download failed: {error}")));
            }
            Err(_) => {}
        }
        refresh();
    });
}

fn start_vision_download(
    ui: &Rc<Ui>,
    row: &adw::ActionRow,
    buttons: &[gtk::Button],
    refresh: Rc<dyn Fn()>,
) {
    let missing: Vec<_> = VISION_HELPER
        .files()
        .into_iter()
        .filter(|file| !ui.models_dir.join(file.rel_path).exists())
        .collect();
    if missing.is_empty() {
        return;
    }
    let sizes: Vec<u64> = missing.iter().map(|file| file.approx_bytes).collect();
    let total_bytes: u64 = sizes.iter().sum();
    for button in buttons {
        button.set_sensitive(false);
    }
    row.set_subtitle("Downloading…");

    let (progress_tx, progress_rx) =
        async_channel::unbounded::<imagiflume_models::DownloadProgress>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);
    let models_dir = ui.models_dir.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(imagiflume_models::download_vision_helper(
                &VISION_HELPER,
                &models_dir,
                progress_tx,
            ))?;
            Ok(())
        })();
        let _ = done_tx.send_blocking(result.map_err(|error| error.to_string()));
    });

    let row = row.clone();
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = progress_rx.recv().await {
            let before: u64 = sizes
                .get(..event.file_index.min(sizes.len()))
                .map(|slice| slice.iter().sum())
                .unwrap_or(0);
            row.set_subtitle(&download_row_subtitle(
                before + event.downloaded,
                total_bytes,
            ));
        }
        let result = done_rx.recv().await;
        match result {
            Ok(Ok(())) => {
                ui.toast.add_toast(adw::Toast::new(&format!(
                    "{} downloaded.",
                    VISION_HELPER.label
                )));
            }
            Ok(Err(error)) => {
                ui.toast
                    .add_toast(adw::Toast::new(&format!("Download failed: {error}")));
            }
            Err(_) => {}
        }
        refresh();
    });
}

fn confirm_delete_vision_helper(
    ui: &Rc<Ui>,
    parent: &adw::PreferencesDialog,
    refresh: Rc<dyn Fn()>,
) {
    if ui.generating.get() || ui.upscaling.get() || ui.classifying.get() {
        ui.toast.add_toast(adw::Toast::new(
            "Cannot delete the vision helper while an image is running.",
        ));
        return;
    }

    let files: Vec<_> = VISION_HELPER
        .files()
        .into_iter()
        .filter(|file| {
            let path = ui.models_dir.join(file.rel_path);
            path.exists() || part_path(&path).exists()
        })
        .collect();
    if files.is_empty() {
        refresh();
        return;
    }
    let bytes: u64 = files
        .iter()
        .map(|file| file_size(&ui.models_dir.join(file.rel_path)))
        .sum();

    let dialog = adw::AlertDialog::builder()
        .heading(format!("Delete {}?", VISION_HELPER.label))
        .body(format!(
            "This permanently removes {} file(s) ({}) from disk.",
            files.len(),
            format_bytes(bytes)
        ))
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));

    {
        let ui = ui.clone();
        let refresh = refresh.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "delete" {
                return;
            }
            let mut freed = 0;
            for file in &files {
                let path = ui.models_dir.join(file.rel_path);
                for candidate in [path.clone(), part_path(&path)] {
                    let size = std::fs::metadata(&candidate)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0);
                    match std::fs::remove_file(&candidate) {
                        Ok(()) => freed += size,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            ui.toast.add_toast(adw::Toast::new(&format!(
                                "Could not delete {}: {error}",
                                candidate.display()
                            )));
                        }
                    }
                }
            }
            refresh();
            ui.toast.add_toast(adw::Toast::new(&format!(
                "Deleted {} file(s) · {}.",
                files.len(),
                format_bytes(freed)
            )));
        });
    }

    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn install(preset: &ModelPreset, dir: &Path) {
        for file in preset.files {
            let path = dir.join(file.rel_path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
    }

    #[test]
    fn shared_files_are_kept_only_for_installed_presets() {
        let dir = std::env::temp_dir().join(format!("flux-delete-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let klein = &imagiflume_core::preset::FLUX2_KLEIN_4B;
        let flux3 = &imagiflume_core::preset::FLUX3_KLEIN;

        install(klein, &dir);
        // FLUX.3 is not installed, so only FLUX.2's own files are non-shared;
        // the VAE becomes deletable along with them.
        assert_eq!(deletable_files(klein, &dir).len(), klein.files.len());

        install(flux3, &dir);
        // Both presets are installed: the VAE is shared and must be kept.
        assert!(file_is_shared("vae/flux2-vae.safetensors", klein.id, &dir));
        let deletable = deletable_files(klein, &dir);
        assert_eq!(deletable.len(), 2);
        assert!(
            deletable
                .iter()
                .all(|file| file.rel_path != "vae/flux2-vae.safetensors")
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
