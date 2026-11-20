use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use flux_core::preset::preset;
use flux_core::style::STYLES;
use flux_core::upscaler::upscaler;
use glib::types::StaticType;
use gtk::{gdk, glib};

use crate::history::Generation;
use crate::{
    Ui, cancel_zoom, refresh_model, set_size, show_composer, thumb_path, update_image_actions,
    update_image_view,
};

struct OrderedHistory {
    entries: Vec<Generation>,
    roots: HashMap<i64, i64>,
    group_sizes: HashMap<i64, usize>,
    /// Newest member of each chain, used as the collapsed header row.
    headers: HashMap<i64, i64>,
}

/// Order entries so each chain is contiguous: groups by newest activity, and
/// inside a group the newest image first with the original root at the bottom.
fn order_entries(entries: Vec<Generation>) -> OrderedHistory {
    let by_id: HashMap<i64, Generation> = entries
        .iter()
        .cloned()
        .map(|entry| (entry.id, entry))
        .collect();

    // The top-most entry reachable through `parent_id` is the chain root.
    let mut roots: HashMap<i64, i64> = HashMap::with_capacity(entries.len());
    for entry in &entries {
        let mut root = entry.id;
        let mut current = entry;
        let mut guard = 0;
        while let Some(parent_id) = current.parent_id {
            let Some(parent) = by_id.get(&parent_id) else {
                break;
            };
            root = parent.id;
            current = parent;
            guard += 1;
            if guard > entries.len() {
                break;
            }
        }
        roots.insert(entry.id, root);
    }

    let mut groups: HashMap<i64, Vec<Generation>> = HashMap::new();
    for entry in &entries {
        if let Some(root) = roots.get(&entry.id) {
            groups.entry(*root).or_default().push(entry.clone());
        }
    }

    let mut group_list: Vec<(i64, i64, Vec<Generation>)> = groups
        .into_iter()
        .map(|(root, mut members)| {
            // Newest first so the chain reads like a stack; then put the
            // original root at the bottom.
            members.sort_by_key(|entry| {
                (
                    std::cmp::Reverse(entry.created_at),
                    std::cmp::Reverse(entry.id),
                )
            });
            if let Some(position) = members.iter().position(|entry| entry.id == root) {
                let root_entry = members.remove(position);
                members.push(root_entry);
            }
            let newest = members
                .iter()
                .map(|entry| entry.created_at)
                .max()
                .unwrap_or(0);
            (root, newest, members)
        })
        .collect();
    group_list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));

    let mut ordered = Vec::with_capacity(entries.len());
    let mut group_sizes: HashMap<i64, usize> = HashMap::new();
    let mut headers: HashMap<i64, i64> = HashMap::new();
    for (root, _, members) in group_list {
        group_sizes.insert(root, members.len());
        if let Some(header) = members.first() {
            headers.insert(root, header.id);
        }
        ordered.extend(members);
    }

    OrderedHistory {
        entries: ordered,
        roots,
        group_sizes,
        headers,
    }
}

/// Rebuild the sidebar list in chain order.
///
/// `ui.entries` is reordered so navigation and rendering agree. Descendants of
/// collapsed chain roots are skipped.
pub(crate) fn rebuild(ui: &Rc<Ui>) {
    rebuild_inner(ui, false);
}

/// Rebuild without jumping the sidebar back to the top.
pub(crate) fn rebuild_preserving_scroll(ui: &Rc<Ui>) {
    rebuild_inner(ui, true);
}

/// Collapse every chain that has more than one image. Used once at startup.
pub(crate) fn collapse_all_groups(ui: &Rc<Ui>) {
    let ordered = order_entries(ui.entries.borrow().clone());
    let mut collapsed = ui.collapsed_groups.borrow_mut();
    for (root, size) in &ordered.group_sizes {
        if *size > 1 {
            collapsed.insert(*root);
        }
    }
}

