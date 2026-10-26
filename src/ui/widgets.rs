//! Reusable hub components matching UI_SPEC.md. Every widget reads its colors from [`Theme::of`].
//!
//! Quick reference:
//! - [`icon`], [`icon_button`] — Phosphor stroke icons ([`Icon`]).
//! - [`primary_button`], [`secondary_button`], [`Button`] — 36 px rounded buttons (`ui.add_enabled(false, Button::primary("…"))` to disable).
//! - [`toggle`] — 36×20 iOS-style switch.
//! - [`section_label`] — 11 px uppercase faint label ("OCTOBER 21, 2026").
//! - [`card`] — rounded bordered container; [`list_card`] + [`ListCard::row`] — bordered list with dividers & hover.
//! - [`settings_group`] + [`SettingsGroup::row`] / [`settings_row`] — grouped rows on `surface_sunken`.
//! - [`underline_tabs`] — "All | Learned" tabs with right-aligned icon buttons.
//! - [`hero_banner`] + [`hero_chip`] — dismissible gradient banner.
//! - [`modal`] — centered dialog with dimmed backdrop (Esc / backdrop click closes).
//! - [`search_field`], [`text_field`], [`pill_badge`], [`divider`], [`empty_state`].

#![allow(dead_code)] // toolkit for the page modules; not every helper is used yet

use super::theme::{self, Theme, TextExt};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Id, Layout, Margin, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, Vec2, Widget, WidgetText, pos2, vec2,
};
use std::hash::Hash;

// ---------------------------------------------------------------------------------------------
// Icons

/// Icon set (Phosphor Regular, MIT). Use [`icon`] or [`icon_button`]; `Icon::X.glyph()` gives the raw char
/// for use inside `RichText` (the proportional families fall back to the icon font).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Home,
    Book,
    Scissors,
    TextAa,
    Gear,
    Help,
    Search,
    Sort,
    Copy,
    More,
    Plus,
    X,
    Bell,
    Sidebar,
    Pencil,
    Trash,
    Sparkle,
    Star,
    Mic,
    Sliders,
    Monitor,
    Shield,
    User,
    Key,
    CaretDown,
    Check,
    Warning,
    Info,
    ExternalLink,
    MagicWand,
    Chat,
    Briefcase,
    Envelope,
    Note,
    Keyboard,
    Clock,
}

impl Icon {
    pub fn glyph(self) -> &'static str {
        use egui_phosphor::regular as p;
        match self {
            Icon::Home => p::HOUSE,
            Icon::Book => p::BOOK_OPEN_TEXT,
            Icon::Scissors => p::SCISSORS,
            Icon::TextAa => p::TEXT_AA,
            Icon::Gear => p::GEAR_SIX,
            Icon::Help => p::QUESTION,
            Icon::Search => p::MAGNIFYING_GLASS,
            Icon::Sort => p::ARROWS_DOWN_UP,
            Icon::Copy => p::COPY,
            Icon::More => p::DOTS_THREE,
            Icon::Plus => p::PLUS,
            Icon::X => p::X,
            Icon::Bell => p::BELL,
            Icon::Sidebar => p::SIDEBAR_SIMPLE,
            Icon::Pencil => p::PENCIL_SIMPLE,
            Icon::Trash => p::TRASH,
            Icon::Sparkle => p::SPARKLE,
            Icon::Star => p::STAR,
            Icon::Mic => p::MICROPHONE,
            Icon::Sliders => p::SLIDERS_HORIZONTAL,
            Icon::Monitor => p::LAPTOP,
            Icon::Shield => p::SHIELD_CHECK,
            Icon::User => p::USER_CIRCLE,
            Icon::Key => p::KEY,
            Icon::CaretDown => p::CARET_DOWN,
            Icon::Check => p::CHECK,
            Icon::Warning => p::WARNING,
            Icon::Info => p::INFO,
            Icon::ExternalLink => p::ARROW_SQUARE_OUT,
            Icon::MagicWand => p::MAGIC_WAND,
            Icon::Chat => p::CHAT_CIRCLE,
            Icon::Briefcase => p::BRIEFCASE,
            Icon::Envelope => p::ENVELOPE_SIMPLE,
            Icon::Note => p::NOTE_PENCIL,
            Icon::Keyboard => p::KEYBOARD,
            Icon::Clock => p::CLOCK,
        }
    }
}

/// Paints `icon` centered in a `size`×`size` box at the cursor.
pub fn icon(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui.painter(), rect.center(), icon, size, color);
    resp
}

