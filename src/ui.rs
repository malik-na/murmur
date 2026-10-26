//! eframe UI: hub window (root viewport), floating pill (secondary viewport) and menu-bar tray.

mod hub;
mod macos_window;
mod pages;
mod pill;
pub mod theme;
mod tray;
pub mod widgets;

use crate::storage::Db;
use crate::types::*;
use crossbeam_channel::Sender;
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn run_ui(
    shared: Arc<UiShared>,
    db: Arc<Db>,
    settings: Arc<Mutex<Settings>>,
    tx: Sender<AppEvent>,
) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur")
            .with_inner_size([1350.0, 850.0])
            .with_min_inner_size([1000.0, 640.0])
            // The GL config is chosen from the root viewport; it must allow alpha so the pill can be transparent.
            .with_transparent(true)
            // A transparent surface also makes the native title bar see-through, so draw our panels under it.
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    eframe::run_native(
        "Murmur",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            macos_window::disable_app_nap();
            *shared.egui_ctx.lock().unwrap() = Some(cc.egui_ctx.clone());
            let tray = match tray::Tray::new() {
                Ok(t) => Some(t),
                Err(e) => {
                    log::error!("tray icon: {e:#}");
                    None
                }
            };
            Ok(Box::new(MurmurApp {
                hub: hub::Hub::new(shared.clone(), db, settings.clone(), tx.clone()),
                pill: pill::Pill,
                shared,
                settings,
                tx,
                tray,
                hub_visible: true,
                quitting: false,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

struct MurmurApp {
    shared: Arc<UiShared>,
    settings: Arc<Mutex<Settings>>,
    tx: Sender<AppEvent>,
    hub: hub::Hub,
    pill: pill::Pill,
    tray: Option<tray::Tray>,
    hub_visible: bool,
    quitting: bool,
}

impl eframe::App for MurmurApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keep ticking while hidden so tray events and state changes are noticed.
        ctx.request_repaint_after(Duration::from_millis(200));

        let actions = self.tray.as_ref().map(|t| t.poll()).unwrap_or_default();
        for action in actions {
            match action {
                tray::TrayAction::Open => {
                    self.hub_visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::TrayAction::PasteLast => {
                    let _ = self.tx.send(AppEvent::PasteLast);
                }
                tray::TrayAction::Quit => {
                    self.hub.flush_settings();
                    let _ = self.tx.send(AppEvent::Quit);
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hub_visible = false;
        }

        if self.hub_visible {
            self.hub.show(ctx);
        }
        self.hub.tick();

        let always_visible = self.settings.lock().unwrap().show_overlay;
        let monitor = ctx.input(|i| i.viewport().monitor_size);
        self.pill.show(ctx, &self.shared, &self.tx, monitor, always_visible);
        macos_window::float_over_all_spaces("Murmur pill");
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4] // the hub paints opaque panels; the pill relies on a clear background
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.hub.flush_settings();
    }
}
