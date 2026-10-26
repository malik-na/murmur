//! Dictionary: vocabulary words and "wrong → right" replacements, including words learned from corrections.

use crate::types::DictEntry;
use crate::ui::hub::Hub;
use crate::ui::theme::{Theme, TextExt};
use crate::ui::widgets::{self, Button, Hero, HeroPalette, Icon};
use eframe::egui::{self, Align, Layout, RichText, Ui};

pub const MAX_WIDTH: f32 = 800.0;

#[derive(Default)]
pub struct State {
    /// 0 = All, 1 = Learned.
    pub tab: usize,
    pub hero_dismissed: bool,
    pub search_open: bool,
    pub search: String,
    /// Newest first instead of the default (starred first, then A→Z).
    pub sort_newest: bool,
    /// Open "Add new" / edit dialog.
    pub editor: Option<Editor>,
}

#[derive(Default)]
pub struct Editor {
    /// Entry being edited; None = new.
    pub id: Option<String>,
    pub word: String,
    pub replacement: String,
    focused: bool,
}

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    let t = Theme::of(ui);
    super::page_header(ui, "Dictionary", |ui| {
        if widgets::primary_button(ui, "Add new").clicked() {
            hub.dict.editor = Some(Editor::default());
        }
    });

    // Hero example chips: the user's own plain words, else generic examples.
    let examples: Vec<String> = {
        let own: Vec<String> = hub.data.words.iter().filter(|e| e.replacement.is_none()).map(|e| e.phrase.clone()).take(3).collect();
        if own.len() >= 3 { own } else { ["Murmur", "Kubernetes", "Dr. Okafor"].iter().map(|s| s.to_string()).collect() }
    };
    let st = &mut hub.dict;
    widgets::underline_tabs(ui, &mut st.tab, &["All", "Learned"], |ui| {
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
        widgets::search_field(ui, &mut st.search, "Search words", w);
        ui.add_space(12.0);
    }

    if !st.hero_dismissed {
        let hero = Hero {
            headline: &[("Teach Murmur ", false), ("your", true), (" words.", false)],
            body: &[
                ("Add names, product terms and jargon so they're spelled right every time. Fix a word once after pasting and Murmur ", false),
                ("learns it automatically", true),
                (".", false),
            ],
            palette: HeroPalette::Ember,
            ..Default::default()
        };
        let mut add: Option<String> = None;
        if widgets::hero_banner(ui, &hero, |ui| {
            if widgets::hero_chip(ui, "Add new word", true).clicked() {
                add = Some(String::new());
            }
            for w in &examples {
                if widgets::hero_chip(ui, w, false).clicked() {
                    add = Some(w.clone());
                }
            }
        }) {
            st.hero_dismissed = true;
        }
        if let Some(word) = add {
            st.editor = Some(Editor { word, ..Default::default() });
        }
        ui.add_space(22.0);
    }

    let q = st.search.trim().to_lowercase();
    let mut rows: Vec<&DictEntry> = hub
        .data
        .words
        .iter()
        .filter(|e| st.tab == 0 || e.source == "user_edits")
        .filter(|e| q.is_empty() || e.phrase.to_lowercase().contains(&q) || e.replacement.as_deref().is_some_and(|r| r.to_lowercase().contains(&q)))
        .collect();
    if st.sort_newest {
        rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    }

    if rows.is_empty() {
        let (title, body) = if !q.is_empty() {
            ("No matches", "No words match your search.")
        } else if st.tab == 1 {
            ("Nothing learned yet", "When you correct a word after pasting, Murmur adds it here.")
        } else {
            ("No words yet", "Try your name, your team's product names, or acronyms you use.")
        };
        widgets::empty_state(ui, Icon::Book, title, body);
    } else {
        let mut action: Option<Action> = None;
        widgets::list_card(ui, |list| {
            for e in rows {
                list.row(&e.id, |ui, hovered| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(&e.phrase).size(15.0).color(t.text));
                    if e.source == "user_edits" {
                        let tip = match e.heard_as.as_deref().filter(|h| !h.is_empty()) {
                            Some(h) => format!("Learned from your correction of “{h}”"),
                            None => "Learned from your correction".to_string(),
                        };
                        widgets::icon(ui, Icon::Sparkle, 15.0, t.accent).on_hover_text(tip);
                    }
                    if e.is_starred {
                        widgets::icon(ui, Icon::Star, 14.0, t.warning).on_hover_text("Starred");
                    }
                    if let Some(r) = &e.replacement {
                        ui.label(RichText::new("→").color(t.text_faint));
                        ui.label(RichText::new(r).size(15.0).color(t.text_muted));
                    }
                    if hovered {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if widgets::icon_button(ui, Icon::Trash, "Delete").clicked() {
                                action = Some(Action::Delete(e.id.clone()));
                            }
                            if widgets::icon_button(ui, Icon::Pencil, "Edit").clicked() {
                                action = Some(Action::Edit(e.clone()));
                            }
                            let star_tip = if e.is_starred { "Unstar" } else { "Star" };
                            if widgets::icon_button(ui, Icon::Star, star_tip).clicked() {
                                action = Some(Action::Star(e.id.clone(), !e.is_starred));
                            }
                        });
                    }
                });
            }
        });
        if let Some(a) = action {
            apply(hub, a);
        }
    }

    editor_dialog(hub, ui.ctx());
}

