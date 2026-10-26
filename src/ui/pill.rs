//! Floating status pill ("Flow bar"): a small transparent window docked bottom-center with an animated pill inside.
//!
//! Geometry follows UI_SPEC.md: resting 40x8 sliver (hover -> 50x30), listening 73x30, hands-free 102x30,
//! processing 98x30, error auto x30, toast auto x30. Size changes ease over ~110 ms.
//!
//! The window itself is resized per state so that mouse passthrough only covers what is needed: while resting it is
//! a tiny non-passthrough hit area (so hover can be detected), in hands-free it is interactive (buttons), otherwise
//! clicks pass through to the apps underneath.
//!
//! Visual check: run with `MURMUR_PILL_DEMO=1` to cycle through resting, hover, PTT, hands-free, processing and error
//! every 2 s with a synthetic waveform (ignores real dictation state).

use crate::types::*;
use crossbeam_channel::Sender;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
    ViewportBuilder, ViewportId, WindowLevel,
};
use std::time::Duration;

/// Window for the active states (fits the widest pill, 102 px, plus error text up to ~ERR_CHARS).
const WIN: Vec2 = Vec2::new(200.0, 44.0);
const TOAST_WIN: Vec2 = Vec2::new(300.0, 44.0);
/// Resting window: just the sliver plus a little slack; hovered window: enough for the 50x30 pill.
const REST_WIN: Vec2 = Vec2::new(48.0, 16.0);
const HOVER_WIN: Vec2 = Vec2::new(60.0, 38.0);
/// Gap between the pill bottom and the window bottom.
const BOTTOM_INSET: f32 = 4.0;
const TOAST_SECS: f32 = 3.5;
const TOAST_FADE: f32 = 0.25;
const TOAST_CHARS: usize = 38;
const ERR_CHARS: usize = 26;
const BARS: usize = 9;
/// Size/opacity transition time (100 ms ease-out).
const ANIM: f32 = 0.11;

const RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);

#[derive(Default)]
pub struct Pill;

fn hover_id() -> Id {
    Id::new("murmur_pill_hovered")
}

impl Pill {
    /// `always_visible` is the "Show floating bar at all times" setting: when off, only the idle
    /// sliver is hidden; dictation, processing, errors and toasts still appear.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        shared: &UiShared,
        tx: &Sender<AppEvent>,
        monitor: Option<Vec2>,
        always_visible: bool,
    ) {
        let mut state = *shared.overlay.lock().unwrap();
        let demo = std::env::var_os("MURMUR_PILL_DEMO").is_some();
        let mut force_hover = false;
        let mut demo_levels: Option<Vec<f32>> = None;
        if demo {
            let t = ctx.input(|i| i.time);
            let step = (t / 2.0) as usize % 6;
            (state, force_hover) = match step {
                0 => (OverlayState::Resting, false),
                1 => (OverlayState::Resting, true),
                2 => (OverlayState::ListeningPtt, false),
                3 => (OverlayState::ListeningHandsFree, false),
                4 => (OverlayState::Processing, false),
                _ => (OverlayState::Error, false),
            };
            demo_levels = Some((0..BARS).map(|i| ((t * 5.0 + i as f64 * 0.9).sin() * 0.5 + 0.5) as f32).collect());
        }
        // Current toast (clearing it once expired); only shown while the pill is otherwise resting.
        let toast = {
            let mut t = shared.toast.lock().unwrap();
            if t.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() >= TOAST_SECS) {
                *t = None;
            }
            t.as_ref().map(|(m, at)| (m.clone(), at.elapsed().as_secs_f32()))
        };
        let toast = toast.filter(|_| state == OverlayState::Resting && !demo);
        let hovered = force_hover || ctx.data(|d| d.get_temp::<bool>(hover_id())).unwrap_or(false);
        let resting_only = state == OverlayState::Resting && toast.is_none();
        // Hidden idle bar: keep the (transparent, click-through) window alive rather than closing
        // it, so its all-Spaces setup is done once and not redone on every dictation.
        let hidden = resting_only && !always_visible && !demo;
        let win = if toast.is_some() {
            TOAST_WIN
        } else if resting_only {
            if hovered { HOVER_WIN } else { REST_WIN }
        } else {
            WIN
        };
        let monitor = monitor.filter(|m| m.x > 0.0 && m.y > 0.0).unwrap_or(Vec2::new(1512.0, 982.0));
        // Window bottom stays at the same height for every state so the pill never jumps.
        let pos = Pos2::new(((monitor.x - win.x) / 2.0).round(), (monitor.y - 70.0 - win.y).round());
        // Resting needs real mouse events for hover; hands-free needs them for its buttons.
        let interactive = (state == OverlayState::ListeningHandsFree || resting_only) && !hidden;

        if hidden {
            ctx.request_repaint_after(Duration::from_millis(250));
        } else if toast.is_some() || state != OverlayState::Resting || demo {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else {
            // Resting: poll a bit faster than before so hover feels responsive.
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        let builder = ViewportBuilder::default()
            .with_title("Murmur pill")
            .with_decorations(false)
            .with_transparent(true)
            .with_has_shadow(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_resizable(false)
            .with_taskbar(false)
            .with_active(false)
            .with_inner_size(win)
            .with_position(pos)
            .with_mouse_passthrough(!interactive);

        ctx.show_viewport_immediate(ViewportId::from_hash_of("murmur_pill"), builder, |ctx, _class| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                if hidden {
                    return;
                }
                let over = resting_only && ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| ui.max_rect().contains(p)));
                ctx.data_mut(|d| d.insert_temp(hover_id(), over));
                draw(ui, state, over || force_hover, shared, tx, demo_levels.as_deref());
                if let Some((msg, age)) = &toast {
                    draw_toast(ui, msg, *age);
                }
            });
        });
    }
}

