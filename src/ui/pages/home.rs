//! Home: greeting, tip banner, stats card and dictation history grouped by day.

use super::{day_label, format_compact, parse_local, time_label};
use crate::ui::hub::Hub;
use crate::ui::theme::{self, Theme, TextExt};
use crate::ui::widgets::{self, Hero, HeroPalette, Icon};
use eframe::egui::{self, Align, Id, Layout, RichText, Ui};

/// Main column (770) + gap (24) + stats card (250).
pub const MAX_WIDTH: f32 = 1044.0;
const STATS_W: f32 = 250.0;
const GAP: f32 = 24.0;

#[derive(Default)]
pub struct State {
    pub hero_dismissed: bool,
    pub search_open: bool,
    /// Number of history rows currently shown (0 = default page size).
    pub shown: usize,
}

const PAGE: usize = 50;

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    let t = Theme::of(ui);
    let title = if hub.user_first_name.is_empty() {
        "Welcome back".to_string()
    } else {
        format!("Welcome back, {}", hub.user_first_name)
    };
    super::page_header(ui, &title, |_| {});

    let w = ui.available_width();
    if w >= 860.0 {
        let main_w = w - STATS_W - GAP;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.vertical(|ui| {
                ui.set_width(main_w);
                main_column(hub, ui);
            });
            ui.add_space(GAP);
            ui.vertical(|ui| {
                ui.set_width(STATS_W);
                stats_card(hub, ui, &t);
            });
        });
    } else {
        stats_card(hub, ui, &t);
        ui.add_space(20.0);
        main_column(hub, ui);
    }
}

fn main_column(hub: &mut Hub, ui: &mut Ui) {
    if !hub.home.hero_dismissed {
        let hero = Hero {
            headline: &[("Just talk. ", false), ("We'll", true), (" tidy up.", false)],
            body: &[("Speak naturally — Murmur handles the punctuation, filler words and spoken commands like ", false), ("“new line”", true), (".", false)],
            palette: HeroPalette::Dusk,
            ..Default::default()
        };
        let mut open_help = false;
        if widgets::hero_banner(ui, &hero, |ui| {
            open_help = widgets::hero_chip(ui, "See shortcuts", true).clicked();
        }) {
            hub.home.hero_dismissed = true;
        }
        if open_help {
            hub.help_open = true;
        }
        ui.add_space(32.0);
    }
    history(hub, ui);
}

fn stats_card(hub: &mut Hub, ui: &mut Ui, t: &Theme) {
    let s = &hub.data.stats;
    let rows = [
        (format_compact(s.total_words), "total words"),
        (format!("{:.0}", s.average_wpm), "wpm"),
        (s.day_streak.to_string(), if s.day_streak == 1 { "day streak" } else { "day streak" }),
    ];
    let status = hub.shared.model_status.lock().unwrap().clone();
    let settings = hub.settings_snapshot();
    widgets::card_frame(ui, t.bg).inner_margin(egui::Margin::ZERO).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 24, top: 14, bottom: 14 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (value, unit) in rows {
                let w = ui.available_width();
                ui.allocate_ui_with_layout(egui::vec2(w, 44.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(value).font(theme::serif(30.0)).color(t.text));
                    ui.label(RichText::new(unit).size(15.0).color(t.text_muted));
                });
            }
        });
        widgets::divider(ui);
        egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 24, top: 18, bottom: 22 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(RichText::new("Speech model").size(15.0).medium().color(t.text));
            let (dot, label) = match status.as_str() {
                "ready" => (egui::Color32::from_rgb(0x2E, 0xA0, 0x5A), "Ready".to_string()),
                "loading" => (t.warning, "Loading…".to_string()),
                "missing" => (t.danger, "Not downloaded".to_string()),
                s if s.starts_with("error") => (t.danger, "Error".to_string()),
                s if s.starts_with("downloading") => (t.warning, super::capitalize(s)),
                other => (t.text_muted, super::capitalize(&other.replace('_', " "))),
            };
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 18.0), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::hover());
                ui.painter().circle_filled(r.left_center() + egui::vec2(4.0, 0.0), 4.0, dot);
                ui.label(RichText::new(label).size(13.0).color(t.text_muted));
            });
            ui.add_space(8.0);
            ui.label(RichText::new("AI cleanup").size(15.0).medium().color(t.text));
            let cleanup = match (settings.use_local_llm, settings.use_llm_cleanup) {
                (true, true) => format!("Local model, then {}", settings.cloud_provider.label()),
                (true, false) => "Local model".to_string(),
                (false, true) => settings.cloud_provider.label().to_string(),
                (false, false) => "Off".to_string(),
            };
            ui.label(RichText::new(cleanup).size(13.0).color(t.text_muted));
        });
    });
}

