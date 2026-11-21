use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use imagiflume_core::size::RatioMatch;
use imagiflume_core::style::STYLES;
use imagiflume_core::{Config, size};

pub(crate) struct Sidebar {
    pub root: gtk::Box,
    pub model_dropdown: gtk::DropDown,
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
    pub batch_spin: gtk::SpinButton,
    pub lora_button: gtk::Button,
    pub lora_summary: gtk::Label,
    pub new_button: gtk::Button,
    pub open_button: gtk::Button,
    pub select_button: gtk::Button,
    pub search_button: gtk::ToggleButton,
    pub selection_bar: gtk::Box,
    pub selection_label: gtk::Label,
    pub selection_all: gtk::Button,
    pub selection_delete: gtk::Button,
    pub selection_cancel: gtk::Button,
    pub history_search: gtk::SearchEntry,
    pub history_search_revealer: gtk::Revealer,
    pub history_filter: Rc<Cell<u32>>,
    pub history_filter_button: gtk::MenuButton,
    pub history_filter_buttons: Vec<gtk::CheckButton>,
    pub history_empty: gtk::Label,
    pub history_list: gtk::ListBox,
    pub queue_expander: adw::ExpanderRow,
    pub queue_list: gtk::ListBox,
    pub queue_count: gtk::Label,
    pub queue_toggle: gtk::Button,
    pub queue_clear: gtk::Button,
    pub queue_pause: gtk::ToggleButton,
}

