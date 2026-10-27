mod accounts;
mod audio;
mod chatgpt_auth;
mod context;
mod format;
mod hotkey;
mod insert;
mod learn;
mod observe;
mod sound;
mod storage;
mod transcribe;
mod types;
mod ui;

use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use audio::Recorder;
use format::FormatInput;
use sound::Cue;
use storage::Db;
use transcribe::Transcriber;
use types::*;

/// Must match CFBundleIdentifier in scripts/bundle.sh.
const OWN_BUNDLE_ID: &str = "dev.naeem.murmur";

/// Fn taps shorter than this are treated as accidental and discarded.
const MIN_HOLD: Duration = Duration::from_millis(250);

struct Recording {
    recorder: Recorder,
    hands_free: bool,
    started: Instant,
    target: TargetContext,
}

struct Controller {
    shared: Arc<UiShared>,
    db: Arc<Db>,
    settings: Arc<Mutex<Settings>>,
    transcriber: Arc<RwLock<Option<Transcriber>>>,
    recording: Option<Recording>,
    /// Bumped on every new dictation and on cancel; a worker only pastes if its generation is current.
    generation: Arc<AtomicU64>,
}

impl Controller {
    fn run(mut self, rx: Receiver<AppEvent>) {
        let device = self.settings.lock().unwrap().input_device.clone();
        std::thread::spawn(move || audio::warm_up(device.as_deref()));
        self.load_model();
        for ev in rx {
            log::debug!("event {ev:?}");
            match ev {
                AppEvent::PttDown => self.start(false),
                AppEvent::PttUp => {
                    if self.recording.as_ref().is_some_and(|r| !r.hands_free) {
                        self.finish();
                    }
                }
                AppEvent::HandsFreeToggle => match &mut self.recording {
                    // PttDown arrives first for Fn+Space, so an in-flight PTT recording is promoted.
                    Some(r) if !r.hands_free => {
                        r.hands_free = true;
                        self.cue(Cue::HandsFreeLock);
                        self.shared.set_overlay(OverlayState::ListeningHandsFree);
                    }
                    Some(_) => self.finish(),
                    None => self.start(true),
                },
                AppEvent::Cancel => self.cancel(),
                AppEvent::PasteLast => {
                    if let Ok(Some(last)) = self.db.last_history() {
                        if let Err(e) = insert::paste_text(&last.formatted_text) {
                            self.fail(format!("Paste failed: {e}"));
                        }
                    }
                }
                AppEvent::ReloadModel => self.load_model(),
                AppEvent::OpenHub => {}
                AppEvent::Quit => std::process::exit(0),
            }
        }
    }

    fn load_model(&self) {
        let file = self.settings.lock().unwrap().model_file.clone();
        let path = data_dir().join("models").join(&file);
        if !path.exists() {
            *self.shared.model_status.lock().unwrap() = "missing".into();
            *self.transcriber.write().unwrap() = None;
            return;
        }
        *self.shared.model_status.lock().unwrap() = "loading".into();
        let (shared, slot) = (self.shared.clone(), self.transcriber.clone());
        std::thread::spawn(move || {
            let status = match Transcriber::load(&path) {
                Ok(t) => {
                    if let Err(e) = t.warm_up() {
                        log::warn!("{e}");
                    }
                    *slot.write().unwrap() = Some(t);
                    "ready".to_string()
                }
                Err(e) => format!("error: {e}"),
            };
            *shared.model_status.lock().unwrap() = status;
        });
    }