fn history(hub: &mut Hub, ui: &mut Ui) {
    let t = Theme::of(ui);
    let search_open = hub.home.search_open || !hub.data.search.is_empty();

    if hub.data.history.is_empty() {
        header_row(hub, ui, "History");
        if search_open {
            search_row(hub, ui);
        }
        if hub.data.search.trim().is_empty() {
            widgets::empty_state(ui, Icon::Mic, "Hold fn and speak in any app", "Your transcripts will show up here.");
        } else {
            widgets::empty_state(ui, Icon::Search, "No matches", "No transcripts match your search.");
        }
        return;
    }

    let mut delete = None;
    // Group consecutive entries by local day.
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut last_day = None;
    let limit = if hub.home.shown == 0 { PAGE } else { hub.home.shown };
    let total = hub.data.history.len();
    for (i, e) in hub.data.history.iter().enumerate().take(limit) {
        let local = parse_local(&e.timestamp);
        let day = local.map(|t| t.date_naive());
        if groups.is_empty() || day != last_day {
            last_day = day;
            groups.push((day_label(local), Vec::new()));
        }
        groups.last_mut().unwrap().1.push(i);
    }

    for (gi, (label, idxs)) in groups.iter().enumerate() {
        if gi == 0 {
            header_row(hub, ui, label);
            if search_open {
                search_row(hub, ui);
            }
        } else {
            ui.add_space(20.0);
            widgets::section_label(ui, label);
            ui.add_space(6.0);
        }
        widgets::list_card(ui, |list| {
            for &i in idxs {
                let e = &hub.data.history[i];
                list.row(&e.id, |ui, hovered| {
                    let time = time_label(parse_local(&e.timestamp));
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(91.0, 20.0), egui::Sense::hover());
                    ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, time, theme::regular(12.5), t.text_muted);

                    let (text, color) = if !e.formatted_text.is_empty() {
                        (e.formatted_text.clone(), t.text)
                    } else if !e.asr_text.is_empty() {
                        (e.asr_text.clone(), t.text)
                    } else {
                        (format!("({})", e.status.replace('_', " ")), t.text_muted)
                    };
                    let text_w = (ui.available_width() - 90.0).clamp(120.0, 520.0);
                    let mut job = egui::text::LayoutJob::single_section(
                        text,
                        egui::TextFormat { font_id: theme::regular(14.5), color, line_height: Some(14.5 * 1.35), ..Default::default() },
                    );
                    job.wrap.max_width = text_w;
                    let galley = ui.painter().layout_job(job);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(text_w, galley.size().y + 28.0), egui::Sense::hover());
                    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, color);

                    let popup_id = Id::new(("history_more", &e.id));
                    if hovered || egui::Popup::is_id_open(ui.ctx(), popup_id) {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let more = widgets::icon_button(ui, Icon::More, "More");
                            egui::Popup::menu(&more).id(popup_id).show(|ui| {
                                if ui.button("Delete").clicked() {
                                    delete = Some(e.id.clone());
                                }
                            });
                            if widgets::icon_button(ui, Icon::Copy, "Copy").clicked() {
                                let text = if e.formatted_text.is_empty() { &e.asr_text } else { &e.formatted_text };
                                ui.ctx().copy_text(text.clone());
                            }
                        });
                    }
                });
            }
        });
    }

    if total > limit {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            if ui.add(widgets::Button::secondary(format!("Load more ({} left)", total - limit))).clicked() {
                hub.home.shown = limit + PAGE;
            }
        });
    }

    if let Some(id) = delete {
        if let Err(e) = hub.db.delete_history(&id) {
            log::warn!("delete history: {e:#}");
        }
        hub.changed();
    }
}

/// Section label with the search toggle at the right.
fn header_row(hub: &mut Hub, ui: &mut Ui, label: &str) {
    ui.horizontal(|ui| {
        widgets::section_label(ui, label);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(ui, Icon::Search, "Search history").clicked() {
                hub.home.search_open = !hub.home.search_open;
                if !hub.home.search_open {
                    hub.data.search.clear();
                }
            }
        });
    });
    ui.add_space(6.0);
}

fn search_row(hub: &mut Hub, ui: &mut Ui) {
    let w = ui.available_width();
    let r = widgets::search_field(ui, &mut hub.data.search, "Search transcripts", w);
    if hub.home.search_open && hub.data.search.is_empty() && !r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        hub.home.search_open = false;
    }
    ui.add_space(10.0);
}
