//! Style: per app category (personal / work / email / other) pick a writing tone.

use crate::types::{Settings, StyleCategory, Tone};
use crate::ui::hub::Hub;
use crate::ui::theme::{self, Theme};
use crate::ui::widgets::{self, Hero, HeroPalette, Icon};
use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, Sense, Stroke, StrokeKind, Ui};

pub const MAX_WIDTH: f32 = 800.0;

pub const CATEGORIES: [(StyleCategory, &str, &str, Icon, &str); 4] = [
    (StyleCategory::Personal, "Personal messages", "personal messengers", Icon::Chat, "Hey, are you free for dinner tonight? I can bring dessert if that works for you."),
    (StyleCategory::Work, "Work messages", "work chat apps", Icon::Briefcase, "The build is green again. I'll merge the fix after lunch and let the team know."),
    (StyleCategory::Email, "Email", "email apps", Icon::Envelope, "Thanks for the quick reply. Tuesday at ten works well for me, see you then."),
    (StyleCategory::Other, "Other", "all other apps", Icon::Note, "Remember to renew the parking permit. It expires on Friday, so do it this week."),
];

pub const TONES: [(Tone, &str, &str); 3] = [
    (Tone::Formal, "Formal.", "Caps + Punctuation"),
    (Tone::Casual, "Casual", "Caps + Less punctuation"),
    (Tone::VeryCasual, "very casual", "No caps + Less punctuation"),
];

pub struct State {
    pub tab: usize,
    /// `examples[category][tone]`, rendered once with `format::apply_tone`.
    pub examples: Vec<Vec<String>>,
}

impl Default for State {
    fn default() -> Self {
        let examples = CATEGORIES
            .iter()
            .map(|c| TONES.iter().map(|(t, _, _)| crate::format::apply_tone(c.4, *t)).collect())
            .collect();
        State { tab: 0, examples }
    }
}

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    super::page_header(ui, "Style", |_| {});
    let labels: Vec<&str> = CATEGORIES.iter().map(|c| c.1).collect();
    widgets::underline_tabs(ui, &mut hub.style.tab, &labels, |_| {});
    let ci = hub.style.tab.min(CATEGORIES.len() - 1);
    let (cat, _, where_, cat_icon, _) = CATEGORIES[ci];

    let headline = format!("How you sound in {where_}");
    let hero = Hero {
        headline: &[(headline.as_str(), false)],
        body: &[("Pick the tone that feels like you. Murmur adjusts capitals and punctuation to match (English only for now).", false)],
        palette: HeroPalette::Dusk,
        height: 140.0,
        dismissible: false,
    };
    let top = ui.cursor().top();
    widgets::hero_banner(ui, &hero, |_| {});
    // Generic category glyphs on the right side of the banner (no real app logos).
    let right = ui.min_rect().right();
    let cy = top + 70.0;
    for (i, ic) in [cat_icon, Icon::Keyboard, Icon::Plus].iter().enumerate() {
        let c = pos2(right - 48.0 - (2 - i) as f32 * 44.0, cy);
        ui.painter().circle(c, 19.0, egui::Color32::from_white_alpha(40), Stroke::new(1.0_f32, egui::Color32::from_white_alpha(70)));
        widgets::paint_icon(ui.painter(), c, *ic, 18.0, egui::Color32::WHITE);
    }
    ui.add_space(18.0);

    hub.with_settings(|hub, s: &mut Settings| {
        let current = s.tone_for(cat);
        let mut chosen = current;
        let w = ui.available_width();
        let gap = 16.0;
        let card_w = (w - 2.0 * gap) / 3.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (ti, (tone, name, sub)) in TONES.iter().enumerate() {
                if tone_card(ui, card_w, name, sub, &hub.style.examples[ci][ti], *tone == current, ti).clicked() {
                    chosen = *tone;
                }
            }
        });
        if chosen != current {
            match cat {
                StyleCategory::Personal => s.tone_personal = chosen,
                StyleCategory::Work => s.tone_work = chosen,
                StyleCategory::Email => s.tone_email = chosen,
                StyleCategory::Other => s.tone_other = chosen,
            }
        }
    });
}

fn tone_card(ui: &mut Ui, w: f32, name: &str, sub: &str, example: &str, selected: bool, idx: usize) -> egui::Response {
    let t = Theme::of(ui);
    let h = 350.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let stroke = if selected {
        Stroke::new(1.5_f32, t.accent)
    } else if resp.hovered() {
        Stroke::new(1.0_f32, theme::mix(t.border, t.text, 0.15))
    } else {
        Stroke::new(1.0_f32, t.border)
    };
    ui.painter().rect(rect, 12.0, t.surface, stroke, StrokeKind::Inside);
    let inner = rect.shrink(16.0);
    ui.painter().text(inner.left_top(), Align2::LEFT_TOP, name, theme::medium(15.0), t.text);
    ui.painter().text(inner.left_top() + vec2(0.0, 24.0), Align2::LEFT_TOP, sub, theme::regular(14.0), t.text);

    // Example message bubble.
    let bubble_w = inner.width();
    let galley = ui.painter().layout(example.to_owned(), theme::regular(14.0), t.text, bubble_w - 24.0);
    let bubble = egui::Rect::from_min_size(pos2(inner.left(), inner.top() + 120.0), vec2(bubble_w, galley.size().y + 24.0));
    ui.painter().rect_filled(bubble, CornerRadius::same(10), t.surface_sunken);
    ui.painter().galley(bubble.min + vec2(12.0, 12.0), galley, t.text);

    // Avatar circle, bottom right.
    let shades = [t.accent.gamma_multiply(0.35), t.accent.gamma_multiply(0.6), theme::mix(t.accent, t.text, 0.45)];
    let c = pos2(inner.right() - 18.0, inner.bottom() - 18.0);
    ui.painter().circle_filled(c, 16.0, shades[idx % 3]);
    ui.painter().text(c, Align2::CENTER_CENTER, "M", theme::medium(14.0), egui::Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