    fn start(&mut self, hands_free: bool) {
        if self.recording.is_some() {
            return;
        }
        // Capture the target before anything of ours can take focus.
        let target = context::frontmost_app();
        keep_local_model_warm(&self.settings.lock().unwrap());
        if let (Some(pid), true) = (target.pid, self.settings.lock().unwrap().auto_learn_words) {
            if target.bundle_id.as_deref() != Some(OWN_BUNDLE_ID) {
                std::thread::spawn(move || observe::enable_app_accessibility(pid));
            }
        }
        let (device, sounds) = {
            let s = self.settings.lock().unwrap();
            (s.input_device.clone(), s.enable_sounds)
        };
        match Recorder::start(device.as_deref(), self.shared.levels.clone()) {
            Ok(recorder) => {
                self.generation.fetch_add(1, Ordering::SeqCst);
                *self.shared.last_error.lock().unwrap() = None;
                self.shared.dictation_active.store(true, Ordering::SeqCst);
                if sounds {
                    sound::play(if hands_free { Cue::HandsFreeLock } else { Cue::Start });
                }
                self.shared.set_overlay(if hands_free {
                    OverlayState::ListeningHandsFree
                } else {
                    OverlayState::ListeningPtt
                });
                self.recording = Some(Recording { recorder, hands_free, started: Instant::now(), target });
            }
            Err(e) => self.fail(format!("Microphone unavailable: {e}")),
        }
    }

    fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(r) = self.recording.take() {
            r.recorder.stop();
        }
        self.shared.dictation_active.store(false, Ordering::SeqCst);
        self.shared.set_overlay(OverlayState::Resting);
    }

    fn finish(&mut self) {
        let Some(rec) = self.recording.take() else { return };
        let held = rec.started.elapsed();
        let duration = rec.recorder.elapsed_secs();
        let samples = rec.recorder.stop();
        if held < MIN_HOLD {
            self.shared.dictation_active.store(false, Ordering::SeqCst);
            self.shared.set_overlay(OverlayState::Resting);
            return;
        }
        self.cue(Cue::Stop);
        self.shared.set_overlay(OverlayState::Processing);

        let job = Job {
            shared: self.shared.clone(),
            db: self.db.clone(),
            settings: self.settings.lock().unwrap().clone(),
            transcriber: self.transcriber.clone(),
            generation: self.generation.clone(),
            my_gen: self.generation.load(Ordering::SeqCst),
            target: rec.target,
            duration,
        };
        std::thread::spawn(move || job.run(samples));
    }

    fn cue(&self, cue: Cue) {
        if self.settings.lock().unwrap().enable_sounds {
            sound::play(cue);
        }
    }

    fn fail(&self, msg: String) {
        fail(&self.shared, self.settings.lock().unwrap().enable_sounds, msg);
    }
}

/// Local model first, then the chosen cloud provider, inside one time budget. None means paste the local-rules text.
/// Cleanup time grows ~30 ms per output word, so a flat limit timed out every long dictation.
/// The configured limit covers dictations up to 20 words; longer ones get 35 ms per extra word,
/// up to three times the setting.
fn cleanup_budget(limit_ms: u64, words: u32) -> Duration {
    let extra = u64::from(words.saturating_sub(20)) * 35;
    Duration::from_millis((limit_ms + extra).min(limit_ms * 3))
}

