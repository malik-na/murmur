//! Design tokens (UI_SPEC.md), fonts and the egui `Style` for both light and dark mode.
//!
//! Use [`Theme::current`] for colors and the font helpers ([`serif`], [`medium`], [`semibold`], …) or the
//! [`TextExt`] trait (`RichText::new("Title").size(24.0).semibold()`) for typography.

#![allow(dead_code)] // toolkit for the page modules; not every helper is used yet

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke, TextStyle,
};
use std::sync::Arc;

/// Color tokens. Field names follow UI_SPEC.md.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    /// Window, sidebar, stats card.
    pub bg: Color32,
    /// Main content card, modal right pane.
    pub surface: Color32,
    /// Grouped settings rows, chips, message bubbles.
    pub surface_sunken: Color32,
    /// Active sidebar item pill (hover uses it at 60%).
    pub nav_active: Color32,
    /// 1 px card borders and row dividers.
    pub border: Color32,
    pub text: Color32,
    /// Secondary text, times, captions.
    pub text_muted: Color32,
    /// Uppercase section labels.
    pub text_faint: Color32,
    pub primary_btn_bg: Color32,
    pub primary_btn_fg: Color32,
    /// Secondary ("Change") button background; its text uses `text`.
    pub secondary_btn_bg: Color32,
    pub toggle_on: Color32,
    pub toggle_off: Color32,
    /// Violet: selected cards, highlights, links.
    pub accent: Color32,
    pub danger: Color32,
    /// Amber, for warnings (billing, permissions missing).
    pub warning: Color32,
    /// Modal backdrop.
    pub overlay: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Theme {
    pub const LIGHT: Theme = Theme {
        dark: false,
        bg: hex(0xF5F4F0),
        surface: hex(0xFCFCFC),
        surface_sunken: hex(0xF1F0EC),
        nav_active: hex(0xEFEBE3),
        border: hex(0xE8E6E1),
        text: hex(0x1A1A1A),
        text_muted: hex(0x6B6A66),
        text_faint: hex(0x9A9893),
        primary_btn_bg: hex(0x141414),
        primary_btn_fg: Color32::WHITE,
        secondary_btn_bg: hex(0xECEAE5),
        toggle_on: hex(0x1B1B1B),
        toggle_off: hex(0xDDDCD8),
        accent: hex(0x7C5CD6),
        danger: hex(0xD64545),
        warning: hex(0xD9822B),
        overlay: Color32::from_black_alpha(71), // 28 %
    };

    pub const DARK: Theme = Theme {
        dark: true,
        bg: hex(0x161615),
        surface: hex(0x1E1E1D),
        surface_sunken: hex(0x262624),
        nav_active: hex(0x2C2B29),
        border: hex(0x33322F),
        text: hex(0xF2F1EE),
        text_muted: hex(0xA3A19C),
        text_faint: hex(0x75736E),
        primary_btn_bg: hex(0xF2F1EE),
        primary_btn_fg: hex(0x141414),
        secondary_btn_bg: hex(0x2F2E2B),
        toggle_on: hex(0xF2F1EE),
        toggle_off: hex(0x45443F),
        accent: hex(0x9B80EA),
        danger: hex(0xE5605A),
        warning: hex(0xE59A45),
        overlay: Color32::from_black_alpha(115), // 45 %
    };

    /// Tokens for the theme egui is currently using (follows the system light/dark setting).
    pub fn current(ctx: &egui::Context) -> Theme {
        if ctx.style().visuals.dark_mode { Theme::DARK } else { Theme::LIGHT }
    }

    /// Same as [`Theme::current`] for code that only has a `Ui`.
    pub fn of(ui: &egui::Ui) -> Theme {
        if ui.visuals().dark_mode { Theme::DARK } else { Theme::LIGHT }
    }
}

// ---------- fonts ----------

/// Named font families registered by [`install`].
pub const SERIF: &str = "serif";
pub const SERIF_ITALIC: &str = "serif-italic";
pub const MEDIUM: &str = "medium";
pub const SEMIBOLD: &str = "semibold";
/// Phosphor icon font only (see `widgets::Icon`).
pub const ICONS: &str = "icons";

