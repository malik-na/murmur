//! Global hotkeys via a CGEventTap on a dedicated CFRunLoop thread.
//!
//! Defaults: Fn hold = push-to-talk, Fn+Space = hands-free toggle, Esc = cancel (only while
//! dictating), Cmd+Ctrl+V = paste last transcript.
//!
//! Event ordering the controller must handle: Fn press always emits `PttDown` immediately, so
//! Fn+Space arrives as `PttDown` then `HandsFreeToggle` ("switch the current recording to
//! hands-free"); the Fn release of that press emits nothing. To stop hands-free with Fn+Space,
//! the controller sees `PttDown` (ignore while already recording) then `HandsFreeToggle` (finish).

use crate::insert::SYNTHETIC_MARKER;
use crate::types::AppEvent;
use anyhow::{anyhow, Context};
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use crossbeam_channel::Sender;
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;
use std::time::Duration;

const KEY_FN: u16 = 63;
const KEY_GLOBE: u16 = 179;
const KEY_SPACE: u16 = 49;
const KEY_ESC: u16 = 53;
const KEY_V: u16 = 9;

const FLAG_SHIFT: u64 = 0x20000;
const FLAG_CONTROL: u64 = 0x40000;
const FLAG_ALT: u64 = 0x80000;
const FLAG_COMMAND: u64 = 0x100000;
const FLAG_FN: u64 = 0x800000;
const MODIFIERS: u64 = FLAG_SHIFT | FLAG_CONTROL | FLAG_ALT | FLAG_COMMAND;

/// kCGKeyboardEventAutorepeat.
const FIELD_AUTOREPEAT: u32 = 8;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

/// The live tap's mach port, used to re-enable it from the callback and the watchdog.
static TAP_PORT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    FlagsChanged,
    KeyDown,
    KeyUp,
}

/// Per-tap state for [`decide`].
#[derive(Debug, Default)]
pub struct KeyState {
    fn_down: bool,
    /// This Fn press was used for a chord (or began with other modifiers): release emits nothing.
    fn_consumed: bool,
    swallow_space_up: bool,
    swallow_esc_up: bool,
    swallow_v_up: bool,
}

/// Pure hotkey logic: returns the event to emit and whether to swallow the key event.
pub fn decide(
    kind: KeyKind,
    keycode: u16,
    flags: u64,
    autorepeat: bool,
    dictation_active: bool,
    st: &mut KeyState,
) -> (Option<AppEvent>, bool) {
    use AppEvent::*;
    use KeyKind::*;
    let fn_flag = flags & FLAG_FN != 0;
    match kind {
        FlagsChanged if (keycode == KEY_FN || keycode == KEY_GLOBE) && fn_flag && !st.fn_down => {
            st.fn_down = true;
            st.fn_consumed = flags & MODIFIERS != 0;
            (if st.fn_consumed { None } else { Some(PttDown) }, false)
        }
        // Any flags change without the Fn bit ends the press (covers a missed Fn-up event).
        FlagsChanged if !fn_flag && st.fn_down => {
            st.fn_down = false;
            (if std::mem::take(&mut st.fn_consumed) { None } else { Some(PttUp) }, false)
        }
        KeyDown if keycode == KEY_SPACE && st.fn_down => {
            st.fn_consumed = true;
            st.swallow_space_up = true;
            (if autorepeat { None } else { Some(HandsFreeToggle) }, true)
        }
        KeyUp if keycode == KEY_SPACE && st.swallow_space_up => {
            st.swallow_space_up = false;
            (None, true)
        }
        KeyDown if keycode == KEY_ESC && (dictation_active || (autorepeat && st.swallow_esc_up)) => {
            st.swallow_esc_up = true;
            (if autorepeat { None } else { Some(Cancel) }, true)
        }
        KeyUp if keycode == KEY_ESC && st.swallow_esc_up => {
            st.swallow_esc_up = false;
            (None, true)
        }
        KeyDown
            if keycode == KEY_V
                && flags & (FLAG_COMMAND | FLAG_CONTROL) == FLAG_COMMAND | FLAG_CONTROL
                && flags & (FLAG_ALT | FLAG_SHIFT) == 0 =>
        {
            st.swallow_v_up = true;
            (if autorepeat { None } else { Some(PasteLast) }, true)
        }
        KeyUp if keycode == KEY_V && st.swallow_v_up => {
            st.swallow_v_up = false;
            (None, true)
        }
        _ => (None, false),
    }
}