/// Local model first, then the cloud provider, inside one time budget. None means paste the local-rules text.
/// Actionable cloud failures are reported to `ui` (a toast once per kind, plus the Settings banner).
fn ai_cleanup(
    s: &Settings,
    text: &str,
    category: StyleCategory,
    tone: Tone,
    vocabulary: &[String],
    ui: Option<&UiShared>,
) -> Option<String> {
    let started = Instant::now();
    let deadline = started + cleanup_budget(s.llm_timeout_ms, format::word_count(text));
    let remaining = || deadline.saturating_duration_since(Instant::now());
    // A reply far longer than the dictation means the model answered it instead of cleaning it.
    let plausible = |out: &str| out.len() <= text.len() * 2 + 40;

    if s.use_local_llm {
        match format::local_llm_cleanup(text, category, tone, vocabulary, &s.local_llm_url, &s.local_llm_model, remaining()) {
            Ok(out) if plausible(&out) => {
                log::info!("cleanup: local model in {} ms", started.elapsed().as_millis());
                return Some(out);
            }
            Ok(_) => log::warn!("cleanup: local model output rejected as implausible"),
            Err(e) => log::warn!("cleanup: local model failed after {} ms: {e:#}", started.elapsed().as_millis()),
        }
    }
    if s.use_llm_cleanup && remaining() > Duration::from_millis(300) {
        let provider = s.cloud_provider;
        let name = provider.label();
        // Signed in with ChatGPT (plan usage granted): Responses API on the user's plan.
        let plan_token = if provider == CloudProvider::ChatGpt { chatgpt_auth::access_token() } else { None };
        let result = match provider {
            CloudProvider::Claude => accounts::api_key(provider)
                .map(|key| ("API key", format::llm_cleanup(text, category, tone, vocabulary, &s.llm_model, &key, remaining()))),
            CloudProvider::ChatGpt if plan_token.is_some() => Some((
                "ChatGPT plan",
                format::chatgpt_plan_cleanup(
                    text,
                    category,
                    tone,
                    vocabulary,
                    &s.openai_model,
                    plan_token.as_deref().unwrap_or_default(),
                    remaining(),
                ),
            )),
            CloudProvider::ChatGpt | CloudProvider::Grok => {
                let (base, model) = if provider == CloudProvider::ChatGpt {
                    (format::OPENAI_BASE_URL, &s.openai_model)
                } else {
                    (format::XAI_BASE_URL, &s.grok_model)
                };
                accounts::api_key(provider).map(|key| {
                    let r = format::openai_compatible_cleanup(text, category, tone, vocabulary, base, model, Some(&key), remaining());
                    ("API key", r)
                })
            }
        };
        match result {
            Some((kind, Ok(out))) if plausible(&out) => {
                log::info!("cleanup: {name} ({kind}) in {} ms", started.elapsed().as_millis());
                if let Some(ui) = ui {
                    let mut issue = ui.cloud_issue.lock().unwrap();
                    if issue.as_ref().is_some_and(|i| i.provider == provider) {
                        *issue = None;
                    }
                }
                return Some(out);
            }
            Some((kind, Ok(_))) => log::warn!("cleanup: {name} ({kind}) output rejected as implausible"),
            Some((kind, Err(e))) => {
                let detail = format!("{e:#}");
                log::warn!("cleanup: {name} ({kind}) failed: {detail}");
                if let (Some(ui), Some(issue_kind)) = (ui, format::classify_cloud_error(&detail)) {
                    report_cloud_issue(ui, CloudIssue { provider, kind: issue_kind, detail });
                }
            }
            None => log::info!("cleanup: no {name} credential"),
        }
    }
    log::info!("cleanup: using local rules only");
    None
}

/// Records the issue for the Settings banner and toasts it the first time this session.
fn report_cloud_issue(ui: &UiShared, issue: CloudIssue) {
    let first = ui.toasted_issues.lock().unwrap().insert((issue.provider, issue.kind));
    if first {
        let p = issue.provider.label();
        ui.show_toast(match issue.kind {
            CloudIssueKind::Billing => format!("{p} out of credit · see Settings"),
            CloudIssueKind::BadKey => format!("{p} key rejected · see Settings"),
            CloudIssueKind::UnknownModel => format!("{p}: unknown model · see Settings"),
            CloudIssueKind::PlanLimit => format!("{p} plan limit · see Settings"),
        });
    }
    *ui.cloud_issue.lock().unwrap() = Some(issue);
    ui.bump_data();
}

/// Pings the local model server so the cleanup model stays loaded. At most once a minute; never blocks.
fn keep_local_model_warm(s: &Settings) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    if !(s.smart_formatting && s.use_local_llm) {
        return;
    }
    {
        let mut last = LAST.lock().unwrap();
        if last.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        *last = Some(Instant::now());
    }
    let (url, model) = (s.local_llm_url.clone(), s.local_llm_model.clone());
    std::thread::spawn(move || {
        let t = Instant::now();
        match format::local_llm_ping(&url, &model) {
            Ok(()) => log::debug!("local model warm ({} ms)", t.elapsed().as_millis()),
            Err(e) => log::debug!("local model warm-up failed: {e:#}"),
        }
    });
}