enum Action {
    Delete(String),
    Star(String, bool),
    Edit(DictEntry),
}

fn apply(hub: &mut Hub, a: Action) {
    let res = match a {
        Action::Delete(id) => hub.db.delete_dict(&id),
        Action::Star(id, s) => hub.db.set_starred(&id, s),
        Action::Edit(e) => {
            hub.dict.editor = Some(Editor { id: Some(e.id), word: e.phrase, replacement: e.replacement.unwrap_or_default(), focused: false });
            return;
        }
    };
    if let Err(e) = res {
        log::warn!("dictionary update: {e:#}");
    }
    hub.changed();
}

fn editor_dialog(hub: &mut Hub, ctx: &egui::Context) {
    let Some(ed) = hub.dict.editor.as_mut() else { return };
    let title = if ed.id.is_some() { "Edit word" } else { "Add new word" };
    let mut save = false;
    let mut cancel = false;
    let out = widgets::modal(ctx, "dict_editor", egui::vec2(460.0, 300.0), |ui| {
        widgets::dialog_body(ui, title, |ui| {
            let t = Theme::of(ui);
            let w = ui.available_width();
            ui.label(RichText::new("Word or phrase").size(13.0).medium().color(t.text_muted));
            let a = widgets::text_field(ui, &mut ed.word, "e.g. Murmur", w);
            if !ed.focused {
                a.request_focus();
                ed.focused = true;
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Replace with (optional)").size(13.0).medium().color(t.text_muted));
            let b = widgets::text_field(ui, &mut ed.replacement, "Leave empty to just teach the spelling", w);
            let enter = (a.lost_focus() || b.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let can = !ed.word.trim().is_empty();
                if ui.add_enabled(can, Button::primary("Save").min_width(80.0)).clicked() || (enter && can) {
                    save = true;
                }
                if ui.add(Button::secondary("Cancel").min_width(80.0)).clicked() {
                    cancel = true;
                }
            });
        });
    });
    if save {
        let ed = hub.dict.editor.take().unwrap();
        let rep = ed.replacement.trim();
        let rep = (!rep.is_empty()).then_some(rep);
        if let Some(old) = &ed.id {
            // The phrase is the key: replace the old row when it was renamed.
            let renamed = hub.data.words.iter().any(|w| &w.id == old && w.phrase != ed.word.trim());
            if renamed {
                if let Err(e) = hub.db.delete_dict(old) {
                    log::warn!("edit word: {e:#}");
                }
            }
        }
        if let Err(e) = hub.db.upsert_dict(ed.word.trim(), rep, false) {
            log::warn!("add word: {e:#}");
        }
        hub.changed();
    } else if cancel || out.close_requested {
        hub.dict.editor = None;
    }
}