pub(crate) fn build(
    config: &Config,
    size_cell: Rc<Cell<(u32, u32)>>,
    image_size: Rc<Cell<(u32, u32)>>,
    size_step: u32,
) -> Sidebar {
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

    let mp = config.mp();
    let ratio = config.ratio();
    let (width, height) = size::size_for_mp_step(mp, ratio, size_step);
    size_cell.set((width, height));

    let mp_scale = make_mp_scale(mp);
    let mp_value = mp_value_label(mp);

    let labels = ratio_labels();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let ratio_model = gtk::StringList::new(&label_refs);
    let custom_ratio = Rc::new(RefCell::new(None));
    let ratio_index = match size::ratio_for_size(ratio.0, ratio.1) {
        RatioMatch::Preset(index) => index as u32,
        RatioMatch::Custom(custom) => {
            ratio_model.append(&size::ratio_label(custom));
            *custom_ratio.borrow_mut() = Some(custom);
            size::RATIOS.len() as u32
        }
    };
    let ratio_dropdown = gtk::DropDown::builder()
        .model(&ratio_model)
        .selected(ratio_index)
        .hexpand(true)
        .tooltip_text("Target aspect ratio")
        .build();

    let ratio_area = gtk::DrawingArea::builder()
        .content_width(180)
        .content_height(180)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    {
        let size_cell = size_cell.clone();
        let image_size = image_size.clone();
        ratio_area.set_draw_func(move |area, cr, width, height| {
            let target = size_cell.get();
            let source = image_size.get();
            let color = area.color();
            let red = color.red() as f64;
            let green = color.green() as f64;
            let blue = color.blue() as f64;
            let pad = 8.0;
            let available_w = (width as f64 - 2.0 * pad).max(1.0);
            let available_h = (height as f64 - 2.0 * pad).max(1.0);
            let target_ratio = target.0.max(1) as f64 / target.1.max(1) as f64;
            let (box_w, box_h) = fit_ratio(target_ratio, available_w, available_h);
            let x = (width as f64 - box_w) / 2.0;
            let y = (height as f64 - box_h) / 2.0;

            cr.rectangle(x, y, box_w, box_h);
            cr.set_source_rgba(red, green, blue, 0.25);
            cr.fill_preserve().ok();
            cr.set_source_rgba(red, green, blue, 0.6);
            cr.set_line_width(2.0);
            cr.stroke().ok();

            let Some(source_ratio) = source_overlay(source, target) else {
                return;
            };
            let inset = 2.0;
            let (inner_w, inner_h) = fit_ratio(
                source_ratio,
                (box_w - inset * 2.0).max(1.0),
                (box_h - inset * 2.0).max(1.0),
            );
            let inner_x = x + (box_w - inner_w) / 2.0;
            let inner_y = y + (box_h - inner_h) / 2.0;

            cr.rectangle(inner_x, inner_y, inner_w, inner_h);
            cr.set_source_rgba(red, green, blue, 0.10);
            cr.fill_preserve().ok();
            cr.set_source_rgba(red, green, blue, 0.85);
            cr.set_line_width(1.5);
            cr.set_dash(&[4.0, 3.0], 0.0);
            cr.stroke().ok();
            cr.set_dash(&[], 0.0);
        });
    }

    let ratio_label = gtk::Label::builder()
        .label(size::ratio_label(ratio))
        .xalign(0.5)
        .css_classes(["heading"])
        .build();
    let mp_label = gtk::Label::builder()
        .label(format!(
            "{:.2} MP - {width} × {height} px",
            size::megapixels(width, height)
        ))
        .xalign(0.5)
        .css_classes(["caption", "dim-label"])
        .build();
    let size_hint = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(30)
        .css_classes(["caption"])
        .build();

    let ratio_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    ratio_text.set_halign(gtk::Align::Fill);
    ratio_text.append(&ratio_label);
    ratio_text.append(&mp_label);

    let ratio_preview = gtk::Box::new(gtk::Orientation::Vertical, 6);
    ratio_preview.set_halign(gtk::Align::Fill);
    ratio_preview.set_margin_top(2);
    ratio_preview.set_margin_bottom(2);
    ratio_preview.append(&ratio_area);
    ratio_preview.append(&ratio_text);
    ratio_preview.append(&size_hint);

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

    let batch_spin = gtk::SpinButton::with_range(1.0, 4.0, 1.0);
    batch_spin.set_value(config.batch_count.clamp(1, 4) as f64);
    batch_spin.set_width_chars(2);
    batch_spin.set_valign(gtk::Align::Center);
    batch_spin.set_tooltip_text(Some("Images to queue per Generate"));
    let batch_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    batch_row.append(&field_label("Batch"));
    batch_row.append(&batch_spin);
    let batch_hint = gtk::Label::builder()
        .label("images")
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["caption", "dim-label"])
        .build();
    batch_row.append(&batch_hint);

    let lora_button = gtk::Button::builder()
        .label("LoRAs…")
        .tooltip_text("Apply LoRA weights to generations")
        .css_classes(["flat"])
        .build();
    let lora_summary = gtk::Label::builder()
        .label("None active")
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["caption", "dim-label"])
        .build();
    let lora_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    lora_row.append(&field_label("LoRAs"));
    lora_row.append(&lora_button);
    lora_row.append(&lora_summary);

    let setup = gtk::Box::new(gtk::Orientation::Vertical, 6);
    setup.append(&row("Model", &model_dropdown));
    setup.append(&model_hint);
    setup.append(&row("Style", &style_dropdown));

    let size_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    size_row.append(&field_label("Size"));
    size_row.append(&mp_scale);
    size_row.append(&mp_value);
    let ratio_select_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    ratio_select_row.append(&field_label("Ratio"));
    ratio_select_row.append(&ratio_dropdown);
    setup.append(&seed_row);
    setup.append(&batch_row);
    setup.append(&lora_row);
    setup.append(&ratio_select_row);
    setup.append(&size_row);
    setup.append(&ratio_preview);

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
    let open_button = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text("Open an image from disk (Ctrl+O)")
        .css_classes(["flat"])
        .build();
    let select_button = gtk::Button::builder()
        .icon_name("selection-mode-symbolic")
        .tooltip_text("Select multiple images to delete")
        .css_classes(["flat"])
        .build();
    let search_button = gtk::ToggleButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Search history (Ctrl+F)")
        .css_classes(["flat"])
        .build();

    let filter_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    filter_box.set_margin_top(6);
    filter_box.set_margin_bottom(6);
    filter_box.set_margin_start(6);
    filter_box.set_margin_end(6);
    let mut history_filter_buttons = Vec::with_capacity(crate::history::FILTER_LABELS.len());
    let mut group_leader: Option<gtk::CheckButton> = None;
    for (index, label) in crate::history::FILTER_LABELS.iter().enumerate() {
        let check = gtk::CheckButton::with_label(label);
        check.set_active(index == 0);
        if let Some(leader) = &group_leader {
            check.set_group(Some(leader));
        } else {
            group_leader = Some(check.clone());
        }
        filter_box.append(&check);
        history_filter_buttons.push(check);
    }
    let filter_popover = gtk::Popover::builder().child(&filter_box).build();
    let history_filter_button = gtk::MenuButton::builder()
        .icon_name("view-filter-symbolic")
        .tooltip_text("Filter history: All images")
        .popover(&filter_popover)
        .css_classes(["flat"])
        .build();

    let history_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    history_header.append(&history_title);
    history_header.append(&search_button);
    history_header.append(&history_filter_button);
    history_header.append(&select_button);
    history_header.append(&open_button);
    history_header.append(&new_button);

    let history_search = gtk::SearchEntry::builder()
        .placeholder_text("Search history")
        .hexpand(true)
        .build();
    history_search.set_tooltip_text(Some("Search prompts, models and seeds (Ctrl+F)"));
    let history_search_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(180)
        .reveal_child(false)
        .child(&history_search)
        .build();

    let history_empty = gtk::Label::builder()
        .label("No images match.")
        .xalign(0.0)
        .visible(false)
        .margin_start(6)
        .css_classes(["caption", "dim-label"])
        .build();

    let selection_label = gtk::Label::builder()
        .label("0 selected")
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["caption"])
        .build();
    let selection_all = gtk::Button::builder()
        .label("All")
        .tooltip_text("Select every visible image")
        .css_classes(["flat"])
        .build();
    let selection_delete = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Delete selected images")
        .sensitive(false)
        .css_classes(["flat", "destructive-action"])
        .build();
    let selection_cancel = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text("Cancel selection")
        .css_classes(["flat"])
        .build();
    let selection_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    selection_bar.add_css_class("selection-bar");
    selection_bar.set_visible(false);
    selection_bar.append(&selection_label);
    selection_bar.append(&selection_all);
    selection_bar.append(&selection_delete);
    selection_bar.append(&selection_cancel);

    let history_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .build();
    let history_scroller = gtk::ScrolledWindow::builder()
        .child(&history_list)
        .vexpand(true)
        .build();

    let queue_count = gtk::Label::builder()
        .label("0")
        .css_classes(["caption"])
        .build();
    let queue_toggle_icon = gtk::Image::from_icon_name("pan-end-symbolic");
    queue_toggle_icon.set_pixel_size(11);
    let queue_toggle_content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    queue_toggle_content.set_valign(gtk::Align::Center);
    queue_toggle_content.append(&queue_count);
    queue_toggle_content.append(&queue_toggle_icon);
    let queue_toggle = gtk::Button::builder()
        .tooltip_text("Expand queue")
        .valign(gtk::Align::Center)
        .css_classes(["chain-toggle"])
        .child(&queue_toggle_content)
        .build();
    let queue_expander = adw::ExpanderRow::builder()
        .title("Queue")
        .subtitle("No tasks running")
        .subtitle_lines(1)
        .expanded(false)
        .build();
    let queue_clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text("Remove all waiting jobs")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let queue_pause = gtk::ToggleButton::builder()
        .icon_name("media-playback-pause-symbolic")
        .tooltip_text("Pause the queue")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    if let Some(arrow) =
        crate::find_widget_by_css(queue_expander.upcast_ref(), "expander-row-arrow")
    {
        arrow.set_visible(false);
    }
    queue_expander.add_suffix(&queue_clear);
    queue_expander.add_suffix(&queue_pause);
    queue_expander.add_suffix(&queue_toggle);

    let queue_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let queue_scroller = gtk::ScrolledWindow::builder()
        .child(&queue_list)
        .propagate_natural_height(true)
        .max_content_height(170)
        .vexpand(false)
        .build();
    queue_expander.add_row(&queue_scroller);

    {
        let expander = queue_expander.clone();
        queue_toggle.connect_clicked(move |_| expander.set_expanded(!expander.is_expanded()));
    }
    {
        let icon = queue_toggle_icon.clone();
        let button = queue_toggle.clone();
        queue_expander.connect_expanded_notify(move |expander| {
            icon.set_icon_name(Some(if expander.is_expanded() {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            }));
            button.set_tooltip_text(Some(if expander.is_expanded() {
                "Collapse queue"
            } else {
                "Expand queue"
            }));
        });
    }

    let history_separator = gtk::Separator::new(gtk::Orientation::Horizontal);

    root.append(&setup_title);
    root.append(&setup);
    root.append(&separator);
    root.append(&queue_expander);
    root.append(&history_separator);
    root.append(&history_header);
    root.append(&history_search_revealer);
    root.append(&selection_bar);
    root.append(&history_empty);
    root.append(&history_scroller);

    Sidebar {
        root,
        model_dropdown,
        model_hint,
        style_dropdown,
        mp_scale,
        mp_value,
        ratio_dropdown,
        ratio_model,
        custom_ratio,
        ratio_area,
        ratio_label,
        mp_label,
        size_hint,
        seed_entry,
        random_button,
        batch_spin,
        lora_button,
        lora_summary,
        new_button,
        open_button,
        select_button,
        search_button,
        selection_bar,
        selection_label,
        selection_all,
        selection_delete,
        selection_cancel,
        history_search,
        history_search_revealer,
        history_filter: Rc::new(Cell::new(0)),
        history_filter_button,
        history_filter_buttons,
        history_empty,
        history_list,
        queue_expander,
        queue_list,
        queue_count,
        queue_toggle,
        queue_clear,
        queue_pause,
    }
}