/// Paints `icon` centered on `center` without allocating space.
pub fn paint_icon(painter: &egui::Painter, center: egui::Pos2, icon: Icon, size: f32, color: Color32) {
    painter.text(center, Align2::CENTER_CENTER, icon.glyph(), theme::icons(size), color);
}

/// Square icon button (28 px, 18 px glyph, muted; hover shows a soft rounded background). Adds `tooltip` if non-empty.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    icon_button_sized(ui, icon, 28.0, 18.0, tooltip)
}

pub fn icon_button_sized(ui: &mut Ui, icon: Icon, box_size: f32, glyph: f32, tooltip: &str) -> Response {
    let t = Theme::of(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(box_size), Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() || resp.is_pointer_button_down_on() {
            ui.painter().rect_filled(rect, 6.0, t.nav_active);
        }
        let color = if !ui.is_enabled() {
            t.text_faint
        } else if resp.hovered() {
            t.text
        } else {
            t.text_muted
        };
        paint_icon(ui.painter(), rect.center(), icon, glyph, color);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) }
}

// ---------------------------------------------------------------------------------------------
// Buttons

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// Black (white in dark mode): "Add new", "Save".
    Primary,
    /// Warm grey: "Change", "Cancel", "Show me how".
    Secondary,
    /// No fill until hovered.
    Ghost,
    /// Red: destructive confirmations.
    Danger,
}

/// A 36 px tall rounded button. Build with [`Button::primary`] / [`Button::secondary`] and add with `ui.add(..)`.
pub struct Button {
    text: String,
    kind: ButtonKind,
    icon: Option<Icon>,
    min_width: f32,
    height: f32,
    font_size: f32,
    radius: f32,
}

impl Button {
    pub fn new(kind: ButtonKind, text: impl Into<String>) -> Self {
        Button { text: text.into(), kind, icon: None, min_width: 0.0, height: 36.0, font_size: 14.0, radius: 8.0 }
    }
    /// Explicit height (e.g. 32 to line up with [`dropdown`] / [`input_field`], 40 for a hero action).
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    /// Corner radius (default 8).
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn primary(text: impl Into<String>) -> Self {
        Self::new(ButtonKind::Primary, text)
    }
    pub fn secondary(text: impl Into<String>) -> Self {
        Self::new(ButtonKind::Secondary, text)
    }
    pub fn ghost(text: impl Into<String>) -> Self {
        Self::new(ButtonKind::Ghost, text)
    }
    pub fn danger(text: impl Into<String>) -> Self {
        Self::new(ButtonKind::Danger, text)
    }
    /// Leading icon.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
    /// 30 px tall, 13 px text.
    pub fn small(mut self) -> Self {
        self.height = 30.0;
        self.font_size = 13.0;
        self
    }
}

impl Widget for Button {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::of(ui);
        let (fill, hover_fill, fg) = match self.kind {
            ButtonKind::Primary => (t.primary_btn_bg, theme::mix(t.primary_btn_bg, t.primary_btn_fg, 0.16), t.primary_btn_fg),
            ButtonKind::Secondary => (t.secondary_btn_bg, theme::mix(t.secondary_btn_bg, t.text, 0.07), t.text),
            ButtonKind::Ghost => (Color32::TRANSPARENT, t.nav_active, t.text),
            ButtonKind::Danger => (t.danger, theme::mix(t.danger, Color32::BLACK, 0.12), Color32::WHITE),
        };
        let galley = ui.painter().layout_no_wrap(self.text.clone(), theme::medium(self.font_size), fg);
        let icon_w = if self.icon.is_some() { self.font_size + 6.0 } else { 0.0 };
        let pad = if self.height < 34.0 { 12.0 } else { 16.0 };
        let w = (galley.size().x + icon_w + 2.0 * pad).max(self.min_width);
        let (rect, resp) = ui.allocate_exact_size(vec2(w, self.height), Sense::click());
        if ui.is_rect_visible(rect) {
            let enabled = ui.is_enabled();
            let bg = if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) { hover_fill } else { fill };
            let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
            ui.painter().rect_filled(rect, self.radius, fade(bg));
            let content_w = galley.size().x + icon_w;
            let mut x = rect.center().x - content_w / 2.0;
            if let Some(i) = self.icon {
                paint_icon(ui.painter(), pos2(x + self.font_size / 2.0, rect.center().y), i, self.font_size + 2.0, fade(fg));
                x += icon_w;
            }
            let pos = pos2(x, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(pos, galley, fade(fg));
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(Button::primary(text))
}

pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(Button::secondary(text))
}

// ---------------------------------------------------------------------------------------------
// Toggle

