//! System: app behavior and sounds.

use crate::ui::hub::Hub;
use crate::ui::widgets;
use eframe::egui::Ui;

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 0.0;
    hub.with_settings(|_hub, s| {
        widgets::group_header(ui, "App settings");
        widgets::settings_group(ui, |g| {
            g.row("Launch app at login", "Coming soon", |ui| {
                let mut off = false;
                ui.add_enabled_ui(false, |ui| widgets::toggle(ui, &mut off));
            });
            g.row("Show floating bar at all times", "Keep the bar visible while idle. It always appears while you dictate.", |ui| {
                widgets::toggle(ui, &mut s.show_overlay);
            });
            g.row("Show app in dock", "Coming soon", |ui| {
                let mut on = true;
                ui.add_enabled_ui(false, |ui| widgets::toggle(ui, &mut on));
            });
        });
        widgets::group_header(ui, "Sound");
        widgets::settings_group(ui, |g| {
            g.row("Dictation sounds", "Play a short cue when dictation starts and stops", |ui| {
                if widgets::toggle(ui, &mut s.enable_sounds).changed() && s.enable_sounds {
                    crate::sound::play(crate::sound::Cue::Start);
                }
            });
        });
    });
}
