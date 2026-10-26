# Murmur — spec

Murmur is a local voice-dictation app for macOS: hold a key, speak, and polished text is pasted
into the focused app. Everything runs on the Mac by default; cloud services are opt-in.

## Design

| Concern | Design |
|---|---|
| Shell | One Rust binary: eframe UI on the main thread, a controller thread for the dictation pipeline |
| Hotkeys | Fn hold = push-to-talk, Fn+Space = hands-free toggle, Esc = cancel, Cmd+Ctrl+V = paste last transcript; implemented with a CGEventTap |
| Audio | cpal at the device rate, resampled to 16 kHz mono f32 |
| Speech to text | whisper.cpp (whisper-rs, Metal) on device; dictionary words are passed as the initial prompt |
| Formatting | Local rules (fillers, spoken commands, corrections, snippets, tone) plus optional AI cleanup: a local LM Studio model first, with an optional cloud fallback (Claude, ChatGPT or Grok) |
| Insert | Save clipboard, set text, synthesize Cmd+V, restore clipboard |
| Storage | SQLite `history` and `dictionary` tables, plus `settings.json` |
| Stats | WPM = words / speech minutes over the latest 100 entries shorter than 400 s; day streak computed from local days |
| UI | Hub window (Home/history, Dictionary, Snippets, Style, Settings), floating dictation bar (pill), menu-bar tray; see `UI_SPEC.md` |
| Sounds | macOS system sounds played with `afplay` |

## Module contracts (src/)

Every module uses `crate::types::*`. Do not change `types.rs` without the integrator.

### storage.rs
```rust
pub struct Db { /* Mutex<rusqlite::Connection> */ }      // Send + Sync
impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Db>;      // creates schema, WAL
    pub fn open_in_memory() -> anyhow::Result<Db>;
    pub fn insert_history(&self, e: &HistoryEntry) -> anyhow::Result<()>;
    pub fn recent_history(&self, limit: usize, search: Option<&str>) -> anyhow::Result<Vec<HistoryEntry>>; // newest first
    pub fn last_history(&self) -> anyhow::Result<Option<HistoryEntry>>;
    pub fn delete_history(&self, id: &str) -> anyhow::Result<()>;
    pub fn clear_history(&self) -> anyhow::Result<()>;
    pub fn stats(&self) -> anyhow::Result<Stats>;
    pub fn list_dictionary(&self, snippets: bool) -> anyhow::Result<Vec<DictEntry>>; // excludes soft-deleted; starred first, then phrase
    pub fn upsert_dict(&self, phrase: &str, replacement: Option<&str>, is_snippet: bool) -> anyhow::Result<DictEntry>; // unique on lower(phrase)+is_snippet
    pub fn delete_dict(&self, id: &str) -> anyhow::Result<()>;   // soft delete
    pub fn set_starred(&self, id: &str, starred: bool) -> anyhow::Result<()>;
    pub fn bump_usage(&self, ids: &[String]) -> anyhow::Result<()>;
}
pub fn load_settings() -> Settings;                       // <data_dir>/settings.json, default on missing/corrupt
pub fn save_settings(s: &Settings) -> anyhow::Result<()>;
```

### format.rs
```rust
pub struct FormatInput<'a> { pub raw: &'a str, pub dictionary: &'a [DictEntry], pub category: StyleCategory, pub tone: Tone, pub smart: bool }
pub struct FormatOutput { pub text: String, pub used_dict_ids: Vec<String> }
pub fn format_local(input: &FormatInput) -> FormatOutput;
pub fn llm_cleanup(text: &str, category: StyleCategory, tone: Tone, vocabulary: &[String], model: &str, api_key: &str) -> anyhow::Result<String>;
pub fn apply_snippets(text: &str, dictionary: &[DictEntry]) -> (String, Vec<String>);
pub fn apply_tone(text: &str, tone: Tone) -> String;
pub fn categorize_app(bundle_id: Option<&str>) -> StyleCategory;
pub fn word_count(text: &str) -> u32;
```
Pipeline used by the controller: `format_local` (spoken commands → filler removal → correction rules) → optional `llm_cleanup` → `apply_snippets` → `apply_tone`.

### audio.rs
```rust
pub fn list_input_devices() -> Vec<String>;
pub struct Recorder { /* owns a capture thread; cpal::Stream is !Send so it lives there */ }
impl Recorder {
    pub fn start(device: Option<&str>, levels: Arc<Mutex<Vec<f32>>>) -> anyhow::Result<Recorder>;
    pub fn stop(self) -> Vec<f32>;            // 16 kHz mono f32, clears levels
    pub fn elapsed_secs(&self) -> f32;
}
pub fn rms(samples: &[f32]) -> f32;
```

### transcribe.rs
```rust
pub struct Transcriber { /* WhisperContext */ }
impl Transcriber {
    pub fn load(model_path: &Path) -> anyhow::Result<Transcriber>;
    pub fn transcribe(&self, samples_16k: &[f32], language: &str, vocabulary: &[String]) -> anyhow::Result<String>; // "" for silence
}
pub fn model_url(file: &str) -> String;
pub fn download_model(file: &str, dest: &Path, progress: impl Fn(u64, Option<u64>)) -> anyhow::Result<()>; // writes .part then renames
```

### macOS integration: hotkey.rs, insert.rs, context.rs, sound.rs
```rust
// hotkey.rs
pub fn start_hotkey_listener(tx: Sender<AppEvent>, dictation_active: Arc<AtomicBool>) -> anyhow::Result<()>; // spawns its own CFRunLoop thread
pub fn accessibility_trusted(prompt: bool) -> bool;
// insert.rs
pub fn paste_text(text: &str) -> anyhow::Result<()>;
// context.rs
pub fn frontmost_app() -> TargetContext;
pub fn open_privacy_pane(pane: &str); // "Accessibility" | "Microphone" | "ListenEvent"
// sound.rs
pub enum Cue { Start, Stop, HandsFreeLock, Paste, Error }
pub fn play(cue: Cue); // non-blocking
```

### ui.rs (+ ui/ submodules if wanted)
```rust
pub fn run_ui(shared: Arc<UiShared>, db: Arc<Db>, settings: Arc<Mutex<Settings>>, tx: Sender<AppEvent>) -> anyhow::Result<()>; // blocks on main thread
```
Hub window (Home: stats + searchable history with copy/delete; Dictionary: words + correction rules; Snippets; Style: tone per category; Settings: mic, language, model + download, sounds, smart formatting, Claude cleanup, history storage, permission status). Floating pill as a second viewport: borderless, transparent, always on top, bottom-center. Menu-bar tray: Open Murmur, Paste last transcript, Quit.

### main.rs (integrator)
Wires channels, runs the controller loop on a thread: PttDown → sound+Recorder::start; PttUp/finish → stop → transcribe → format → paste → store; Cancel → discard.
