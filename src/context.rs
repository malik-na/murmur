//! Frontmost-app context and System Settings shortcuts.

use crate::types::TargetContext;
use objc2_app_kit::NSWorkspace;

/// Bundle id and display name of the frontmost application. Safe from any thread.
pub fn frontmost_app() -> TargetContext {
    objc2::rc::autoreleasepool(|_| {
        let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
            return TargetContext::default();
        };
        TargetContext {
            pid: Some(app.processIdentifier()).filter(|&p| p > 0),
            bundle_id: app.bundleIdentifier().map(|s| s.to_string()),
            app_name: app.localizedName().map(|s| s.to_string()),
        }
    })
}

/// Brings the app with `bundle_id` back to the front if it isn't already. Returns whether focus moved.
pub fn activate(bundle_id: &str) -> bool {
    if frontmost_app().bundle_id.as_deref() == Some(bundle_id) {
        return false;
    }
    objc2::rc::autoreleasepool(|_| {
        let id = objc2_foundation::NSString::from_str(bundle_id);
        let apps = objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(&id);
        apps.firstObject()
            .is_some_and(|app| app.activateWithOptions(objc2_app_kit::NSApplicationActivationOptions::empty()))
    })
}

/// Opens Privacy & Security at `pane` ("Accessibility" | "Microphone" | "ListenEvent").
pub fn open_privacy_pane(pane: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?Privacy_{pane}");
    if let Err(e) = std::process::Command::new("open").arg(url).spawn().and_then(|mut c| c.wait()) {
        log::warn!("open privacy pane {pane}: {e}");
    }
}