fn rebuild_inner(ui: &Rc<Ui>, preserve_scroll: bool) {
    let adjustment = if preserve_scroll {
        ui.history_list
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_then(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
            .map(|scroller| scroller.vadjustment())
    } else {
        None
    };
    let offset = adjustment
        .as_ref()
        .map(|adjustment| adjustment.value())
        .unwrap_or(0.0);

    while let Some(child) = ui.history_list.first_child() {
        ui.history_list.remove(&child);
    }

    let ordered = order_entries(ui.entries.borrow().clone());
    *ui.entries.borrow_mut() = ordered.entries.clone();

    let by_id: HashMap<i64, Generation> = ui
        .entries
        .borrow()
        .iter()
        .cloned()
        .map(|entry| (entry.id, entry))
        .collect();
    {
        let mut collapsed = ui.collapsed_groups.borrow_mut();
        collapsed.retain(|id| by_id.contains_key(id));
    }
    let collapsed = ui.collapsed_groups.borrow().clone();
    let expand_all = crate::history_filter_active(ui);
    for entry in ui.entries.borrow().iter() {
        if !entry_visible(entry, &ordered, &collapsed, expand_all) {
            continue;
        }
        let root = ordered.roots.get(&entry.id).copied().unwrap_or(entry.id);
        let header = ordered.headers.get(&root).copied().unwrap_or(root);
        let group_size = ordered.group_sizes.get(&root).copied().unwrap_or(1);
        let is_header = entry.id == header;
        let depth = if group_size > 1 && !is_header { 1 } else { 0 };
        let is_collapsed = !expand_all && is_header && group_size > 1 && collapsed.contains(&root);
        let toggle_root = (!expand_all && is_header && group_size > 1).then_some(root);
        let first_in_chain = group_size > 1 && entry.id == root;
        add_row(
            ui,
            entry,
            depth,
            group_size,
            is_collapsed,
            toggle_root,
            first_in_chain,
        );
    }

    if let Some(adjustment) = adjustment {
        // Restore immediately and again after the new rows have been laid out.
        adjustment.set_value(offset);
        glib::idle_add_local_once(move || {
            let max = (adjustment.upper() - adjustment.page_size()).max(0.0);
            adjustment.set_value(offset.min(max));
        });
    }
    if let Some(display) = ui.state.borrow().display.clone() {
        select_without_reveal(ui, &display);
    }
}

/// Whether an entry is rendered at all.
///
/// A collapsed chain shows only its newest row (the header); everything else
/// appears below it when expanded. Filtering forces every chain open.
fn entry_visible(
    entry: &Generation,
    ordered: &OrderedHistory,
    collapsed: &HashSet<i64>,
    expand_all: bool,
) -> bool {
    let root = ordered.roots.get(&entry.id).copied().unwrap_or(entry.id);
    let group_size = ordered.group_sizes.get(&root).copied().unwrap_or(1);
    if group_size <= 1 || expand_all {
        return true;
    }
    let header = ordered.headers.get(&root).copied().unwrap_or(root);
    entry.id == header || !collapsed.contains(&root)
}

pub(crate) fn add_row(
    ui: &Rc<Ui>,
    entry: &Generation,
    depth: usize,
    group_size: usize,
    collapsed: bool,
    toggle_root: Option<i64>,
    first_in_chain: bool,
) {
    let row = gtk::ListBoxRow::new();
    // Row activation resolves entries by id, never by position.
    row.set_widget_name(&entry.id.to_string());

    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row_box.set_margin_top(6);
    row_box.set_margin_bottom(6);
    row_box.set_margin_start(8 + depth as i32 * 16);
    row_box.set_margin_end(8);

    if depth > 0 {
        // Thin guide that makes a chain read as one block in the sidebar.
        let guide = gtk::Separator::new(gtk::Orientation::Vertical);
        guide.set_width_request(2);
        guide.set_margin_top(4);
        guide.set_margin_bottom(4);
        guide.add_css_class("history-guide");
        row_box.append(&guide);
    }

    let thumbnail = gtk::Picture::builder()
        .width_request(if depth > 0 { 44 } else { 56 })
        .height_request(if depth > 0 { 44 } else { 56 })
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Cover)
        .build();
    thumbnail.add_css_class("card");
    let thumb = thumb_path(entry.id);
    if thumb.exists()
        && let Ok(texture) = gdk::Texture::from_filename(&thumb)
    {
        thumbnail.set_paintable(Some(&texture));
    }

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let prompt_label = gtk::Label::builder()
        .label(&entry.prompt)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(18)
        .build();
    let mut meta = String::new();
    if first_in_chain {
        meta.push_str("first · ");
    }
    meta.push_str(&format!(
        "{} · {}×{}",
        entry_model_label(entry),
        entry.width,
        entry.height
    ));
    let meta_label = gtk::Label::builder()
        .label(meta)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(22)
        .css_classes(["caption", "dim-label"])
        .build();
    text.append(&prompt_label);
    text.append(&meta_label);

    let actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
    actions.set_valign(gtk::Align::Center);
    let favorite_button = gtk::Button::builder()
        .icon_name(if entry.favorite {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        })
        .tooltip_text(if entry.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        })
        .width_request(26)
        .height_request(26)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    let info_button = gtk::Button::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text("Details")
        .css_classes(["flat"])
        .build();
    let delete_button = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Delete image")
        .css_classes(["flat"])
        .build();
    if crate::history_entry_locked(ui, Path::new(&entry.image_path)) {
        delete_button.set_visible(false);
        delete_button.set_tooltip_text(Some("Used by a queued job"));
    }
    actions.append(&info_button);
    actions.append(&delete_button);

    row_box.append(&thumbnail);
    row_box.append(&text);
    row_box.append(&favorite_button);
    if let Some(root) = toggle_root {
        let toggle = gtk::Button::builder()
            .tooltip_text(if collapsed {
                "Expand chain"
            } else {
                "Collapse chain"
            })
            .valign(gtk::Align::Center)
            .css_classes(["history-toggle"])
            .build();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        let icon = gtk::Image::from_icon_name(if collapsed {
            "pan-end-symbolic"
        } else {
            "pan-down-symbolic"
        });
        icon.set_pixel_size(11);
        let count = gtk::Label::builder()
            .label(group_size.to_string())
            .css_classes(["caption"])
            .build();
        content.append(&icon);
        content.append(&count);
        toggle.set_child(Some(&content));
        {
            let ui = ui.clone();
            toggle.connect_clicked(move |_| {
                {
                    let mut collapsed = ui.collapsed_groups.borrow_mut();
                    if !collapsed.remove(&root) {
                        collapsed.insert(root);
                    }
                }
                // Rebuilding from inside the button's own signal is avoided by
                // deferring to the next idle iteration.
                let ui = ui.clone();
                glib::idle_add_local_once(move || rebuild_preserving_scroll(&ui));
            });
        }
        row_box.append(&toggle);
    }
    row_box.append(&actions);
    row.set_child(Some(&row_box));

    let mut tooltip = if kind_label(&entry.kind).is_some() {
        format!(
            "{}\n{} · {} × {} · {:.1}s{}",
            entry.prompt,
            entry_model_label(entry),
            entry.width,
            entry.height,
            entry.duration_ms as f64 / 1000.0,
            entry
                .source_path
                .as_deref()
                .map(|path| format!("\nsource: {path}"))
                .unwrap_or_default()
        )
    } else {
        format!(
            "{}\n{} · {} × {} · {} steps · {:.1}s · seed {}\nstyle: {}{}",
            entry.prompt,
            entry_model_label(entry),
            entry.width,
            entry.height,
            entry.steps,
            entry.duration_ms as f64 / 1000.0,
            entry.seed,
            style_label(&entry.style),
            match entry.reference_paths.len() {
                0 => String::new(),
                1 => format!("\nreference: {}", entry.reference_paths[0]),
                count => format!("\nreferences: {count} images"),
            }
        )
    };
    if group_size > 1 {
        tooltip.push_str(&format!("\nchain: {group_size} images"));
    }
    if first_in_chain {
        tooltip.push_str("\nfirst image in chain");
    }
    row.set_tooltip_text(Some(&tooltip));

    {
        let ui = ui.clone();
        let entry = entry.clone();
        favorite_button.connect_clicked(move |_| toggle_favorite(&ui, &entry));
    }
    {
        let ui = ui.clone();
        let entry = entry.clone();
        info_button.connect_clicked(move |_| show_info(&ui, &entry));
    }
    {
        let ui = ui.clone();
        let entry = entry.clone();
        delete_button.connect_clicked(move |_| confirm_delete(&ui, &entry));
    }

    ui.history_list.append(&row);
}

