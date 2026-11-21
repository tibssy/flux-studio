use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use imagiflume_core::config::CustomEffect;
use imagiflume_core::effect_pack;

use crate::history::Generation;
use crate::main_view::effect_item;
use crate::{GenerationMode, Ui, start_effect};

/// Prompt that the save-effect button should capture for `entry`.
///
/// Only prompt-based edits qualify: text-to-image results have no reference,
/// and upscales or built-in effects are already represented in the menu.
pub(crate) fn savable_prompt(entry: &Generation) -> Option<String> {
    if entry.kind != "generate" || entry.reference_path.is_none() {
        return None;
    }
    let prompt = imagiflume_core::style::compose_prompt(&entry.prompt, &entry.style);
    (!prompt.trim().is_empty()).then_some(prompt)
}

/// Prompt for the image currently on screen, when it can be saved.
pub(crate) fn current_savable_prompt(ui: &Rc<Ui>) -> Option<String> {
    let display = ui.state.borrow().display.clone()?;
    let entries = ui.entries.borrow();
    entries
        .iter()
        .find(|entry| Path::new(&entry.image_path) == display.as_path())
        .and_then(savable_prompt)
}

/// Ask for a name and store the current image's prompt as a user effect.
pub(crate) fn show_save_dialog(ui: &Rc<Ui>) {
    let Some(prompt) = current_savable_prompt(ui) else {
        ui.toast
            .add_toast(adw::Toast::new("This image was not made with a prompt."));
        return;
    };
    show_editor(ui, None, &default_name(&prompt), &prompt);
}

fn show_edit_dialog(ui: &Rc<Ui>, index: usize) {
    let effect = ui.config.borrow().custom_effects.get(index).cloned();
    let Some(effect) = effect else {
        return;
    };
    show_editor(ui, Some(index), &effect.name, &effect.prompt);
}

fn show_editor(ui: &Rc<Ui>, editing: Option<usize>, initial_name: &str, initial_prompt: &str) {
    let editing_effect = editing.is_some();
    let title = if editing_effect {
        "Edit effect"
    } else {
        "Save effect"
    };
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(480)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        title,
        if editing_effect {
            "Change the name or prompt"
        } else {
            "Reuse this prompt later"
        },
    )));
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::builder()
        .label("Save")
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&save);
    header.pack_end(&cancel);
    toolbar.add_top_bar(&header);

    let page = adw::PreferencesPage::new();

    let details = adw::PreferencesGroup::builder().title("Effect").build();
    let name_row = adw::EntryRow::builder()
        .title("Name")
        .max_length(64)
        .text(initial_name)
        .build();
    let duplicate = gtk::Label::builder()
        .label("An effect with this name already exists.")
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .css_classes(["caption", "error"])
        .build();
    details.add(&name_row);
    details.add(&duplicate);
    page.add(&details);

    let prompt_group = adw::PreferencesGroup::builder().title("Prompt").build();
    let prompt_view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    prompt_view.buffer().set_text(initial_prompt);
    let prompt_scroller = gtk::ScrolledWindow::builder()
        .child(&prompt_view)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(90)
        .max_content_height(180)
        .propagate_natural_height(true)
        .build();
    let prompt_field = gtk::Box::new(gtk::Orientation::Vertical, 0);
    prompt_field.add_css_class("prompt-field");
    prompt_field.append(&prompt_scroller);
    prompt_group.add(&prompt_field);
    page.add(&prompt_group);

    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let refresh: Rc<dyn Fn()> = {
        let ui = ui.clone();
        let name_row = name_row.clone();
        let prompt_view = prompt_view.clone();
        let save = save.clone();
        let duplicate = duplicate.clone();
        Rc::new(move || {
            let name = name_row.text().trim().to_string();
            let taken = !name.is_empty() && name_taken(&ui, &name, editing);
            duplicate.set_visible(taken);
            save.set_sensitive(!name.is_empty() && !taken && !prompt_text(&prompt_view).is_empty());
        })
    };
    {
        let refresh = refresh.clone();
        name_row.connect_changed(move |_| refresh());
    }
    {
        let refresh = refresh.clone();
        prompt_view.buffer().connect_changed(move |_| refresh());
    }
    refresh();

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let ui = ui.clone();
        let dialog = dialog.clone();
        let name_row = name_row.clone();
        let prompt_view = prompt_view.clone();
        save.connect_clicked(move |_| {
            let name = name_row.text().trim().to_string();
            let prompt = prompt_text(&prompt_view);
            if name.is_empty() || prompt.is_empty() || name_taken(&ui, &name, editing) {
                return;
            }
            {
                let mut config = ui.config.borrow_mut();
                let previous = match editing {
                    Some(index) => {
                        if index >= config.custom_effects.len() {
                            return;
                        }
                        Some((
                            index,
                            std::mem::replace(
                                &mut config.custom_effects[index],
                                CustomEffect {
                                    name: name.clone(),
                                    prompt: prompt.clone(),
                                },
                            ),
                        ))
                    }
                    None => {
                        config.custom_effects.push(CustomEffect {
                            name: name.clone(),
                            prompt: prompt.clone(),
                        });
                        None
                    }
                };
                if let Err(error) = config.save() {
                    match previous {
                        Some((index, effect)) => config.custom_effects[index] = effect,
                        None => {
                            config.custom_effects.pop();
                        }
                    }
                    tracing::warn!(%error, "failed to save custom effects");
                    ui.toast
                        .add_toast(adw::Toast::new("Could not save the effect."));
                    return;
                }
            }
            rebuild_menu(&ui);
            let action = if editing_effect { "updated" } else { "saved" };
            ui.toast
                .add_toast(adw::Toast::new(&format!("Effect “{name}” {action}.")));
            dialog.close();
        });
    }

    dialog.present(Some(&ui.window));
    if editing_effect {
        prompt_view.grab_focus();
    } else {
        name_row.select_region(0, -1);
        name_row.grab_focus();
    }
}