/// iOS-style 36×20 switch: black (white in dark mode) when on, grey when off; the knob slides over 120 ms.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let t = Theme::of(ui);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
    if ui.is_rect_visible(rect) {
        let fade = |c: Color32| if ui.is_enabled() { c } else { c.gamma_multiply(0.5) };
        ui.painter().rect_filled(rect, 10.0, fade(theme::mix(t.toggle_off, t.toggle_on, k)));
        let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), k);
        let knob = if t.dark { theme::mix(Color32::from_gray(0xE8), t.bg, k) } else { Color32::WHITE };
        let c = pos2(x, rect.center().y);
        ui.painter().circle_filled(c + vec2(0.0, 0.5), 8.0, Color32::from_black_alpha(25));
        ui.painter().circle_filled(c, 8.0, fade(knob));
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, ""));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------------------------------------
// Text bits

/// 11 px uppercase, letter-spaced, faint label.
pub fn section_label(ui: &mut Ui, text: &str) -> Response {
    let t = Theme::of(ui);
    ui.add(egui::Label::new(section_label_text(text, t.text_faint)).selectable(false))
}

pub fn section_label_text(text: &str, color: Color32) -> RichText {
    RichText::new(text.to_uppercase()).size(11.0).medium().extra_letter_spacing(0.9).color(color)
}

/// Small rounded badge, e.g. "Learned" or "Beta".
pub fn pill_badge(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::medium(11.5), fg);
    let size = galley.size() + vec2(14.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, rect.height() / 2.0, bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

/// 1 px horizontal rule in `border`.
pub fn divider(ui: &mut Ui) {
    let t = Theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0_f32, t.border));
}

/// Centered icon + title + body for empty lists.
pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, body: &str) {
    let t = Theme::of(ui);
    ui.add_space(36.0);
    ui.vertical_centered(|ui| {
        self::icon(ui, icon, 28.0, t.text_faint);
        ui.add_space(4.0);
        ui.label(RichText::new(title).size(15.0).medium().color(t.text));
        if !body.is_empty() {
            ui.label(RichText::new(body).color(t.text_muted));
        }
    });
    ui.add_space(36.0);
}

// ---------------------------------------------------------------------------------------------
// Cards & lists

/// Frame for a rounded card: 1 px `border`, radius 16, 24 px padding. Adjust with the `Frame` builder methods.
pub fn card_frame(ui: &Ui, fill: Color32) -> Frame {
    let t = Theme::of(ui);
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(24))
}

/// Rounded bordered card filling the available width. `fill` is usually `t.surface` or `t.bg`.
pub fn card<R>(ui: &mut Ui, fill: Color32, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    card_frame(ui, fill).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    })
}

/// A bordered list (radius 12) whose rows get dividers and a hover highlight. See [`ListCard::row`].
pub fn list_card<R>(ui: &mut Ui, add: impl FnOnce(&mut ListCard<'_>) -> R) -> R {
    let t = Theme::of(ui);
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(&mut ListCard { ui, rows: 0 })
        })
        .inner
}

pub struct ListCard<'a> {
    pub ui: &'a mut Ui,
    rows: usize,
}

impl ListCard<'_> {
    /// One row: min 52 px tall, 16 px horizontal padding, content laid out left-to-right and vertically centered.
    /// `add` gets `hovered` (from the previous frame) so it can reveal trailing actions. Returns a hover-only response.
    pub fn row(&mut self, id_salt: impl Hash, add: impl FnOnce(&mut Ui, bool)) -> Response {
        let first = self.rows == 0;
        self.rows += 1;
        list_row(self.ui, id_salt, first, add)
    }
}

/// A list row (usually via [`ListCard::row`]). `first` suppresses the divider above it.
/// The row is at least 52 px tall and grows with its content (height is remembered from the previous frame so
/// that content can be vertically centered).
pub fn list_row(ui: &mut Ui, id_salt: impl Hash, first: bool, add: impl FnOnce(&mut Ui, bool)) -> Response {
    let t = Theme::of(ui);
    let id = ui.id().with(("list_row", id_salt));
    let prev_h = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(52.0).max(52.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), prev_h), Sense::hover());
    let hovered = ui.rect_contains_pointer(rect);
    if !first {
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, t.border));
    }
    if hovered {
        ui.painter().rect_filled(rect.shrink2(vec2(4.0, 3.0)), 8.0, t.surface_sunken);
    }
    let inner = rect.shrink2(vec2(16.0, 0.0));
    let mut cui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)).id_salt(id));
    cui.spacing_mut().item_spacing.y = 4.0;
    add(&mut cui, hovered);
    let h = cui.min_rect().height().max(52.0);
    if (h - prev_h).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, h));
        ui.ctx().request_discard("list row height");
    }
    resp
}

