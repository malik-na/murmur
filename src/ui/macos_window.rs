//! AppKit tweaks egui/winit can't express: a pill that floats over every Space and full-screen
//! app, and a process that macOS doesn't throttle while another app is in front.

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSStatusWindowLevel, NSWindowCollectionBehavior};
use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

/// Makes the window titled `title` visible on all Spaces (including over full-screen apps) at
/// status-bar level, and keeps it shown while Murmur isn't the active app. Cheap enough to call
/// every frame: it only touches a window whose settings drifted (e.g. after it was recreated).
pub fn float_over_all_spaces(title: &str) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let want = NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        // macOS 13+: lets the window join other apps' full-screen Spaces, not just our own.
        | NSWindowCollectionBehavior::CanJoinAllApplications;
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().iter() {
        if window.title().to_string() != title {
            continue;
        }
        if window.collectionBehavior() != want || window.level() != NSStatusWindowLevel {
            // Both are needed to appear over another app's full-screen Space (verified on macOS 27):
            // the CanJoinAllApplications flag, applied while the process is briefly an accessory
            // app. Switching back keeps the Dock icon; the window keeps the behavior. Electron does
            // the same dance for visibleOnFullScreen. Runs once, since the pill window is kept alive.
            let regular = app.activationPolicy() == NSApplicationActivationPolicy::Regular;
            if regular {
                app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            }
            window.setCollectionBehavior(want);
            window.setLevel(NSStatusWindowLevel);
            window.setHidesOnDeactivate(false);
            if regular {
                app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            }
            log::debug!("pill window set to float over all Spaces");
        }
    }
}

/// Opts out of App Nap so the event loop keeps running at full speed in the background; otherwise
/// the pill can lag or not appear when dictation starts in another app. Lasts for the process.
pub fn disable_app_nap() {
    let options = NSActivityOptions::UserInitiatedAllowingIdleSystemSleep | NSActivityOptions::LatencyCritical;
    let reason = NSString::from_str("Murmur listens for the dictation key and shows the dictation bar");
    let token: Retained<objc2::runtime::ProtocolObject<dyn NSObjectProtocol>> =
        NSProcessInfo::processInfo().beginActivityWithOptions_reason(options, &reason);
    std::mem::forget(token);
}
