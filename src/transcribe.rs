//! On-device speech recognition with whisper.cpp (whisper-rs, Metal) and model download.

use anyhow::{anyhow, bail, Context};
use once_cell::sync::Lazy;
use regex::Regex;
use std::io::{Read, Write};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const RATE: usize = 16_000;
const MIN_SAMPLES: usize = RATE * 3 / 10; // 0.3 s
const PAD_TO: usize = RATE * 11 / 10; // whisper.cpp rejects < 1.0 s
const SILENCE_RMS: f32 = 0.001;
const LOW_ENERGY_RMS: f32 = 0.008;
const MAX_PROMPT_WORDS: usize = 50;

pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    pub fn load(model_path: &Path) -> anyhow::Result<Transcriber> {
        install_log_hooks();
        if !model_path.is_file() {
            bail!("model not found: {}", model_path.display());
        }
        let path = model_path.to_str().ok_or_else(|| anyhow!("model path is not UTF-8"))?;
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true).flash_attn(true);
        let ctx = WhisperContext::new_with_params(path, params)
            .map_err(|e| anyhow!("loading whisper model {}: {e:?}", model_path.display()))?;
        Ok(Transcriber { ctx })
    }

    /// Runs one throwaway inference so Metal compiles and caches its shaders now, not on the user's
    /// first dictation (a cold shader cache costs ~14 s on an M3 Max; warm runs take ~0.4 s).
    pub fn warm_up(&self) -> anyhow::Result<()> {
        let mut state = self.ctx.create_state().map_err(|e| anyhow!("whisper state: {e:?}"))?;
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        p.set_language(Some("en"));
        p.set_no_timestamps(true);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        let noise: Vec<f32> = (0..PAD_TO).map(|i| ((i * 7919 % 101) as f32 / 101.0 - 0.5) * 0.01).collect();
        state.full(p, &noise).map_err(|e| anyhow!("whisper warm-up failed: {e:?}"))?;
        Ok(())
    }

    /// Returns "" for silence or when whisper only produced a known hallucination.
    pub fn transcribe(&self, samples_16k: &[f32], language: &str, vocabulary: &[String]) -> anyhow::Result<String> {
        let energy = crate::audio::rms(samples_16k);
        if should_skip(samples_16k.len(), energy) {
            return Ok(String::new());
        }
        let mut audio = samples_16k.to_vec();
        if audio.len() < PAD_TO {
            audio.resize(PAD_TO, 0.0);
        }

        let mut state = self.ctx.create_state().map_err(|e| anyhow!("whisper state: {e:?}"))?;
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
        p.set_n_threads(threads as i32);
        let lang = language.trim();
        p.set_language(if lang.is_empty() || lang.eq_ignore_ascii_case("auto") { None } else { Some(lang) });
        p.set_translate(false);
        p.set_no_context(true);
        p.set_no_timestamps(true);
        p.set_single_segment(false);
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        p.set_suppress_blank(true);
        p.set_suppress_nst(true);
        p.set_initial_prompt(&initial_prompt(vocabulary));

        state.full(p, &audio).map_err(|e| anyhow!("whisper inference failed: {e:?}"))?;
        let raw: String = state
            .as_iter()
            .filter_map(|seg| seg.to_str_lossy().ok().map(|s| s.into_owned()))
            .collect::<Vec<_>>()
            .join(" ");
        let secs = samples_16k.len() as f32 / RATE as f32;
        Ok(clean_output(&raw, secs, energy))
    }
}

fn should_skip(len: usize, energy: f32) -> bool {
    len < MIN_SAMPLES || energy < SILENCE_RMS
}

/// whisper imitates the style of its prompt, so a short, well-punctuated sample nudges it toward
/// commas, sentence breaks and capitals; dictionary words follow as preferred spellings.
const PUNCTUATION_PRIMER: &str = "Hi Sam, thanks for the update. Let's meet on Friday, okay? I'll send the notes.";

fn initial_prompt(vocabulary: &[String]) -> String {
    match vocabulary_prompt(vocabulary) {
        Some(v) => format!("{PUNCTUATION_PRIMER} {v}"),
        None => PUNCTUATION_PRIMER.to_string(),
    }
}

fn vocabulary_prompt(vocabulary: &[String]) -> Option<String> {
    let words: Vec<&str> = vocabulary
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty() && !w.contains('\0'))
        .take(MAX_PROMPT_WORDS)
        .collect();
    (!words.is_empty()).then(|| format!("Vocabulary: {}.", words.join(", ")))
}

static SQUARE_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[[^\]]*\]|♪+").unwrap());
static SOUND_TAG: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)[(*][^)*]*\b(music|silence|silent|blank|laugh\w*|applause|inaudible|indistinct|noise|cough\w*|sigh\w*|breath\w*|static|beep\w*|clap\w*|click\w*|foreign language|no audio|sound|upbeat|whoosh\w*)\b[^)*]*[)*]",
    )
    .unwrap()
});
static SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

/// Phrases whisper invents on near-silent or very short input.
const SILENCE_PHRASES: &[&str] = &[
    "thank you",
    "thank you very much",
    "thanks",
    "thanks for watching",
    "thank you for watching",
    "thanks for listening",
    "thank you for listening",
    "please subscribe",
    "subscribe to my channel",
    "bye",
    "you",
    "so",
];