fn fail(shared: &UiShared, sounds: bool, msg: String) {
    log::warn!("{msg}");
    *shared.last_error.lock().unwrap() = Some(msg);
    shared.dictation_active.store(false, Ordering::SeqCst);
    shared.set_overlay(OverlayState::Error);
    if sounds {
        sound::play(Cue::Error);
    }
}

/// Transcribe → format → paste → store, off the controller thread so Esc stays responsive.
struct Job {
    shared: Arc<UiShared>,
    db: Arc<Db>,
    settings: Settings,
    transcriber: Arc<RwLock<Option<Transcriber>>>,
    generation: Arc<AtomicU64>,
    my_gen: u64,
    target: TargetContext,
    duration: f32,
}

impl Job {
    fn current(&self) -> bool {
        self.generation.load(Ordering::SeqCst) == self.my_gen
    }

    fn run(self, samples: Vec<f32>) {
        match self.process(&samples) {
            Ok(()) => {
                if self.current() {
                    self.shared.dictation_active.store(false, Ordering::SeqCst);
                    self.shared.set_overlay(OverlayState::Resting);
                }
            }
            Err(e) if self.current() => fail(&self.shared, self.settings.enable_sounds, e.to_string()),
            Err(_) => {}
        }
    }

    /// Watches the field we just pasted into and learns the terms the user corrects.
    /// Ends after 60 s, on send, or when the next dictation starts.
    fn watch_for_corrections(&self, pasted: &str) {
        let Some(anchor) = observe::capture_after_paste(pasted) else {
            log::debug!("edit watch: target field isn't readable");
            return;
        };
        let (db, shared, generation, my_gen) = (self.db.clone(), self.shared.clone(), self.generation.clone(), self.my_gen);
        std::thread::spawn(move || {
            let obs = observe::watch(anchor, Duration::from_secs(60), || generation.load(Ordering::SeqCst) != my_gen);
            log::debug!("edit watch ended: {:?}", obs.end_reason);
            for term in learn::learn_from_edit(&obs.pasted, &obs.final_text) {
                match db.add_learned(&term.word, &term.heard_as) {
                    Ok(Some(_)) => {
                        log::info!("learned {:?} (heard as {:?})", term.word, term.heard_as);
                        shared.show_toast(format!("Added “{}” to dictionary", term.word));
                        shared.bump_data();
                    }
                    Ok(None) => {}
                    Err(e) => log::warn!("saving learned word: {e:#}"),
                }
            }
        });
    }

    fn process(&self, samples: &[f32]) -> anyhow::Result<()> {
        let s = &self.settings;
        let dictionary = self.db.list_dictionary(false)?;
        let snippets = self.db.list_dictionary(true)?;
        let vocabulary: Vec<String> = dictionary
            .iter()
            .map(|d| d.replacement.clone().unwrap_or_else(|| d.phrase.clone()))
            .collect();

        let raw = {
            let guard = self.transcriber.read().unwrap();
            let t = guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No speech model yet. Open Murmur → Settings to download one."))?;
            t.transcribe(samples, &s.language, &vocabulary)?
        };
        if !self.current() {
            return Ok(());
        }

        let mut entry = HistoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            tz_offset_minutes: chrono::Local::now().offset().local_minus_utc() / 60,
            app: self.target.app_name.clone().or(self.target.bundle_id.clone()),
            asr_text: raw.clone(),
            formatted_text: String::new(),
            duration: self.duration,
            num_words: 0,
            status: "empty".into(),
            language: Some(s.language.clone()),
        };
        if raw.trim().is_empty() {
            return Ok(());
        }

        let category = format::categorize_app(self.target.bundle_id.as_deref());
        let tone = s.tone_for(category);
        let local = format::format_local(&FormatInput { raw: &raw, dictionary: &dictionary, category, tone, smart: s.smart_formatting });
        let mut used = local.used_dict_ids;
        let mut text = local.text;
        entry.status = "raw_transcript".into();

