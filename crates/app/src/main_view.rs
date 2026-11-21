use adw::prelude::*;

pub(crate) struct MainView {
    pub root: gtk::Box,
    pub stack: gtk::Stack,
    pub picture: gtk::Picture,
    pub picture_back: gtk::Picture,
    pub reference_strip: gtk::Box,
    pub reference_thumbs: gtk::Box,
    pub reference_add: gtk::Button,
    pub prompt: gtk::TextView,
    pub prompt_history_button: gtk::MenuButton,
    pub prompt_history_list: gtk::ListBox,
    pub composer: gtk::Revealer,
    pub reference_button: gtk::Button,
    pub camera_button: gtk::Button,
    pub improve_button: gtk::Button,
    pub generate: gtk::Button,
    pub progress_row: gtk::Box,
    pub progress: gtk::ProgressBar,
    /// Number of jobs waiting behind the running one.
    pub queue_status: gtk::Label,
    /// Floating queue indicator when the sidebar is hidden.
    pub queue_bubble: gtk::Button,
    pub queue_bubble_spinner: gtk::Spinner,
    pub queue_bubble_label: gtk::Label,
    pub cancel: gtk::Button,
    pub status: gtk::Label,
    pub crop_overlay: gtk::DrawingArea,
    pub zoom_overlay: gtk::DrawingArea,
    pub edit_button: gtk::Button,
    pub upscale_button: gtk::Button,
    pub variation_button: gtk::Button,
    pub adjust_button: gtk::Button,
    /// Returns from a history image to the running job's live preview.
    pub live_button: gtk::Button,
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
    pub effects_import: gtk::Button,
    pub effects_export: gtk::Button,
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
    pub copy_button: gtk::Button,
    pub folder_button: gtk::Button,
    pub compare_button: gtk::ToggleButton,
    pub compare_area: crate::compare_view::CompareView,
}