/// Wire the import/export buttons in the effects popover.
pub(crate) fn install_tools(ui: &Rc<Ui>) {
    {
        let ui = ui.clone();
        ui.effects_import.clone().connect_clicked(move |_| {
            ui.effects_popover.popdown();
            import_effects(&ui);
        });
    }
    {
        let ui = ui.clone();
        ui.effects_export.clone().connect_clicked(move |_| {
            ui.effects_popover.popdown();
            export_effects(&ui);
        });
    }
}

fn export_effects(ui: &Rc<Ui>) {
    let effects = ui.config.borrow().custom_effects.clone();
    if effects.is_empty() {
        ui.toast
            .add_toast(adw::Toast::new("There are no effects to export."));
        return;
    }

    let dialog = gtk::FileDialog::builder()
        .title("Export effects")
        .modal(true)
        .build();
    dialog.set_initial_name(Some("imagiflume-effects.json"));
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Imagiflume effects"));
    filter.add_pattern("*.json");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    let ui = ui.clone();
    let window = ui.window.clone();
    dialog.save(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            ui.toast
                .add_toast(adw::Toast::new("Could not save the effects file."));
            return;
        };
        let text = match effect_pack::to_json(&effects) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(%error, "failed to serialize effects");
                ui.toast
                    .add_toast(adw::Toast::new("Could not write the effects file."));
                return;
            }
        };
        if let Err(error) = std::fs::write(&path, text) {
            tracing::warn!(%error, path = %path.display(), "failed to write effects");
            ui.toast
                .add_toast(adw::Toast::new("Could not write the effects file."));
            return;
        }
        ui.toast.add_toast(adw::Toast::new(&format!(
            "Exported {} effect{}.",
            effects.len(),
            if effects.len() == 1 { "" } else { "s" }
        )));
    });
}

fn import_effects(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title("Import effects")
        .modal(true)
        .build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Imagiflume effects"));
    filter.add_pattern("*.json");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    let ui = ui.clone();
    let window = ui.window.clone();
    dialog.open(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            ui.toast
                .add_toast(adw::Toast::new("Could not read that file."));
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "failed to read effects");
                ui.toast
                    .add_toast(adw::Toast::new("Could not read that file."));
                return;
            }
        };
        let incoming = match effect_pack::from_json(&text) {
            Ok(effects) => effects,
            Err(message) => {
                ui.toast.add_toast(adw::Toast::new(&message));
                return;
            }
        };

        let report = {
            let mut config = ui.config.borrow_mut();
            let original = config.custom_effects.clone();
            let report = effect_pack::merge(&mut config.custom_effects, incoming);
            if let Err(error) = config.save() {
                config.custom_effects = original;
                tracing::warn!(%error, "failed to save imported effects");
                None
            } else {
                Some(report)
            }
        };
        match report {
            Some(report) => {
                rebuild_menu(&ui);
                ui.toast.add_toast(adw::Toast::new(&report.summary()));
            }
            None => ui
                .toast
                .add_toast(adw::Toast::new("Could not save the imported effects.")),
        }
    });
}

