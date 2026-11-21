use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use glib::types::StaticType;
use gtk::{gdk, glib};
use imagiflume_core::preset::preset;
use imagiflume_core::style::STYLES;
use imagiflume_core::upscaler::upscaler;

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

/// Fold a chain; its expander row animates shut.
pub(crate) fn collapse_group(ui: &Rc<Ui>, root: i64) {
    set_group_collapsed(ui, root, true);
}

fn set_group_collapsed(ui: &Rc<Ui>, root: i64, collapsed: bool) {
    if let Some(expander) = ui.chain_expanders.borrow().get(&root).cloned() {
        if expander.is_expanded() != !collapsed {
            expander.set_expanded(!collapsed);
        }
        return;
    }
    // No expander is rendered (for example while a filter forces chains open);
    // remember the state for the next rebuild.
    let mut groups = ui.collapsed_groups.borrow_mut();
    if collapsed {
        groups.insert(root);
    } else {
        groups.remove(&root);
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
    ui.chain_expanders.borrow_mut().clear();
    ui.chain_toggles.borrow_mut().clear();
    ui.chain_headers.borrow_mut().clear();
    ui.chain_child_rows.borrow_mut().clear();

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

    // Chain children are added to their header's expander row, which animates
    // them open and shut itself.
    let mut chain: Option<(i64, adw::ExpanderRow)> = None;

    ui.expander_guard.set(true);
    for entry in ui.entries.borrow().iter() {
        let root = ordered.roots.get(&entry.id).copied().unwrap_or(entry.id);
        let header = ordered.headers.get(&root).copied().unwrap_or(root);
        let group_size = ordered.group_sizes.get(&root).copied().unwrap_or(1);
        let is_header = entry.id == header;
        let depth = if group_size > 1 && !is_header { 1 } else { 0 };
        let first_in_chain = group_size > 1 && entry.id == root;
        let group = RowGroup {
            group_size,
            first_in_chain,
        };

        let chain_child = !expand_all && group_size > 1 && !is_header;
        if chain_child {
            let Some((open_root, expander)) = &chain else {
                continue;
            };
            debug_assert_eq!(*open_root, root);
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&entry.id.to_string());
            row.set_child(Some(&build_row_box(ui, entry, depth, group)));
            expander.add_row(&row);
            ui.chain_child_rows
                .borrow_mut()
                .entry(root)
                .or_default()
                .push((entry.id, row));
        } else if !expand_all && group_size > 1 {
            // Chain header, rendered as an expander row.
            let expander =
                build_chain_header(ui, entry, group_size, collapsed.contains(&root), root);
            ui.history_list.append(&expander);
            chain = Some((root, expander));
        } else {
            chain = None;
            let row = gtk::ListBoxRow::new();
            // Row activation resolves entries by id, never by position.
            row.set_widget_name(&entry.id.to_string());
            row.set_child(Some(&build_row_box(ui, entry, depth, group)));
            ui.history_list.append(&row);
        }
    }
    ui.expander_guard.set(false);

    // Rows that are no longer rendered must not stay selected, or a bulk delete
    // would remove images the user cannot see.
    if ui.history_selection_mode.get() {
        let visible: HashSet<i64> = visible_ids(ui).into_iter().collect();
        ui.history_selected
            .borrow_mut()
            .retain(|id| visible.contains(id));
        crate::update_selection_ui(ui);
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
#[cfg(test)]
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

struct RowGroup {
    group_size: usize,
    first_in_chain: bool,
}

/// Build the content of one history row: checkbox, chain guide, thumbnail,
/// prompt and meta labels, favorite/info/delete actions.
///
/// Chain headers use [`build_chain_header`] instead; this is for standalone
/// rows and for rows inside a chain.
fn build_row_box(ui: &Rc<Ui>, entry: &Generation, depth: usize, group: RowGroup) -> gtk::Box {
    let group_size = group.group_size;
    let first_in_chain = group.first_in_chain;

    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if depth > 0 {
        // Tighter rows inside a chain so the group reads as one block.
        row_box.set_margin_top(3);
        row_box.set_margin_bottom(3);
        row_box.add_css_class("chain-child");
    } else {
        row_box.set_margin_top(6);
        row_box.set_margin_bottom(6);
    }
    row_box.set_margin_start(8 + depth as i32 * 8);
    row_box.set_margin_end(8);

    let selecting = ui.history_selection_mode.get();
    if selecting {
        let check = gtk::CheckButton::new();
        check.set_active(ui.history_selected.borrow().contains(&entry.id));
        check.set_valign(gtk::Align::Center);
        check.set_tooltip_text(Some("Select this image"));
        {
            let ui = ui.clone();
            let id = entry.id;
            check.connect_toggled(move |check| {
                crate::set_history_selected(&ui, id, check.is_active());
            });
        }
        row_box.append(&check);
    }

    if depth > 0 {
        // Thin guide that makes a chain read as one block in the sidebar.
        let guide = gtk::Separator::new(gtk::Orientation::Vertical);
        guide.set_width_request(2);
        guide.set_margin_top(4);
        guide.set_margin_bottom(4);
        guide.add_css_class("history-guide");
        row_box.append(&guide);
    }

    let thumbnail = history_thumbnail(entry, depth);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let prompt_label = gtk::Label::builder()
        .label(&entry.prompt)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(18)
        .build();
    let meta_label = gtk::Label::builder()
        .label(row_meta(entry, first_in_chain))
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(22)
        .css_classes(["caption", "dim-label"])
        .build();
    text.append(&prompt_label);
    text.append(&meta_label);

    let favorite_button = favorite_button(ui, entry, selecting);
    let actions = actions_box(ui, entry, selecting);

    row_box.append(&thumbnail);
    row_box.append(&text);
    row_box.append(&favorite_button);
    row_box.append(&actions);

    row_box.set_tooltip_text(Some(&row_tooltip(entry, group_size, first_in_chain)));

    {
        let ui = ui.clone();
        let entry = entry.clone();
        favorite_button.connect_clicked(move |_| toggle_favorite(&ui, &entry));
    }
    connect_row_actions(ui, entry, &actions);

    // Rows inside a chain are not direct list rows, so they handle clicks
    // themselves.
    if depth > 0 {
        let ui = ui.clone();
        let entry = entry.clone();
        let row = row_box.clone();
        let gesture = gtk::GestureClick::new();
        gesture.connect_released(move |gesture, _, x, y| {
            // Checkboxes and action buttons handle their own clicks.
            if let Some(picked) = row.pick(x, y, gtk::PickFlags::DEFAULT)
                && inside_interactive(&picked)
            {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            select_chain_header(&ui, &entry);
        });
        row_box.add_controller(gesture);
    }

    row_box
}

/// Build a chain header as an expander row. Its child rows are added to it by
/// the caller, and libadwaita animates the expansion.
fn build_chain_header(
    ui: &Rc<Ui>,
    entry: &Generation,
    group_size: usize,
    collapsed: bool,
    root: i64,
) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::builder()
        .title(&entry.prompt)
        .subtitle(row_meta(entry, false))
        .title_lines(1)
        .expanded(!collapsed)
        .build();
    expander.set_widget_name(&entry.id.to_string());
    expander.set_tooltip_text(Some(&row_tooltip(entry, group_size, false)));
    // The expander's inner AdwActionRow toggles expansion whenever it is
    // activated, so make it inert and use our own click handling: clicking the
    // header opens the image, clicking the arrow area expands the chain.
    expander.set_activatable(false);
    if let Some(header) = expander_header_row(&expander) {
        header.set_activatable(false);
    }

    {
        let ui = ui.clone();
        let entry = entry.clone();
        let row = expander.clone();
        // 0 = ignore, 1 = select the image, 2 = toggle the chain.
        let action = Rc::new(std::cell::Cell::new(0u8));
        let gesture = gtk::GestureClick::new();
        // Run before the header list box, which claims clicks for row
        // activation, so the header can be selected without expanding.
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let row = row.clone();
            let action = action.clone();
            gesture.connect_pressed(move |gesture, _, x, y| {
                let Some(picked) = row.pick(x, y, gtk::PickFlags::DEFAULT) else {
                    action.set(0);
                    return;
                };
                // The expander's child rows and its buttons handle their clicks.
                if inside_expander_children(&picked) || inside_interactive(&picked) {
                    action.set(0);
                    return;
                }
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let on_arrow = is_expander_arrow(&picked) || x >= row.width() as f64 - 36.0;
                action.set(if on_arrow { 2 } else { 1 });
            });
        }
        gesture.connect_released(move |_, _, _, _| match action.replace(0) {
            1 => select_chain_header(&ui, &entry),
            2 => row.set_expanded(!row.is_expanded()),
            _ => {}
        });
        expander.add_controller(gesture);
    }
    {
        let ui = ui.clone();
        let entry = entry.clone();
        let row = expander.clone();
        let key = gtk::EventControllerKey::new();
        key.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Return | gdk::Key::KP_Enter => {
                select_chain_header(&ui, &entry);
                glib::Propagation::Stop
            }
            gdk::Key::space => {
                row.set_expanded(!row.is_expanded());
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        expander.add_controller(key);
    }

    let selecting = ui.history_selection_mode.get();
    if selecting {
        let check = gtk::CheckButton::new();
        check.set_active(ui.history_selected.borrow().contains(&entry.id));
        check.set_valign(gtk::Align::Center);
        check.set_tooltip_text(Some("Select this image"));
        {
            let ui = ui.clone();
            let id = entry.id;
            check.connect_toggled(move |check| {
                crate::set_history_selected(&ui, id, check.is_active());
            });
        }
        expander.add_prefix(&check);
    }
    expander.add_prefix(&history_thumbnail(entry, 0));

    // Replace libadwaita's bare arrow with one count/arrow pill.
    if let Some(arrow) = crate::find_widget_by_css(expander.upcast_ref(), "expander-row-arrow") {
        arrow.set_visible(false);
    }
    // Suffixes are prepended, so add in reverse visual order: the action column
    // stays flush right like standalone rows, then the star, then the pill.
    let actions = actions_box(ui, entry, selecting);
    expander.add_suffix(&actions);
    let favorite = favorite_button(ui, entry, selecting);
    expander.add_suffix(&favorite);
    let toggle = chain_toggle_button(group_size, collapsed);
    ui.chain_toggles.borrow_mut().insert(root, toggle.clone());
    expander.add_suffix(&toggle.0);
    {
        let row = expander.clone();
        toggle
            .0
            .connect_clicked(move |_| row.set_expanded(!row.is_expanded()));
    }

    {
        let ui = ui.clone();
        let entry = entry.clone();
        favorite.connect_clicked(move |_| toggle_favorite(&ui, &entry));
    }
    connect_row_actions(ui, entry, &actions);

    ui.chain_headers.borrow_mut().insert(entry.id, root);
    ui.chain_expanders
        .borrow_mut()
        .insert(root, expander.clone());

    {
        let ui = ui.clone();
        expander.connect_expanded_notify(move |expander| {
            if ui.expander_guard.get() {
                return;
            }
            if let Some((button, icon)) = ui.chain_toggles.borrow().get(&root) {
                icon.set_icon_name(Some(if expander.is_expanded() {
                    "pan-down-symbolic"
                } else {
                    "pan-end-symbolic"
                }));
                button.set_tooltip_text(Some(if expander.is_expanded() {
                    "Collapse chain"
                } else {
                    "Expand chain"
                }));
            }
            if expander.is_expanded() {
                ui.collapsed_groups.borrow_mut().remove(&root);
                return;
            }
            ui.collapsed_groups.borrow_mut().insert(root);
            // Folded rows must not stay selected, or a later bulk delete would
            // remove images the user can no longer see.
            let removed = {
                let children = ui
                    .chain_child_rows
                    .borrow()
                    .get(&root)
                    .cloned()
                    .unwrap_or_default();
                let mut selected = ui.history_selected.borrow_mut();
                children
                    .iter()
                    .filter(|(id, _)| selected.remove(id))
                    .count()
            };
            if removed > 0 {
                crate::update_selection_ui(&ui);
            }
        });
    }

    expander
}

fn history_thumbnail(entry: &Generation, depth: usize) -> gtk::Picture {
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
    thumbnail
}

fn favorite_button(ui: &Rc<Ui>, entry: &Generation, selecting: bool) -> gtk::Button {
    let button = gtk::Button::builder()
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
    button.set_visible(!selecting);
    let _ = ui;
    button
}

fn actions_box(ui: &Rc<Ui>, entry: &Generation, selecting: bool) -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
    actions.set_valign(gtk::Align::Center);
    actions.set_visible(!selecting);
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
    actions
}

/// Wire the info and delete buttons inside a row's action column.
fn connect_row_actions(ui: &Rc<Ui>, entry: &Generation, actions: &gtk::Box) {
    let mut child = actions.first_child();
    let mut index = 0;
    while let Some(widget) = child {
        child = widget.next_sibling();
        let Ok(button) = widget.downcast::<gtk::Button>() else {
            continue;
        };
        let ui = ui.clone();
        let entry = entry.clone();
        match index {
            0 => {
                button.connect_clicked(move |_| show_info(&ui, &entry));
            }
            _ => {
                button.connect_clicked(move |_| confirm_delete(&ui, &entry));
            }
        }
        index += 1;
    }
}

fn row_meta(entry: &Generation, first_in_chain: bool) -> String {
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
    meta
}

fn row_tooltip(entry: &Generation, group_size: usize, first_in_chain: bool) -> String {
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
    tooltip
}

/// The count/arrow pill on a chain header: count, chevron, and the icon used
/// to update the chevron when the chain expands.
fn chain_toggle_button(group_size: usize, collapsed: bool) -> (gtk::Button, gtk::Image) {
    let button = gtk::Button::builder()
        .tooltip_text(if collapsed {
            "Expand chain"
        } else {
            "Collapse chain"
        })
        .valign(gtk::Align::Center)
        .css_classes(["chain-toggle"])
        .build();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    content.set_valign(gtk::Align::Center);
    let count = gtk::Label::builder()
        .label(group_size.to_string())
        .css_classes(["caption"])
        .build();
    let icon = gtk::Image::from_icon_name(if collapsed {
        "pan-end-symbolic"
    } else {
        "pan-down-symbolic"
    });
    icon.set_pixel_size(11);
    content.append(&count);
    content.append(&icon);
    button.set_child(Some(&content));
    (button, icon)
}

/// The `AdwActionRow` embedded in an expander's header list.
fn expander_header_row(expander: &adw::ExpanderRow) -> Option<gtk::ListBoxRow> {
    expander
        .first_child()?
        .first_child()?
        .first_child()?
        .downcast::<gtk::ListBoxRow>()
        .ok()
}

/// Whether a widget is libadwaita's expander arrow.
fn is_expander_arrow(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class("expander-row-arrow") {
            return true;
        }
        current = widget.parent();
    }
    false
}