/// Small filled status dot (8 px), vertically centered in a 16 px box.
pub fn status_dot(ui: &mut Ui, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    resp
}

// ---------------------------------------------------------------------------------------------
// Settings rows

/// Grouped settings rows on a `surface_sunken` card (radius 12) with dividers between rows.
///
/// ```ignore
/// settings_group(ui, |g| {
///     g.row("Dictation sounds", "", |ui| { toggle(ui, &mut s.enable_sounds); });
///     g.row("Microphone", "Built-in mic", |ui| { secondary_button(ui, "Change"); });
/// });
/// ```
pub fn settings_group<R>(ui: &mut Ui, add: impl FnOnce(&mut SettingsGroup<'_>) -> R) -> R {
    let t = Theme::of(ui);
    let r = Frame::new()
        .fill(t.surface_sunken)
        .corner_radius(CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(&mut SettingsGroup { ui, rows: 0 })
        })
        .inner;
    ui.add_space(24.0);
    r
}

pub struct SettingsGroup<'a> {
    pub ui: &'a mut Ui,
    rows: usize,
}

impl SettingsGroup<'_> {
    /// Title (14 px medium) + optional caption (13 px muted) on the left, `control` right-aligned and centered.
    pub fn row(&mut self, title: &str, caption: impl Into<WidgetText>, control: impl FnOnce(&mut Ui)) -> Response {
        self.divider();
        self.rows += 1;
        settings_row(self.ui, title, caption, control)
    }

    /// Free-form content inside the group (16 px horizontal / 14 px vertical padding), with a divider above.
    pub fn custom<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.divider();
        self.rows += 1;
        Frame::new()
            .inner_margin(Margin::symmetric(16, 14))
            .show(self.ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                add(ui)
            })
            .inner
    }

    fn divider(&mut self) {
        if self.rows > 0 {
            let t = Theme::of(self.ui);
            let (rect, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), 1.0), Sense::hover());
            self.ui.painter().hline(rect.shrink2(vec2(16.0, 0.0)).x_range(), rect.center().y, Stroke::new(1.0_f32, t.border));
        }
    }
}

/// A settings row without the group frame: 16×14 padding, title 14 px medium, caption 13 px muted.
pub fn settings_row(ui: &mut Ui, title: &str, caption: impl Into<WidgetText>, control: impl FnOnce(&mut Ui)) -> Response {
    let t = Theme::of(ui);
    let id = ui.id().with(("settings_row", title));
    let pad = vec2(16.0, 14.0);
    let prev_h = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(32.0);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, prev_h.max(32.0) + 2.0 * pad.y), Sense::hover());
    let inner = rect.shrink2(pad);

    // Control first (right-aligned, vertically centered) so we know how much room the text has.
    let mut cui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)).id_salt(id));
    control(&mut cui);
    let ctrl = cui.min_rect();
    let ctrl_w = if ctrl.width() > 0.5 { ctrl.width() + 24.0 } else { 0.0 };
    let text_w = (inner.width() - ctrl_w).max(120.0);

    let title_g = ui.painter().layout(title.to_owned(), theme::medium(14.0), t.text, text_w);
    let caption: WidgetText = caption.into();
    let caption_g = if caption.is_empty() {
        None
    } else {
        let style = ui.style().clone();
        let job = caption
            .color(t.text_muted)
            .into_layout_job(&style, egui::FontSelection::FontId(theme::regular(13.0)), Align::Min);
        let mut job = (*job).clone();
        job.wrap.max_width = text_w;
        Some(ui.fonts_mut(|f| f.layout_job(job)))
    };
    let gap = 2.0;
    let text_h = title_g.size().y + caption_g.as_ref().map_or(0.0, |g| g.size().y + gap);
    let mut y = inner.center().y - text_h / 2.0;
    let tg_h = title_g.size().y;
    ui.painter().galley(pos2(inner.left(), y), title_g, t.text);
    y += tg_h + gap;
    if let Some(g) = caption_g {
        ui.painter().galley(pos2(inner.left(), y), g, t.text_muted);
    }

    let content_h = text_h.max(ctrl.height());
    if (content_h - prev_h).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, content_h));
        ui.ctx().request_discard("settings row height");
    }
    resp
}

/// 14 px semibold header above a settings group ("App settings", "Sound"), 10 px gap below.
/// Groups end with a 24 px gap, so no leading space is added here.
pub fn group_header(ui: &mut Ui, text: &str) {
    let t = Theme::of(ui);
    let sp = ui.spacing().item_spacing.y;
    ui.label(RichText::new(text).size(14.0).semibold().color(t.text));
    ui.add_space((10.0 - sp).max(0.0));
}

