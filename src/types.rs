//! Types shared across modules. Every module depends on this file; keep it stable.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Events that drive the dictation controller (sent over a crossbeam channel).
#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    /// Push-to-talk key went down: start recording.
    PttDown,
    /// Push-to-talk key released: stop recording and process.
    PttUp,
    /// Hands-free toggle pressed (Fn+Space): start if idle, finish if recording.
    HandsFreeToggle,
    /// Esc while recording or processing: discard.
    Cancel,
    /// Paste the most recent transcript again.
    PasteLast,
    /// Show the hub window.
    OpenHub,
    /// Settings changed the model file or a download finished: reload whisper.
    ReloadModel,
    Quit,
}

/// What the floating pill shows. One variant per pill state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayState {
    #[default]
    Resting,
    /// Recording in push-to-talk mode (shows waveform).
    ListeningPtt,
    /// Recording in hands-free mode (waveform plus cancel/finish buttons).
    ListeningHandsFree,
    Processing,
    Error,
}

/// App category used to pick a writing style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum StyleCategory {
    Personal,
    Work,
    Email,
    Other,
}

/// Cloud model used for AI cleanup when the local model is off, unreachable or too slow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum CloudProvider {
    /// Anthropic Messages API, API key.
    Claude,
    /// OpenAI: Sign in with ChatGPT (plan usage) or an API key.
    ChatGpt,
    /// xAI (OpenAI-compatible API), API key.
    Grok,
}

impl CloudProvider {
    pub const ALL: [CloudProvider; 3] = [CloudProvider::Claude, CloudProvider::ChatGpt, CloudProvider::Grok];
    pub fn label(self) -> &'static str {
        match self {
            CloudProvider::Claude => "Claude",
            CloudProvider::ChatGpt => "ChatGPT",
            CloudProvider::Grok => "Grok",
        }
    }
}

/// A cloud cleanup failure the user can act on. Timeouts and network blips aren't issues:
/// falling back to the local result is the expected behaviour for those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloudIssueKind {
    /// Out of credit / billing not set up.
    Billing,
    /// Key missing, wrong or revoked; ChatGPT session expired.
    BadKey,
    /// The model id isn't available to this account.
    UnknownModel,
    /// ChatGPT plan usage limit for Murmur reached.
    PlanLimit,
}

#[derive(Debug, Clone)]
pub struct CloudIssue {
    pub provider: CloudProvider,
    pub kind: CloudIssueKind,
    /// The provider's own error text, for the Settings banner.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tone {
    Formal,
    Casual,
    VeryCasual,
}

/// Persisted as JSON at `<data_dir>/settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// whisper.cpp ggml model file name inside `<data_dir>/models/`.
    pub model_file: String,
    /// ISO language code or "auto".
    pub language: String,
    /// Input device name; None = system default.
    pub input_device: Option<String>,
    pub enable_sounds: bool,
    /// Remove fillers, fix capitalization/spacing, apply spoken commands.
    pub smart_formatting: bool,
    /// AI cleanup through a local OpenAI-compatible server (LM Studio). Tried first.
    pub use_local_llm: bool,
    /// Base URL ending in /v1, e.g. "http://localhost:1234/v1".
    pub local_llm_url: String,
    pub local_llm_model: String,
    /// Fallback: cloud cleanup through `cloud_provider` when it has a credential.
    pub use_llm_cleanup: bool,
    pub cloud_provider: CloudProvider,
    /// Claude model id.
    pub llm_model: String,
    pub openai_model: String,
    pub grok_model: String,
    /// Total time AI cleanup may take before the local-rules result is pasted instead.
    pub llm_timeout_ms: u64,
    pub tone_personal: Tone,
    pub tone_work: Tone,
    pub tone_email: Tone,
    pub tone_other: Tone,
    /// Keep audio-free history rows. false = never store history.
    pub store_history: bool,
    pub show_overlay: bool,
    /// Watch pasted text for corrections and add corrected terms to the dictionary.
    pub auto_learn_words: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model_file: "ggml-large-v3-turbo-q5_0.bin".into(),
            language: "en".into(),
            input_device: None,
            enable_sounds: true,
            smart_formatting: true,
            use_local_llm: true,
            local_llm_url: "http://localhost:1234/v1".into(),
            local_llm_model: "gemma-4-26b-a4b-it-mlx".into(),
            use_llm_cleanup: true,
            cloud_provider: CloudProvider::Claude,
            llm_model: "claude-haiku-5-5".into(),
            openai_model: "gpt-6-luna".into(),
            grok_model: "grok-4.20-0309-non-reasoning".into(),
            llm_timeout_ms: 3000,
            tone_personal: Tone::Casual,
            tone_work: Tone::Formal,
            tone_email: Tone::Formal,
            tone_other: Tone::Casual,
            store_history: true,
            show_overlay: true,
            auto_learn_words: true,
        }
    }
}