/// Restore the prompt, model, style, size and seed recorded with an entry.
pub(crate) fn restore_settings(ui: &Rc<Ui>, entry: &Generation) {
    if matches!(entry.kind.as_str(), "upscale" | "adjust" | "import") {
        return;
    }

    ui.prompt.buffer().set_text(&entry.prompt);
    ui.seed_entry.set_text(&entry.seed);

    if let Some(index) = ui
        .models
        .borrow()
        .iter()
        .position(|preset| preset.id == entry.model)
    {
        ui.model_dropdown.set_selected(index as u32);
    } else {
        ui.toast.add_toast(adw::Toast::new(&format!(
            "{} is not installed; using the selected model.",
            model_label(&entry.model)
        )));
    }
    if let Some(index) = STYLES.iter().position(|style| style.id == entry.style) {
        ui.style_dropdown.set_selected(index as u32);
    }

    set_size(ui, entry.width, entry.height);
}

pub(crate) fn kind_label(kind: &str) -> Option<&'static str> {
    match kind {
        "upscale" => Some("Upscaled"),
        "adjust" => Some("Adjusted"),
        "import" => Some("Opened"),
        "custom" => Some("Effect"),
        "colorize" => Some("Colorized"),
        "recover" => Some("Recovered"),
        "enhance" => Some("Enhanced"),
        "denoise" => Some("Denoised"),
        "photoreal" => Some("Photoreal"),
        "background" => Some("New background"),
        "night" => Some("Night"),
        "day" => Some("Daylight"),
        "expand" => Some("Expanded"),
        _ => None,
    }
}