// ---------------------------------------------------------------------------------------------
// Dropdown / segmented control

fn control_border(t: &Theme, hovered: bool, focused: bool) -> Color32 {
    if focused {
        theme::mix(t.border, t.text, 0.45)
    } else if hovered {
        theme::mix(t.border, t.text, 0.18)
    } else {
        t.border
    }
}

/// Dropdown: 32 px field (`surface`, 1 px border, radius 8, text left, caret right) opening a rounded
/// `surface` popup with 30 px rows, hover tint and a check on the selected row. `options` are `(value, label)`.
/// The returned response is marked `changed()` when the selection changes.
pub fn dropdown<T: PartialEq + Clone, S: AsRef<str>>(
    ui: &mut Ui,
    id_salt: impl Hash,
    selected: &mut T,
    options: &[(T, S)],
    width: f32,
) -> Response {
    let t = Theme::of(ui);
    let id = ui.id().with(("dropdown", id_salt));
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    let resp_id = resp.id;
    let _ = id;
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let label = options.iter().find(|(v, _)| v == selected).map(|(_, l)| l.as_ref().to_owned()).unwrap_or_default();
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
        ui.painter().rect_filled(rect, 8.0, fade(t.surface));
        ui.painter().rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0_f32, fade(control_border(&t, resp.hovered() && enabled, open))),
            StrokeKind::Inside,
        );
        let text_w = rect.width() - 12.0 - 34.0;
        let g = ui.painter().layout_no_wrap(label, theme::regular(14.0), fade(t.text));
        let clip = Rect::from_min_size(rect.min + vec2(12.0, 0.0), vec2(text_w, rect.height()));
        ui.painter()
            .with_clip_rect(clip)
            .galley(pos2(clip.left(), rect.center().y - g.size().y / 2.0), g, fade(t.text));
        paint_icon(ui.painter(), pos2(rect.right() - 16.0, rect.center().y), Icon::CaretDown, 13.0, fade(t.text_muted));
    }
    let _ = resp_id;
    let popup_w = rect.width().max(180.0);
    let frame = Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(4))
        .shadow(egui::epaint::Shadow {
            offset: [0, 6],
            blur: 20,
            spread: 0,
            color: Color32::from_black_alpha(if t.dark { 110 } else { 36 }),
        });
    let mut chosen: Option<T> = None;
    egui::Popup::menu(&resp)
        .gap(4.0)
        .width(popup_w)
        .frame(frame)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.set_min_width(popup_w - 8.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().max_height(300.0).auto_shrink([true, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (value, text) in options {
                    let is_sel = value == selected;
                    let (r, rr) = ui.allocate_exact_size(vec2(ui.available_width().max(popup_w - 8.0), 30.0), Sense::click());
                    if rr.hovered() {
                        ui.painter().rect_filled(r, 6.0, t.surface_sunken);
                    }
                    let font = if is_sel { theme::medium(14.0) } else { theme::regular(14.0) };
                    let g = ui.painter().layout_no_wrap(text.as_ref().to_owned(), font, t.text);
                    let clip = Rect::from_min_max(r.min + vec2(10.0, 0.0), r.max - vec2(30.0, 0.0));
                    ui.painter().with_clip_rect(clip).galley(pos2(clip.left(), r.center().y - g.size().y / 2.0), g, t.text);
                    if is_sel {
                        paint_icon(ui.painter(), pos2(r.right() - 16.0, r.center().y), Icon::Check, 14.0, t.text);
                    }
                    if rr.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        chosen = Some(value.clone());
                    }
                }
            });
        });
    if let Some(v) = chosen {
        if &v != selected {
            *selected = v;
            resp.mark_changed();
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Segmented control: sunken track (radius 8, 32 px), selected segment on a `surface` chip with a soft shadow.
/// Segments share the width of the widest label. Marked `changed()` when the selection changes.
pub fn segmented<T: PartialEq + Clone, S: AsRef<str>>(ui: &mut Ui, id_salt: impl Hash, selected: &mut T, options: &[(T, S)]) -> Response {
    let t = Theme::of(ui);
    let id = ui.id().with(("segmented", id_salt));
    let galleys: Vec<_> =
        options.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.as_ref().to_owned(), theme::medium(13.0), t.text)).collect();
    let seg_w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max).max(48.0) + 28.0;
    let n = options.len().max(1) as f32;
    let (rect, mut resp) = ui.allocate_exact_size(vec2(seg_w * n + 4.0, 32.0), Sense::hover());
    // On a sunken group card the track needs a slightly deeper tone to read.
    let track = theme::mix(t.surface_sunken, if t.dark { Color32::BLACK } else { t.text }, if t.dark { 0.25 } else { 0.06 });
    ui.painter().rect_filled(rect, 8.0, track);
    let inner = rect.shrink(2.0);
    let idx = options.iter().position(|(v, _)| v == selected);
    let k = ui.ctx().animate_value_with_time(id, idx.unwrap_or(0) as f32, 0.12);
    if idx.is_some() {
        let chip = Rect::from_min_size(inner.min + vec2(k * seg_w, 0.0), vec2(seg_w, inner.height()));
        ui.painter().rect_filled(chip.translate(vec2(0.0, 1.0)).expand(0.5), 6.5, Color32::from_black_alpha(if t.dark { 60 } else { 18 }));
        ui.painter().rect_filled(chip, 6.0, t.surface);
        ui.painter().rect_stroke(chip, 6.0, Stroke::new(1.0_f32, Color32::from_black_alpha(if t.dark { 40 } else { 8 })), StrokeKind::Inside);
    }
    for (i, ((value, _), g)) in options.iter().zip(galleys).enumerate() {
        let seg = Rect::from_min_size(inner.min + vec2(i as f32 * seg_w, 0.0), vec2(seg_w, inner.height()));
        let r = ui.interact(seg, id.with(i), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let on = value == selected;
        let color = if on { t.text } else { t.text_muted };
        let g = ui.painter().layout_no_wrap(g.text().to_owned(), if on { theme::medium(13.0) } else { theme::regular(13.0) }, color);
        ui.painter().galley(seg.center() - g.size() / 2.0, g, color);
        if r.clicked() && !on {
            *selected = value.clone();
            resp.mark_changed();
        }
    }
    resp
}

