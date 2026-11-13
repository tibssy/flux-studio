use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use flux_core::upscaler::{self, UPSCALERS};
use gtk::glib;

use crate::Ui;

/// Show the upscale dialog for the image currently on screen.
pub(crate) fn show(ui: &Rc<Ui>) {
    if ui.state.borrow().display.is_none() {
        return;
    }
    let (width, height) = ui.image_size.get();
    if width == 0 || height == 0 {
        return;
    }

    let (initial_model, initial_factor) = {
        let config = ui.config.borrow();
        let model = UPSCALERS
            .iter()
            .position(|preset| preset.id == config.upscaler_id)
            .unwrap_or(0);
        let factor = if config.upscale_factor == 2 { 2 } else { 4 };
        (model, factor)
    };

    let dialog = adw::Dialog::builder()
        .title("Upscale image")
        .content_width(480)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Upscale image",
        &format!("{width} × {height}"),
    )));
    let cancel = gtk::Button::with_label("Cancel");
    let upscale = gtk::Button::builder()
        .label("Upscale")
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&upscale);
    header.pack_end(&cancel);
    toolbar.add_top_bar(&header);

    let page = adw::PreferencesPage::new();

    let output_group = adw::PreferencesGroup::builder().title("Output").build();
    let size_row = adw::ActionRow::builder().title("Output size").build();
    let warning = gtk::Label::builder()
        .label("Too large: the 4× pass would exceed 8192 px.")
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .css_classes(["caption", "error"])
        .build();
    let scale_row = adw::ActionRow::builder()
        .title("Scale")
        .subtitle("2× is downscaled from a native 4× pass")
        .build();
    let scale_dropdown = gtk::DropDown::from_strings(&["2×", "4×"]);
    scale_dropdown.set_selected(if initial_factor == 2 { 0 } else { 1 });
    scale_row.add_suffix(&scale_dropdown);
    scale_row.set_activatable_widget(Some(&scale_dropdown));
    output_group.add(&size_row);
    output_group.add(&scale_row);
    output_group.add(&warning);
    page.add(&output_group);

    let model_group = adw::PreferencesGroup::builder().title("Model").build();
    let labels: Vec<&str> = UPSCALERS.iter().map(|preset| preset.label).collect();
    let model_dropdown = gtk::DropDown::from_strings(&labels);
    model_dropdown.set_selected(initial_model as u32);
    let model_row = adw::ActionRow::builder().title("Upscaler").build();
    model_row.add_suffix(&model_dropdown);
    model_row.set_activatable_widget(Some(&model_dropdown));
    let details_row = adw::ActionRow::builder().title("Details").build();
    let license_row = adw::ActionRow::builder().title("License").build();
    let download_row = adw::ActionRow::builder()
        .title("Model not installed")
        .build();
    let download_button = gtk::Button::builder()
        .label("Download")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    download_row.add_suffix(&download_button);
    let progress = gtk::ProgressBar::builder()
        .show_text(true)
        .visible(false)
        .build();
    let status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption", "dim-label"])
        .build();
    model_group.add(&model_row);
    model_group.add(&details_row);
    model_group.add(&license_row);
    model_group.add(&download_row);
    model_group.add(&progress);
    model_group.add(&status);
    page.add(&model_group);

    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let selected_model = Rc::new(RefCell::new(initial_model));
    let selected_factor = Rc::new(RefCell::new(initial_factor));

    let refresh: Rc<dyn Fn()> = {
        let ui = ui.clone();
        let size_row = size_row.clone();
        let warning = warning.clone();
        let details_row = details_row.clone();
        let license_row = license_row.clone();
        let download_row = download_row.clone();
        let download_button = download_button.clone();
        let upscale = upscale.clone();
        let selected_model = selected_model.clone();
        let selected_factor = selected_factor.clone();
        Rc::new(move || {
            let index = *selected_model.borrow();
            let preset = &UPSCALERS[index.min(UPSCALERS.len() - 1)];
            let factor = *selected_factor.borrow();
            let size = upscaler::output_size(width, height, factor);
            match size {
                Some((output_width, output_height)) => size_row.set_subtitle(&format!(
                    "{width} × {height} → {output_width} × {output_height}"
                )),
                None => size_row.set_subtitle(&format!("{width} × {height} → exceeds 8192 px")),
            }
            warning.set_visible(size.is_none());
            details_row.set_subtitle(&format!(
                "{} MB · ~{} · {}",
                preset.approx_bytes() / 1_048_576,
                format_duration(preset.estimated_seconds(width, height)),
                preset.description
            ));
            license_row.set_subtitle(&format!("{} · {}", preset.license, preset.credits));

            let installed = preset.is_installed(&ui.models_dir);
            download_row.set_visible(!installed);
            download_row.set_subtitle(&format!(
                "{} · {}",
                preset.label,
                format_bytes(preset.approx_bytes())
            ));
            download_button.set_sensitive(!installed);
            upscale.set_sensitive(installed && size.is_some());
        })
    };
    refresh();

    {
        let refresh = refresh.clone();
        let selected_model = selected_model.clone();
        model_dropdown.connect_selected_notify(move |dropdown| {
            *selected_model.borrow_mut() = dropdown.selected() as usize;
            refresh();
        });
    }
    {
        let refresh = refresh.clone();
        let selected_factor = selected_factor.clone();
        scale_dropdown.connect_selected_notify(move |dropdown| {
            *selected_factor.borrow_mut() = if dropdown.selected() == 0 { 2 } else { 4 };
            refresh();
        });
    }
    {
        let ui = ui.clone();
        let progress = progress.clone();
        let status = status.clone();
        let trigger = download_button.clone();
        let refresh = refresh.clone();
        let selected_model = selected_model.clone();
        trigger.connect_clicked(move |_| {
            let preset = &UPSCALERS[*selected_model.borrow()];
            if preset.is_installed(&ui.models_dir) {
                refresh();
                return;
            }
            download_button.set_sensitive(false);
            progress.set_visible(true);
            progress.set_fraction(0.0);
            progress.set_text(Some("Starting…"));
            status.set_text(&format!("Downloading {}…", preset.label));

            let (progress_tx, progress_rx) =
                async_channel::unbounded::<flux_models::DownloadProgress>();
            let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);
            let models_dir = ui.models_dir.clone();
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(flux_models::download_upscaler(
                        preset,
                        &models_dir,
                        progress_tx,
                    ))?;
                    Ok(())
                })();
                let _ = done_tx.send_blocking(result.map_err(|error| error.to_string()));
            });

            let progress = progress.clone();
            let status = status.clone();
            let download_button = download_button.clone();
            let refresh = refresh.clone();
            let ui = ui.clone();
            glib::spawn_future_local(async move {
                while let Ok(event) = progress_rx.recv().await {
                    let fraction = if event.total > 0 {
                        event.downloaded as f64 / event.total as f64
                    } else {
                        0.0
                    };
                    progress.set_fraction(fraction.min(1.0));
                    progress.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
                    status.set_text(&format!(
                        "{} · {:.1} / {:.1} MB",
                        event.file,
                        event.downloaded as f64 / 1e6,
                        event.total as f64 / 1e6
                    ));
                }
                let result = done_rx.recv().await;
                progress.set_visible(false);
                download_button.set_sensitive(true);
                match result {
                    Ok(Ok(())) => {
                        status.set_text("");
                        ui.toast.add_toast(adw::Toast::new("Upscaler downloaded."));
                        refresh();
                    }
                    Ok(Err(error)) => {
                        status.set_text(&error);
                        ui.toast
                            .add_toast(adw::Toast::new(&format!("Download failed: {error}")));
                    }
                    Err(_) => {
                        status.set_text("Download failed.");
                        ui.toast.add_toast(adw::Toast::new("Download failed."));
                    }
                }
            });
        });
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let dialog = dialog.clone();
        let ui = ui.clone();
        let selected_model = selected_model.clone();
        let selected_factor = selected_factor.clone();
        upscale.connect_clicked(move |_| {
            let preset = &UPSCALERS[*selected_model.borrow()];
            let factor = *selected_factor.borrow();
            dialog.close();
            crate::start_upscale(&ui, preset, factor);
        });
    }

    dialog.present(Some(&ui.window));
}

fn format_duration(seconds: u32) -> String {
    if seconds < 90 {
        format!("{seconds} s")
    } else {
        let minutes = seconds / 60;
        let rest = seconds % 60;
        if rest == 0 {
            format!("{minutes} min")
        } else {
            format!("{minutes} min {rest} s")
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_048_576.0)
}
