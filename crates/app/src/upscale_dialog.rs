use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use imagiflume_core::upscaler;

use crate::Ui;

/// Show the upscale dialog for the image currently on screen.
pub(crate) fn show(ui: &Rc<Ui>) {
    if ui.zooming.get() || ui.classifying.get() {
        return;
    }
    let Some(target) = crate::upscale_target(ui) else {
        return;
    };
    let (width, height) = target.dimensions();
    if width == 0 || height == 0 {
        return;
    }

    // Only offer weights that can actually be used. Downloads and deletions
    // live in Preferences, which lists every preset.
    let installed = upscaler::installed_upscalers(&ui.models_dir);
    if installed.is_empty() {
        ui.toast.add_toast(adw::Toast::new(
            "No upscaler installed. Download one in Preferences → Models.",
        ));
        return;
    }
    let installed = Rc::new(installed);

    let (initial_model, initial_factor) = {
        let config = ui.config.borrow();
        let model = installed
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
    let pending_note = gtk::Label::builder()
        .label("Upscales the running generation's result when it finishes.")
        .xalign(0.0)
        .wrap(true)
        .visible(target.is_pending())
        .css_classes(["caption", "dim-label"])
        .build();
    output_group.add(&pending_note);
    page.add(&output_group);

    let model_group = adw::PreferencesGroup::builder().title("Model").build();
    let labels: Vec<&str> = installed.iter().map(|preset| preset.label).collect();
    let model_dropdown = gtk::DropDown::from_strings(&labels);
    model_dropdown.set_selected(initial_model as u32);
    let model_row = adw::ActionRow::builder().title("Upscaler").build();
    model_row.add_suffix(&model_dropdown);
    model_row.set_activatable_widget(Some(&model_dropdown));
    let details_row = adw::ActionRow::builder().title("Details").build();
    let license_row = adw::ActionRow::builder().title("License").build();
    let more_row = adw::ActionRow::builder()
        .title("More upscalers…")
        .subtitle("Download additional models in Preferences.")
        .build();
    let more_button = gtk::Button::builder()
        .label("Preferences")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    more_row.add_suffix(&more_button);
    model_group.add(&model_row);
    model_group.add(&details_row);
    model_group.add(&license_row);
    if installed.len() < upscaler::UPSCALERS.len() {
        model_group.add(&more_row);
    }
    page.add(&model_group);

    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let selected_model = Rc::new(RefCell::new(initial_model));
    let selected_factor = Rc::new(RefCell::new(initial_factor));

    let refresh: Rc<dyn Fn()> = {
        let size_row = size_row.clone();
        let warning = warning.clone();
        let details_row = details_row.clone();
        let license_row = license_row.clone();
        let upscale = upscale.clone();
        let selected_model = selected_model.clone();
        let selected_factor = selected_factor.clone();
        let installed = installed.clone();
        Rc::new(move || {
            let index = (*selected_model.borrow()).min(installed.len() - 1);
            let preset = installed[index];
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
            upscale.set_sensitive(size.is_some());
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
        let dialog = dialog.clone();
        let ui = ui.clone();
        more_button.connect_clicked(move |_| {
            dialog.close();
            crate::preferences::show_preferences(&ui, true);
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
        let target = target.clone();
        let trigger = upscale.clone();
        let installed = installed.clone();
        trigger.connect_clicked(move |_| {
            let index = (*selected_model.borrow()).min(installed.len() - 1);
            let preset = installed[index];
            let factor = *selected_factor.borrow();
            dialog.close();
            crate::start_upscale(&ui, preset, factor, target.clone());
        });
    }

    dialog.present(Some(&ui.window));
    upscale.set_receives_default(true);
    glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || {
        upscale.grab_focus();
    });
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
