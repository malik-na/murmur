//! Hub pages. Each page module exposes:
//! - `pub const MAX_WIDTH: f32` — width of the centered content column inside the content card;
//! - `#[derive(Default)] pub struct State` — page-local UI state, stored in `Hub` (`hub.home`, `hub.dict`, …);
//! - `pub fn show(hub: &mut Hub, ui: &mut Ui)` — draws the page (the shell already scrolls and centers it).
//!
//! Settings is different: it is a modal with sections, see `settings::show(hub, ui, section)`.

pub mod dictionary;
pub mod home;
pub mod settings;
pub mod snippets;
pub mod style;

use super::theme::{Theme, TextExt};
use chrono::{DateTime, Local};
use eframe::egui::{self, Align, Layout, RichText, Ui};

/// Page title row: 24 px semibold title on the left, `right` controls right-aligned (e.g. "Add new").
pub fn page_header(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    let t = Theme::of(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        ui.label(RichText::new(title).size(24.0).semibold().color(t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(20.0);
}

pub fn parse_local(ts: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(ts).ok().map(|t| t.with_timezone(&Local))
}

/// "TODAY", "YESTERDAY" or "OCTOBER 21, 2026".
pub fn day_label(t: Option<DateTime<Local>>) -> String {
    let Some(t) = t else { return "EARLIER".into() };
    let today = Local::now().date_naive();
    let d = t.date_naive();
    if d == today {
        "TODAY".into()
    } else if Some(d) == today.pred_opt() {
        "YESTERDAY".into()
    } else {
        t.format("%B %-d, %Y").to_string().to_uppercase()
    }
}

/// "10:47pm".
pub fn time_label(t: Option<DateTime<Local>>) -> String {
    t.map(|t| t.format("%-I:%M%P").to_string()).unwrap_or_default()
}

pub fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 950 → "950", 13_140 → "13.1K", 2_400_000 → "2.4M".
pub fn format_compact(n: u64) -> String {
    match n {
        0..=9_999 => format_thousands(n),
        10_000..=999_999 => trim_zero(format!("{:.1}", n as f64 / 1000.0)) + "K",
        _ => trim_zero(format!("{:.1}", n as f64 / 1_000_000.0)) + "M",
    }
}

fn trim_zero(s: String) -> String {
    s.strip_suffix(".0").map(str::to_owned).unwrap_or(s)
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Opens a URL in the default browser.
pub fn open_url(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

/// A region that is clickable as a whole while buttons inside it still get their own clicks.
pub fn clickable(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> egui::Response {
    ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), add)
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_numbers() {
        assert_eq!(format_compact(950), "950");
        assert_eq!(format_compact(9_999), "9,999");
        assert_eq!(format_compact(13_140), "13.1K");
        assert_eq!(format_compact(20_000), "20K");
        assert_eq!(format_compact(2_400_000), "2.4M");
    }
}
