//! Feedback cues using built-in macOS system sounds via `afplay` (thread-safe, non-blocking).

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    Start,
    Stop,
    HandsFreeLock,
    Paste,
    Error,
}

fn sound_name(cue: Cue) -> &'static str {
    match cue {
        Cue::Start => "Tink",
        Cue::Stop => "Pop",
        Cue::HandsFreeLock => "Morse",
        Cue::Paste => "Bottle",
        Cue::Error => "Funk",
    }
}

fn sound_path(cue: Cue) -> String {
    format!("/System/Library/Sounds/{}.aiff", sound_name(cue))
}

/// Plays `cue` at low volume without blocking the caller.
pub fn play(cue: Cue) {
    let path = sound_path(cue);
    std::thread::spawn(move || {
        // Waiting on the child reaps it so no zombies accumulate.
        if let Err(e) = Command::new("/usr/bin/afplay").args(["-v", "0.4", &path]).status() {
            log::warn!("afplay {path}: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_cue_sounds_exist() {
        for c in [Cue::Start, Cue::Stop, Cue::HandsFreeLock, Cue::Paste, Cue::Error] {
            assert!(std::path::Path::new(&sound_path(c)).exists(), "{c:?}");
        }
    }
}
