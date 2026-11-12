use adw::prelude::*;

pub(crate) struct MainView {
    pub root: gtk::Box,
    pub stack: gtk::Stack,
    pub picture: gtk::Picture,
    pub reference_bar: gtk::Box,
    pub reference_name: gtk::Label,
    pub prompt: gtk::TextView,
    pub reference_button: gtk::Button,
    pub generate: gtk::Button,
    pub cancel: gtk::Button,
    pub progress: gtk::ProgressBar,
    pub status: gtk::Label,
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
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&picture, Some("image"));
    stack.set_visible_child_name("empty");

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
        .min_content_height(64)
        .max_content_height(120)
        .propagate_natural_height(true)
        .build();
    prompt_scroller.add_css_class("prompt-field");

    let reference_button = gtk::Button::builder()
        .icon_name("image-x-generic-symbolic")
        .tooltip_text("Choose a reference image to edit")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();

    let generate = gtk::Button::builder()
        .label("Generate")
        .css_classes(["suggested-action", "pill"])
        .valign(gtk::Align::Center)
        .build();

    let cancel = gtk::Button::builder()
        .label("Cancel")
        .css_classes(["pill"])
        .valign(gtk::Align::Center)
        .visible(false)
        .build();

    let progress = gtk::ProgressBar::builder()
        .hexpand(true)
        .show_text(true)
        .visible(false)
        .build();

    let status = gtk::Label::builder()
        .label("Ready")
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption", "dim-label"])
        .build();

    let composer_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    composer_row.append(&reference_button);
    composer_row.append(&prompt_scroller);
    composer_row.append(&cancel);
    composer_row.append(&generate);

    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 8);
    bottom.set_margin_top(12);
    bottom.set_margin_bottom(12);
    bottom.set_margin_start(12);
    bottom.set_margin_end(12);
    bottom.append(&progress);
    bottom.append(&composer_row);
    bottom.append(&status);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&reference_bar);
    root.append(&stack);
    root.append(&bottom);

    MainView {
        root,
        stack,
        picture,
        reference_bar,
        reference_name,
        prompt,
        reference_button,
        generate,
        cancel,
        progress,
        status,
    }
}