/// Rebuild the "Your effects" section from the config.
pub(crate) fn rebuild_menu(ui: &Rc<Ui>) {
    while let Some(child) = ui.custom_effects_box.first_child() {
        ui.custom_effects_box.remove(&child);
    }

    let effects = ui.config.borrow().custom_effects.clone();
    ui.custom_effects_section.set_visible(!effects.is_empty());
    ui.effects_export.set_sensitive(!effects.is_empty());

    for (index, effect) in effects.iter().enumerate() {
        let button = effect_item(
            "user-bookmarks-symbolic",
            &effect.name,
            &snippet(&effect.prompt),
        );
        {
            let ui = ui.clone();
            let prompt = effect.prompt.clone();
            button.connect_clicked(move |_| {
                ui.effects_popover.popdown();
                start_effect(
                    &ui,
                    GenerationMode::Custom,
                    &prompt,
                    "Open an image to apply this effect.",
                );
            });
        }

        let edit = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Edit effect")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        {
            let ui = ui.clone();
            edit.connect_clicked(move |_| {
                ui.effects_popover.popdown();
                show_edit_dialog(&ui, index);
            });
        }

        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete effect")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        {
            let ui = ui.clone();
            remove.connect_clicked(move |_| delete_effect(&ui, index));
        }

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        button.set_hexpand(true);
        row.append(&button);
        row.append(&edit);
        row.append(&remove);
        ui.custom_effects_box.append(&row);
    }
}

fn delete_effect(ui: &Rc<Ui>, index: usize) {
    let removed = {
        let mut config = ui.config.borrow_mut();
        if index >= config.custom_effects.len() {
            return;
        }
        let effect = config.custom_effects.remove(index);
        if let Err(error) = config.save() {
            config.custom_effects.insert(index, effect);
            tracing::warn!(%error, "failed to save custom effects");
            ui.toast
                .add_toast(adw::Toast::new("Could not delete the effect."));
            return;
        }
        effect
    };
    rebuild_menu(ui);
    ui.toast.add_toast(adw::Toast::new(&format!(
        "Effect “{}” deleted.",
        removed.name
    )));
}

fn name_taken(ui: &Rc<Ui>, name: &str, ignore: Option<usize>) -> bool {
    let name = name.trim().to_lowercase();
    ui.config
        .borrow()
        .custom_effects
        .iter()
        .enumerate()
        .any(|(index, effect)| Some(index) != ignore && effect.name.trim().to_lowercase() == name)
}

fn prompt_text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .trim()
        .to_string()
}

/// A short, editable starting point for the effect name.
fn default_name(prompt: &str) -> String {
    let words: Vec<&str> = prompt.split_whitespace().take(4).collect();
    let joined = words.join(" ");
    let trimmed = joined.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    trimmed.chars().take(48).collect()
}

fn snippet(prompt: &str) -> String {
    let mut text: String = prompt.chars().take(60).collect();
    if prompt.chars().count() > 60 {
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, reference: Option<&str>, style: &str) -> Generation {
        Generation {
            id: 1,
            prompt: "repair this photo".into(),
            model: "flux2-klein-4b".into(),
            style: style.into(),
            width: 512,
            height: 512,
            steps: 4,
            seed: "1".into(),
            created_at: 100,
            duration_ms: 1000,
            image_path: "/tmp/out.png".into(),
            reference_path: reference.map(str::to_string),
            reference_paths: reference
                .map(|path| vec![path.to_string()])
                .unwrap_or_default(),
            kind: kind.into(),
            source_path: None,
            parent_id: None,
            favorite: false,
        }
    }

    #[test]
    fn saves_prompt_based_edits() {
        let prompt = savable_prompt(&entry("generate", Some("/tmp/in.png"), "none"));
        assert_eq!(prompt.as_deref(), Some("repair this photo"));
    }

    #[test]
    fn composes_the_style_suffix() {
        let prompt = savable_prompt(&entry("generate", Some("/tmp/in.png"), "cinematic"));
        let prompt = prompt.unwrap();
        assert!(prompt.starts_with("repair this photo, "));
        assert!(prompt.contains("cinematic film still"));
    }

    #[test]
    fn ignores_text_to_image_and_derived_results() {
        assert!(savable_prompt(&entry("generate", None, "none")).is_none());
        assert!(savable_prompt(&entry("upscale", Some("/tmp/in.png"), "none")).is_none());
        assert!(savable_prompt(&entry("colorize", Some("/tmp/in.png"), "none")).is_none());
        assert!(savable_prompt(&entry("custom", Some("/tmp/in.png"), "none")).is_none());
    }

    #[test]
    fn default_name_uses_the_first_words() {
        assert_eq!(
            default_name("restore this old damaged photograph, remove scratches"),
            "restore this old damaged"
        );
        assert_eq!(default_name("  "), "");
    }
}
