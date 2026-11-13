use adw::prelude::*;

pub(crate) struct MainView {
    pub root: gtk::Box,
    pub stack: gtk::Stack,
    pub picture: gtk::Picture,
    pub reference_bar: gtk::Box,
    pub reference_name: gtk::Label,
    pub prompt: gtk::TextView,
    pub composer: gtk::Revealer,
    pub reference_button: gtk::Button,
    pub improve_button: gtk::Button,
    pub generate: gtk::Button,
    pub progress_row: gtk::Box,
    pub progress: gtk::ProgressBar,
    pub cancel: gtk::Button,
    pub status: gtk::Label,
    pub crop_overlay: gtk::DrawingArea,
    pub edit_button: gtk::Button,
    pub crop_button: gtk::ToggleButton,
    pub undo_button: gtk::Button,
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

    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&picture, Some("image"));
    stack.set_visible_child_name("empty");

    let crop_overlay = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .visible(false)
        .build();

    let edit_button = gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text("Edit this image")
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
    let image_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    image_actions.set_halign(gtk::Align::End);
    image_actions.set_valign(gtk::Align::Start);
    image_actions.set_margin_top(12);
    image_actions.set_margin_end(12);
    image_actions.append(&edit_button);
    image_actions.append(&crop_button);
    image_actions.append(&undo_button);

    let image_overlay = gtk::Overlay::builder()
        .child(&stack)
        .hexpand(true)
        .vexpand(true)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    image_overlay.add_overlay(&crop_overlay);
    image_overlay.add_overlay(&image_actions);

    let reference_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    reference_bar.add_css_class("reference-bar");
    reference_bar.set_visible(false);
    reference_bar.set_margin_top(12);
    reference_bar.set_margin_start(12);
    reference_bar.set_margin_end(12);
    let reference_icon = gtk::Image::from_icon_name("image-x-generic-symbolic");
    let reference_caption = gtk::Label::builder()
        .label("Editing reference")
        .css_classes(["caption"])
        .build();
    let reference_name = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .css_classes(["caption"])
        .build();
    reference_bar.append(&reference_icon);
    reference_bar.append(&reference_caption);
    reference_bar.append(&reference_name);

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

    let reference_button = gtk::Button::builder()
        .icon_name("image-x-generic-symbolic")
        .tooltip_text("Choose a reference image to edit")
        .valign(gtk::Align::Center)
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
        .label("Cancel")
        .tooltip_text("Cancel generation")
        .css_classes(["pill"])
        .build();
    let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    progress_row.set_visible(false);
    progress_row.append(&progress);
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
    root.append(&reference_bar);
    root.append(&image_overlay);
    root.append(&bottom);

    MainView {
        root,
        stack,
        picture,
        reference_bar,
        reference_name,
        prompt,
        composer,
        reference_button,
        improve_button,
        generate,
        progress_row,
        progress,
        cancel,
        status,
        crop_overlay,
        edit_button,
        crop_button,
        undo_button,
    }
}
