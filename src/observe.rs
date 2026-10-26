//! Watch the field Murmur just pasted into and report what the pasted region looks like after the
//! user's corrections, so corrected terms can be learned.
//!
//! Flow: [`capture_after_paste`] reads the focused element via Accessibility right after ⌘V,
//! finds the pasted text in its value and remembers ~40 chars of context on each side. [`watch`]
//! then polls the element's value every 400 ms, re-locates the region between those two anchors
//! (see [`locate`]) and stops on submit, emptying, lost anchors, timeout, idle or the next paste.
//!
//! All string positions in this module are byte offsets into Rust `String`s, always on char
//! boundaries. AX ranges are UTF-16 code units and are converted with [`utf16_to_byte`].

use crate::types::{EditEndReason, EditObservation};
use core_foundation::base::{CFGetTypeID, CFRange, CFRelease, CFTypeID, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;
use std::time::{Duration, Instant};

/// Context kept on each side of the pasted region, in chars.
const ANCHOR_CHARS: usize = 40;
/// Values longer than this (UTF-16 units) are not observed.
const MAX_VALUE_LEN: usize = 200_000;
const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// After at least one edit, stop once the region has been stable this long.
const IDLE_AFTER_EDIT: Duration = Duration::from_secs(20);
/// Consecutive polls without a located region before giving up.
const MAX_MISSES: u32 = 2;
/// Shorter anchor lengths (chars) tried when the full anchor no longer matches.
const SHRINK_STEPS: [usize; 3] = [24, 12, 6];

// ---------------------------------------------------------------------------------------------
// FFI
// ---------------------------------------------------------------------------------------------

type AXUIElementRef = *const c_void;
type AXError = i32;
const AX_SUCCESS: AXError = 0;
/// kAXValueTypeCFRange.
const AX_VALUE_TYPE_CFRANGE: u32 = 4;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout_seconds: f32) -> AXError;
    fn AXUIElementSetAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: CFTypeRef) -> AXError;
    fn AXValueGetTypeID() -> CFTypeID;
    fn AXValueGetValue(value: CFTypeRef, the_type: u32, value_ptr: *mut c_void) -> bool;
}

/// Asks a Chromium/Electron app to build its accessibility tree, which it otherwise only does when
/// VoiceOver is running; without it the app's text fields have no readable AXValue. Native apps
/// reject the attribute, which is harmless. Done once per process; call it early (at key-down),
/// because Chromium builds the tree asynchronously.
pub fn enable_app_accessibility(pid: i32) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static DONE: Mutex<Option<HashSet<i32>>> = Mutex::new(None);
    if !DONE.lock().unwrap().get_or_insert_with(HashSet::new).insert(pid) {
        return;
    }
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return;
        }
        AXUIElementSetMessagingTimeout(app, 1.0);
        let attr = CFString::from_static_string("AXManualAccessibility");
        let yes = core_foundation::boolean::CFBoolean::true_value();
        let err = AXUIElementSetAttributeValue(app, attr.as_concrete_TypeRef(), yes.as_CFTypeRef());
        log::debug!("observe: AXManualAccessibility on pid {pid}: {}", if err == AX_SUCCESS { "enabled" } else { "not supported" });
        CFRelease(app);
    }
}

fn frontmost_pid() -> Option<i32> {
    objc2::rc::autoreleasepool(|_| {
        objc2_app_kit::NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier())
            .filter(|&pid| pid > 0)
    })
}

/// Copies an attribute; the caller owns (must CFRelease) the returned object.
unsafe fn copy_attr(element: AXUIElementRef, name: &'static str) -> Option<CFTypeRef> {
    let attr = CFString::from_static_string(name);
    let mut out: CFTypeRef = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut out);
    if err != AX_SUCCESS || out.is_null() {
        if !out.is_null() {
            CFRelease(out);
        }
        return None;
    }
    Some(out)
}