fn ratio_labels() -> Vec<String> {
    size::RATIOS
        .iter()
        .map(|&ratio| size::ratio_label(ratio))
        .collect()
}

fn make_mp_scale(value: f64) -> gtk::Scale {
    let scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        size::MP_MIN,
        size::MP_MAX,
        0.01,
    );
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_digits(2);
    scale.set_round_digits(2);
    scale.set_value(value);
    scale.set_tooltip_text(Some("Target size in megapixels"));
    for mark in [0.5, 1.0, 1.5] {
        scale.add_mark(mark, gtk::PositionType::Bottom, None);
    }
    scale
}

fn mp_value_label(value: f64) -> gtk::Label {
    gtk::Label::builder()
        .label(format!("{value:.2} MP"))
        .width_chars(7)
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

/// Largest rectangle with `ratio` that fits in the available area.
fn fit_ratio(ratio: f64, max_width: f64, max_height: f64) -> (f64, f64) {
    if max_width / max_height > ratio {
        (max_height * ratio, max_height)
    } else {
        (max_width, max_width / ratio)
    }
}

/// The source aspect to overlay inside the target, or `None` when they match.
fn source_overlay(source: (u32, u32), target: (u32, u32)) -> Option<f64> {
    const TOLERANCE: f64 = 0.01;
    if source.0 == 0 || source.1 == 0 || target.0 == 0 || target.1 == 0 {
        return None;
    }
    let source_ratio = source.0 as f64 / source.1 as f64;
    let target_ratio = target.0 as f64 / target.1 as f64;
    ((source_ratio - target_ratio).abs() / target_ratio > TOLERANCE).then_some(source_ratio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_rectangles_into_the_available_area() {
        let (width, height) = fit_ratio(16.0 / 9.0, 96.0, 96.0);
        assert!((width - 96.0).abs() < 1e-9);
        assert!((height - 54.0).abs() < 1e-9);

        let (width, height) = fit_ratio(9.0 / 16.0, 96.0, 96.0);
        assert!((width - 54.0).abs() < 1e-9);
        assert!((height - 96.0).abs() < 1e-9);

        let (width, height) = fit_ratio(4.0 / 3.0, 96.0, 96.0);
        assert!((width - 96.0).abs() < 1e-9);
        assert!((height - 72.0).abs() < 1e-9);
    }

    #[test]
    fn overlays_the_source_only_when_the_aspect_differs() {
        assert_eq!(source_overlay((1024, 768), (1024, 768)), None);
        assert_eq!(source_overlay((0, 0), (1024, 768)), None);
        assert_eq!(source_overlay((1024, 768), (0, 0)), None);

        let ratio = source_overlay((1920, 1080), (1024, 768)).unwrap();
        assert!((ratio - 16.0 / 9.0).abs() < 1e-9);

        let ratio = source_overlay((1024, 768), (1280, 1024)).unwrap();
        assert!((ratio - 4.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn ignores_tiny_aspect_differences() {
        // 1024x768 and 1025x768 differ by far less than one percent.
        assert_eq!(source_overlay((1024, 768), (1025, 768)), None);
    }
}