/// Pill rect of the given size: horizontally centered, bottom `BOTTOM_INSET` above the window bottom.
fn anchored(ui: &egui::Ui, size: Vec2) -> Rect {
    let r = ui.max_rect();
    Rect::from_center_size(Pos2::new(r.center().x, r.bottom() - BOTTOM_INSET - size.y / 2.0), size)
}

fn draw(ui: &mut egui::Ui, state: OverlayState, hovered: bool, shared: &UiShared, tx: &Sender<AppEvent>, demo_levels: Option<&[f32]>) {
    let ctx = ui.ctx().clone();
    let id = Id::new("murmur_pill_anim");
    let err_msg = if state == OverlayState::Error {
        short(&shared.last_error.lock().unwrap().clone().unwrap_or_else(|| "Something went wrong".into()), ERR_CHARS)
    } else {
        String::new()
    };
    let target = match state {
        OverlayState::Resting if hovered => Vec2::new(50.0, 30.0),
        OverlayState::Resting => Vec2::new(40.0, 8.0),
        OverlayState::ListeningPtt => Vec2::new(73.0, 30.0),
        OverlayState::Processing => Vec2::new(98.0, 30.0),
        OverlayState::ListeningHandsFree => Vec2::new(102.0, 30.0),
        OverlayState::Error => {
            let g = ui.painter().layout_no_wrap(err_msg.clone(), FontId::proportional(11.5), Color32::WHITE);
            Vec2::new((g.size().x + 40.0).min(WIN.x - 4.0), 30.0)
        }
    };
    let size = Vec2::new(
        ctx.animate_value_with_time(id.with("w"), target.x, ANIM),
        ctx.animate_value_with_time(id.with("h"), target.y, ANIM),
    );
    let rect = anchored(ui, size);
    let painter = ui.painter();
    // Corner radius: 6 for the sliver, fully round once expanded.
    let grow = ((size.y - 8.0) / 22.0).clamp(0.0, 1.0);
    let radius = CornerRadius::from(6.0 + (size.y / 2.0 - 6.0) * grow);
    let resting_look = state == OverlayState::Resting && !hovered;

    // Fill: black @ 55% (sliver) -> @ 85% (active); the expansion animation drives the blend.
    let fill = Color32::from_black_alpha((140.0 + (217.0 - 140.0) * grow) as u8);
    let stroke = if state == OverlayState::Error {
        Stroke::new(1.5_f32, RED)
    } else {
        // White @ 50% on the sliver, fading to a faint rim on the expanded pill.
        Stroke::new(1.0_f32, Color32::from_white_alpha((128.0 - (128.0 - 40.0) * grow) as u8))
    };
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside);
    let _ = resting_look;

    // Content fades in once the pill has (mostly) grown.
    let alpha = ((size.y - 18.0) / 12.0).clamp(0.0, 1.0);
    if alpha <= 0.0 || state == OverlayState::Resting {
        return;
    }
    let white = Color32::from_white_alpha((235.0 * alpha) as u8);
    let c = rect.center();

    match state {
        OverlayState::ListeningPtt => bars(ui, c, size.y, alpha, shared, demo_levels),
        OverlayState::ListeningHandsFree => {
            bars(ui, c, size.y, alpha, shared, demo_levels);
            let cancel = Rect::from_center_size(Pos2::new(rect.left() + 17.0, c.y), Vec2::splat(22.0));
            let finish = Rect::from_center_size(Pos2::new(rect.right() - 17.0, c.y), Vec2::splat(22.0));
            let rc = ui.interact(cancel, id.with("cancel"), Sense::click());
            let rf = ui.interact(finish, id.with("finish"), Sense::click());
            let p = ui.painter();
            // Cancel: translucent circle with a white X.
            let a = if rc.hovered() { 70 } else { 40 };
            p.circle_filled(cancel.center(), 9.0, Color32::from_white_alpha((a as f32 * alpha) as u8));
            let s = Stroke::new(1.5_f32, white);
            let k = cancel.center();
            p.line_segment([k + Vec2::new(-3.0, -3.0), k + Vec2::new(3.0, 3.0)], s);
            p.line_segment([k + Vec2::new(-3.0, 3.0), k + Vec2::new(3.0, -3.0)], s);
            // Stop: red rounded square.
            let red = if rf.hovered() { Color32::from_rgb(0xFF, 0x5A, 0x5F) } else { RED };
            p.rect_filled(Rect::from_center_size(finish.center(), Vec2::splat(12.0)), 3.0, red.gamma_multiply(alpha));
            if rc.clicked() {
                let _ = tx.send(AppEvent::Cancel);
            }
            if rf.clicked() {
                let _ = tx.send(AppEvent::HandsFreeToggle);
            }
            if rc.hovered() || rf.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        OverlayState::Processing => {
            let t = ctx.input(|i| i.time) as f32;
            for i in 0..3 {
                // Sequential pulse: each dot lags the previous one.
                let phase = (t * 6.0 - i as f32 * 0.9).sin() * 0.5 + 0.5;
                let a = (0.3 + 0.7 * phase) * alpha;
                let pos = Pos2::new(c.x + (i as f32 - 1.0) * 10.0, c.y);
                painter.circle_filled(pos, 2.4 + phase * 0.8, Color32::from_white_alpha((235.0 * a) as u8));
            }
        }
        OverlayState::Error => {
            painter.text(c, Align2::CENTER_CENTER, err_msg, FontId::proportional(11.5), white);
        }
        OverlayState::Resting => {}
    }
}

