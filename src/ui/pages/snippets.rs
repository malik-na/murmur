//! Snippets: say a trigger phrase, Murmur types the expansion.

use crate::types::DictEntry;
use crate::ui::hub::Hub;
use crate::ui::theme::{Theme, TextExt};
use crate::ui::widgets::{self, Button, Hero, HeroPalette, Icon};
use eframe::egui::{self, Align, Layout, RichText, Ui};

pub const MAX_WIDTH: f32 = 800.0;

#[derive(Default)]
pub struct State {
    pub tab: usize,
    pub hero_dismissed: bool,
    pub search_open: bool,
    pub search: String,
    /// Newest first instead of A→Z.
    pub sort_newest: bool,
    pub editor: Option<Editor>,
}

#[derive(Default)]
pub struct Editor {
    pub id: Option<String>,
    pub trigger: String,
    pub expansion: String,
    focused: bool,
}

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    let t = Theme::of(ui);
    super::page_header(ui, "Snippets", |ui| {
        if widgets::primary_button(ui, "Add new").clicked() {
            hub.snippets.editor = Some(Editor::default());
        }
    });

    // Hero example chips: the user's own snippets (trigger → expansion), else generic triggers.
    let examples: Vec<(String, Option<String>)> = {
        let own: Vec<_> = hub.data.snippets.iter().take(3).map(|e| (e.phrase.clone(), e.replacement.clone())).collect();
        if own.is_empty() {
            ["my calendar link", "my address", "email sign-off"].iter().map(|s| (s.to_string(), None)).collect()
        } else {
            own
        }
    };
    let st = &mut hub.snippets;
    widgets::underline_tabs(ui, &mut st.tab, &["All"], |ui| {
        if widgets::icon_button(ui, Icon::Sort, if st.sort_newest { "Sort A→Z" } else { "Sort by newest" }).clicked() {
            st.sort_newest = !st.sort_newest;
        }
        if widgets::icon_button(ui, Icon::Search, "Search").clicked() {
            st.search_open = !st.search_open;
            st.search.clear();
        }
    });
    if st.search_open {
        let w = ui.available_width();
        widgets::search_field(ui, &mut st.search, "Search snippets", w);
        ui.add_space(12.0);
    }

    if !st.hero_dismissed {
        let hero = Hero {
            headline: &[("Say less, ", false), ("type", true), (" more.", false)],
            body: &[("Save links, addresses and sign-offs behind a short phrase. Say the phrase and Murmur types the ", false), ("full text", true), (".", false)],
            palette: HeroPalette::Ocean,
            ..Default::default()
        };
        let mut add: Option<(String, String)> = None;
        if widgets::hero_banner(ui, &hero, |ui| {
            if widgets::hero_chip(ui, "Add new snippet", true).clicked() {
                add = Some(Default::default());
            }
            for (trigger, exp) in &examples {
                let first = exp.as_deref().and_then(|e| e.lines().next()).unwrap_or_default();
                let label = if first.is_empty() { trigger.clone() } else { format!("{trigger}  →  {}", clip(first, 28)) };
                if widgets::hero_chip(ui, &label, false).clicked() {
                    add = Some((trigger.clone(), exp.clone().unwrap_or_default()));
                }
            }
        }) {
            st.hero_dismissed = true;
        }
        if let Some((trigger, expansion)) = add {
            st.editor = Some(Editor { trigger, expansion, ..Default::default() });
        }
        ui.add_space(22.0);
    }

    let q = st.search.trim().to_lowercase();
    let mut rows: Vec<&DictEntry> = hub
        .data
        .snippets
        .iter()
        .filter(|e| q.is_empty() || e.phrase.to_lowercase().contains(&q) || e.replacement.as_deref().is_some_and(|r| r.to_lowercase().contains(&q)))
        .collect();
    if st.sort_newest {
        rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    }

    if rows.is_empty() {
        let (title, body) = if q.is_empty() {
            ("No snippets yet", "Add one for text you type often.")
        } else {
            ("No matches", "No snippets match your search.")
        };
        widgets::empty_state(ui, Icon::Scissors, title, body);
    } else {
        let mut delete = None;
        let mut edit = None;
        widgets::list_card(ui, |list| {
            for e in rows {
                let expansion = e.replacement.as_deref().unwrap_or_default();
                list.row(&e.id, |ui, hovered| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(&e.phrase).size(15.0).color(t.text));
                    ui.label(RichText::new("→").color(t.text_faint));
                    let w = (ui.available_width() - if hovered { 80.0 } else { 0.0 }).max(60.0);
                    let one_line = expansion.lines().next().unwrap_or_default();
                    ui.add_sized(
                        [w, 20.0],
                        egui::Label::new(RichText::new(one_line).size(15.0).color(t.text_muted)).truncate(),
                    )
                    .on_hover_text(expansion);
                    if hovered {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if widgets::icon_button(ui, Icon::Trash, "Delete").clicked() {
                                delete = Some(e.id.clone());
                            }
                            if widgets::icon_button(ui, Icon::Pencil, "Edit").clicked() {
                                edit = Some(e.clone());
                            }
                        });
                    }
                });
            }
        });
        if let Some(id) = delete {
            if let Err(e) = hub.db.delete_dict(&id) {
                log::warn!("delete snippet: {e:#}");
            }
            hub.changed();
        }
        if let Some(e) = edit {
            hub.snippets.editor = Some(Editor { id: Some(e.id), trigger: e.phrase, expansion: e.replacement.unwrap_or_default(), focused: false });
        }
    }

    editor_dialog(hub, ui.ctx());
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>().trim_end()) }
}

