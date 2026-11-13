use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use flux_core::preset::{ModelFileSpec, ModelPreset, PRESETS};
use gtk::glib;

use crate::{Ui, active_preset, refresh_model_list};

const REPO_URL: &str = "https://github.com/tibssy/flux-studio";
const ISSUES_URL: &str = "https://github.com/tibssy/flux-studio/issues";
const LICENSE_URL: &str = "https://github.com/tibssy/flux-studio/blob/main/LICENSE";
const FLUX2_KLEIN_URL: &str = "https://huggingface.co/black-forest-labs/FLUX.2-klein-4B";
const FLUX3_KLEIN_URL: &str = "https://huggingface.co/black-forest-labs/flux-3-image-klein";
const QWEN3_URL: &str = "https://huggingface.co/Qwen/Qwen3-4B";
const QWEN3_VL_URL: &str = "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct";
const SDCPP_URL: &str = "https://github.com/leejet/stable-diffusion.cpp";
const LLAMA_CPP_URL: &str = "https://github.com/ggml-org/llama.cpp";
const FLUX2_GGUF_URL: &str = "https://huggingface.co/unsloth/FLUX.2-klein-4B-GGUF";
const FLUX3_GGUF_URL: &str = "https://huggingface.co/leejet/flux-3-image-klein-GGUF";

struct PresetRow {
    preset: &'static ModelPreset,
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

    let interface_group = adw::PreferencesGroup::builder().title("Interface").build();
    let auto_hide_row = adw::SwitchRow::builder()
        .title("Auto-hide sidebar")
        .subtitle(format!(
            "Hide it {} seconds after focus and the pointer leave",
            crate::AUTO_HIDE_SECONDS
        ))
        .build();
    auto_hide_row.set_active(ui.config.borrow().sidebar_auto_hide);
    {
        let ui = ui.clone();
        auto_hide_row.connect_active_notify(move |row| {
            let active = row.is_active();
            {
                let mut config = ui.config.borrow_mut();
                if config.sidebar_auto_hide != active {
                    config.sidebar_auto_hide = active;
                    let _ = config.save();
                }
            }
            crate::refresh_auto_hide(&ui);
        });
    }
    interface_group.add(&auto_hide_row);
    general.add(&interface_group);
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
            .icon_name("document-save-symbolic")
            .tooltip_text("Download model files")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        button.set_sensitive(!preset.is_installed(&ui.models_dir));
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
                entry.button.set_visible(!installed);
                entry.button.set_sensitive(!installed);

                let deletable = deletable_files(entry.preset, &models_dir);
                entry.delete_button.set_visible(!deletable.is_empty());
                entry.delete_button.set_sensitive(!deletable.is_empty());
            }
        })
    };
    refresh();

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
    about.add(&about_group);

    let project_group = adw::PreferencesGroup::builder().title("Project").build();
    project_group.add(&link_row(
        "Website",
        "github.com/tibssy/flux-studio",
        REPO_URL,
    ));
    project_group.add(&link_row(
        "Report an Issue",
        "github.com/tibssy/flux-studio/issues",
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
        "Qwen3-VL-4B-Instruct text encoder",
        "Alibaba Qwen · Apache-2.0",
        QWEN3_VL_URL,
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
        "FLUX.3 GGUF quantization",
        "leejet · follows the FLUX.3 license",
        FLUX3_GGUF_URL,
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
            "Model weights are downloaded separately and remain under their authors' licenses. Flux Studio is not affiliated with Black Forest Labs.",
        )
        .build();
    legal_group.add(&link_row("Flux Studio", "GPL-3.0-or-later", LICENSE_URL));
    legal_group.add(&link_row(
        "FLUX.2 klein 4B weights",
        "Apache-2.0",
        FLUX2_KLEIN_URL,
    ));
    legal_group.add(&link_row(
        "FLUX.2 VAE",
        "Apache-2.0",
        "https://huggingface.co/Comfy-Org/Flux-2-Klein-4B",
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
            refresh_model_list(&ui);
        });
    }
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

        let klein = &flux_core::preset::FLUX2_KLEIN_4B;
        let flux3 = &flux_core::preset::FLUX3_KLEIN;

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
