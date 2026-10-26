//! Privacy: history storage and macOS permissions.

use crate::ui::hub::Hub;
use crate::ui::theme::Theme;
use crate::ui::widgets::{self, Button};
use eframe::egui::Ui;
use std::time::{Duration, Instant};

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let t = Theme::of(ui);
    hub.with_settings(|hub, s| {
        widgets::group_header(ui, "History");
        widgets::settings_group(ui, |g| {
            g.row("Save history", "Keep transcripts on this Mac. Audio is never stored.", |ui| {
                widgets::toggle(ui, &mut s.store_history);
            });
            let confirm = hub.prefs.confirm_clear;
            let caption = if confirm { "Delete all transcripts? This can't be undone." } else { "Remove every saved transcript" };
            g.row("Clear history", caption, |ui| {
                if !confirm {
                    if ui.add(Button::secondary("Clear…").height(32.0).min_width(100.0)).clicked() {
                        hub.prefs.confirm_clear = true;
                    }
                } else {
                    if ui.add(Button::danger("Clear").height(32.0).min_width(80.0)).clicked() {
                        if let Err(e) = hub.db.clear_history() {
                            log::warn!("clear history: {e:#}");
                        }
                        hub.prefs.confirm_clear = false;
                        hub.changed();
                    }
                    if ui.add(Button::secondary("Cancel").height(32.0)).clicked() {
                        hub.prefs.confirm_clear = false;
                    }
                }
            });
        });
    });

    widgets::group_header(ui, "Permissions");
    if hub.prefs.ax_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
        hub.prefs.ax_trusted = crate::hotkey::accessibility_trusted(false);
        hub.prefs.ax_checked = Some(Instant::now());
    }
    let ax_trusted = hub.prefs.ax_trusted;
    let ax = if ax_trusted { "Granted" } else { "Needed to detect the fn key and paste text" };
    widgets::settings_group(ui, |g| {
        for (title, desc, pane) in [
            ("Accessibility", ax, "Accessibility"),
            ("Microphone", "Needed to hear you", "Microphone"),
            ("Input Monitoring", "Needed for the global shortcut", "ListenEvent"),
        ] {
            g.row(title, desc, |ui| {
                if ui.add(Button::secondary("Open settings").height(32.0)).clicked() {
                    let pane = pane.to_string();
                    std::thread::spawn(move || crate::context::open_privacy_pane(&pane));
                }
                if pane == "Accessibility" {
                    let c = if ax_trusted { t.text } else { t.warning };
                    widgets::status_dot(ui, c);
                }
            });
        }
    });
}