        // Already-clean or very short dictations skip the model: it can't improve them, and it is
        // most of the latency.
        if s.smart_formatting && format::needs_ai_cleanup(&raw) {
            if let Some(cleaned) = ai_cleanup(s, &text, category, tone, &vocabulary, Some(&self.shared)) {
                text = cleaned;
            }
        }
        let (expanded, snippet_ids) = format::apply_snippets(&text, &snippets);
        used.extend(snippet_ids);
        text = if s.smart_formatting { format::apply_tone(&expanded, tone) } else { expanded };
        if s.smart_formatting {
            entry.status = "formatted".into();
        }

        if !self.current() {
            return Ok(());
        }
        // A lone spoken command ("period") can format down to nothing; don't paste or store a blank row.
        if text.trim().is_empty() {
            return Ok(());
        }
        let into_murmur = self.target.bundle_id.as_deref() == Some(OWN_BUNDLE_ID);
        if into_murmur {
            // Dictating with our own window in front: keep the text, don't type it into the hub.
            // With history off, the clipboard is the only place it can go.
            if !s.store_history {
                arboard::Clipboard::new()?.set_text(text.clone())?;
            }
        } else {
            // Clicking the pill's finish button activates Murmur; hand focus back before pasting.
            if let Some(id) = &self.target.bundle_id {
                if context::activate(id) {
                    std::thread::sleep(Duration::from_millis(120));
                }
            }
            insert::paste_text(&text)?;
            if s.enable_sounds {
                sound::play(Cue::Paste);
            }
            if s.auto_learn_words {
                self.watch_for_corrections(&text);
            }
        }

        entry.num_words = format::word_count(&text);
        entry.formatted_text = text;
        if s.store_history {
            self.db.insert_history(&entry)?;
        }
        self.db.bump_usage(&used)?;
        self.shared.bump_data();
        Ok(())
    }
}