fn editor_dialog(hub: &mut Hub, ctx: &egui::Context) {
    let Some(ed) = hub.snippets.editor.as_mut() else { return };
    let title = if ed.id.is_some() { "Edit snippet" } else { "Add new snippet" };
    let mut save = false;
    let mut cancel = false;
    let out = widgets::modal(ctx, "snippet_editor", egui::vec2(520.0, 380.0), |ui| {
        widgets::dialog_body(ui, title, |ui| {
            let t = Theme::of(ui);
            let w = ui.available_width();
            ui.label(RichText::new("Trigger phrase").size(13.0).medium().color(t.text_muted));
            let a = widgets::text_field(ui, &mut ed.trigger, "e.g. my calendar link", w);
            if !ed.focused {
                a.request_focus();
                ed.focused = true;
            }
            let enter_trigger = a.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(4.0);
            ui.label(RichText::new("Expands to").size(13.0).medium().color(t.text_muted));
            egui::Frame::new()
                .fill(t.surface)
                .stroke(egui::Stroke::new(1.0_f32, t.border))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut ed.expansion)
                            .hint_text(RichText::new("The full text to type").color(t.text_faint))
                            .frame(false)
                            .desired_rows(5)
                            .desired_width(w - 20.0)
                            .margin(egui::Margin::ZERO),
                    );
                });
            let n = crate::format::word_count(&ed.expansion);
            ui.label(RichText::new(format!("{n} word{} · ⌘↵ to save", if n == 1 { "" } else { "s" })).size(12.5).color(t.text_faint));
            ui.add_space(10.0);
            // Enter saves from the trigger field; in the multiline field Cmd/Ctrl+Enter saves.
            let cmd_enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let can = !ed.trigger.trim().is_empty() && !ed.expansion.trim().is_empty();
                if ui.add_enabled(can, Button::primary("Save").min_width(80.0)).clicked() || ((enter_trigger || cmd_enter) && can) {
                    save = true;
                }
                if ui.add(Button::secondary("Cancel").min_width(80.0)).clicked() {
                    cancel = true;
                }
            });
        });
    });
    if save {
        let ed = hub.snippets.editor.take().unwrap();
        if let Some(old) = &ed.id {
            let renamed = hub.data.snippets.iter().any(|s| &s.id == old && s.phrase != ed.trigger.trim());
            if renamed {
                if let Err(e) = hub.db.delete_dict(old) {
                    log::warn!("edit snippet: {e:#}");
                }
            }
        }
        if let Err(e) = hub.db.upsert_dict(ed.trigger.trim(), Some(ed.expansion.trim_end()), true) {
            log::warn!("add snippet: {e:#}");
        }
        hub.changed();
    } else if cancel || out.close_requested {
        hub.snippets.editor = None;
    }
}
