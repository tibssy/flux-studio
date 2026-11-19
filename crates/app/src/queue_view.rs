//! Sidebar section listing waiting and running jobs.
//!
//! Finished jobs disappear from here: their result is in History, with a toast
//! (and an optional desktop notification) marking the transition.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::Ui;
use crate::queue;

/// Rebuild the queue rows from the queue state.
pub(crate) fn rebuild(ui: &Rc<Ui>) {
    let (waiting, running) = {
        let queue = ui.queue.borrow();
        (
            queue.pending().cloned().collect::<Vec<_>>(),
            queue.running().cloned(),
        )
    };

    let pending_count = waiting.len();
    let visible = pending_count > 0 || running.is_some();

    ui.queue_box.set_visible(visible);
    ui.queue_waiting.set_text(&match pending_count {
        0 => String::new(),
        1 => "1 waiting".into(),
        count => format!("{count} waiting"),
    });
    ui.queue_waiting.set_visible(pending_count > 0);
    ui.queue_clear.set_sensitive(pending_count > 0);

    ui.queue_status.set_visible(pending_count > 0);
    ui.queue_status.set_text(&match pending_count {
        0 => String::new(),
        1 => "1 waiting".into(),
        count => format!("{count} waiting"),
    });

    while let Some(child) = ui.queue_list.first_child() {
        ui.queue_list.remove(&child);
    }
    ui.queue_running_row.replace(None);

    if let Some(running) = running {
        let row = running_row(ui, &running);
        ui.queue_list.append(&row);
        ui.queue_running_row.replace(Some(row));
    }
    for job in &waiting {
        ui.queue_list.append(&waiting_row(ui, job));
    }

    if visible {
        ensure_ticker(ui);
    } else {
        stop_ticker(ui);
    }
    crate::update_queue_bubble(ui);
}

fn running_row(ui: &Rc<Ui>, running: &queue::Running) -> adw::ActionRow {
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(16, 16);
    spinner.start();

    let live = !running.job.is_upscale();
    let row = adw::ActionRow::builder()
        .title(running.job.title())
        .subtitle(running_subtitle(running))
        .activatable(live)
        .build();
    row.set_title_lines(1);
    row.set_subtitle_lines(1);
    row.add_prefix(&spinner);

    if live {
        row.set_tooltip_text(Some("Show the running job's live preview"));
        let activated_ui = ui.clone();
        row.connect_activated(move |_| crate::show_live(&activated_ui));
        // ActionRow does not emit `activated` for plain clicks outside a
        // preferences window, so treat a click anywhere on the row as well.
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let click_ui = ui.clone();
        click.connect_released(move |_, _, _, _| crate::show_live(&click_ui));
        row.add_controller(click);

        let show = gtk::Button::builder()
            .icon_name("view-reveal-symbolic")
            .tooltip_text("Show the running job's live preview")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        {
            let ui = ui.clone();
            show.connect_clicked(move |_| crate::show_live(&ui));
        }
        row.add_suffix(&show);
    }

    let cancel = gtk::Button::builder()
        .icon_name("process-stop-symbolic")
        .tooltip_text("Cancel this job")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    {
        let ui = ui.clone();
        cancel.connect_clicked(move |_| cancel_running(&ui));
    }
    row.add_suffix(&cancel);
    row
}

fn waiting_row(ui: &Rc<Ui>, job: &queue::Job) -> adw::ActionRow {
    let icon = gtk::Image::from_icon_name("appointment-soon-symbolic");
    icon.set_pixel_size(16);

    let row = adw::ActionRow::builder()
        .title(job.title())
        .subtitle(format!("{} · waiting", job.meta()))
        .activatable(false)
        .build();
    row.set_title_lines(1);
    row.set_subtitle_lines(1);
    row.add_prefix(&icon);

    let remove = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text("Remove from the queue")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    {
        let ui = ui.clone();
        let id = job.id;
        remove.connect_clicked(move |_| remove_job(&ui, id));
    }
    row.add_suffix(&remove);
    row
}

fn running_subtitle(running: &queue::Running) -> String {
    let elapsed = running.started.elapsed();
    let time = if elapsed.as_secs() >= 60 {
        format!("{}m {:02}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
    } else {
        format!("{}s", elapsed.as_secs())
    };
    let status = if running.cancelling {
        "Cancelling".to_string()
    } else if let Some((current, total)) = running.step {
        format!("Step {current}/{total}")
    } else if running.job.is_upscale() {
        "Upscaling".to_string()
    } else {
        "Loading model".to_string()
    };
    format!("{} · {status} · {time}", running.job.meta())
}

/// Refresh the running row's elapsed time and step.
fn update_running(ui: &Rc<Ui>) {
    let subtitle = {
        let queue = ui.queue.borrow();
        queue.running().map(running_subtitle)
    };
    let Some(subtitle) = subtitle else {
        return;
    };
    if let Some(row) = ui.queue_running_row.borrow().as_ref() {
        row.set_subtitle(&subtitle);
    }
}

fn ensure_ticker(ui: &Rc<Ui>) {
    if ui.queue_tick.borrow().is_some() {
        return;
    }
    let tick_ui = Rc::clone(ui);
    let id = glib::timeout_add_local(Duration::from_secs(1), move || {
        update_running(&tick_ui);
        if tick_ui.queue.borrow().running().is_some() {
            glib::ControlFlow::Continue
        } else {
            tick_ui.queue_tick.replace(None);
            glib::ControlFlow::Break
        }
    });
    ui.queue_tick.replace(Some(id));
}

fn stop_ticker(ui: &Rc<Ui>) {
    if let Some(id) = ui.queue_tick.take() {
        id.remove();
    }
}

fn remove_job(ui: &Rc<Ui>, id: u64) {
    let removed = ui.queue.borrow_mut().remove(id);
    if let Some(job) = removed {
        cleanup_job(&job);
        rebuild(ui);
        crate::refresh_history_locks(ui);
    }
}

fn cleanup_job(job: &queue::Job) {
    if let Some(dir) = &job.temp_dir {
        let _ = std::fs::remove_dir_all(dir);
    }
}

pub(crate) fn cancel_running(ui: &Rc<Ui>) {
    {
        let mut queue = ui.queue.borrow_mut();
        let Some(running) = queue.running_mut() else {
            return;
        };
        if running.cancelling {
            return;
        }
        running.cancelling = true;
    }
    let upscaling = ui.upscaling.get();
    let result = if upscaling {
        ui.backend.cancel_upscale()
    } else {
        ui.backend.cancel()
    };
    if let Err(error) = result {
        tracing::warn!(%error, "failed to cancel the running job");
    }
    rebuild(ui);
    ui.status.set_text("Cancelling…");
}

/// Drop every waiting job, leaving the running one alone.
pub(crate) fn clear_pending(ui: &Rc<Ui>) {
    let removed = ui.queue.borrow_mut().clear_pending();
    if removed.is_empty() {
        return;
    }
    for job in &removed {
        cleanup_job(job);
    }
    rebuild(ui);
    crate::refresh_history_locks(ui);
    ui.status.set_text(&match removed.len() {
        1 => "Removed 1 waiting job".into(),
        count => format!("Removed {count} waiting jobs"),
    });
}

/// Note the step announced by the backend so the queue row stays current.
pub(crate) fn set_step(ui: &Rc<Ui>, current: u32, total: u32) {
    if let Some(running) = ui.queue.borrow_mut().running_mut() {
        running.step = Some((current, total));
    }
    update_running(ui);
}