pub fn serif(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SERIF.into()))
}
pub fn serif_italic(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SERIF_ITALIC.into()))
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
/// Figtree Regular.
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
pub fn icons(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// Typography shortcuts for `RichText`.
pub trait TextExt {
    fn medium(self) -> Self;
    fn semibold(self) -> Self;
    fn serif(self) -> Self;
    fn serif_italic(self) -> Self;
}

impl TextExt for RichText {
    fn medium(self) -> Self {
        self.family(FontFamily::Name(MEDIUM.into()))
    }
    fn semibold(self) -> Self {
        self.family(FontFamily::Name(SEMIBOLD.into()))
    }
    fn serif(self) -> Self {
        self.family(FontFamily::Name(SERIF.into()))
    }
    fn serif_italic(self) -> Self {
        self.family(FontFamily::Name(SERIF_ITALIC.into()))
    }
}

fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("figtree", include_bytes!("../../assets/fonts/Figtree-Regular.ttf"));
    add("figtree-medium", include_bytes!("../../assets/fonts/Figtree-Medium.ttf"));
    add("figtree-semibold", include_bytes!("../../assets/fonts/Figtree-SemiBold.ttf"));
    add("ebgaramond", include_bytes!("../../assets/fonts/EBGaramond-Regular.ttf"));
    add("ebgaramond-italic", include_bytes!("../../assets/fonts/EBGaramond-Italic.ttf"));
    fonts.font_data.insert(
        "phosphor".to_owned(),
        Arc::new(FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()).tweak(egui::FontTweak {
            y_offset_factor: 0.08,
            ..Default::default()
        })),
    );

    // egui's bundled fonts stay as fallbacks (symbols, emoji).
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |primary: &str| {
        let mut v = vec![primary.to_owned(), "phosphor".to_owned()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, family("figtree"));
    fonts.families.insert(FontFamily::Name(MEDIUM.into()), family("figtree-medium"));
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), family("figtree-semibold"));
    fonts.families.insert(FontFamily::Name(SERIF.into()), family("ebgaramond"));
    fonts.families.insert(FontFamily::Name(SERIF_ITALIC.into()), family("ebgaramond-italic"));
    fonts.families.insert(FontFamily::Name(ICONS.into()), vec!["phosphor".to_owned()]);
    fonts
}

// ---------- style ----------

/// Registers fonts and configures spacing, rounding, text styles and colors for both themes.
///
/// `MURMUR_THEME=light|dark` forces a theme (handy for screenshots); otherwise egui follows the system.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    match std::env::var("MURMUR_THEME").as_deref() {
        Ok("light") => ctx.set_theme(egui::ThemePreference::Light),
        Ok("dark") => ctx.set_theme(egui::ThemePreference::Dark),
        _ => {}
    }
    for (egui_theme, t) in [(egui::Theme::Light, Theme::LIGHT), (egui::Theme::Dark, Theme::DARK)] {
        ctx.style_mut_of(egui_theme, |s| apply(s, &t));
    }
}

fn apply(s: &mut egui::Style, t: &Theme) {
    use FontFamily::Proportional;
    s.text_styles = [
        (TextStyle::Small, FontId::new(12.5, Proportional)),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Heading, semibold(24.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();

    let sp = &mut s.spacing;
    sp.item_spacing = egui::vec2(8.0, 8.0);
    sp.button_padding = egui::vec2(12.0, 6.0);
    sp.interact_size.y = 30.0;
    sp.combo_height = 280.0;
    sp.menu_margin = egui::Margin::same(6);
    sp.window_margin = egui::Margin::same(12);
    sp.scroll = egui::style::ScrollStyle::floating();

    let v = &mut s.visuals;
    *v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = t.bg;
    v.window_fill = t.surface;
    v.window_stroke = Stroke::new(1.0_f32, t.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if t.dark { 90 } else { 28 }) };
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if t.dark { 80 } else { 22 }) };
    v.extreme_bg_color = t.surface;
    v.text_edit_bg_color = Some(t.surface);
    v.faint_bg_color = t.surface_sunken;
    v.code_bg_color = t.surface_sunken;
    v.hyperlink_color = t.accent;
    v.weak_text_color = Some(t.text_muted);
    v.error_fg_color = t.danger;
    v.warn_fg_color = t.warning;
    v.selection.bg_fill = t.accent.gamma_multiply(if t.dark { 0.45 } else { 0.25 });
    v.selection.stroke = Stroke::new(1.0_f32, t.accent);
    v.striped = false;

    let r = CornerRadius::same(8);
    let hover_fill = mix(t.secondary_btn_bg, t.text, 0.06);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.surface;
    w.noninteractive.weak_bg_fill = t.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.border); // separators
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, t.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = t.toggle_off; // checkbox / slider background
    w.inactive.weak_bg_fill = t.secondary_btn_bg; // button fill
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, t.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = mix(t.toggle_off, t.text, 0.08);
    w.hovered.weak_bg_fill = hover_fill;
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.5_f32, t.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = mix(t.toggle_off, t.text, 0.14);
    w.active.weak_bg_fill = mix(t.secondary_btn_bg, t.text, 0.12);
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.5_f32, t.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = t.surface;
    w.open.weak_bg_fill = hover_fill;
    w.open.bg_stroke = Stroke::new(1.0_f32, t.border);
    w.open.fg_stroke = Stroke::new(1.0_f32, t.text);
    w.open.corner_radius = r;
}

/// Linear blend from `a` to `b` (t = 0 → a, 1 → b), opaque.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