/// Waveform: ~9 white bars, 2 px wide with 2 px gaps, rounded ends; newest level on the right, heights eased per bar.
fn bars(ui: &egui::Ui, center: Pos2, height: f32, alpha: f32, shared: &UiShared, demo_levels: Option<&[f32]>) {
    let levels = match demo_levels {
        Some(l) => l.to_vec(),
        None => shared.levels.lock().unwrap().clone(),
    };
    let pad = BARS.saturating_sub(levels.len());
    let recent = &levels[levels.len().saturating_sub(BARS)..];
    let (w, gap, min_h) = (2.0, 2.0, 2.0);
    let total = BARS as f32 * w + (BARS - 1) as f32 * gap;
    let max_h = height - 12.0;
    let ctx = ui.ctx();
    for i in 0..BARS {
        let level = if i < pad { 0.0 } else { recent[i - pad] };
        let target = min_h + (level.clamp(0.0, 1.0).powf(0.6) * (max_h - min_h));
        let h = ctx.animate_value_with_time(Id::new("murmur_pill_bar").with(i), target, 0.08);
        let x = center.x - total / 2.0 + i as f32 * (w + gap) + w / 2.0;
        let r = Rect::from_center_size(Pos2::new(x, center.y), Vec2::new(w, h.max(min_h)));
        ui.painter().rect_filled(r, 1.0, Color32::from_white_alpha((235.0 * alpha) as u8));
    }
}

fn short(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>().trim_end())
    }
}

/// Small dark label with the toast message, fading in and out over its lifetime.
fn draw_toast(ui: &mut egui::Ui, msg: &str, age: f32) {
    let ctx = ui.ctx().clone();
    // Fade in over the first TOAST_FADE seconds (animate_* would start at its target), animate out at the end.
    let fade_in = (age / TOAST_FADE).clamp(0.0, 1.0);
    let out_target = if age < TOAST_SECS - TOAST_FADE { 1.0 } else { 0.0 };
    let a = fade_in * ctx.animate_value_with_time(Id::new("murmur_pill_toast"), out_target, TOAST_FADE);
    if a <= 0.0 {
        return;
    }
    let font = FontId::proportional(11.5);
    let text = short(msg, TOAST_CHARS);
    let galley = ui.painter().layout_no_wrap(text, font, Color32::from_white_alpha((235.0 * a) as u8));
    let size = Vec2::new(galley.size().x + 28.0, 30.0);
    let rect = anchored(ui, size);
    let painter = ui.painter();
    let radius = CornerRadius::from(size.y / 2.0);
    painter.rect_filled(rect, radius, Color32::from_black_alpha((217.0 * a) as u8));
    painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, Color32::from_white_alpha((45.0 * a) as u8)), StrokeKind::Inside);
    painter.galley(Pos2::new(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
}
