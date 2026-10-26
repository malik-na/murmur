//! Text insertion: put text on the clipboard, synthesize ⌘V, restore the clipboard.

use anyhow::{anyhow, Context};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::{thread, time::Duration};

/// Written to `EVENT_SOURCE_USER_DATA` on events we post so the hotkey tap ignores them.
pub const SYNTHETIC_MARKER: i64 = 0x4D55_524D; // "MURM"

const KEY_V: u16 = 9;

/// Pastes `text` into the focused app.
///
/// Only a plain-text clipboard is restored afterwards; if the previous clipboard held
/// non-text data (images, files, rich content only), it is replaced by `text`.
/// The clipboard is also left alone if something else changed it during the paste window.
pub fn paste_text(text: &str) -> anyhow::Result<()> {
    let mut cb = arboard::Clipboard::new().context("open clipboard")?;
    let previous = cb.get_text().ok();
    cb.set_text(text).context("set clipboard")?;
    thread::sleep(Duration::from_millis(30));
    post_cmd_v()?;
    thread::sleep(Duration::from_millis(250));
    if let Some(prev) = previous {
        if cb.get_text().ok().as_deref() == Some(text) {
            cb.set_text(prev).context("restore clipboard")?;
        }
    }
    Ok(())
}

fn post_cmd_v() -> anyhow::Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow!("CGEventSource creation failed"))?;
    for down in [true, false] {
        let ev = CGEvent::new_keyboard_event(source.clone(), KEY_V, down)
            .map_err(|_| anyhow!("CGEvent creation failed"))?;
        // Explicit flags so physically held modifiers (e.g. Ctrl from Cmd+Ctrl+V) don't leak in.
        ev.set_flags(CGEventFlags::CGEventFlagCommand);
        ev.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, SYNTHETIC_MARKER);
        ev.post(CGEventTapLocation::HID);
    }
    Ok(())
}