/// Reads a string attribute. `Err(())` = read failed; `Ok(None)` = present but not a string.
unsafe fn string_attr(element: AXUIElementRef, name: &'static str) -> Result<Option<String>, ()> {
    let Some(v) = copy_attr(element, name) else { return Err(()) };
    if CFGetTypeID(v) != CFString::type_id() {
        CFRelease(v);
        return Ok(None);
    }
    // Create rule: takes ownership of `v`.
    let s = CFString::wrap_under_create_rule(v as CFStringRef);
    if s.char_len() as usize > MAX_VALUE_LEN {
        return Ok(None);
    }
    Ok(Some(s.to_string()))
}

/// Caret / selection as a UTF-16 CFRange.
unsafe fn selected_range(element: AXUIElementRef) -> Option<CFRange> {
    let v = copy_attr(element, "AXSelectedTextRange")?;
    let mut range = CFRange { location: 0, length: 0 };
    let ok = CFGetTypeID(v) == AXValueGetTypeID()
        && AXValueGetValue(v, AX_VALUE_TYPE_CFRANGE, &mut range as *mut CFRange as *mut c_void);
    CFRelease(v);
    (ok && range.location >= 0).then_some(range)
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

/// A retained handle to the field Murmur pasted into, plus where the paste landed.
pub struct PasteAnchor {
    element: AXUIElementRef,
    pasted: String,
    before: String,
    after: String,
    /// Byte offset of the region start in `initial_value` (search hint).
    start: usize,
    /// The pasted region as found in the field at capture time.
    region: String,
}

// AX calls are made from one background thread at a time; the element ref is a CF object.
unsafe impl Send for PasteAnchor {}

impl Drop for PasteAnchor {
    fn drop(&mut self) {
        if !self.element.is_null() {
            unsafe { CFRelease(self.element) };
        }
    }
}

impl std::fmt::Debug for PasteAnchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Lengths only: this text is the user's dictation and surrounding field content, which
        // must never reach a log at any level.
        f.debug_struct("PasteAnchor")
            .field("pasted_chars", &self.pasted.chars().count())
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .field("start", &self.start)
            .finish()
    }
}

/// Call right after a paste completes (~250 ms after ⌘V). Returns `None` when the focused field
/// can't be observed (none, password field, non-string or huge value, pasted text not found).
pub fn capture_after_paste(pasted: &str) -> Option<PasteAnchor> {
    if pasted.trim().is_empty() {
        return None;
    }
    // A Chromium app that just had its accessibility tree enabled may need a moment before the
    // focused field and its value are readable.
    for attempt in 0..6 {
        if let Some(anchor) = try_capture(pasted) {
            return Some(anchor);
        }
        if attempt < 5 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    None
}

fn try_capture(pasted: &str) -> Option<PasteAnchor> {
    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }
        AXUIElementSetMessagingTimeout(system, 1.0);
        let mut focused = copy_attr(system, "AXFocusedUIElement");
        CFRelease(system);
        if focused.is_none() {
            // The system-wide query fails with kAXErrorCannotComplete for some apps (observed on
            // macOS 27 with a plain NSTextView); asking the frontmost app directly works.
            focused = frontmost_pid().and_then(|pid| {
                let app = AXUIElementCreateApplication(pid);
                if app.is_null() {
                    return None;
                }
                AXUIElementSetMessagingTimeout(app, 1.0);
                let f = copy_attr(app, "AXFocusedUIElement");
                CFRelease(app);
                f
            });
        }
        let element = focused? as AXUIElementRef;
        // From here on `anchor` owns `element` (released on drop, including early returns).
        let mut anchor = PasteAnchor {
            element,
            pasted: pasted.to_string(),
            before: String::new(),
            after: String::new(),
            start: 0,
            region: String::new(),
        };
        AXUIElementSetMessagingTimeout(element, 1.0);

        let is_secure = |s: Result<Option<String>, ()>| {
            matches!(s, Ok(Some(ref r)) if r == "AXSecureTextField")
        };
        if is_secure(string_attr(element, "AXRole")) || is_secure(string_attr(element, "AXSubrole"))
        {
            return None;
        }
        let value = string_attr(element, "AXValue").ok()??;
        let caret = selected_range(element)
            .map(|r| utf16_to_byte(&value, (r.location + r.length.max(0)) as usize));

        let (start, end) = find_pasted(&value, pasted, caret)?;
        let (before, after) = context_around(&value, start, end);
        anchor.before = before.to_string();
        anchor.after = after.to_string();
        anchor.start = start;
        anchor.region = value[start..end].to_string();
        log::debug!("observe: captured {anchor:?}");
        Some(anchor)
    }
}