/// Open a chain header's image, or toggle its checkbox in selection mode.
fn select_chain_header(ui: &Rc<Ui>, entry: &Generation) {
    if ui.history_selection_mode.get() {
        let selected = !ui.history_selected.borrow().contains(&entry.id);
        crate::set_history_selected(ui, entry.id, selected);
        rebuild_preserving_scroll(ui);
        return;
    }
    load_entry(ui, entry);
}

/// Whether a widget is inside the expander's revealed child area (rather than
/// its header).
fn inside_expander_children(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.downcast_ref::<gtk::Revealer>().is_some() {
            return true;
        }
        current = widget.parent();
    }
    false
}

/// Whether a widget or one of its ancestors handles clicks itself.
fn inside_interactive(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.downcast_ref::<gtk::CheckButton>().is_some()
            || widget.downcast_ref::<gtk::Button>().is_some()
        {
            return true;
        }
        current = widget.parent();
    }
    false
}

/// Restore the model, style, size and seed recorded with an entry.
///
/// The prompt is deliberately left alone: the composer starts empty and past
/// prompts are loaded from the prompt picker.
pub(crate) fn restore_settings(ui: &Rc<Ui>, entry: &Generation) {
    if matches!(entry.kind.as_str(), "upscale" | "adjust" | "import") {
        return;
    }

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
    crate::reload_history(ui, true);

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
            select_without_reveal(ui, &display);
        }
        update_image_actions(ui);
    }

    ui.toast.add_toast(adw::Toast::new("Image deleted."));
}