/// Settings-style single-line input: 32 px, `surface` fill, 1 px border, radius 8, muted hint; `password` masks it.
pub fn input_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32, password: bool) -> Response {
    let t = Theme::of(ui);
    let mut frame = Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(11, 0))
        .begin(ui);
    let inner_w = (width - 24.0).max(40.0);
    let resp = {
        let ui = &mut frame.content_ui;
        ui.add(
            egui::TextEdit::singleline(text)
                .password(password)
                .font(theme::regular(14.0))
                .hint_text(RichText::new(hint).color(t.text_faint))
                .frame(false)
                .desired_width(inner_w)
                .min_size(vec2(inner_w, 30.0))
                .vertical_align(Align::Center)
                .margin(Margin::ZERO),
        )
    };
    frame.frame.stroke = Stroke::new(1.0_f32, control_border(&t, resp.hovered(), resp.has_focus()));
    frame.end(ui);
    resp
}

// ---------------------------------------------------------------------------------------------
// Tabs

/// Underline tabs ("All", "Learned") with a 1 px baseline; `right` adds right-aligned controls (icon buttons).
/// Returns true when the selection changed.
pub fn underline_tabs(ui: &mut Ui, selected: &mut usize, labels: &[&str], right: impl FnOnce(&mut Ui)) -> bool {
    let t = Theme::of(ui);
    let mut changed = false;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        ui.set_min_height(32.0);
        for (i, label) in labels.iter().enumerate() {
            let sel = *selected == i;
            let color = if sel { t.text } else { t.text_muted };
            let galley = ui.painter().layout_no_wrap(label.to_string(), theme::medium(14.0), color);
            let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x, 32.0), Sense::click());
            let color = if resp.hovered() && !sel { t.text } else { color };
            ui.painter().galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0 - 2.0), galley, color);
            if sel {
                ui.painter().hline(rect.x_range(), rect.bottom() + 3.5, Stroke::new(2.0_f32, t.text));
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !sel {
                *selected = i;
                changed = true;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            right(ui)
        });
    });
    let r = row.response.rect;
    ui.painter().hline(r.x_range(), r.bottom() + 4.5, Stroke::new(1.0_f32, t.border));
    ui.add_space(14.0);
    changed
}

// ---------------------------------------------------------------------------------------------
// Hero banner

/// Gradient palettes for [`hero_banner`] (procedural, no photos).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeroPalette {
    /// Deep indigo → plum with a warm amber glow.
    Dusk,
    /// Teal night with a soft cyan glow.
    Ocean,
    /// Umber → rose with a golden glow.
    Ember,
}

/// What to show in a [`hero_banner`]. Text segments are `(text, emphasis)`: emphasis = italic in the serif
/// headline and semibold in the body.
pub struct Hero<'a> {
    pub headline: &'a [(&'a str, bool)],
    pub body: &'a [(&'a str, bool)],
    pub palette: HeroPalette,
    pub height: f32,
    pub dismissible: bool,
}