/// Blocks, polling the element's value every 400 ms until an end condition, then returns.
/// `stop()` is checked at least each poll; true → end with `NextPaste`.
pub fn watch(anchor: PasteAnchor, window: Duration, stop: impl Fn() -> bool) -> EditObservation {
    let mut tracker = Tracker::new(&anchor);
    let started = Instant::now();
    let (end_reason, region) = 'outer: loop {
        // Sleep in small steps so `stop` is honoured promptly.
        let poll_at = Instant::now() + POLL_INTERVAL;
        while Instant::now() < poll_at {
            if stop() {
                break 'outer (EditEndReason::NextPaste, tracker.last_region.clone());
            }
            if started.elapsed() >= window {
                break 'outer (EditEndReason::WindowElapsed, tracker.last_region.clone());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let value = unsafe { string_attr(anchor.element, "AXValue") };
        let poll = match value {
            Ok(Some(v)) => Some(v),
            _ => None,
        };
        if let Some(end) = tracker.step(poll.as_deref(), started.elapsed()) {
            break end;
        }
    };
    let obs = EditObservation {
        pasted: anchor.pasted.clone(),
        final_text: region.trim().to_string(),
        end_reason,
    };
    log::debug!(
        "observe: ended ({:?}), pasted {} chars, final {} chars, changed: {}",
        obs.end_reason,
        obs.pasted.chars().count(),
        obs.final_text.chars().count(),
        obs.pasted != obs.final_text
    );
    obs
}

// ---------------------------------------------------------------------------------------------
// Pure logic
// ---------------------------------------------------------------------------------------------

/// Converts a UTF-16 offset (as AX reports) into a byte offset on a char boundary, clamped.
fn utf16_to_byte(s: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    s.len()
}

/// Byte offset of the char boundary `n` chars before `i` (or 0).
fn back_chars(s: &str, i: usize, n: usize) -> usize {
    if n == 0 {
        return i;
    }
    s[..i].char_indices().rev().nth(n - 1).map_or(0, |(j, _)| j)
}

/// Byte offset of the char boundary `n` chars after `i` (or the end).
fn fwd_chars(s: &str, i: usize, n: usize) -> usize {
    s[i..].char_indices().nth(n).map_or(s.len(), |(j, _)| i + j)
}

/// Up to [`ANCHOR_CHARS`] chars of `value` before `start` and after `end`.
fn context_around(value: &str, start: usize, end: usize) -> (&str, &str) {
    let b = back_chars(value, start, ANCHOR_CHARS);
    let a = fwd_chars(value, end, ANCHOR_CHARS);
    (&value[b..start], &value[end..a])
}

/// Finds `pasted` in `value`: the occurrence ending at or nearest before the caret, else the last
/// one. Falls back to the whitespace-trimmed text (apps sometimes drop a trailing space/newline).
fn find_pasted(value: &str, pasted: &str, caret: Option<usize>) -> Option<(usize, usize)> {
    for needle in [pasted, pasted.trim()] {
        if needle.is_empty() {
            continue;
        }
        let hits: Vec<usize> = value.match_indices(needle).map(|(i, _)| i).collect();
        if hits.is_empty() {
            continue;
        }
        let n = needle.len();
        let pick = caret
            .and_then(|c| hits.iter().copied().filter(|&i| i + n <= c).max())
            .unwrap_or(*hits.last().unwrap());
        return Some((pick, pick + n));
    }
    None
}

/// Candidate anchors: the full anchor, then ever shorter pieces nearest the region.
/// `suffix` = keep the end of the anchor (the `before` side); otherwise keep the start.
fn anchor_variants(anchor: &str, suffix: bool) -> Vec<&str> {
    let n = anchor.chars().count();
    let mut out = vec![anchor];
    for k in SHRINK_STEPS {
        if k < n {
            out.push(if suffix {
                &anchor[back_chars(anchor, anchor.len(), k)..]
            } else {
                &anchor[..fwd_chars(anchor, 0, k)]
            });
        }
    }
    out
}

/// Locates the region between `before` and `after` in `value`, returning its byte range.
///
/// `hint` is the byte offset where the region started last time; when `before` occurs several
/// times the occurrence ending closest to it wins. `after` is the first occurrence at/after the
/// end of `before`. An empty `before` means the region starts at 0; an empty `after` means it runs
/// to the end. If a full anchor no longer matches (the user edited right next to the region),
/// shorter pieces of it adjacent to the region are tried.
fn locate(value: &str, before: &str, after: &str, hint: usize) -> Option<(usize, usize)> {
    let hint = hint.min(value.len());
    let starts: Vec<usize> = if before.is_empty() {
        vec![0]
    } else {
        anchor_variants(before, true)
            .into_iter()
            .find_map(|b| {
                let mut ends: Vec<usize> =
                    value.match_indices(b).map(|(i, m)| i + m.len()).collect();
                if ends.is_empty() {
                    return None;
                }
                ends.sort_by_key(|&e| e.abs_diff(hint));
                Some(ends)
            })?
    };
    // Try the start nearest the hint first; fall back to other occurrences if `after` is missing.
    for start in starts {
        if after.is_empty() {
            return Some((start, value.len()));
        }
        let rest = &value[start..];
        if let Some(end) = anchor_variants(after, false)
            .into_iter()
            .find_map(|a| rest.find(a).map(|i| start + i))
        {
            return Some((start, end));
        }
    }
    None
}

fn is_newline(c: char) -> bool {
    c == '\n' || c == '\r'
}

/// Per-poll state machine for [`watch`], free of AX and wall-clock so it can be unit-tested.
#[derive(Debug)]
struct Tracker {
    before: String,
    after: String,
    hint: usize,
    /// Upper bound on a plausible region length (bytes); larger matches count as misses.
    max_region: usize,
    /// Last successfully located region (untrimmed).
    last_region: String,
    /// The char right after `last_region` in the field was a newline.
    last_followed_by_nl: bool,
    misses: u32,
    /// Time (since watch start) of the last region change, if there was one.
    last_change: Option<Duration>,
}

impl Tracker {
    fn new(a: &PasteAnchor) -> Tracker {
        Tracker::from_parts(&a.before, &a.after, a.start, &a.region, &a.pasted)
    }

    fn from_parts(before: &str, after: &str, start: usize, region: &str, pasted: &str) -> Tracker {
        Tracker {
            before: before.to_string(),
            after: after.to_string(),
            hint: start,
            max_region: pasted.len() * 4 + 400,
            last_region: region.to_string(),
            last_followed_by_nl: after.starts_with(is_newline),
            misses: 0,
            last_change: None,
        }
    }

    /// Feeds one poll (`None` = value unreadable) at time `now` since the watch began. Returns the
    /// end reason and the (untrimmed) final region once observation should stop.
    fn step(&mut self, value: Option<&str>, now: Duration) -> Option<(EditEndReason, String)> {
        let Some(value) = value else {
            return Some((EditEndReason::Unreadable, self.last_region.clone()));
        };
        if value.trim().is_empty() {
            let sent = self.last_region.ends_with(is_newline) || self.last_followed_by_nl;
            let reason = if sent { EditEndReason::Submitted } else { EditEndReason::TextboxEmptied };
            return Some((reason, self.last_region.clone()));
        }
        let found = locate(value, &self.before, &self.after, self.hint)
            .filter(|(s, e)| e - s <= self.max_region);
        let Some((start, end)) = found else {
            self.misses += 1;
            if self.misses >= MAX_MISSES {
                return Some((EditEndReason::AnchorLost, self.last_region.clone()));
            }
            return None;
        };
        self.misses = 0;
        let region = &value[start..end];
        let newline_appended =
            region.ends_with(is_newline) && !self.last_region.ends_with(is_newline);
        if region != self.last_region {
            self.last_change = Some(now);
        }
        // Re-anchor on the current text so edits next to the region don't erode the anchors.
        let (b, a) = context_around(value, start, end);
        self.before = b.to_string();
        self.after = a.to_string();
        self.hint = start;
        self.last_followed_by_nl = a.starts_with(is_newline);
        self.last_region = region.to_string();
        if newline_appended {
            return Some((EditEndReason::Submitted, self.last_region.clone()));
        }
        if let Some(t) = self.last_change {
            if now.saturating_sub(t) >= IDLE_AFTER_EDIT {
                return Some((EditEndReason::WindowElapsed, self.last_region.clone()));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region<'a>(v: &'a str, b: &str, a: &str, hint: usize) -> Option<&'a str> {
        locate(v, b, a, hint).map(|(s, e)| &v[s..e])
    }

    #[test]
    fn utf16_conversion() {
        let s = "a😀bé";
        assert_eq!(utf16_to_byte(s, 0), 0);
        assert_eq!(utf16_to_byte(s, 1), 1);
        assert_eq!(utf16_to_byte(s, 3), 5); // after the surrogate pair
        assert_eq!(utf16_to_byte(s, 2), 5); // mid-pair rounds up to a boundary
        assert_eq!(utf16_to_byte(s, 4), 6);
        assert_eq!(utf16_to_byte(s, 5), s.len());
        assert_eq!(utf16_to_byte(s, 99), s.len());
    }

    #[test]
    fn char_stepping() {
        let s = "héllo";
        assert_eq!(back_chars(s, s.len(), 2), 4);
        assert_eq!(back_chars(s, s.len(), 99), 0);
        assert_eq!(back_chars(s, 3, 0), 3);
        assert_eq!(fwd_chars(s, 0, 2), 3);
        assert_eq!(fwd_chars(s, 0, 99), s.len());
    }

    #[test]
    fn context_is_char_limited() {
        let v = format!("{}PASTE{}", "x".repeat(50), "é".repeat(50));
        let s = 50;
        let (b, a) = context_around(&v, s, s + 5);
        assert_eq!(b, "x".repeat(40));
        assert_eq!(a, "é".repeat(40));
    }

    #[test]
    fn find_pasted_prefers_occurrence_before_caret() {
        let v = "hi there. hi there. hi there.";
        // caret right after the second occurrence
        assert_eq!(find_pasted(v, "hi there", Some(18)), Some((10, 18)));
        // caret a bit later still picks the nearest-before one
        assert_eq!(find_pasted(v, "hi there", Some(25)), Some((10, 18)));
        // no caret → last occurrence
        assert_eq!(find_pasted(v, "hi there", None), Some((20, 28)));
        // caret before every occurrence → last occurrence
        assert_eq!(find_pasted(v, "there", Some(1)), Some((23, 28)));
        // trailing whitespace dropped by the app
        assert_eq!(find_pasted("say hi", "hi ", None), Some((4, 6)));
        assert_eq!(find_pasted("nope", "hi", None), None);
    }

    #[test]
    fn edit_inside_region() {
        let v = "Hello team, I use Claude Code daily. Thanks";
        assert_eq!(
            region(v, "Hello team, ", ". Thanks", 12),
            Some("I use Claude Code daily")
        );
    }

    #[test]
    fn edits_outside_region_far_away() {
        let v = "PREFIX ADDED. Hello team, I use Cloud Code daily. Thanks a lot!";
        assert_eq!(region(v, "Hello team, ", ". Thanks", 12), Some("I use Cloud Code daily"));
    }

    #[test]
    fn edit_adjacent_to_region_uses_shrunk_anchor() {
        // before anchor was 40 chars; user rewrote its beginning.
        let before = "The quick brown fox jumps over the lazy ";
        assert_eq!(before.chars().count(), 40);
        let v = "A slow red fox jumps over the lazy dog went home";
        assert_eq!(region(v, before, " went home", 0), Some("dog"));
        // after anchor's tail edited
        let after = " and then they all went home together ok";
        let v2 = "Start: the region and then they all left";
        assert_eq!(region(v2, "Start: ", after, 0), Some("the region"));
    }

    #[test]
    fn empty_anchors() {
        // paste at start of field
        assert_eq!(region("Hello world. rest", "", ". rest", 0), Some("Hello world"));
        // paste at end of field
        assert_eq!(region("intro: Hello world!!", "intro: ", "", 0), Some("Hello world!!"));
        // whole field
        assert_eq!(region("everything", "", "", 0), Some("everything"));
        // empty value with empty anchors
        assert_eq!(region("", "", "", 5), Some(""));
    }

    #[test]
    fn region_deleted_entirely() {
        assert_eq!(region("Hi . Bye", "Hi ", ". Bye", 3), Some(""));
    }

    #[test]
    fn repeated_before_anchor_uses_hint() {
        let v = "ok: one. ok: two. ok: three.";
        assert_eq!(region(v, "ok: ", ".", 0), Some("one"));
        assert_eq!(region(v, "ok: ", ".", 13), Some("two"));
        assert_eq!(region(v, "ok: ", ".", 27), Some("three"));
    }

    #[test]
    fn after_anchor_missing_for_nearest_start_falls_back() {
        // Nearest "A:" has no "END" after it; earlier one does.
        let v = "A: x END A: y";
        assert_eq!(region(v, "A: ", " END", 12), Some("x"));
    }

    #[test]
    fn anchors_gone() {
        assert_eq!(region("totally different", "Hello team, ", ". Thanks", 0), None);
        assert_eq!(region("Hello team, but no end", "Hello team, ", "QQQQQQQ", 0), None);
    }

    #[test]
    fn multibyte_and_emoji() {
        let v = "Grüße 👋🏽 — ich nutze Claude Code 🚀 täglich… Danke";
        let r = region(v, "Grüße 👋🏽 — ", " täglich… Danke", 0);
        assert_eq!(r, Some("ich nutze Claude Code 🚀"));
        // shrinking anchors with multibyte chars stays on char boundaries
        let before = "ééééééééééééééééééééééééééééééééééééééé ";
        assert_eq!(before.chars().count(), 40);
        let v2 = "üüüüüüüüüüüüüüüüüüééééééééééééééééééééé 日本語テキスト🎉 end";
        assert_eq!(region(v2, before, " end", 0), Some("日本語テキスト🎉"));
    }

    #[test]
    fn hint_beyond_value_is_clamped() {
        assert_eq!(region("a: x.", "a: ", ".", 10_000), Some("x"));
    }

    // ---- Tracker / end conditions ----

    fn secs(s: f32) -> Duration {
        Duration::from_secs_f32(s)
    }

    fn tracker(before: &str, region: &str, after: &str) -> Tracker {
        Tracker::from_parts(before, after, before.len(), region, region)
    }

    #[test]
    fn unchanged_keeps_watching() {
        let mut t = tracker("Hi ", "cloud code", " ok");
        for i in 0..100 {
            assert_eq!(t.step(Some("Hi cloud code ok"), secs(i as f32 * 0.4)), None);
        }
    }

    #[test]
    fn unreadable_returns_last_good_region() {
        let mut t = tracker("Hi ", "cloud code", " ok");
        assert_eq!(t.step(Some("Hi Claude Code ok"), secs(1.0)), None);
        assert_eq!(
            t.step(None, secs(1.4)),
            Some((EditEndReason::Unreadable, "Claude Code".into()))
        );
    }

    #[test]
    fn emptied_without_newline_is_textbox_emptied() {
        let mut t = tracker("", "cloud code", "");
        assert_eq!(t.step(Some("Claude Code"), secs(1.0)), None);
        assert_eq!(
            t.step(Some("  \n"), secs(1.4)),
            Some((EditEndReason::TextboxEmptied, "Claude Code".into()))
        );
    }

    #[test]
    fn emptied_after_newline_is_submitted() {
        // Region followed by a newline in the field, then cleared.
        let mut t = tracker("", "cloud code", "\nsig");
        assert_eq!(t.step(Some("Claude Code\nsig"), secs(1.0)), None);
        // field cleared
        assert_eq!(
            t.step(Some(""), secs(1.4)),
            Some((EditEndReason::Submitted, "Claude Code".into()))
        );
    }

    #[test]
    fn newline_appended_is_submitted() {
        let mut t = tracker("", "cloud code", "");
        assert_eq!(t.step(Some("Claude Code"), secs(1.0)), None);
        let (reason, text) = t.step(Some("Claude Code\n"), secs(1.4)).unwrap();
        assert_eq!(reason, EditEndReason::Submitted);
        assert_eq!(text.trim(), "Claude Code");
    }

    #[test]
    fn newline_inside_after_anchor_gap_is_submitted() {
        let mut t = tracker("Note: ", "cloud code", " end");
        assert_eq!(
            t.step(Some("Note: Claude Code\r end"), secs(1.0)).map(|x| x.0),
            Some(EditEndReason::Submitted)
        );
    }

    #[test]
    fn pasted_text_with_trailing_newline_is_not_submitted_immediately() {
        let mut t = tracker("", "hello\n", "");
        assert_eq!(t.step(Some("hello\n"), secs(0.4)), None);
    }

    #[test]
    fn anchor_lost_after_two_misses() {
        let mut t = tracker("Hello team, ", "cloud code", ". Thanks");
        assert_eq!(t.step(Some("Hello team, Claude Code. Thanks"), secs(0.4)), None);
        assert_eq!(t.step(Some("something unrelated"), secs(0.8)), None);
        assert_eq!(
            t.step(Some("something else unrelated"), secs(1.2)),
            Some((EditEndReason::AnchorLost, "Claude Code".into()))
        );
    }

    #[test]
    fn single_miss_recovers() {
        let mut t = tracker("Hello team, ", "cloud code", ". Thanks");
        assert_eq!(t.step(Some("garbage mid-render"), secs(0.4)), None);
        assert_eq!(t.step(Some("Hello team, Claude Code. Thanks"), secs(0.8)), None);
        assert_eq!(t.step(Some("zzz"), secs(1.2)), None);
        assert_eq!(t.last_region, "Claude Code");
    }

    #[test]
    fn idle_after_edit_ends() {
        let mut t = tracker("Hi ", "cloud code", " ok");
        assert_eq!(t.step(Some("Hi Claude Code ok"), secs(2.0)), None);
        assert_eq!(t.step(Some("Hi Claude Code ok"), secs(21.9)), None);
        assert_eq!(
            t.step(Some("Hi Claude Code ok"), secs(22.0)),
            Some((EditEndReason::WindowElapsed, "Claude Code".into()))
        );
    }

    #[test]
    fn reanchoring_follows_successive_adjacent_edits() {
        let before = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ";
        let mut t = tracker(before, "cloud code", " zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz");
        let after = " zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
        // Edit the far end of the before-context, then again: each poll re-anchors.
        let v1 = format!("XXXXXXXXXXXXXXXXXXXXaaaaaaaaaaaaaaaaaaa Claude Code{after}");
        assert_eq!(t.step(Some(&v1), secs(0.4)), None);
        let v2 = format!("YYYYYYYYYYYYYYYYYYYYYYYYYYYYYYYYXXXXXXXXXXXXXXXXXXXXaaaaaaaaaaaaaaaaaaa Claude Code{after}");
        assert_eq!(t.step(Some(&v2), secs(0.8)), None);
        assert_eq!(t.last_region, "Claude Code");
    }

    #[test]
    fn implausibly_large_region_counts_as_miss() {
        let mut t = tracker("", "hi", "END");
        let big = format!("{}END", "x".repeat(10_000));
        assert_eq!(t.step(Some(&big), secs(0.4)), None);
        assert_eq!(t.misses, 1);
    }

    /// Manual smoke test against a real focused text field. Run with
    /// `MURMUR_OBSERVE_TEXT='…' cargo test observe::tests::smoke -- --ignored --nocapture`
    /// after pasting that text into a window you own.
    #[test]
    #[ignore]
    fn smoke() {
        let text = std::env::var("MURMUR_OBSERVE_TEXT").expect("set MURMUR_OBSERVE_TEXT");
        let secs: u64 = std::env::var("MURMUR_OBSERVE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
        let anchor = capture_after_paste(&text).expect("capture failed");
        println!("captured: {anchor:?}");
        let obs = watch(anchor, Duration::from_secs(secs), || false);
        println!("observation: {obs:?}");
    }
}