pub(crate) fn status_label(entry: &Generation) -> String {
    let label = entry_model_label(entry);
    if let Some(action) = kind_label(&entry.kind) {
        format!("{action} · {label} · {} × {}", entry.width, entry.height)
    } else {
        format!("History · {label} · {} × {}", entry.width, entry.height)
    }
}

/// Show a history entry on the canvas with the image actions, without making
/// it the edit reference.
pub(crate) fn load_entry(ui: &Rc<Ui>, entry: &Generation) {
    crate::pause_live_view(ui);
    cancel_zoom(ui);
    restore_settings(ui, entry);

    ui.undo.borrow_mut().clear();
    ui.state
        .borrow_mut()
        .preview(PathBuf::from(&entry.image_path));
    update_image_view(ui);
    refresh_model(ui);
    show_composer(ui, false);
    ui.status.set_text(&status_label(entry));
}

/// Load a history entry as the edit reference and bring the composer back.
pub(crate) fn use_as_reference(ui: &Rc<Ui>, entry: &Generation) {
    crate::pause_live_view(ui);
    cancel_zoom(ui);
    restore_settings(ui, entry);

    ui.undo.borrow_mut().clear();
    ui.state.borrow_mut().load(PathBuf::from(&entry.image_path));
    update_image_view(ui);
    refresh_model(ui);
    show_composer(ui, true);

    let name = Path::new(&entry.image_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.image_path.clone());
    ui.status.set_text(&format!("Editing reference · {name}"));
}

fn toggle_favorite(ui: &Rc<Ui>, entry: &Generation) {
    let favorite = !entry.favorite;
    if let Err(error) = ui.history.borrow().set_favorite(entry.id, favorite) {
        tracing::warn!(%error, id = entry.id, "failed to update favorite");
        ui.toast
            .add_toast(adw::Toast::new("Could not update the favorite."));
        return;
    }
    crate::reload_history(ui, true);
    ui.toast.add_toast(adw::Toast::new(if favorite {
        "Added to favorites."
    } else {
        "Removed from favorites."
    }));
}

