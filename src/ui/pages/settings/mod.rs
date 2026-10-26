//! Settings modal content. The shell (hub/mod.rs) draws the modal, left nav, serif title and scroll area;
//! each section file draws only its rows: `pub fn show(hub: &mut Hub, ui: &mut Ui)`.

mod accounts;
mod ai_cleanup;
mod general;
mod privacy;
mod system;

use crate::types::*;
use crate::ui::hub::Hub;
use crate::ui::widgets::Icon;
use eframe::egui::Ui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    System,
    AiCleanup,
    Privacy,
    Accounts,
}

impl Section {
    /// Left-nav groups: (uppercase label, sections).
    pub const GROUPS: &'static [(&'static str, &'static [Section])] = &[
        ("Settings", &[Section::General, Section::System, Section::AiCleanup, Section::Privacy]),
        ("Account", &[Section::Accounts]),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::System => "System",
            Section::AiCleanup => "AI cleanup",
            Section::Privacy => "Privacy",
            Section::Accounts => "Accounts & keys",
        }
    }

    /// Serif title at the top of the pane.
    pub fn title(self) -> &'static str {
        self.label()
    }

    pub fn icon(self) -> Icon {
        match self {
            Section::General => Icon::Sliders,
            Section::System => Icon::Monitor,
            Section::AiCleanup => Icon::MagicWand,
            Section::Privacy => Icon::Shield,
            Section::Accounts => Icon::User,
        }
    }
}

pub(crate) enum TestState {
    Idle,
    Running(CloudProvider),
    Done(CloudProvider, Result<(), String>),
}

/// Outcome of a background ChatGPT sign-in / sign-out, tagged with the attempt it belongs to.
pub(crate) enum AuthEvent {
    SignedIn(u64, Result<(), String>),
    SignedOut(u64, Result<(), String>),
}

/// Settings-modal UI state (lives in `hub.prefs`).
pub struct State {
    /// Cached input devices; reset whenever the modal opens.
    pub(crate) devices: Option<Vec<String>>,
    pub(crate) models_dir: PathBuf,
    /// Provider whose credentials the Accounts section shows; None = the active cloud provider.
    pub(crate) accounts_provider: Option<CloudProvider>,
    /// Text typed into the API-key field; cleared on save, never shown back.
    pub(crate) key_input: String,
    /// Inline result of the last Save/Remove: (provider, ok, message).
    pub(crate) key_msg: Option<(CloudProvider, bool, String)>,
    /// Result of a background connection test.
    pub(crate) test_result: Arc<Mutex<TestState>>,
    pub(crate) ax_trusted: bool,
    pub(crate) ax_checked: Option<Instant>,
    pub(crate) confirm_clear: bool,

    /// Cached `chatgpt_auth::account()` and when it was read.
    pub(crate) chatgpt_account: Option<crate::chatgpt_auth::Account>,
    pub(crate) chatgpt_checked: Option<Instant>,
    /// Id of the sign-in/out attempt being waited on; bumping it abandons the pending one.
    pub(crate) auth_attempt: u64,
    pub(crate) auth_pending: Option<(u64, bool)>, // (attempt, is_sign_out)
    pub(crate) auth_events: Arc<Mutex<Vec<AuthEvent>>>,
    pub(crate) auth_error: Option<String>,
    /// Show the one-time "you're using your plan" card after a successful sign-in.
    pub(crate) plan_notice_due: bool,
    pub(crate) plan_notice_dismissed: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            devices: None,
            models_dir: data_dir().join("models"),
            accounts_provider: None,
            key_input: String::new(),
            key_msg: None,
            test_result: Arc::new(Mutex::new(TestState::Idle)),
            ax_trusted: false,
            ax_checked: None,
            confirm_clear: false,
            chatgpt_account: None,
            chatgpt_checked: None,
            auth_attempt: 0,
            auth_pending: None,
            auth_events: Arc::new(Mutex::new(Vec::new())),
            auth_error: None,
            plan_notice_due: false,
            plan_notice_dismissed: false,
        }
    }
}

/// Called by the shell right before the modal opens.
pub fn on_open(hub: &mut Hub) {
    hub.prefs.devices = None;
    hub.prefs.confirm_clear = false;
}

/// Called by the shell after the modal closed.
pub fn on_close(hub: &mut Hub) {
    hub.prefs.confirm_clear = false;
    hub.prefs.key_input.clear();
}

/// Draws one section's content below the title.
pub fn show(hub: &mut Hub, ui: &mut Ui, section: Section) {
    match section {
        Section::General => general::show(hub, ui),
        Section::System => system::show(hub, ui),
        Section::AiCleanup => ai_cleanup::show(hub, ui),
        Section::Privacy => privacy::show(hub, ui),
        Section::Accounts => accounts::show(hub, ui),
    }
}