/// The widget a history entry is rendered as. Chain headers and standalone
/// rows are list rows; entries inside a chain are rows owned by its expander.
fn find_entry_widget(ui: &Ui, id: i64) -> Option<gtk::Widget> {
    let wanted = id.to_string();
    let mut child = ui.history_list.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        let Ok(list_row) = row.downcast::<gtk::ListBoxRow>() else {
            continue;
        };
        if list_row.widget_name() == wanted {
            return Some(list_row.upcast());
        }
        let Ok(header_id) = list_row.widget_name().parse::<i64>() else {
            continue;
        };
        let Some(root) = ui.chain_headers.borrow().get(&header_id).copied() else {
            continue;
        };
        if let Some(children) = ui.chain_child_rows.borrow().get(&root)
            && let Some((_, child_row)) = children.iter().find(|(child_id, _)| *child_id == id)
        {
            return Some(child_row.clone().upcast());
        }
    }
    None
}

/// Whether an entry is currently unfolded in the sidebar.
fn entry_shown(ui: &Ui, id: i64) -> bool {
    let children = ui.chain_child_rows.borrow();
    for (root, rows) in children.iter() {
        if rows.iter().any(|(child_id, _)| *child_id == id) {
            return ui
                .chain_expanders
                .borrow()
                .get(root)
                .is_some_and(|expander| expander.is_expanded());
        }
    }
    drop(children);
    find_entry_widget(ui, id).is_some()
}