fn show_info(ui: &Rc<Ui>, entry: &Generation) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(4);
    content.set_margin_bottom(4);
    content.set_margin_start(4);
    content.set_margin_end(4);

    if entry.kind == "upscale" {
        content.append(&kv("Action", &entry.prompt));
        content.append(&kv("Upscaler", entry_model_label(entry)));
        content.append(&kv(
            "Size",
            &format!("{} × {} px", entry.width, entry.height),
        ));
        content.append(&kv(
            "Duration",
            &format!("{:.1} s", entry.duration_ms as f64 / 1000.0),
        ));
        content.append(&kv("Created", &created_label(entry.created_at)));
        if let Some(source) = entry.source_path.as_deref() {
            content.append(&kv("Source", source));
        }
        content.append(&kv("File", &entry.image_path));
    } else if entry.kind == "adjust" {
        content.append(&kv("Adjustments", &entry.prompt));
        content.append(&kv("Model", model_label(&entry.model)));
        content.append(&kv(
            "Size",
            &format!("{} × {} px", entry.width, entry.height),
        ));
        content.append(&kv(
            "Duration",
            &format!("{:.1} s", entry.duration_ms as f64 / 1000.0),
        ));
        content.append(&kv("Created", &created_label(entry.created_at)));
        if let Some(source) = entry.source_path.as_deref() {
            content.append(&kv("Source", source));
        }
        content.append(&kv("File", &entry.image_path));
    } else if entry.kind == "import" {
        content.append(&kv(
            "Size",
            &format!("{} × {} px", entry.width, entry.height),
        ));
        content.append(&kv("Created", &created_label(entry.created_at)));
        content.append(&kv("File", &entry.image_path));
    } else {
        content.append(&kv("Prompt", &entry.prompt));
        content.append(&kv("Model", model_label(&entry.model)));
        content.append(&kv("Style", style_label(&entry.style)));
        content.append(&kv("Effective prompt", &effective_prompt(entry)));
        content.append(&kv(
            "Size",
            &format!("{} × {} px", entry.width, entry.height),
        ));
        content.append(&kv(
            "Sampling",
            &format!(
                "{} steps · CFG {:.1} · {}",
                entry.steps,
                preset(&entry.model).map(|preset| preset.cfg).unwrap_or(1.0),
                preset(&entry.model)
                    .map(|preset| preset.sampler.as_str())
                    .unwrap_or("euler")
            ),
        ));
        content.append(&kv("Seed", &entry.seed));
        content.append(&kv(
            "Duration",
            &format!("{:.1} s", entry.duration_ms as f64 / 1000.0),
        ));
        content.append(&kv("Created", &created_label(entry.created_at)));
        if entry.kind == "generate" {
            for (index, reference) in entry.reference_paths.iter().enumerate() {
                let label = if entry.reference_paths.len() == 1 {
                    "Reference".to_string()
                } else {
                    format!("Reference {}", index + 1)
                };
                content.append(&kv(&label, reference));
            }
        } else if let Some(source) = entry.source_path.as_deref() {
            content.append(&kv("Source", source));
        }
        content.append(&kv("File", &entry.image_path));
    }

    let scroller = gtk::ScrolledWindow::builder()
        .child(&content)
        .min_content_width(760)
        .max_content_height(380)
        .propagate_natural_height(true)
        .build();

    let dialog = adw::AlertDialog::builder()
        .heading(match entry.kind.as_str() {
            "upscale" => "Upscale details",
            "adjust" => "Adjustment details",
            "colorize" => "Colorize details",
            "night" => "Night details",
            "day" => "Daylight details",
            "expand" => "Expand details",
            _ => "Image details",
        })
        .content_width(880)
        .extra_child(&scroller)
        .build();
    dialog.add_response("close", "Close");
    if entry.kind != "upscale" && entry.kind != "adjust" {
        dialog.add_response("copy", "Copy prompt");
    }
    dialog.add_response("use", "Use as reference");
    dialog.set_response_appearance("use", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));

    {
        let ui = ui.clone();
        let entry = entry.clone();
        dialog.connect_response(None, move |_, response| match response {
            "copy" => {
                ui.window.clipboard().set_text(&entry.prompt);
                ui.toast.add_toast(adw::Toast::new("Prompt copied."));
            }
            "use" => use_as_reference(&ui, &entry),
            _ => {}
        });
    }

    dialog.present(Some(&ui.window));
}

fn confirm_delete(ui: &Rc<Ui>, entry: &Generation) {
    let dialog = adw::AlertDialog::builder()
        .heading("Delete image?")
        .body(format!(
            "This permanently removes “{}” and its history entry.",
            snippet(&entry.prompt)
        ))
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));

    {
        let ui = ui.clone();
        let entry = entry.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                delete_entry(&ui, &entry);
            }
        });
    }

    dialog.present(Some(&ui.window));
}

fn delete_entry(ui: &Rc<Ui>, entry: &Generation) {
    if let Err(error) = ui.history.borrow().delete(entry.id) {
        ui.toast
            .add_toast(adw::Toast::new(&format!("Could not delete: {error}")));
        return;
    }

    if let Err(error) = std::fs::remove_file(&entry.image_path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = entry.image_path, "failed to remove image");
    }
    let _ = std::fs::remove_file(thumb_path(entry.id));

    ui.entries.borrow_mut().retain(|item| item.id != entry.id);
    crate::reload_history(ui, ui.generating.get() || ui.upscaling.get());

    let path = PathBuf::from(&entry.image_path);
    let is_active = {
        let state = ui.state.borrow();
        state.display.as_deref() == Some(path.as_path())
            || state.references.iter().any(|reference| reference == &path)
    };
    if is_active {
        ui.state.borrow_mut().new_image();
        update_image_view(ui);
        ui.status.set_text("Image deleted");
    } else {
        if let Some(display) = ui.state.borrow().display.clone() {
            highlight(ui, &display);
        }
        update_image_actions(ui);
    }

    ui.toast.add_toast(adw::Toast::new("Image deleted."));
}