impl Default for Hero<'_> {
    fn default() -> Self {
        Hero { headline: &[], body: &[], palette: HeroPalette::Dusk, height: 180.0, dismissible: true }
    }
}

/// Full-width rounded (16) gradient banner with a serif headline, body text and an actions row
/// (use [`hero_chip`] / [`Button`] inside `actions`). Returns true when the ✕ was clicked.
pub fn hero_banner(ui: &mut Ui, hero: &Hero<'_>, actions: impl FnOnce(&mut Ui)) -> bool {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, hero.height), Sense::hover());
    let tex = hero_texture(ui.ctx(), hero.palette);
    ui.painter().add(
        egui::epaint::RectShape::filled(rect, 16.0, Color32::WHITE)
            .with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
    );

    let pad = 32.0;
    let content = rect.shrink2(vec2(pad, 28.0));
    let mut cui = ui.new_child(UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Min)));
    cui.set_max_width((w * 0.72).min(content.width()));
    cui.spacing_mut().item_spacing.y = 4.0;
    let mut head = egui::text::LayoutJob::default();
    for (s, em) in hero.headline {
        let font = if *em { theme::serif_italic(32.0) } else { theme::serif(32.0) };
        head.append(s, 0.0, egui::TextFormat::simple(font, Color32::WHITE));
    }
    cui.add(egui::Label::new(head).selectable(false));
    let mut body = egui::text::LayoutJob::default();
    for (s, em) in hero.body {
        let font = if *em { theme::semibold(14.5) } else { theme::regular(14.5) };
        body.append(s, 0.0, egui::TextFormat::simple(font, Color32::from_white_alpha(225)));
    }
    body.wrap.max_width = cui.max_rect().width();
    cui.add(egui::Label::new(body).selectable(false));
    cui.add_space(12.0);
    cui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        actions(ui)
    });

    if hero.dismissible {
        let c = pos2(rect.right() - 28.0, rect.top() + 28.0);
        let r = Rect::from_center_size(c, Vec2::splat(24.0));
        let resp = ui.interact(r, ui.id().with(("hero_close", hero.palette)), Sense::click());
        let a = if resp.hovered() { 110 } else { 70 };
        ui.painter().circle_filled(c, 12.0, Color32::from_black_alpha(a));
        paint_icon(ui.painter(), c, Icon::X, 13.0, Color32::WHITE);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Dismiss").clicked() {
            return true;
        }
    }
    false
}

/// Chip inside a hero banner: `strong` = light fill with dark text, otherwise translucent white with white text.
pub fn hero_chip(ui: &mut Ui, text: &str, strong: bool) -> Response {
    let (fg, bg, hover) = if strong {
        (Color32::from_rgb(0x1A, 0x1A, 0x1A), Color32::from_white_alpha(235), Color32::WHITE)
    } else {
        (Color32::WHITE, Color32::from_white_alpha(45), Color32::from_white_alpha(70))
    };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::medium(14.0), fg);
    let size = vec2(galley.size().x + 28.0, 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(rect, 8.0, if resp.hovered() { hover } else { bg });
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Builds (once per palette) a small RGBA gradient texture; bilinear sampling makes it smooth at any size.
fn hero_texture(ctx: &egui::Context, palette: HeroPalette) -> egui::TextureHandle {
    let id = Id::new(("murmur_hero_texture", palette));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let (w, h) = (192usize, 64usize);
    // (base stops left→right, glow center, glow color, glow radius)
    let (stops, glows): (&[[f32; 3]], &[([f32; 2], [f32; 3], f32)]) = match palette {
        HeroPalette::Dusk => (
            &[[0.10, 0.11, 0.22], [0.20, 0.13, 0.27], [0.33, 0.17, 0.25], [0.24, 0.14, 0.20]],
            &[([0.72, 0.45], [0.95, 0.58, 0.28], 0.38), ([0.18, 0.1], [0.30, 0.38, 0.75], 0.35), ([0.95, 0.9], [0.85, 0.35, 0.45], 0.3)],
        ),
        HeroPalette::Ocean => (
            &[[0.04, 0.12, 0.18], [0.06, 0.20, 0.26], [0.10, 0.18, 0.28], [0.08, 0.12, 0.20]],
            &[([0.75, 0.35], [0.30, 0.75, 0.80], 0.36), ([0.2, 0.9], [0.25, 0.35, 0.70], 0.3)],
        ),
        HeroPalette::Ember => (
            &[[0.20, 0.10, 0.08], [0.35, 0.16, 0.14], [0.45, 0.22, 0.20], [0.30, 0.14, 0.16]],
            &[([0.70, 0.40], [0.98, 0.72, 0.35], 0.38), ([0.15, 0.8], [0.80, 0.35, 0.45], 0.3)],
        ),
    };
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let u = x as f32 / (w - 1) as f32;
            let v = y as f32 / (h - 1) as f32;
            let f = u * (stops.len() - 1) as f32;
            let i = (f.floor() as usize).min(stops.len() - 2);
            let k = smooth(f - i as f32);
            let mut c = [0.0f32; 3];
            for ch in 0..3 {
                c[ch] = stops[i][ch] + (stops[i + 1][ch] - stops[i][ch]) * k;
            }
            for (center, color, radius) in glows {
                // Aspect-correct distance (banner is ~3:1).
                let dx = (u - center[0]) * 3.0;
                let dy = v - center[1];
                let d2 = (dx * dx + dy * dy) / (radius * radius * 9.0);
                let g = (-d2 * 2.2).exp() * 0.75;
                for ch in 0..3 {
                    c[ch] += (color[ch] - c[ch]) * g;
                }
            }
            // Darken the left side slightly so white text stays readable.
            let shade = 0.78 + 0.22 * smooth(u.min(1.0));
            pixels.push(Color32::from_rgb(
                (c[0] * shade * 255.0).clamp(0.0, 255.0) as u8,
                (c[1] * shade * 255.0).clamp(0.0, 255.0) as u8,
                (c[2] * shade * 255.0).clamp(0.0, 255.0) as u8,
            ));
        }
    }
    let image = egui::ColorImage::new([w, h], pixels);
    let tex = ctx.load_texture(format!("hero-{palette:?}"), image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

// ---------------------------------------------------------------------------------------------
// Inputs

/// Rounded single-line text input (36 px, 1 px border, `surface` fill).
pub fn text_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    let t = Theme::of(ui);
    let mut resp = None;
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            resp = Some(ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(RichText::new(hint).color(t.text_faint))
                    .frame(false)
                    .desired_width(width - 20.0)
                    .margin(Margin::ZERO),
            ));
        });
    resp.unwrap()
}