fn clean_output(raw: &str, secs: f32, energy: f32) -> String {
    let text = SQUARE_TAG.replace_all(raw, " ");
    let text = SOUND_TAG.replace_all(&text, " ");
    // On near-silence whisper can echo its prompt back verbatim; never paste that.
    let text = text.replace(PUNCTUATION_PRIMER, " ");
    let text = SPACES.replace_all(&text, " ").trim().to_string();
    if !text.chars().any(char::is_alphanumeric) {
        return String::new();
    }
    if secs < 1.5 || energy < LOW_ENERGY_RMS {
        let norm: String = text
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '\'')
            .collect();
        let norm = SPACES.replace_all(norm.trim(), " ");
        if SILENCE_PHRASES.contains(&norm.as_ref()) {
            return String::new();
        }
    }
    text
}

/// Without whisper-rs's `log_backend` feature this simply silences whisper.cpp / ggml stderr output.
fn install_log_hooks() {
    whisper_rs::install_logging_hooks();
}

pub fn model_url(file: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file}")
}

/// Streams the model to `<dest>.part`, then renames it into place.
pub fn download_model(file: &str, dest: &Path, progress: impl Fn(u64, Option<u64>)) -> anyhow::Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = dest.with_extension("part");
    let result = (|| -> anyhow::Result<()> {
        let url = model_url(file);
        let resp = ureq::get(&url).call().with_context(|| format!("GET {url}"))?;
        let total = resp.body().content_length().or_else(|| {
            resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok())
        });
        let mut reader = resp.into_body().into_reader();
        let mut out = std::fs::File::create(&part).with_context(|| format!("creating {}", part.display()))?;
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0u64;
        progress(0, total);
        loop {
            let n = reader.read(&mut buf).context("reading model download")?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, total);
        }
        out.sync_all()?;
        if let Some(t) = total {
            if done != t {
                bail!("download incomplete: {done} of {t} bytes");
            }
        }
        std::fs::rename(&part, dest).with_context(|| format!("moving model to {}", dest.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_echo_dropped() {
        assert_eq!(clean_output(PUNCTUATION_PRIMER, 3.0, 0.05), "");
        assert_eq!(clean_output(&format!("{PUNCTUATION_PRIMER} Ship it."), 3.0, 0.05), "Ship it.");
        assert!(initial_prompt(&[]).starts_with(PUNCTUATION_PRIMER));
    }

    #[test]
    fn transcriber_is_send_sync() {
        fn check<T: Send + Sync>() {}
        check::<Transcriber>();
    }

    #[test]
    fn skips_short_or_silent_audio() {
        assert!(should_skip(RATE / 10, 0.1));
        assert!(should_skip(RATE * 2, 0.0));
        assert!(!should_skip(RATE / 2, 0.05));
    }

    #[test]
    fn drops_tags_and_silence_hallucinations() {
        assert_eq!(clean_output(" [BLANK_AUDIO]", 3.0, 0.1), "");
        assert_eq!(clean_output("(silence)", 3.0, 0.1), "");
        assert_eq!(clean_output("[Music] ♪♪", 3.0, 0.1), "");
        assert_eq!(clean_output(" Thank you.", 1.0, 0.1), "");
        assert_eq!(clean_output("Thanks for watching!", 4.0, 0.002), "");
        assert_eq!(clean_output("*laughs* Hello there.", 3.0, 0.1), "Hello there.");
        assert_eq!(clean_output("Hello [Music] world (upbeat music)", 3.0, 0.1), "Hello world");
    }

    #[test]
    fn keeps_real_speech() {
        assert_eq!(clean_output(" Thank you.", 3.0, 0.05), "Thank you.");
        assert_eq!(clean_output(" Meet me at noon (tentatively).", 2.0, 0.05), "Meet me at noon (tentatively).");
        assert_eq!(clean_output("  Thank you for the update.  ", 1.0, 0.05), "Thank you for the update.");
    }

    #[test]
    fn builds_vocabulary_prompt() {
        assert_eq!(vocabulary_prompt(&[]), None);
        let v = vec!["Murmur".to_string(), " ".into(), "whisper.cpp".into()];
        assert_eq!(vocabulary_prompt(&v).as_deref(), Some("Vocabulary: Murmur, whisper.cpp."));
        let many: Vec<String> = (0..80).map(|i| format!("w{i}")).collect();
        assert_eq!(vocabulary_prompt(&many).unwrap().matches(", ").count(), MAX_PROMPT_WORDS - 1);
    }

    #[test]
    fn model_url_format() {
        assert_eq!(
            model_url("ggml-base.en.bin"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
        );
    }

    #[test]
    #[ignore = "needs a downloaded model in <data_dir>/models"]
    fn transcribes_silence_to_empty() {
        let path = crate::types::data_dir().join("models").join(crate::types::Settings::default().model_file);
        let t = Transcriber::load(&path).unwrap();
        assert_eq!(t.transcribe(&vec![0.0; RATE * 2], "en", &[]).unwrap(), "");
    }
}