pub(crate) fn build() -> MainView {
    let empty = adw::StatusPage::builder()
        .icon_name("image-x-generic-symbolic")
        .title("No image yet")
        .description("Describe an image below, then press Generate.")
        .build();

    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .hexpand(true)
        .vexpand(true)
        .build();
    let picture_back = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .hexpand(true)
        .vexpand(true)
        .build();

    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(350)
        .build();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&picture_back, Some("back"));
    stack.add_named(&picture, Some("image"));
    stack.set_visible_child_name("empty");

    // Zoom is a view-only scale: the stack grows inside this scroller, while
    // every overlay above it keeps working in screen coordinates.
    let scroller = gtk::ScrolledWindow::builder()
        .child(&stack)
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .overlay_scrolling(true)
        .build();
    let scroller_clip = crate::rounded_clip::RoundedClip::new(&scroller);

    let crop_overlay = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .visible(false)
        .build();
    let zoom_overlay = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .visible(false)
        .can_target(false)
        .build();

    let edit_button = gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text("Edit this image")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let upscale_button = gtk::Button::builder()
        .icon_name("zoom-in-symbolic")
        .tooltip_text("Upscale this image")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let variation_button = gtk::Button::builder()
        .icon_name("media-playlist-shuffle-symbolic")
        .tooltip_text("Generate a variation (seed + 1)")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let adjust_button = gtk::Button::builder()
        .icon_name("preferences-color-symbolic")
        .tooltip_text("Adjust colors")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let live_button = gtk::Button::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text("Show the running job's live preview")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let colorize_item = effect_item(
        "color-select-symbolic",
        "Colorize",
        "Restore color in black-and-white photos",
    );
    let recover_item = effect_item(
        "view-refresh-symbolic",
        "Recover",
        "Renew old, scratched or damaged photos",
    );
    let enhance_item = effect_item("starred-symbolic", "Enhance", "Sharpen detail and clarity");
    let denoise_item = effect_item(
        "edit-clear-symbolic",
        "Denoise",
        "Remove grain and compression artifacts",
    );
    let photoreal_item = effect_item(
        "camera-photo-symbolic",
        "Photoreal",
        "Turn a drawing or cartoon into a realistic photo",
    );
    let background_item = effect_item(
        "image-x-generic-symbolic",
        "Replace background",
        "Swap the background while keeping the subject",
    );
    let auto_lighting_item = effect_item(
        "daytime-sunset-symbolic",
        "Auto day ↔ night",
        "Detect the lighting and convert it",
    );
    let daylight_item = effect_item(
        "weather-clear-symbolic",
        "To daylight",
        "Convert this night scene into daylight",
    );
    let night_item = effect_item(
        "weather-clear-night-symbolic",
        "To night",
        "Convert this daytime scene into night",
    );
    let expand_item = effect_item(
        "zoom-fit-best-symbolic",
        "Expand",
        "Extend the image to the sidebar size and ratio",
    );

    let effects_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    effects_box.set_margin_top(6);
    effects_box.set_margin_bottom(6);
    effects_box.set_margin_start(6);
    effects_box.set_margin_end(6);
    effects_box.append(&colorize_item);
    effects_box.append(&recover_item);
    effects_box.append(&enhance_item);
    effects_box.append(&denoise_item);
    effects_box.append(&photoreal_item);
    effects_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    effects_box.append(&background_item);
    effects_box.append(&auto_lighting_item);
    effects_box.append(&daylight_item);
    effects_box.append(&night_item);
    effects_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    effects_box.append(&expand_item);

    let custom_effects_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let custom_scroller = gtk::ScrolledWindow::builder()
        .child(&custom_effects_box)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(240)
        .build();
    let custom_effects_section = gtk::Box::new(gtk::Orientation::Vertical, 2);
    custom_effects_section.set_visible(false);
    custom_effects_section.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let custom_heading = gtk::Label::builder()
        .label("Your effects")
        .xalign(0.0)
        .margin_top(4)
        .margin_start(10)
        .css_classes(["caption", "dim-label"])
        .build();
    custom_effects_section.append(&custom_heading);
    custom_effects_section.append(&custom_scroller);
    effects_box.append(&custom_effects_section);

    let effects_import = gtk::Button::builder()
        .label("Import…")
        .tooltip_text("Load effects from an Imagiflume effects file")
        .css_classes(["flat"])
        .build();
    let effects_export = gtk::Button::builder()
        .label("Export…")
        .tooltip_text("Save your effects to a file")
        .css_classes(["flat"])
        .build();
    let effects_tools = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    effects_tools.set_margin_top(4);
    effects_tools.append(&effects_import);
    effects_tools.append(&effects_export);
    effects_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    effects_box.append(&effects_tools);

    let effects_popover = gtk::Popover::builder()
        .child(&effects_box)
        .css_classes(["effects-popover"])
        .build();
    let effects_button = gtk::MenuButton::builder()
        .icon_name("applications-graphics-symbolic")
        .tooltip_text("Image effects")
        .visible(false)
        .css_classes(["osd", "circular"])
        .popover(&effects_popover)
        .build();
    let save_effect_button = gtk::Button::builder()
        .icon_name("bookmark-new-symbolic")
        .tooltip_text("Save this prompt as an effect")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let back_button = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Back to view")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let nav_up_button = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .tooltip_text("Newer image (scroll up)")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let nav_down_button = gtk::Button::builder()
        .icon_name("go-down-symbolic")
        .tooltip_text("Older image (scroll down)")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let crop_button = gtk::ToggleButton::builder()
        .icon_name("edit-select-symbolic")
        .tooltip_text("Select an area to crop and use as reference")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let undo_button = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Undo crop")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let copy_button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy this image")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let folder_button = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Show in folder")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let compare_button = gtk::ToggleButton::builder()
        .icon_name("object-flip-horizontal-symbolic")
        .tooltip_text("Compare with the source image")
        .visible(false)
        .css_classes(["osd", "circular"])
        .build();
    let compare_area = crate::compare_view::CompareView::new();
    compare_area.set_hexpand(true);
    compare_area.set_vexpand(true);
    compare_area.set_halign(gtk::Align::Fill);
    compare_area.set_valign(gtk::Align::Fill);
    compare_area.set_visible(false);
    compare_area.set_can_target(true);
    compare_area.add_css_class("compare-overlay");
    let image_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    image_actions.set_halign(gtk::Align::End);
    image_actions.set_valign(gtk::Align::Start);
    image_actions.set_margin_top(12);
    image_actions.set_margin_end(12);
    image_actions.append(&live_button);
    image_actions.append(&effects_button);
    image_actions.append(&save_effect_button);
    image_actions.append(&adjust_button);
    image_actions.append(&upscale_button);
    image_actions.append(&variation_button);
    image_actions.append(&edit_button);
    image_actions.append(&crop_button);
    image_actions.append(&undo_button);
    image_actions.append(&back_button);
    image_actions.append(&copy_button);
    image_actions.append(&folder_button);
    image_actions.append(&compare_button);

    let image_overlay = gtk::Overlay::builder()
        .child(&scroller_clip)
        .hexpand(true)
        .vexpand(true)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let nav_actions = gtk::Box::new(gtk::Orientation::Vertical, 6);
    nav_actions.set_halign(gtk::Align::End);
    nav_actions.set_valign(gtk::Align::Center);
    nav_actions.set_margin_end(12);
    nav_actions.append(&nav_up_button);
    nav_actions.append(&nav_down_button);

    let zoom_out_button = gtk::Button::builder()
        .icon_name("zoom-out-symbolic")
        .tooltip_text("Zoom out")
        .css_classes(["flat", "circular"])
        .build();
    let zoom_label = gtk::Label::builder()
        .label("100%")
        .width_chars(5)
        .css_classes(["caption", "numeric"])
        .build();
    let zoom_in_button = gtk::Button::builder()
        .icon_name("zoom-in-symbolic")
        .tooltip_text("Zoom in")
        .css_classes(["flat", "circular"])
        .build();
    let zoom_fit_button = gtk::Button::builder()
        .icon_name("zoom-fit-best-symbolic")
        .tooltip_text("Fit to window")
        .css_classes(["flat", "circular"])
        .build();
    let queue_bubble_spinner = gtk::Spinner::new();
    queue_bubble_spinner.set_size_request(14, 14);
    queue_bubble_spinner.set_visible(false);
    let queue_bubble_label = gtk::Label::builder().css_classes(["caption"]).build();
    let queue_bubble_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    queue_bubble_content.append(&queue_bubble_spinner);
    queue_bubble_content.append(&queue_bubble_label);
    let queue_bubble = gtk::Button::builder()
        .child(&queue_bubble_content)
        .tooltip_text("Open the job queue")
        .halign(gtk::Align::Start)
        .valign(gtk::Align::End)
        .margin_start(16)
        .margin_bottom(16)
        .visible(false)
        .css_classes(["queue-bubble"])
        .build();
    queue_bubble.update_property(&[gtk::accessible::Property::Label("Open the job queue")]);

    let zoom_controls = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    zoom_controls.add_css_class("osd");
    zoom_controls.add_css_class("zoom-controls");
    zoom_controls.set_halign(gtk::Align::End);
    zoom_controls.set_valign(gtk::Align::End);
    zoom_controls.set_margin_bottom(12);
    zoom_controls.set_margin_end(12);
    zoom_controls.set_visible(false);
    zoom_controls.append(&zoom_out_button);
    zoom_controls.append(&zoom_label);
    zoom_controls.append(&zoom_in_button);
    zoom_controls.append(&zoom_fit_button);

    image_overlay.add_overlay(&crop_overlay);
    image_overlay.add_overlay(&zoom_overlay);
    image_overlay.add_overlay(&compare_area);
    image_overlay.add_overlay(&image_actions);
    image_overlay.add_overlay(&nav_actions);
    image_overlay.add_overlay(&zoom_controls);
    image_overlay.add_overlay(&queue_bubble);

    let reference_thumbs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    reference_thumbs.set_valign(gtk::Align::Center);

    let reference_caption = gtk::Label::builder()
        .label("References")
        .css_classes(["caption", "dim-label"])
        .build();

    let reference_add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add reference images")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();

    let reference_strip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    reference_strip.add_css_class("reference-strip");
    reference_strip.set_visible(false);
    reference_strip.set_margin_top(8);
    reference_strip.set_margin_bottom(4);
    reference_strip.set_margin_start(12);
    reference_strip.set_margin_end(12);
    reference_strip.append(&reference_caption);
    reference_strip.append(&reference_thumbs);
    reference_strip.append(&reference_add);

    let prompt = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();

    let prompt_scroller = gtk::ScrolledWindow::builder()
        .child(&prompt)
        .hexpand(true)
        .min_content_height(58)
        .max_content_height(120)
        .propagate_natural_height(true)
        .build();

    let prompt_history_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let prompt_history_scroller = gtk::ScrolledWindow::builder()
        .child(&prompt_history_list)
        .propagate_natural_height(true)
        .max_content_height(280)
        .min_content_width(320)
        .build();
    let prompt_history_popover = gtk::Popover::builder()
        .child(&prompt_history_scroller)
        .build();
    let prompt_history_button = gtk::MenuButton::builder()
        .icon_name("document-open-recent-symbolic")
        .tooltip_text("Load a previous prompt")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .popover(&prompt_history_popover)
        .build();

    let reference_button = gtk::Button::builder()
        .icon_name("image-x-generic-symbolic")
        .tooltip_text("Add reference images to edit")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();

    let camera_button = gtk::Button::builder()
        .icon_name("camera-photo-symbolic")
        .tooltip_text("Take a photo with the camera")
        .valign(gtk::Align::Center)
        .visible(false)
        .css_classes(["flat"])
        .build();

    let improve_button = gtk::Button::builder()
        .icon_name("star-new-symbolic")
        .tooltip_text("Improve the prompt with a local language model")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();

    let generate = gtk::Button::builder()
        .icon_name("document-send-symbolic")
        .tooltip_text("Generate (Ctrl+Enter)")
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action", "circular"])
        .build();

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    actions.set_margin_start(6);
    actions.set_margin_end(6);
    actions.set_margin_bottom(6);
    actions.append(&reference_button);
    actions.append(&camera_button);
    actions.append(&prompt_history_button);
    actions.append(&improve_button);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&spacer);
    actions.append(&generate);

    let prompt_field = gtk::Box::new(gtk::Orientation::Vertical, 0);
    prompt_field.add_css_class("prompt-field");
    prompt_field.set_hexpand(true);
    prompt_field.append(&prompt_scroller);
    prompt_field.append(&actions);

    let composer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(250)
        .reveal_child(true)
        .child(&prompt_field)
        .build();

    let progress = gtk::ProgressBar::builder()
        .hexpand(true)
        .show_text(true)
        .build();
    let cancel = gtk::Button::builder()
        .icon_name("process-stop-symbolic")
        .tooltip_text("Cancel generation")
        .css_classes(["destructive-action", "circular"])
        .build();
    let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    progress_row.set_visible(false);
    let queue_status = gtk::Label::builder()
        .visible(false)
        .css_classes(["caption", "dim-label"])
        .build();
    progress_row.append(&progress);
    progress_row.append(&queue_status);
    progress_row.append(&cancel);

    let status = gtk::Label::builder()
        .label("Ready")
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption", "dim-label"])
        .build();

    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 8);
    bottom.set_margin_top(12);
    bottom.set_margin_bottom(12);
    bottom.set_margin_start(12);
    bottom.set_margin_end(12);
    bottom.append(&progress_row);
    bottom.append(&composer);
    bottom.append(&status);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&reference_strip);
    root.append(&image_overlay);
    root.append(&bottom);

    MainView {
        root,
        stack,
        picture,
        picture_back,
        reference_strip,
        reference_thumbs,
        reference_add,
        prompt,
        prompt_history_button,
        prompt_history_list,
        composer,
        reference_button,
        camera_button,
        improve_button,
        generate,
        progress_row,
        progress,
        queue_status,
        queue_bubble,
        queue_bubble_spinner,
        queue_bubble_label,
        cancel,
        status,
        crop_overlay,
        zoom_overlay,
        adjust_button,
        zoom_scroller: scroller,
        zoom_controls,
        zoom_out_button,
        zoom_label,
        zoom_in_button,
        zoom_fit_button,
        edit_button,
        upscale_button,
        variation_button,
        live_button,
        effects_button,
        effects_popover,
        save_effect_button,
        custom_effects_section,
        custom_effects_box,
        effects_import,
        effects_export,
        colorize_item,
        recover_item,
        enhance_item,
        denoise_item,
        photoreal_item,
        background_item,
        auto_lighting_item,
        daylight_item,
        night_item,
        expand_item,
        back_button,
        nav_up_button,
        nav_down_button,
        crop_button,
        undo_button,
        copy_button,
        folder_button,
        compare_button,
        compare_area,
    }
}

pub(crate) fn effect_item(icon_name: &str, title: &str, subtitle: &str) -> gtk::Button {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_valign(gtk::Align::Start);

    let title = gtk::Label::builder().label(title).xalign(0.0).build();
    let subtitle = gtk::Label::builder()
        .label(subtitle)
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(34)
        .css_classes(["caption", "dim-label"])
        .build();

    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    text.append(&title);
    text.append(&subtitle);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&icon);
    row.append(&text);

    gtk::Button::builder()
        .child(&row)
        .css_classes(["flat"])
        .build()
}
