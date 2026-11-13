use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use flux_core::style::STYLES;
use flux_core::{Config, size};

pub(crate) struct Sidebar {
    pub root: gtk::Box,
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
    pub new_button: gtk::Button,
    pub history_list: gtk::ListBox,
}

pub(crate) fn build(config: &Config, size_cell: Rc<Cell<(u32, u32)>>) -> Sidebar {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let setup_title = gtk::Label::builder()
        .label("Setup")
        .xalign(0.0)
        .css_classes(["heading"])
        .build();

    let model_dropdown = gtk::DropDown::from_strings(&[]);
    model_dropdown.set_hexpand(true);
    model_dropdown.set_tooltip_text(Some("Installed models"));

    let model_hint = gtk::Label::builder()
        .label("Looking for installed models…")
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(34)
        .css_classes(["caption", "dim-label"])
        .build();

    let style_labels: Vec<&str> = STYLES.iter().map(|style| style.label).collect();
    let style_dropdown = gtk::DropDown::from_strings(&style_labels);
    let style_index = STYLES
        .iter()
        .position(|style| style.id == config.style_id)
        .unwrap_or(0);
    style_dropdown.set_selected(style_index as u32);
    style_dropdown.set_hexpand(true);
    style_dropdown.set_tooltip_text(Some("Appends a look preset to the prompt"));

    let width = size::snap(config.default_width);
    let height = size::snap(config.default_height);
    size_cell.set((width, height));

    let width_scale = make_scale(width);
    let height_scale = make_scale(height);
    let width_value = value_label(width);
    let height_value = value_label(height);

    let ratio_area = gtk::DrawingArea::builder()
        .content_width(112)
        .content_height(112)
        .valign(gtk::Align::Start)
        .build();
    {
        let size_cell = size_cell.clone();
        ratio_area.set_draw_func(move |area, cr, width, height| {
            let (image_w, image_h) = size_cell.get();
            let color = area.color();
            let pad = 8.0;
            let available_w = (width as f64 - 2.0 * pad).max(1.0);
            let available_h = (height as f64 - 2.0 * pad).max(1.0);
            let ratio = image_w as f64 / image_h.max(1) as f64;
            let (box_w, box_h) = if available_w / available_h > ratio {
                (available_h * ratio, available_h)
            } else {
                (available_w, available_w / ratio)
            };
            let x = (width as f64 - box_w) / 2.0;
            let y = (height as f64 - box_h) / 2.0;

            cr.rectangle(x, y, box_w, box_h);
            cr.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                0.25,
            );
            cr.fill_preserve().ok();
            cr.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                0.6,
            );
            cr.set_line_width(2.0);
            cr.stroke().ok();
        });
    }

    let ratio_label = gtk::Label::builder()
        .label(size::aspect_label(width, height))
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    let mp_label = gtk::Label::builder()
        .label(format!("{:.2} MP", size::megapixels(width, height)))
        .xalign(0.0)
        .css_classes(["caption", "dim-label"])
        .build();
    let size_hint = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(30)
        .css_classes(["caption"])
        .build();

    let ratio_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    ratio_text.set_valign(gtk::Align::Center);
    ratio_text.append(&ratio_label);
    ratio_text.append(&mp_label);
    ratio_text.append(&size_hint);

    let ratio_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    ratio_row.set_halign(gtk::Align::Center);
    ratio_row.append(&ratio_area);
    ratio_row.append(&ratio_text);

    let seed_entry = gtk::Entry::builder()
        .placeholder_text("random")
        .hexpand(true)
        .input_purpose(gtk::InputPurpose::Digits)
        .build();
    let random_button = gtk::Button::builder()
        .icon_name("media-playlist-shuffle-symbolic")
        .tooltip_text("New random seed")
        .css_classes(["flat"])
        .build();

    let seed_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    seed_row.append(&field_label("Seed"));
    seed_row.append(&seed_entry);
    seed_row.append(&random_button);

    let advanced_summary = gtk::Label::builder()
        .label("")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption", "dim-label"])
        .build();
    let offload_switch = gtk::Switch::builder().active(config.offload).build();
    let offload_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let offload_label = gtk::Label::builder()
        .label("Offload weights to CPU")
        .xalign(0.0)
        .hexpand(true)
        .build();
    offload_row.append(&offload_label);
    offload_row.append(&offload_switch);

    let advanced_body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    advanced_body.append(&advanced_summary);
    advanced_body.append(&offload_row);

    let advanced_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&advanced_body)
        .reveal_child(false)
        .build();
    let advanced_toggle = gtk::ToggleButton::builder()
        .label("Advanced")
        .halign(gtk::Align::Start)
        .css_classes(["flat"])
        .build();
    {
        let revealer = advanced_revealer.clone();
        advanced_toggle.connect_toggled(move |button| {
            revealer.set_reveal_child(button.is_active());
        });
    }

    let setup = gtk::Box::new(gtk::Orientation::Vertical, 6);
    setup.append(&row("Model", &model_dropdown));
    setup.append(&model_hint);
    setup.append(&row("Style", &style_dropdown));

    let width_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    width_row.append(&field_label("Width"));
    width_row.append(&width_scale);
    width_row.append(&width_value);
    let height_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    height_row.append(&field_label("Height"));
    height_row.append(&height_scale);
    height_row.append(&height_value);
    setup.append(&width_row);
    setup.append(&height_row);
    setup.append(&ratio_row);
    setup.append(&seed_row);
    setup.append(&advanced_toggle);
    setup.append(&advanced_revealer);

    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);

    let history_title = gtk::Label::builder()
        .label("History")
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["heading"])
        .build();
    let new_button = gtk::Button::builder()
        .label("New")
        .icon_name("list-add-symbolic")
        .tooltip_text("Start a new image without a reference (Ctrl+N)")
        .css_classes(["flat"])
        .build();
    let history_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    history_header.append(&history_title);
    history_header.append(&new_button);

    let history_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .build();
    let history_scroller = gtk::ScrolledWindow::builder()
        .child(&history_list)
        .vexpand(true)
        .build();

    root.append(&setup_title);
    root.append(&setup);
    root.append(&separator);
    root.append(&history_header);
    root.append(&history_scroller);

    Sidebar {
        root,
        model_dropdown,
        model_hint,
        style_dropdown,
        width_scale,
        height_scale,
        width_value,
        height_value,
        ratio_area,
        ratio_label,
        mp_label,
        size_hint,
        seed_entry,
        random_button,
        offload_switch,
        advanced_summary,
        new_button,
        history_list,
    }
}

fn make_scale(value: u32) -> gtk::Scale {
    let scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        size::MIN_SIZE as f64,
        size::MAX_SIZE as f64,
        size::SIZE_STEP as f64,
    );
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_round_digits(0);
    scale.set_value(value as f64);
    scale.set_tooltip_text(Some(&format!(
        "{}–{} px, steps of {}",
        size::MIN_SIZE,
        size::MAX_SIZE,
        size::SIZE_STEP
    )));
    for mark in [512, 768, 1024, 1280] {
        scale.add_mark(mark as f64, gtk::PositionType::Bottom, None);
    }
    scale
}

fn value_label(value: u32) -> gtk::Label {
    gtk::Label::builder()
        .label(value.to_string())
        .width_chars(4)
        .xalign(1.0)
        .css_classes(["caption", "dim-label"])
        .build()
}

fn field_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .width_chars(6)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build()
}

fn row(text: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&field_label(text));
    row.append(widget);
    row
}