fn reenable_tap() {
    let port = TAP_PORT.load(Ordering::SeqCst);
    if !port.is_null() {
        unsafe {
            if !CGEventTapIsEnabled(port as CFMachPortRef) {
                log::warn!("event tap was disabled; re-enabling");
                CGEventTapEnable(port as CFMachPortRef, true);
            }
        }
    }
}

fn make_callback(
    tx: Sender<AppEvent>,
    active: Arc<AtomicBool>,
) -> impl Fn(*const c_void, CGEventType, &CGEvent) -> CallbackResult + Send + 'static {
    let state = RefCell::new(KeyState::default());
    move |_proxy, etype, event| {
        let kind = match etype as u32 {
            t if t == CGEventType::FlagsChanged as u32 => KeyKind::FlagsChanged,
            t if t == CGEventType::KeyDown as u32 => KeyKind::KeyDown,
            t if t == CGEventType::KeyUp as u32 => KeyKind::KeyUp,
            t if t == CGEventType::TapDisabledByTimeout as u32
                || t == CGEventType::TapDisabledByUserInput as u32 =>
            {
                reenable_tap();
                return CallbackResult::Keep;
            }
            _ => return CallbackResult::Keep,
        };
        if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == SYNTHETIC_MARKER {
            return CallbackResult::Keep;
        }
        let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
        let autorepeat = event.get_integer_value_field(FIELD_AUTOREPEAT) != 0;
        let (ev, swallow) = decide(
            kind,
            keycode,
            event.get_flags().bits(),
            autorepeat,
            active.load(Ordering::SeqCst),
            &mut state.borrow_mut(),
        );
        if let Some(ev) = ev {
            let _ = tx.send(ev);
        }
        if swallow { CallbackResult::Drop } else { CallbackResult::Keep }
    }
}

/// Spawns the tap thread. Errors if no tap can be created (missing Accessibility /
/// Input Monitoring permission). Falls back to a listen-only tap when an active one is refused;
/// then push-to-talk works but Fn+Space/Esc/Cmd+Ctrl+V leak through to the focused app.
pub fn start_hotkey_listener(tx: Sender<AppEvent>, dictation_active: Arc<AtomicBool>) -> anyhow::Result<()> {
    unsafe {
        if !CGPreflightListenEventAccess() {
            CGRequestListenEventAccess();
        }
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
    std::thread::Builder::new()
        .name("hotkey-tap".into())
        .spawn(move || {
            let create = |options| {
                CGEventTap::new(
                    CGEventTapLocation::Session,
                    CGEventTapPlacement::HeadInsertEventTap,
                    options,
                    vec![CGEventType::FlagsChanged, CGEventType::KeyDown, CGEventType::KeyUp],
                    make_callback(tx.clone(), dictation_active.clone()),
                )
            };
            let tap = match create(CGEventTapOptions::Default).or_else(|_| {
                log::warn!("active event tap refused; falling back to listen-only (keys can't be swallowed)");
                create(CGEventTapOptions::ListenOnly)
            }) {
                Ok(t) => t,
                Err(_) => {
                    let _ = ready_tx.send(Err(
                        "CGEventTapCreate failed: grant Accessibility and Input Monitoring".into(),
                    ));
                    return;
                }
            };
            let Ok(source) = tap.mach_port().create_runloop_source(0) else {
                let _ = ready_tx.send(Err("event tap run loop source creation failed".into()));
                return;
            };
            CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
            tap.enable();
            TAP_PORT.store(tap.mach_port().as_concrete_TypeRef() as *mut c_void, Ordering::SeqCst);
            // Watchdog: the system can silently disable a tap; check every 5 s.
            std::thread::spawn(|| loop {
                std::thread::sleep(Duration::from_secs(5));
                if TAP_PORT.load(Ordering::SeqCst).is_null() {
                    break;
                }
                reenable_tap();
            });
            let _ = ready_tx.send(Ok(()));
            CFRunLoop::run_current();
            TAP_PORT.store(std::ptr::null_mut(), Ordering::SeqCst);
            drop(tap);
        })
        .context("spawn hotkey thread")?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .context("hotkey thread did not start")?
        .map_err(|e| anyhow!(e))
}

/// Whether this process is trusted for Accessibility; `prompt` shows the system dialog if not.
pub fn accessibility_trusted(prompt: bool) -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let opts = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::from(prompt))]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef() as *const c_void)
    }
}

