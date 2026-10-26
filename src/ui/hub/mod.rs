//! Hub window shell: top bar, sidebar, the rounded content card that hosts a page, and the Settings modal.
//!
//! Pages live in `src/ui/pages/*` and each exposes `pub fn show(hub: &mut Hub, ui: &mut Ui)` plus a
//! `pub const MAX_WIDTH: f32` (content column width) and a `State` struct stored here as `hub.<page>`.
//! Page code should only need the `pub(crate)` API below; the shell itself rarely needs to change.

use super::pages::{self, settings::Section};
use super::theme::Theme;
use super::widgets::{self, Icon};
use crate::storage::{save_settings, Db};
use crate::types::*;
use crossbeam_channel::Sender;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Pages reachable from the sidebar. Settings is a modal, see [`Hub::open_settings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Home,
    Dictionary,
    Snippets,
    Style,
}

pub const TOP_BAR_H: f32 = 52.0;
pub const SIDEBAR_W: f32 = 216.0;
pub const SIDEBAR_COLLAPSED_W: f32 = 64.0;
/// Gap between the content card and the window's right/bottom edges.
pub const CARD_INSET: f32 = 12.0;
pub const SETTINGS_MODAL_SIZE: egui::Vec2 = egui::Vec2::new(960.0, 640.0);

/// Database-backed lists shared by several pages, re-queried when `UiShared::data_version` or `search` changes.
#[derive(Default)]
pub struct DataCache {
    seen_version: Option<u64>,
    seen_search: String,
    /// Full-text filter for `history` (Home's search box writes this).
    pub search: String,
    pub history: Vec<HistoryEntry>,
    pub stats: Stats,
    pub words: Vec<DictEntry>,
    pub snippets: Vec<DictEntry>,
}

impl DataCache {
    fn refresh(&mut self, db: &Db, version: u64) {
        if self.seen_version == Some(version) && self.seen_search == self.search {
            return;
        }
        self.seen_version = Some(version);
        self.seen_search = self.search.clone();
        let q = self.search.trim();
        self.history = ok(db.recent_history(200, (!q.is_empty()).then_some(q)), "history");
        self.stats = ok(db.stats(), "stats");
        self.words = ok(db.list_dictionary(false), "dictionary");
        self.snippets = ok(db.list_dictionary(true), "snippets");
    }
}

pub struct Hub {
    pub(crate) shared: Arc<UiShared>,
    pub(crate) db: Arc<Db>,
    pub(crate) settings: Arc<Mutex<Settings>>,
    pub(crate) tx: Sender<AppEvent>,
    pub(crate) data: DataCache,
    /// First name from the macOS account ("Welcome back, …"); empty if unknown.
    pub(crate) user_first_name: String,

    pub(crate) route: Route,
    /// Open Settings modal section, `None` when closed.
    pub(crate) settings_modal: Option<Section>,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) help_open: bool,
    save_due: Option<Instant>,

    // Per-page state; each page file owns its struct.
    pub(crate) home: pages::home::State,
    pub(crate) dict: pages::dictionary::State,
    pub(crate) snippets: pages::snippets::State,
    pub(crate) style: pages::style::State,
    pub(crate) prefs: pages::settings::State,
}

impl Hub {
    /// `MURMUR_HUB_PAGE` picks the start page for visual checks: home, dictionary, snippets, style,
    /// settings-general, settings-system, settings-ai, settings-privacy, settings-accounts.
    pub fn new(shared: Arc<UiShared>, db: Arc<Db>, settings: Arc<Mutex<Settings>>, tx: Sender<AppEvent>) -> Hub {
        let (route, settings_modal) = match std::env::var("MURMUR_HUB_PAGE").as_deref().unwrap_or("") {
            "dictionary" => (Route::Dictionary, None),
            "snippets" => (Route::Snippets, None),
            "style" => (Route::Style, None),
            "settings-general" => (Route::Home, Some(Section::General)),
            "settings-system" => (Route::Home, Some(Section::System)),
            "settings-ai" => (Route::Home, Some(Section::AiCleanup)),
            "settings-privacy" => (Route::Home, Some(Section::Privacy)),
            "settings-accounts" => (Route::Home, Some(Section::Accounts)),
            _ => (Route::Home, None),
        };
        Hub {
            shared,
            db,
            settings,
            tx,
            data: DataCache::default(),
            user_first_name: first_name(),
            route,
            settings_modal,
            sidebar_collapsed: false,
            help_open: false,
            save_due: None,
            home: Default::default(),
            dict: Default::default(),
            snippets: Default::default(),
            style: Default::default(),
            prefs: Default::default(),
        }
    }

    // ---------- API for pages ----------

    /// Something we wrote changed the database: tell the controller and re-query next frame.
    pub(crate) fn changed(&self) {
        self.shared.bump_data();
    }

    /// Copy of the current settings (read-only use).
    pub(crate) fn settings_snapshot(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Runs `f` against a copy of the settings; if anything changed it is published, saved (debounced) and
    /// side effects run (model reload).
    pub(crate) fn with_settings<R>(&mut self, f: impl FnOnce(&mut Hub, &mut Settings) -> R) -> R {
        let before = self.settings.lock().unwrap().clone();
        let mut s = before.clone();
        let r = f(self, &mut s);
        if serde_json::to_value(&s).ok() != serde_json::to_value(&before).ok() {
            let model_changed = s.model_file != before.model_file;
            *self.settings.lock().unwrap() = s;
            self.save_due = Some(Instant::now() + Duration::from_millis(600));
            if model_changed {
                let _ = self.tx.send(AppEvent::ReloadModel);
            }
        }
        r
    }

    pub(crate) fn open_settings(&mut self, section: Section) {
        if self.settings_modal.is_none() {
            pages::settings::on_open(self);
        }
        self.settings_modal = Some(section);
    }

    #[allow(dead_code)]
    pub(crate) fn navigate(&mut self, route: Route) {
        self.route = route;
    }

    // ---------- lifecycle (called from ui.rs) ----------

    /// Writes debounced settings to disk; call every frame.
    pub fn tick(&mut self) {
        if self.save_due.is_some_and(|t| Instant::now() >= t) {
            self.flush_settings();
        }
    }

    pub fn flush_settings(&mut self) {
        if self.save_due.take().is_some() {
            let s = self.settings.lock().unwrap().clone();
            if let Err(e) = save_settings(&s) {
                log::warn!("saving settings: {e:#}");
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let version = self.shared.data_version.load(Ordering::SeqCst);
        self.data.refresh(&self.db, version);
        let t = Theme::current(ctx);

        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ctx, |ui| {
            let full = ui.max_rect();
            let side_w = if self.sidebar_collapsed { SIDEBAR_COLLAPSED_W } else { SIDEBAR_W };
            let top = Rect::from_min_max(full.min, pos2(full.right(), full.top() + TOP_BAR_H));
            let side = Rect::from_min_max(pos2(full.left(), top.bottom()), pos2(full.left() + side_w, full.bottom()));
            let card = Rect::from_min_max(
                pos2(side.right(), top.bottom()),
                pos2(full.right() - CARD_INSET, full.bottom() - CARD_INSET),
            );
            self.top_bar(ui, top, &t);
            self.sidebar(ui, side, &t);
            self.content_card(ui, card, &t);
        });

        if let Some(section) = self.settings_modal {
            self.settings_modal_ui(ctx, section, &t);
        }
        if self.help_open {
            self.help_modal(ctx, &t);
        }
    }

    // ---------- chrome ----------

    fn top_bar(&mut self, ui: &mut Ui, rect: Rect, t: &Theme) {
        // Empty top bar area drags the window (title bar is hidden); double-click zooms like a native title bar.
        let drag = ui.interact(rect, ui.id().with("titlebar_drag"), Sense::click_and_drag());
        if drag.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }

        let y = rect.center().y;
        let at = |ui: &mut Ui, x: f32, icon: Icon, tip: &str| {
            let r = Rect::from_center_size(pos2(x, y), egui::Vec2::splat(28.0));
            ui.scope_builder(UiBuilder::new().max_rect(r), |ui| widgets::icon_button_sized(ui, icon, 28.0, 19.0, tip)).inner
        };
        if at(ui, rect.left() + 108.0, Icon::Sidebar, if self.sidebar_collapsed { "Show sidebar" } else { "Hide sidebar" }).clicked() {
            self.sidebar_collapsed = !self.sidebar_collapsed;
        }
        if at(ui, rect.right() - 27.0, Icon::User, "Accounts & keys").clicked() {
            self.open_settings(Section::Accounts);
        }
        at(ui, rect.right() - 66.0, Icon::Bell, "No new notifications");
        let _ = t;
    }

    fn sidebar(&mut self, ui: &mut Ui, rect: Rect, t: &Theme) {
        let collapsed = self.sidebar_collapsed;
        let inner = rect.shrink2(vec2(12.0, 0.0));
        let mut ui = ui.new_child(UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
        ui.spacing_mut().item_spacing.y = 4.0;

        // Logo row (centered at y≈80 in a 1350×850 window).
        ui.add_space(14.0);
        let (logo, _) = ui.allocate_exact_size(vec2(inner.width(), 28.0), Sense::hover());
        let mark = Rect::from_min_size(pos2(logo.left() + if collapsed { 8.0 } else { 8.0 }, logo.top() + 4.0), vec2(22.0, 20.0));
        paint_logo_mark(ui.painter(), mark, t.text);
        if !collapsed {
            ui.painter().text(
                pos2(mark.right() + 8.0, logo.center().y),
                Align2::LEFT_CENTER,
                "Murmur",
                super::theme::semibold(20.0),
                t.text,
            );
        }
        ui.add_space(26.0);

        let has_issue = self.shared.cloud_issue.lock().unwrap().is_some();
        for (route, icon, label) in [
            (Route::Home, Icon::Home, "Home"),
            (Route::Dictionary, Icon::Book, "Dictionary"),
            (Route::Snippets, Icon::Scissors, "Snippets"),
            (Route::Style, Icon::TextAa, "Style"),
        ] {
            if nav_item(&mut ui, icon, label, self.route == route, collapsed, None).clicked() {
                self.route = route;
            }
        }

        // Bottom group pinned to the bottom edge.
        let bottom_h = 2.0 * 36.0 + 4.0 + 12.0 + 12.0;
        let bottom = Rect::from_min_max(pos2(inner.left(), inner.bottom() - bottom_h), inner.max);
        let mut b = ui.new_child(UiBuilder::new().max_rect(bottom).layout(egui::Layout::top_down(egui::Align::Min)));
        b.spacing_mut().item_spacing.y = 4.0;
        let (line, _) = b.allocate_exact_size(vec2(bottom.width(), 9.0), Sense::hover());
        b.painter().hline(line.x_range(), line.top(), Stroke::new(1.0_f32, t.border));
        let badge = has_issue.then_some("!");
        if nav_item(&mut b, Icon::Gear, "Settings", false, collapsed, badge).clicked() {
            self.open_settings(if has_issue { Section::AiCleanup } else { Section::General });
        }
        if nav_item(&mut b, Icon::Help, "Help", false, collapsed, None).clicked() {
            self.help_open = true;
        }
    }

    fn content_card(&mut self, ui: &mut Ui, card: Rect, t: &Theme) {
        ui.painter().rect(card, 16.0, t.surface, Stroke::new(1.0_f32, t.border), StrokeKind::Inside);
        // Keep scrolled content clear of the rounded corners.
        let view = card.shrink(1.0);
        let mut cui = ui.new_child(UiBuilder::new().max_rect(view).layout(egui::Layout::top_down(egui::Align::Min)));
        cui.set_clip_rect(view);
        let max_w = match self.route {
            Route::Home => pages::home::MAX_WIDTH,
            Route::Dictionary => pages::dictionary::MAX_WIDTH,
            Route::Snippets => pages::snippets::MAX_WIDTH,
            Route::Style => pages::style::MAX_WIDTH,
        };
        egui::ScrollArea::vertical()
            .id_salt(("page", self.route))
            .auto_shrink([false, false])
            .show(&mut cui, |ui| {
                let avail = ui.available_width();
                let w = max_w.min(avail - 80.0).max(200.0);
                let left = ((avail - w) / 2.0).floor();
                ui.add_space(32.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(left);
                    ui.vertical(|ui| {
                        ui.set_width(w);
                        self.banners(ui);
                        match self.route {
                            Route::Home => pages::home::show(self, ui),
                            Route::Dictionary => pages::dictionary::show(self, ui),
                            Route::Snippets => pages::snippets::show(self, ui),
                            Route::Style => pages::style::show(self, ui),
                        }
                        ui.add_space(40.0);
                    });
                });
            });
        // Scrolled content is clipped square; mask the outside of the rounded corners with the window
        // background and re-stroke the border so content appears to end inside the rounded card.
        let r = 16.0_f32;
        let mut mesh = egui::Mesh::default();
        for corner in [
            pos2(card.left(), card.top()),
            pos2(card.right(), card.top()),
            pos2(card.right(), card.bottom()),
            pos2(card.left(), card.bottom()),
        ] {
            let sx = if corner.x == card.left() { 1.0 } else { -1.0 };
            let sy = if corner.y == card.top() { 1.0 } else { -1.0 };
            let center = pos2(corner.x + sx * r, corner.y + sy * r);
            let n = 12;
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(corner, t.bg);
            for i in 0..=n {
                let a = std::f32::consts::FRAC_PI_2 * i as f32 / n as f32;
                // Arc from the point on the horizontal edge to the point on the vertical edge.
                let p = pos2(center.x - sx * r * a.sin(), center.y - sy * r * a.cos());
                mesh.colored_vertex(p, t.bg);
            }
            for i in 0..n as u32 {
                mesh.add_triangle(base, base + 1 + i, base + 2 + i);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter().rect_stroke(card, 16.0, Stroke::new(1.0_f32, t.border), StrokeKind::Inside);
    }

    /// App-wide notices above any page: missing speech model, last dictation error.
    fn banners(&mut self, ui: &mut Ui) {
        let t = Theme::of(ui);
        let status = self.shared.model_status.lock().unwrap().clone();
        if status == "missing" {
            notice(ui, t.accent, Icon::Info, "No speech model is installed yet. Download one to start dictating.", |ui| {
                if ui.add(widgets::Button::secondary("Open Settings").small()).clicked() {
                    self.open_settings(Section::General);
                }
            });
        }
        if *self.shared.overlay.lock().unwrap() == OverlayState::Error {
            if let Some(err) = self.shared.last_error.lock().unwrap().clone() {
                notice(ui, t.danger, Icon::Warning, &err, |_| {});
            }
        }
    }

    // ---------- Settings modal ----------

    fn settings_modal_ui(&mut self, ctx: &egui::Context, section: Section, t: &Theme) {
        let out = widgets::modal(ctx, "settings_modal", SETTINGS_MODAL_SIZE, |ui| {
            let rect = ui.max_rect();
            let nav = Rect::from_min_max(rect.min, pos2(rect.left() + 208.0, rect.bottom()));
            let pane = Rect::from_min_max(pos2(nav.right(), rect.top()), rect.max);
            ui.allocate_rect(rect, Sense::hover());
            // Left nav on `bg`, rounded only on the left.
            ui.painter().rect_filled(
                nav,
                egui::CornerRadius { nw: 16, sw: 16, ne: 0, se: 0 },
                t.bg,
            );
            ui.painter().vline(nav.right(), nav.y_range(), Stroke::new(1.0_f32, t.border));

            let mut nui = ui.new_child(UiBuilder::new().max_rect(nav.shrink2(vec2(8.0, 0.0))));
            nui.spacing_mut().item_spacing.y = 2.0;
            nui.add_space(22.0);
            let mut chosen = None;
            for (label, items) in Section::GROUPS {
                nui.horizontal(|ui| {
                    ui.add_space(8.0);
                    widgets::section_label(ui, label);
                });
                nui.add_space(4.0);
                for s in *items {
                    if modal_nav_item(&mut nui, s.icon(), s.label(), section == *s).clicked() {
                        chosen = Some(*s);
                    }
                }
                nui.add_space(14.0);
            }
            ui.painter().text(
                pos2(nav.left() + 16.0, nav.bottom() - 26.0),
                Align2::LEFT_CENTER,
                format!("Murmur v{}", env!("CARGO_PKG_VERSION")),
                super::theme::regular(12.0),
                t.text_muted,
            );

            // The serif title stays put while the section scrolls beneath it.
            const HEADER: f32 = 84.0;
            let pane = pane.shrink2(vec2(0.0, 10.0));
            let (header, body) = pane.split_top_bottom_at_y(pane.top() + HEADER);
            ui.painter().text(
                pos2(header.left() + 48.0, header.top() + 26.0),
                Align2::LEFT_TOP,
                section.title(),
                super::theme::serif(28.0),
                t.text,
            );
            let mut pui = ui.new_child(UiBuilder::new().max_rect(body));
            pui.set_clip_rect(body);
            egui::ScrollArea::vertical()
                .id_salt(("settings_pane", section))
                .auto_shrink([false, false])
                .show(&mut pui, |ui| {
                    egui::Frame::new().inner_margin(egui::Margin { left: 48, right: 48, top: 0, bottom: 32 }).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        pages::settings::show(self, ui, section);
                    });
                });
            chosen
        });
        if let Some(s) = out.inner {
            self.settings_modal = Some(s);
        }
        if out.close_requested {
            self.settings_modal = None;
            pages::settings::on_close(self);
        }
    }

    fn help_modal(&mut self, ctx: &egui::Context, t: &Theme) {
        let out = widgets::modal(ctx, "help_modal", vec2(420.0, 260.0), |ui| {
            widgets::dialog_body(ui, "Using Murmur", |ui| {
                for (keys, what) in [
                    ("Hold fn", "Speak, release to paste"),
                    ("fn + space", "Hands-free dictation"),
                    ("esc", "Cancel the current dictation"),
                ] {
                    ui.horizontal(|ui| {
                        widgets::pill_badge(ui, keys, t.text, t.surface_sunken);
                        ui.label(RichText::new(what).color(t.text_muted));
                    });
                }
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| widgets::primary_button(ui, "Done").clicked())
                    .inner
            })
        });
        if out.inner || out.close_requested {
            self.help_open = false;
        }
    }
}

// ---------- shell pieces ----------

/// Sidebar item: 36 px tall, radius 8, 18 px icon + 15 px medium label. Active = `nav_active`, hover = 60 %.
fn nav_item(ui: &mut Ui, icon: Icon, label: &str, active: bool, collapsed: bool, badge: Option<&str>) -> egui::Response {
    let t = Theme::of(ui);
    let w = if collapsed { 40.0 } else { ui.available_width() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
    let fill = if active {
        t.nav_active
    } else if resp.hovered() {
        t.nav_active.gamma_multiply(0.6)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    let icon_c = pos2(rect.left() + 12.0 + 9.0, rect.center().y);
    widgets::paint_icon(ui.painter(), icon_c, icon, 18.0, t.text);
    if !collapsed {
        ui.painter().text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, label, super::theme::medium(15.0), t.text);
    }
    if let Some(b) = badge {
        let c = if collapsed { pos2(rect.right() - 6.0, rect.top() + 8.0) } else { pos2(rect.right() - 16.0, rect.center().y) };
        ui.painter().circle_filled(c, 8.0, t.danger);
        ui.painter().text(c, Align2::CENTER_CENTER, b, super::theme::semibold(11.0), Color32::WHITE);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if collapsed { resp.on_hover_text(label) } else { resp }
}

/// Settings modal nav item: 34 px, 16 px icon, 14 px label.
fn modal_nav_item(ui: &mut Ui, icon: Icon, label: &str, active: bool) -> egui::Response {
    let t = Theme::of(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let fill = if active {
        t.nav_active
    } else if resp.hovered() {
        t.nav_active.gamma_multiply(0.6)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    widgets::paint_icon(ui.painter(), pos2(rect.left() + 18.0, rect.center().y), icon, 16.0, t.text);
    ui.painter().text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, label, super::theme::medium(14.0), t.text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Murmur's mark: five rounded waveform bars.
pub fn paint_logo_mark(painter: &egui::Painter, rect: Rect, color: Color32) {
    let heights = [0.45, 0.8, 1.0, 0.65, 0.35];
    let bar = 2.6;
    let gap = (rect.width() - bar * heights.len() as f32) / (heights.len() - 1) as f32;
    for (i, h) in heights.iter().enumerate() {
        let x = rect.left() + i as f32 * (bar + gap) + bar / 2.0;
        let half = rect.height() * h / 2.0;
        painter.line_segment(
            [pos2(x, rect.center().y - half + bar / 2.0), pos2(x, rect.center().y + half - bar / 2.0)],
            Stroke::new(bar, color),
        );
        painter.circle_filled(pos2(x, rect.center().y - half + bar / 2.0), bar / 2.0, color);
        painter.circle_filled(pos2(x, rect.center().y + half - bar / 2.0), bar / 2.0, color);
    }
}

/// Inline notice strip (tinted with `color`) with optional trailing controls.
pub fn notice(ui: &mut Ui, color: Color32, icon: Icon, text: &str, extra: impl FnOnce(&mut Ui)) {
    let t = Theme::of(ui);
    egui::Frame::new()
        .fill(color.gamma_multiply(if t.dark { 0.16 } else { 0.08 }))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.45)))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                widgets::icon(ui, icon, 17.0, color);
                ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), extra);
            });
        });
    ui.add_space(16.0);
}

// ---------- helpers shared by pages ----------

pub(crate) fn ok<T: Default>(r: anyhow::Result<T>, what: &str) -> T {
    r.unwrap_or_else(|e| {
        log::warn!("loading {what}: {e:#}");
        T::default()
    })
}

/// First word of `NSFullUserName()`, falling back to `$USER`.
fn first_name() -> String {
    let full = objc2_foundation::NSFullUserName().to_string();
    let name = full.split_whitespace().next().map(str::to_owned).unwrap_or_default();
    if name.is_empty() { std::env::var("USER").unwrap_or_default() } else { name }
}