fn row_by_id(list: &gtk::ListBox, id: i64) -> Option<gtk::ListBoxRow> {
    let wanted = id.to_string();
    let mut child = list.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
            && row.widget_name() == wanted
        {
            return Some(row);
        }
        child = next;
    }
    None
}

/// Highlight the history row for `path` without scrolling to it.
///
/// Reloads triggered by search, filters or favorites should not move the
/// sidebar while the user is reading it.
pub(crate) fn select_without_reveal(ui: &Rc<Ui>, path: &Path) {
    let id = ui
        .entries
        .borrow()
        .iter()
        .find(|entry| Path::new(&entry.image_path) == path)
        .map(|entry| entry.id);
    let Some(id) = id else {
        ui.history_list.unselect_all();
        return;
    };
    match row_by_id(&ui.history_list, id) {
        Some(row) => ui.history_list.select_row(Some(&row)),
        None => ui.history_list.unselect_all(),
    }
}

/// Highlight the history row for `path` and scroll it into view.
pub(crate) fn highlight(ui: &Rc<Ui>, path: &Path) {
    let id = ui
        .entries
        .borrow()
        .iter()
        .find(|entry| Path::new(&entry.image_path) == path)
        .map(|entry| entry.id);
    let Some(id) = id else {
        ui.history_list.unselect_all();
        return;
    };

    // The newest row of a collapsed chain is rendered as the header, so most
    // selections are already visible. Select it without expanding the chain.
    if let Some(row) = row_by_id(&ui.history_list, id) {
        ui.history_list.select_row(Some(&row));
        reveal_row(&ui.history_list, &row);
        return;
    }

    // An older hidden row cannot be selected: expand its chain (and any
    // collapsed ancestors) first.
    let mut expanded = false;
    {
        let entries = ui.entries.borrow();
        let mut collapsed = ui.collapsed_groups.borrow_mut();
        let mut current = entries.iter().find(|entry| entry.id == id);
        let mut guard = 0;
        while let Some(entry) = current {
            let Some(parent_id) = entry.parent_id else {
                break;
            };
            if collapsed.remove(&parent_id) {
                expanded = true;
            }
            current = entries.iter().find(|entry| entry.id == parent_id);
            guard += 1;
            if guard > entries.len() {
                break;
            }
        }
        // Selecting the chain root itself should also open the chain.
        if collapsed.remove(&id) {
            expanded = true;
        }
    }
    if expanded {
        rebuild(ui);
    }

    let Some(row) = row_by_id(&ui.history_list, id) else {
        return;
    };
    ui.history_list.select_row(Some(&row));
    reveal_row(&ui.history_list, &row);
}