/// True while some app (e.g. a password field) has Secure Event Input on; the tap sees no keys then.
pub fn secure_input_enabled() -> bool {
    unsafe { IsSecureEventInputEnabled() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AppEvent::*;
    use KeyKind::*;

    fn d(st: &mut KeyState, k: KeyKind, code: u16, flags: u64) -> (Option<AppEvent>, bool) {
        decide(k, code, flags, false, false, st)
    }

    #[test]
    fn fn_hold_is_push_to_talk() {
        let mut st = KeyState::default();
        assert_eq!(d(&mut st, FlagsChanged, KEY_FN, FLAG_FN), (Some(PttDown), false));
        assert_eq!(d(&mut st, KeyDown, 0, FLAG_FN), (None, false));
        assert_eq!(d(&mut st, FlagsChanged, KEY_FN, 0), (Some(PttUp), false));
        assert_eq!(d(&mut st, FlagsChanged, KEY_FN, 0), (None, false));
    }

    #[test]
    fn fn_space_toggles_hands_free_and_swallows_space() {
        let mut st = KeyState::default();
        assert_eq!(d(&mut st, FlagsChanged, KEY_FN, FLAG_FN), (Some(PttDown), false));
        assert_eq!(d(&mut st, KeyDown, KEY_SPACE, FLAG_FN), (Some(HandsFreeToggle), true));
        assert_eq!(decide(KeyDown, KEY_SPACE, FLAG_FN, true, false, &mut st), (None, true));
        assert_eq!(d(&mut st, FlagsChanged, KEY_FN, 0), (None, false));
        assert_eq!(d(&mut st, KeyUp, KEY_SPACE, 0), (None, true));
        assert_eq!(d(&mut st, KeyDown, KEY_SPACE, 0), (None, false));
        assert_eq!(d(&mut st, KeyUp, KEY_SPACE, 0), (None, false));
    }

    #[test]
    fn fn_with_other_modifier_does_nothing() {
        let mut st = KeyState::default();
        assert_eq!(d(&mut st, FlagsChanged, KEY_FN, FLAG_FN | FLAG_CONTROL), (None, false));
        assert_eq!(d(&mut st, FlagsChanged, KEY_FN, FLAG_CONTROL), (None, false));
    }

    #[test]
    fn missed_fn_release_recovered_by_other_flag_change() {
        let mut st = KeyState::default();
        d(&mut st, FlagsChanged, KEY_FN, FLAG_FN);
        assert_eq!(d(&mut st, FlagsChanged, 56, FLAG_SHIFT), (Some(PttUp), false));
    }

    #[test]
    fn esc_only_while_dictating() {
        let mut st = KeyState::default();
        assert_eq!(decide(KeyDown, KEY_ESC, 0, false, false, &mut st), (None, false));
        assert_eq!(decide(KeyUp, KEY_ESC, 0, false, false, &mut st), (None, false));
        assert_eq!(decide(KeyDown, KEY_ESC, 0, false, true, &mut st), (Some(Cancel), true));
        // Up is swallowed even though dictation already ended.
        assert_eq!(decide(KeyUp, KEY_ESC, 0, false, false, &mut st), (None, true));
    }

    #[test]
    fn cmd_ctrl_v_pastes_last() {
        let mut st = KeyState::default();
        let f = FLAG_COMMAND | FLAG_CONTROL;
        assert_eq!(d(&mut st, KeyDown, KEY_V, f), (Some(PasteLast), true));
        assert_eq!(d(&mut st, KeyUp, KEY_V, f), (None, true));
        assert_eq!(d(&mut st, KeyDown, KEY_V, FLAG_COMMAND), (None, false));
        assert_eq!(d(&mut st, KeyDown, KEY_V, f | FLAG_SHIFT), (None, false));
    }
}