/// `murmur --transcribe file.wav`: run the dictation pipeline on a mono 16-bit or float WAV and print each stage.
/// `murmur --bench <dir>`: runs every WAV in `dir` through the real pipeline with the saved
/// settings and prints per-stage timings, plus how long cleanup takes with no time limit.
fn cli_bench(dir: &str) -> anyhow::Result<()> {
    let settings = storage::load_settings();
    let transcriber = Transcriber::load(&data_dir().join("models").join(&settings.model_file))?;
    transcriber.warm_up()?;
    if settings.use_local_llm {
        let _ = format::local_llm_ping(&settings.local_llm_url, &settings.local_llm_model);
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    files.sort();
    let mut unlimited = settings.clone();
    unlimited.llm_timeout_ms = 30_000;
    unlimited.use_llm_cleanup = false;
    println!("{:<22}{:>6}{:>8}{:>9}{:>10}{:>11}{:>8}  result", "file", "audio", "asr", "cleanup", "uncapped", "total", "words");
    let (mut sum_total, mut sum_unc) = (0u128, 0u128);
    for f in files {
        let audio = read_wav(f.to_str().unwrap_or_default())?;
        let t0 = Instant::now();
        let raw = transcriber.transcribe(&audio, &settings.language, &[])?;
        let asr = t0.elapsed().as_millis();
        let tone = settings.tone_for(StyleCategory::Other);
        let local = format::format_local(&FormatInput { raw: &raw, dictionary: &[], category: StyleCategory::Other, tone, smart: true });
        let t1 = Instant::now();
        let needed = format::needs_ai_cleanup(&raw);
        let cleaned = if needed { ai_cleanup(&settings, &local.text, StyleCategory::Other, tone, &[], None) } else { None };
        let cleanup = t1.elapsed().as_millis();
        let t2 = Instant::now();
        let _ = ai_cleanup(&unlimited, &local.text, StyleCategory::Other, tone, &[], None);
        let unc = t2.elapsed().as_millis();
        sum_total += asr + cleanup;
        sum_unc += unc;
        println!(
            "{:<22}{:>5.1}s{:>6}ms{:>7}ms{:>8}ms{:>9}ms{:>8}  {}",
            f.file_name().unwrap_or_default().to_string_lossy(),
            audio.len() as f32 / 16_000.0,
            asr,
            cleanup,
            unc,
            asr + cleanup,
            format::word_count(&local.text),
            match (needed, cleaned.is_some()) {
                (false, _) => "skipped (already clean)",
                (true, true) => "AI cleaned",
                (true, false) => "TIMEOUT -> local rules",
            }
        );
    }
    println!("sum of totals: {sum_total} ms · sum of uncapped cleanup: {sum_unc} ms");
    Ok(())
}

/// Mono 16 kHz samples from a 16-bit PCM or float WAV file (for the CLI tools).
fn read_wav(path: &str) -> anyhow::Result<Vec<f32>> {
    let bytes = std::fs::read(path)?;
    anyhow::ensure!(bytes.len() > 44 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE", "not a WAV file");
    let (mut pos, mut format, mut rate, mut bits, mut channels) = (12, 0u16, 0u32, 0u16, 1u16);
    let mut samples = Vec::new();
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into()?) as usize;
        let body = &bytes[pos + 8..(pos + 8 + len).min(bytes.len())];
        match id {
            b"fmt " => {
                format = u16::from_le_bytes([body[0], body[1]]);
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes(body[4..8].try_into()?);
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => {
                samples = match (format, bits) {
                    (1, 16) => body.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
                    (3, 32) => body.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect(),
                    _ => anyhow::bail!("unsupported WAV encoding (format {format}, {bits} bits)"),
                };
            }
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    let mono: Vec<f32> = samples.chunks(channels as usize).map(|f| f.iter().sum::<f32>() / f.len() as f32).collect();
    Ok(audio::resample(&mono, rate, 16_000))
}

fn cli_transcribe(path: &str) -> anyhow::Result<()> {
    let audio = read_wav(path)?;

    let settings = storage::load_settings();
    let model = data_dir().join("models").join(&settings.model_file);
    let t0 = Instant::now();
    let transcriber = Transcriber::load(&model)?;
    println!("load      {:>6} ms", t0.elapsed().as_millis());
    // Test vocabulary without touching the dictionary: MURMUR_VOCAB="Claude,Priya".
    let vocabulary: Vec<String> = std::env::var("MURMUR_VOCAB")
        .map(|v| v.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect())
        .unwrap_or_default();
    let mut raw = String::new();
    for pass in ["cold", "warm"] {
        let t1 = Instant::now();
        raw = transcriber.transcribe(&audio, &settings.language, &vocabulary)?;
        println!("{pass:<10}{:>6} ms  ({:.1} s audio)", t1.elapsed().as_millis(), audio.len() as f32 / 16_000.0);
    }
    println!("raw:       {raw}");
    let tone = settings.tone_for(StyleCategory::Other);
    let local = format::format_local(&FormatInput { raw: &raw, dictionary: &[], category: StyleCategory::Other, tone, smart: true });
    println!("local:     {}", format::apply_tone(&local.text, tone));
    let t2 = Instant::now();
    if let Some(cleaned) = ai_cleanup(&settings, &local.text, StyleCategory::Other, tone, &vocabulary, None) {
        println!("ai ({:>4} ms): {}", t2.elapsed().as_millis(), format::apply_tone(&cleaned, tone));
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("murmur=info")).init();
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, dir] = args.as_slice() {
        if flag == "--bench" {
            return cli_bench(dir);
        }
    }
    if args.get(1).map(String::as_str) == Some("--cloud-check") {
        // Run the configured cloud provider directly (local model skipped). Never prints credentials.
        let mut s = storage::load_settings();
        s.use_local_llm = false;
        s.use_llm_cleanup = true;
        s.llm_timeout_ms = 10_000;
        let p = s.cloud_provider;
        let plan = p == CloudProvider::ChatGpt && chatgpt_auth::account().is_some_and(|a| a.plan_usage);
        let model = match p {
            CloudProvider::Claude => &s.llm_model,
            CloudProvider::ChatGpt => &s.openai_model,
            CloudProvider::Grok => &s.grok_model,
        };
        println!(
            "provider {} · model {model} · credential: {}",
            p.label(),
            if plan { "ChatGPT plan" } else { accounts::key_source(p).unwrap_or("none") }
        );
        for raw in [
            "hey so I talked to Priya and she said the launch is moving to the 14th which honestly works better for us so can you update the roadmap",
            "actually wait let's not merge that yet the tests are flaky on main and I want to see a green run first",
        ] {
            let t = Instant::now();
            let ui = UiShared::default();
            match ai_cleanup(&s, raw, StyleCategory::Other, Tone::Casual, &[], Some(&ui)) {
                Some(out) => println!("[{:>4} ms] {out}", t.elapsed().as_millis()),
                None => println!("[{:>4} ms] (cloud cleanup failed; see the warning above)", t.elapsed().as_millis()),
            }
            let issue = ui.cloud_issue.lock().unwrap().clone();
            let toast = ui.toast.lock().unwrap().clone().map(|t| t.0);
            if let Some(issue) = issue {
                println!("  → reported as {:?}; toast: {toast:?}", issue.kind);
            }
        }
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--mic-bench") {
        // Time from "start" to the first captured audio, three times in a row.
        let levels = Arc::new(Mutex::new(Vec::new()));
        for pass in 1..=3 {
            let t = Instant::now();
            let rec = Recorder::start(None, levels.clone())?;
            let opened = t.elapsed();
            while levels.lock().unwrap().is_empty() && t.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(2));
            }
            println!("pass {pass}: stream open {:>5} ms, first audio {:>5} ms", opened.as_millis(), t.elapsed().as_millis());
            rec.stop();
        }
        return Ok(());
    }
    if let [_, flag, path] = args.as_slice() {
        if flag == "--transcribe" {
            return cli_transcribe(path);
        }
    }

    let settings = Arc::new(Mutex::new(storage::load_settings()));
    let db = Arc::new(Db::open(&data_dir().join("murmur.sqlite"))?);
    let shared = Arc::new(UiShared::default());
    let (tx, rx): (Sender<AppEvent>, Receiver<AppEvent>) = unbounded();

    if !hotkey::accessibility_trusted(true) {
        log::warn!("Accessibility permission missing: hotkeys and paste won't work until granted");
    }
    if let Err(e) = hotkey::start_hotkey_listener(tx.clone(), shared.dictation_active.clone()) {
        *shared.last_error.lock().unwrap() = Some(format!("Hotkey unavailable: {e}"));
        log::error!("hotkey listener failed: {e}");
    }

    let controller = Controller {
        shared: shared.clone(),
        db: db.clone(),
        settings: settings.clone(),
        transcriber: Arc::new(RwLock::new(None)),
        recording: None,
        generation: Arc::new(AtomicU64::new(0)),
    };
    std::thread::spawn(move || controller.run(rx));

    let warm_settings = settings.clone();
    std::thread::spawn(move || loop {
        keep_local_model_warm(&warm_settings.lock().unwrap().clone());
        std::thread::sleep(Duration::from_secs(240));
    });

    // Error pills fade back to resting after a few seconds.
    let s2 = shared.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        let mut o = s2.overlay.lock().unwrap();
        if *o == OverlayState::Error && !s2.dictation_active.load(Ordering::SeqCst) {
            drop(o);
            std::thread::sleep(Duration::from_secs(3));
            o = s2.overlay.lock().unwrap();
            if *o == OverlayState::Error {
                *o = OverlayState::Resting;
            }
        }
    });

    ui::run_ui(shared, db, settings, tx)?;
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_budget_scales_with_length() {
        assert_eq!(cleanup_budget(3000, 4), Duration::from_millis(3000));
        assert_eq!(cleanup_budget(3000, 20), Duration::from_millis(3000));
        assert_eq!(cleanup_budget(3000, 111), Duration::from_millis(6185));
        assert_eq!(cleanup_budget(3000, 1000), Duration::from_millis(9000));
    }
}