/// Ids of the entries currently unfolded in the sidebar.
pub(crate) fn visible_ids(ui: &Ui) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut child = ui.history_list.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        let Ok(list_row) = row.downcast::<gtk::ListBoxRow>() else {
            continue;
        };
        let Ok(id) = list_row.widget_name().parse::<i64>() else {
            continue;
        };
        ids.push(id);
        let Some(root) = ui.chain_headers.borrow().get(&id).copied() else {
            continue;
        };
        let expanded = ui
            .chain_expanders
            .borrow()
            .get(&root)
            .is_some_and(|expander| expander.is_expanded());
        if expanded && let Some(children) = ui.chain_child_rows.borrow().get(&root) {
            ids.extend(children.iter().map(|(child_id, _)| *child_id));
        }
    }
    ids
}

/// Drop the manual selection highlight from every chain child.
fn clear_child_selection(ui: &Ui) {
    for rows in ui.chain_child_rows.borrow().values() {
        for (_, row) in rows {
            row.remove_css_class("history-child-selected");
        }
    }
}

/// Highlight the history entry for `path` without scrolling to it.
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
        clear_selection(ui);
        return;
    };
    if !entry_shown(ui, id) {
        clear_selection(ui);
        return;
    }
    select_entry(ui, id);
}

/// Clear the list selection and any manual chain-child highlight.
pub(crate) fn clear_selection(ui: &Rc<Ui>) {
    clear_child_selection(ui);
    ui.history_list.unselect_all();
}

