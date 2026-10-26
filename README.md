# Murmur

Local voice dictation for macOS. Hold **fn**, speak, and polished text is pasted into whatever app you are using. Inspired by Wispr Flow.

- On-device transcription with [whisper.cpp](https://github.com/ggerganov/whisper.cpp) (Metal accelerated).
- Optional AI cleanup of the transcript through a local [LM Studio](https://lmstudio.ai) model, with a cloud fallback to Claude, ChatGPT or Grok.
- Personal dictionary, snippets, per-context writing styles, and learning from your corrections.
- A floating dictation bar, a menu-bar item, and a searchable history.

## Requirements

- Apple Silicon Mac, macOS 13 or later
- Rust toolchain ([rustup](https://rustup.rs))
- `cmake` (for building whisper.cpp), e.g. `brew install cmake`

## Build and install

```sh
./scripts/bundle.sh
cp -R target/Murmur.app /Applications/
```

`bundle.sh` builds a release binary and wraps it in `target/Murmur.app`.

**Signing.** macOS ties privacy permissions to the app's code signature. If a self-signed identity named `Murmur Dev` exists, the script uses it and your Microphone, Accessibility and Input Monitoring grants survive rebuilds. Otherwise it falls back to an ad-hoc signature, and you must re-grant permissions after every rebuild. To create the identity: open Keychain Access, choose Keychain Access → Certificate Assistant → Create a Certificate, name it `Murmur Dev`, set the type to Code Signing, and create it. Set `MURMUR_SIGN_IDENTITY` to use a different name.

## First run

1. Open Murmur and download a speech model in Settings.
2. Grant **Microphone**, **Accessibility** and **Input Monitoring** when prompted (System Settings → Privacy & Security).
3. In System Settings → Keyboard, set "Press 🌐 key to" to **Do Nothing** so the fn key is free for Murmur.

## Usage

| Action | Keys |
|---|---|
| Push-to-talk | Hold **fn**, speak, release |
| Hands-free | **fn + space**; press again to stop |
| Cancel | **esc** |
| Paste last transcript | **⌘ ⌃ V** |

Spoken commands are converted while you dictate, for example "new line", "new paragraph", "comma" and other punctuation words.

## Developer commands

```sh
cargo test                              # unit tests
cargo run --release -- --transcribe file.wav   # transcribe a WAV file with the configured model
cargo run --release -- --cloud-check    # run the configured cloud provider on sample text
cargo run --release -- --mic-bench      # measure microphone start-up latency
MURMUR_PILL_DEMO=1 cargo run --release  # cycle the dictation bar through its states
```

`scripts/e2e_fn.swift` and `scripts/e2e_target.swift` are end-to-end helpers: the first presses the hotkeys and speaks a phrase with `say`, the second is a focused text view that prints what it receives. See the comments at the top of each file for usage.

See [SPEC.md](SPEC.md) for architecture and module contracts and [UI_SPEC.md](UI_SPEC.md) for the interface design.

## Privacy

Audio and transcripts stay on your Mac unless you enable a cloud provider for AI cleanup, in which case the transcript text (not audio) is sent to that provider. API keys are stored in the macOS Keychain.

## License

MIT, see [LICENSE](LICENSE). The bundled fonts (Figtree and EB Garamond) are licensed under the SIL Open Font License 1.1; see `assets/fonts`.
