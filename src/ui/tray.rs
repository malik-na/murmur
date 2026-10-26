//! Menu-bar tray icon with a procedurally drawn template glyph.

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub enum TrayAction {
    Open,
    PasteLast,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    open: MenuId,
    paste: MenuId,
    quit: MenuId,
}

impl Tray {
    /// Must run on the main thread after the event loop exists.
    pub fn new() -> anyhow::Result<Tray> {
        let open = MenuItem::new("Open Murmur", true, None);
        let paste = MenuItem::new("Paste last transcript", true, None);
        let quit = MenuItem::new("Quit Murmur", true, None);
        let menu = Menu::with_items(&[&open, &paste, &PredefinedMenuItem::separator(), &quit])?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(glyph()?)
            .with_icon_as_template(true)
            .with_tooltip("Murmur")
            .build()?;
        Ok(Tray { _icon: icon, open: open.id().clone(), paste: paste.id().clone(), quit: quit.id().clone() })
    }

    pub fn poll(&self) -> Vec<TrayAction> {
        MenuEvent::receiver()
            .try_iter()
            .filter_map(|e| match e.id() {
                id if *id == self.open => Some(TrayAction::Open),
                id if *id == self.paste => Some(TrayAction::PasteLast),
                id if *id == self.quit => Some(TrayAction::Quit),
                _ => None,
            })
            .collect()
    }
}

/// 32×32 microphone glyph, black + alpha (macOS tints template images), 4×4 supersampled.
fn glyph() -> anyhow::Result<Icon> {
    const N: usize = 32;
    let seg_dist = |x: f32, y: f32, ay: f32, by: f32| {
        let cy = y.clamp(ay, by);
        ((x - 16.0).powi(2) + (y - cy).powi(2)).sqrt()
    };
    let inside = |x: f32, y: f32| {
        let capsule = seg_dist(x, y, 8.5, 14.5) <= 4.6;
        let r = ((x - 16.0).powi(2) + (y - 14.0).powi(2)).sqrt();
        let cradle = y >= 14.0 && (r - 8.6).abs() <= 1.25;
        let stem = (x - 16.0).abs() <= 1.25 && (22.5..=26.5).contains(&y);
        let base = (x - 16.0).abs() <= 5.5 && (25.6..=28.0).contains(&y);
        capsule || cradle || stem || base
    };
    let mut rgba = vec![0u8; N * N * 4];
    for py in 0..N {
        for px in 0..N {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = px as f32 + (sx as f32 + 0.5) / 4.0;
                    let y = py as f32 + (sy as f32 + 0.5) / 4.0;
                    hits += inside(x, y) as u32;
                }
            }
            rgba[(py * N + px) * 4 + 3] = (hits * 255 / 16) as u8;
        }
    }
    Ok(Icon::from_rgba(rgba, N as u32, N as u32)?)
}