fn select_entry(ui: &Rc<Ui>, id: i64) {
    let Some(widget) = find_entry_widget(ui, id) else {
        return;
    };
    clear_selection(ui);
    if let Ok(row) = widget.clone().downcast::<gtk::ListBoxRow>() {
        // Only direct list rows can carry the list selection; rows inside an
        // expander get the manual highlight instead.
        let list: gtk::Widget = ui.history_list.clone().upcast();
        if row.parent().is_some_and(|parent| parent == list) {
            ui.history_list.select_row(Some(&row));
        } else {
            row.add_css_class("history-child-selected");
        }
    }
}

/// Highlight the history entry for `path` and scroll it into view.
pub(crate) fn highlight(ui: &Rc<Ui>, path: &Path) {
    let id = ui
        .entries
        .borrow()
        .iter()
        .find(|entry| Path::new(&entry.image_path) == path)
        .map(|entry| entry.id);
    let Some(id) = id else {
        clear_selection(ui);
        return;
    };

    // The newest row of a collapsed chain is visible as the header, so most
    // selections are already in place.
    if entry_shown(ui, id) {
        select_and_reveal(ui, id);
        return;
    }

    // Expand the chain (and any collapsed ancestors) and let the expander
    // finish animating before scrolling the target into view.
    let mut roots = Vec::new();
    {
        let entries = ui.entries.borrow();
        let collapsed = ui.collapsed_groups.borrow();
        let mut current = entries.iter().find(|entry| entry.id == id);
        let mut guard = 0;
        while let Some(entry) = current {
            let Some(parent_id) = entry.parent_id else {
                break;
            };
            if collapsed.contains(&parent_id) {
                roots.push(parent_id);
            }
            current = entries.iter().find(|entry| entry.id == parent_id);
            guard += 1;
            if guard > entries.len() {
                break;
            }
        }
        // Selecting the chain root itself should also open the chain.
        if collapsed.contains(&id) {
            roots.push(id);
        }
    }
    for root in roots {
        set_group_collapsed(ui, root, false);
    }

    if entry_shown(ui, id) {
        let ui = ui.clone();
        let path = path.to_path_buf();
        glib::timeout_add_local_once(Duration::from_millis(240), move || {
            // A newer navigation may have moved the canvas elsewhere while the
            // chain was opening; only follow the image that is still shown.
            let current = ui.state.borrow().display.clone();
            if current.as_deref() == Some(path.as_path()) {
                select_and_reveal(&ui, id);
            }
        });
    } else {
        // Every row is built, so this is only a fallback.
        select_and_reveal(ui, id);
    }
}

fn select_and_reveal(ui: &Rc<Ui>, id: i64) {
    let Some(widget) = find_entry_widget(ui, id) else {
        return;
    };
    select_entry(ui, id);
    reveal_row(&ui.history_list, &widget);
}

/// Scroll a history entry into view without moving keyboard focus.
fn reveal_row(list: &gtk::ListBox, widget: &gtk::Widget) {
    let Some(scroller) = list
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_then(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
    else {
        return;
    };
    let Some(bounds) = widget.compute_bounds(list) else {
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
    imagiflume_core::style::compose_prompt(&entry.prompt, &entry.style)
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
    imagiflume_core::style::label(id)
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