/// Scroll a history row into view without moving keyboard focus.
fn reveal_row(list: &gtk::ListBox, row: &gtk::ListBoxRow) {
    let Some(scroller) = list
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_then(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
    else {
        return;
    };
    let Some(bounds) = row.compute_bounds(list) else {
        return;
    };
    let adjustment = scroller.vadjustment();
    let top = bounds.y() as f64;
    let bottom = top + bounds.height() as f64;
    let page = adjustment.page_size();
    let value = adjustment.value();
    if top < value {
        adjustment.set_value(top);
    } else if bottom > value + page {
        adjustment.set_value(bottom - page);
    }
}

fn kv(title: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let title = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(["caption", "dim-label"])
        .build();
    let value = gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    row.append(&title);
    row.append(&value);
    row
}

fn effective_prompt(entry: &Generation) -> String {
    flux_core::style::compose_prompt(&entry.prompt, &entry.style)
}

fn created_label(created_at: i64) -> String {
    glib::DateTime::from_unix_local(created_at)
        .and_then(|date| date.format("%Y-%m-%d %H:%M"))
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn snippet(prompt: &str) -> String {
    let mut text: String = prompt.chars().take(60).collect();
    if prompt.chars().count() > 60 {
        text.push('…');
    }
    text
}

fn model_label(id: &str) -> &str {
    preset(id).map(|model| model.label).unwrap_or(id)
}

pub(crate) fn entry_model_label(entry: &Generation) -> &str {
    if entry.kind == "import" {
        "Imported image"
    } else if entry.kind == "upscale" {
        upscaler(entry.model.as_str())
            .map(|model| model.label)
            .unwrap_or(entry.model.as_str())
    } else {
        model_label(&entry.model)
    }
}

fn style_label(id: &str) -> &str {
    flux_core::style::label(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, parent: Option<i64>, created_at: i64) -> Generation {
        Generation {
            id,
            prompt: format!("entry {id}"),
            model: "flux2-klein-4b".into(),
            style: "none".into(),
            width: 512,
            height: 512,
            steps: 4,
            seed: "1".into(),
            created_at,
            duration_ms: 1000,
            image_path: format!("/tmp/{id}.png"),
            reference_path: None,
            reference_paths: Vec::new(),
            kind: "generate".into(),
            source_path: None,
            parent_id: parent,
            favorite: false,
        }
    }

    fn ids(entries: &[Generation]) -> Vec<i64> {
        entries.iter().map(|entry| entry.id).collect()
    }

    #[test]
    fn groups_chains_newest_first_with_root_last() {
        let ordered = order_entries(vec![
            entry(3, Some(2), 300),
            entry(2, Some(1), 200),
            entry(1, None, 100),
            entry(4, None, 250),
        ]);
        assert_eq!(ids(&ordered.entries), vec![3, 2, 1, 4]);
        assert_eq!(ordered.group_sizes.get(&1), Some(&3));
        assert_eq!(ordered.group_sizes.get(&4), Some(&1));
        assert_eq!(ordered.headers.get(&1), Some(&3));
        assert_eq!(ordered.headers.get(&4), Some(&4));
    }

    #[test]
    fn newest_activity_moves_a_chain_up() {
        let ordered = order_entries(vec![
            entry(1, None, 100),
            entry(2, Some(1), 400),
            entry(3, None, 300),
        ]);
        assert_eq!(ids(&ordered.entries), vec![2, 1, 3]);
        assert_eq!(ordered.headers.get(&1), Some(&2));
    }

    #[test]
    fn missing_parent_starts_a_new_chain() {
        let ordered = order_entries(vec![entry(1, None, 100), entry(2, Some(99), 200)]);
        assert_eq!(ids(&ordered.entries), vec![2, 1]);
        assert_eq!(ordered.group_sizes.get(&2), Some(&1));
    }

    #[test]
    fn branches_are_newest_first_under_their_root() {
        let ordered = order_entries(vec![
            entry(4, Some(1), 400),
            entry(1, None, 100),
            entry(3, Some(1), 300),
            entry(2, Some(1), 200),
        ]);
        assert_eq!(ids(&ordered.entries), vec![4, 3, 2, 1]);
        assert_eq!(ordered.group_sizes.get(&1), Some(&4));
        assert_eq!(ordered.headers.get(&1), Some(&4));
    }

    fn visible_ids(
        ordered: &OrderedHistory,
        collapsed: &HashSet<i64>,
        expand_all: bool,
    ) -> Vec<i64> {
        ordered
            .entries
            .iter()
            .filter(|entry| entry_visible(entry, ordered, collapsed, expand_all))
            .map(|entry| entry.id)
            .collect()
    }

    #[test]
    fn collapsed_chains_show_only_their_newest_header() {
        let ordered = order_entries(vec![
            entry(1, None, 100),
            entry(2, Some(1), 200),
            entry(3, Some(2), 300),
            entry(4, None, 400),
        ]);
        assert_eq!(ids(&ordered.entries), vec![4, 3, 2, 1]);

        let collapsed: HashSet<i64> = [1].into_iter().collect();
        assert_eq!(visible_ids(&ordered, &collapsed, false), vec![4, 3]);
        assert_eq!(
            visible_ids(&ordered, &collapsed, true),
            vec![4, 3, 2, 1],
            "filters force chains open"
        );

        let collapsed: HashSet<i64> = HashSet::new();
        assert_eq!(visible_ids(&ordered, &collapsed, false), vec![4, 3, 2, 1]);
    }

    #[test]
    fn labels_imported_images() {
        let mut imported = entry(1, None, 100);
        imported.kind = "import".into();
        imported.model = "import".into();
        assert_eq!(kind_label("import"), Some("Opened"));
        assert_eq!(entry_model_label(&imported), "Imported image");
    }
}