impl Settings {
    pub fn tone_for(&self, c: StyleCategory) -> Tone {
        match c {
            StyleCategory::Personal => self.tone_personal,
            StyleCategory::Work => self.tone_work,
            StyleCategory::Email => self.tone_email,
            StyleCategory::Other => self.tone_other,
        }
    }
}

/// Info about the app the user is dictating into.
#[derive(Debug, Clone, Default)]
pub struct TargetContext {
    pub pid: Option<i32>,
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
}

/// One dictation, as stored in the `history` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    /// UTC RFC 3339.
    pub timestamp: String,
    pub tz_offset_minutes: i32,
    pub app: Option<String>,
    pub asr_text: String,
    pub formatted_text: String,
    /// Recording length in seconds.
    pub duration: f32,
    pub num_words: u32,
    /// "formatted" | "raw_transcript" | "empty" | "error" | "dismissed"
    pub status: String,
    pub language: Option<String>,
}

/// A dictionary word (replacement = None, a vocabulary hint for ASR),
/// a correction rule (phrase -> replacement), or a snippet (is_snippet).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DictEntry {
    pub id: String,
    pub phrase: String,
    pub replacement: Option<String>,
    pub is_snippet: bool,
    pub is_starred: bool,
    pub frequency_used: u32,
    pub created_at: String,
    pub modified_at: String,
    /// "manual" or "user_edits" (learned from a correction the user made after a paste).
    pub source: String,
    /// For learned words: what Murmur had pasted instead, e.g. "Cloud Code" for "Claude Code".
    pub heard_as: Option<String>,
}

/// Why watching a pasted text for user edits stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditEndReason {
    /// The observation window (60 s) ran out.
    WindowElapsed,
    /// The text was sent: a trailing newline appeared or the field was cleared right after Return.
    Submitted,
    TextboxEmptied,
    /// The text around the paste changed so much the pasted region can't be located any more.
    AnchorLost,
    /// Another dictation started.
    NextPaste,
    /// The field couldn't be read (no AX value, secure field, element gone).
    Unreadable,
}

/// The pasted text and what the same region of the field contained when observation ended.
#[derive(Debug, Clone)]
pub struct EditObservation {
    pub pasted: String,
    pub final_text: String,
    pub end_reason: EditEndReason,
}

/// A term learned from a user's correction: `word` replaced `heard_as` in the pasted text.
#[derive(Debug, Clone, PartialEq)]
pub struct LearnedTerm {
    pub word: String,
    pub heard_as: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub total_words: u64,
    pub total_dictations: u64,
    /// Words per minute over the latest 100 dictations shorter than 400 s.
    pub average_wpm: f32,
    /// Consecutive local days, ending today or yesterday, with at least one dictation.
    pub day_streak: u32,
}

pub fn data_dir() -> PathBuf {
    let d = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("Murmur");
    let _ = std::fs::create_dir_all(d.join("models"));
    d
}

/// State shared between the controller thread and the UI (main thread).
#[derive(Default)]
pub struct UiShared {
    pub overlay: std::sync::Mutex<OverlayState>,
    /// Recent input levels (0..1), newest last, at most 32. Written by audio, read by the pill.
    pub levels: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
    /// Message shown in the pill / hub banner while OverlayState::Error, cleared on next dictation.
    pub last_error: std::sync::Mutex<Option<String>>,
    /// "not_loaded" | "loading" | "ready" | "missing" | "downloading 42%" | "error: ..."
    pub model_status: std::sync::Mutex<String>,
    /// Set by the controller whenever history/dictionary changed so the hub re-queries.
    pub data_version: std::sync::atomic::AtomicU64,
    /// True while recording or processing; the hotkey tap only swallows Esc when this is set.
    pub dictation_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Short message shown in the pill for a few seconds, e.g. "Added “Claude Code” to dictionary".
    pub toast: std::sync::Mutex<Option<(String, std::time::Instant)>>,
    /// Latest actionable cloud failure; cleared when that provider succeeds again. Shown in Settings.
    pub cloud_issue: std::sync::Mutex<Option<CloudIssue>>,
    /// (provider, kind) pairs already toasted this session, so each problem is announced once.
    pub toasted_issues: std::sync::Mutex<std::collections::HashSet<(CloudProvider, CloudIssueKind)>>,
    /// Set once the UI starts, so state changes from other threads can wake it immediately.
    pub egui_ctx: std::sync::Mutex<Option<egui::Context>>,
}

impl UiShared {
    pub fn set_overlay(&self, s: OverlayState) {
        *self.overlay.lock().unwrap() = s;
        self.wake_ui();
    }
    pub fn show_toast(&self, msg: impl Into<String>) {
        *self.toast.lock().unwrap() = Some((msg.into(), std::time::Instant::now()));
        self.wake_ui();
    }
    /// Repaint now rather than at the next scheduled tick (the pill must react when dictation starts).
    pub fn wake_ui(&self) {
        if let Some(ctx) = self.egui_ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }
    pub fn bump_data(&self) {
        self.data_version.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