/// Rounded search input with a leading magnifying glass.
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    let t = Theme::of(ui);
    let mut resp = None;
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin { left: 8, right: 10, top: 5, bottom: 5 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                icon(ui, Icon::Search, 15.0, t.text_muted);
                resp = Some(ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(RichText::new(hint).color(t.text_faint))
                        .frame(false)
                        .desired_width(width - 40.0)
                        .margin(Margin::ZERO),
                ));
            });
        });
    resp.unwrap()
}

// ---------------------------------------------------------------------------------------------
// Modal

pub struct ModalOutput<R> {
    pub inner: R,
    /// Esc pressed (and no popup open) or the backdrop was clicked.
    pub close_requested: bool,
}

/// Centered dialog of `size` (clamped to the window) on a dimmed `overlay` backdrop, `surface` fill, radius 16,
/// no inner padding (wrap content in `Frame::new().inner_margin(..)`). Caller owns the open flag:
///
/// ```ignore
/// if state.open && modal(ctx, "add_word", vec2(440., 300.), |ui| ...).close_requested { state.open = false; }
/// ```
pub fn modal<R>(ctx: &egui::Context, id: impl Hash, size: Vec2, add: impl FnOnce(&mut Ui) -> R) -> ModalOutput<R> {
    let t = Theme::current(ctx);
    let screen = ctx.content_rect();
    let size = vec2(size.x.min(screen.width() - 40.0), size.y.min(screen.height() - 40.0));
    let frame = Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(16))
        .shadow(ctx.style().visuals.window_shadow);
    let resp = egui::Modal::new(Id::new(id)).backdrop_color(t.overlay).frame(frame).show(ctx, |ui| {
        ui.set_min_size(size);
        ui.set_max_size(size);
        add(ui)
    });
    let popup_open = resp.any_popup_open;
    let backdrop = resp.backdrop_response.clicked() && !popup_open;
    let esc = !popup_open && resp.is_top_modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    ModalOutput { inner: resp.inner, close_requested: backdrop || esc }
}

/// Title + body + right-aligned footer layout for small form dialogs inside [`modal`].
pub fn dialog_body<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Theme::of(ui);
    Frame::new()
        .inner_margin(Margin::same(24))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(18.0).semibold().color(t.text));
            ui.add_space(12.0);
            add(ui)
        })
        .inner
}

/// Paints a 1 px border rect (helper for custom cards).
pub fn stroke_rect(ui: &Ui, rect: Rect, radius: f32, color: Color32, width: f32) {
    ui.painter().rect_stroke(rect, radius, Stroke::new(width, color), StrokeKind::Inside);
}
