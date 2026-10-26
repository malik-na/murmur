//! Microphone capture (cpal) and resampling to 16 kHz mono f32.

use anyhow::{anyhow, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use crossbeam_channel::{bounded, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const TARGET_RATE: u32 = 16_000;
const MAX_LEVELS: usize = 32;
const LEVEL_WINDOW_SECS: f32 = 0.03;

pub fn list_input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

pub struct Recorder {
    stop_tx: Sender<()>,
    thread: Option<JoinHandle<()>>,
    buffer: Arc<Mutex<Vec<f32>>>,
    levels: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    started: Instant,
}

impl Recorder {
    pub fn start(device: Option<&str>, levels: Arc<Mutex<Vec<f32>>>) -> anyhow::Result<Recorder> {
        let buffer = Arc::new(Mutex::new(Vec::<f32>::with_capacity(48_000 * 30)));
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let (ready_tx, ready_rx) = bounded::<anyhow::Result<u32>>(1);
        let name = device.map(str::to_owned);
        let (buf, lv) = (buffer.clone(), levels.clone());

        let thread = std::thread::Builder::new()
            .name("murmur-audio".into())
            .spawn(move || match build_stream(name.as_deref(), buf, lv) {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .context("spawning audio thread")?;

        let rate = match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(rate)) => rate,
            Ok(Err(e)) => {
                let _ = thread.join();
                return Err(e);
            }
            Err(_) => {
                let _ = stop_tx.try_send(());
                return Err(anyhow!("timed out opening the microphone (permission prompt pending?)"));
            }
        };
        levels.lock().unwrap().clear();
        Ok(Recorder { stop_tx, thread: Some(thread), buffer, levels, rate, started: Instant::now() })
    }

    /// Stops capture and returns 16 kHz mono samples.
    pub fn stop(mut self) -> Vec<f32> {
        let _ = self.stop_tx.try_send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let raw = std::mem::take(&mut *self.buffer.lock().unwrap());
        self.levels.lock().unwrap().clear();
        resample(&raw, self.rate, TARGET_RATE)
    }

    pub fn elapsed_secs(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Covers the cancel path where the Recorder is dropped without stop().
        let _ = self.stop_tx.try_send(());
    }
}

/// Initializes CoreAudio and resolves the input device without opening a stream (so the mic
/// indicator stays off). The first lookup in a fresh process is the slow part of starting to record.
pub fn warm_up(device: Option<&str>) {
    let t = Instant::now();
    let host = cpal::default_host();
    let found = device
        .and_then(|n| host.input_devices().ok()?.find(|d| d.name().map(|x| x == n).unwrap_or(false)))
        .or_else(|| host.default_input_device());
    let config = found.as_ref().map(|d| d.default_input_config());
    log::debug!("audio warm-up: device {} in {} ms", if matches!(config, Some(Ok(_))) { "ready" } else { "unavailable" }, t.elapsed().as_millis());
}

fn build_stream(
    name: Option<&str>,
    buffer: Arc<Mutex<Vec<f32>>>,
    levels: Arc<Mutex<Vec<f32>>>,
) -> anyhow::Result<(cpal::Stream, u32)> {
    let t = Instant::now();
    let host = cpal::default_host();
    let named = name.and_then(|n| {
        let found = host.input_devices().ok()?.find(|d| d.name().map(|x| x == n).unwrap_or(false));
        if found.is_none() {
            log::warn!("input device {n:?} not found, using default");
        }
        found
    });
    let device = named
        .or_else(|| host.default_input_device())
        .ok_or_else(|| anyhow!("no microphone found"))?;
    let supported = device.default_input_config().context("querying microphone config")?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let (rate, channels) = (config.sample_rate.0, config.channels as usize);
    log::info!(
        "recording from {:?} at {rate} Hz, {channels} ch, {format:?} (device lookup {} ms)",
        device.name().ok(),
        t.elapsed().as_millis()
    );

    let stream = match format {
        SampleFormat::F32 => input_stream::<f32>(&device, &config, channels, rate, buffer, levels),
        SampleFormat::I16 => input_stream::<i16>(&device, &config, channels, rate, buffer, levels),
        SampleFormat::U16 => input_stream::<u16>(&device, &config, channels, rate, buffer, levels),
        SampleFormat::I32 => input_stream::<i32>(&device, &config, channels, rate, buffer, levels),
        other => return Err(anyhow!("unsupported sample format {other:?}")),
    }
    .context("opening microphone stream (is microphone access allowed?)")?;
    stream.play().context("starting microphone stream")?;
    log::debug!("microphone stream running {} ms after start", t.elapsed().as_millis());
    Ok((stream, rate))
}

fn input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    rate: u32,
    buffer: Arc<Mutex<Vec<f32>>>,
    levels: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let window = ((rate as f32 * LEVEL_WINDOW_SECS) as usize).max(1);
    let (mut sum_sq, mut count) = (0f32, 0usize);
    let mut mono = Vec::<f32>::new();
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            mono.clear();
            mono.extend(data.chunks(channels.max(1)).map(|frame| {
                frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() / frame.len() as f32
            }));
            for &s in &mono {
                sum_sq += s * s;
                count += 1;
                if count >= window {
                    let level = level_from_rms((sum_sq / count as f32).sqrt());
                    let mut lv = levels.lock().unwrap();
                    lv.push(level);
                    if lv.len() > MAX_LEVELS {
                        let excess = lv.len() - MAX_LEVELS;
                        lv.drain(..excess);
                    }
                    (sum_sq, count) = (0.0, 0);
                }
            }
            buffer.lock().unwrap().extend_from_slice(&mono);
        },
        |e| log::warn!("audio stream error: {e}"),
        None,
    )
}

/// Maps RMS to 0..1 on a dB scale: -60 dBFS → 0, -12 dBFS → 1.
fn level_from_rms(r: f32) -> f32 {
    if r <= 0.0 {
        return 0.0;
    }
    ((20.0 * r.log10() + 60.0) / 48.0).clamp(0.0, 1.0)
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Band-limited resampler: Hann-windowed sinc evaluated at each output position.
/// The cutoff tracks the lower of the two Nyquist rates, so downsampling is anti-aliased.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let step = from as f64 / to as f64;
    // Cutoff in cycles per input sample, a little under the output Nyquist.
    let fc = 0.5 * (to as f64 / from as f64).min(1.0) * 0.92;
    let half = (8.0 / (2.0 * fc)).ceil() as isize; // ~8 zero crossings per side
    let out_len = ((input.len() as f64) / step).floor() as usize;
    let n = input.len() as isize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let t = i as f64 * step;
        let center = t.floor() as isize;
        let (mut acc, mut wsum) = (0f64, 0f64);
        for k in (center - half + 1).max(0)..=(center + half).min(n - 1) {
            let d = t - k as f64;
            let x = 2.0 * fc * d;
            let sinc = if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
            let win = 0.5 + 0.5 * (std::f64::consts::PI * d / half as f64).cos();
            let w = sinc * win;
            acc += input[k as usize] as f64 * w;
            wsum += w;
        }
        out.push(if wsum.abs() > 1e-12 { (acc / wsum) as f32 } else { 0.0 });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(freq: f32, rate: u32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize).map(|i| amp * (2.0 * PI * freq * i as f32 / rate as f32).sin()).collect()
    }

    #[test]
    fn rms_of_known_signals() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5; 100]) - 0.5).abs() < 1e-6);
        let r = rms(&sine(440.0, 16_000, 1.0, 1.0));
        assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3, "{r}");
    }

    #[test]
    fn level_mapping_is_monotonic_and_bounded() {
        assert_eq!(level_from_rms(0.0), 0.0);
        assert_eq!(level_from_rms(1e-5), 0.0);
        assert_eq!(level_from_rms(1.0), 1.0);
        assert!(level_from_rms(0.01) < level_from_rms(0.05));
    }

    #[test]
    fn downsample_preserves_in_band_tone() {
        let out = resample(&sine(440.0, 48_000, 1.0, 0.8), 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        let want = sine(440.0, 16_000, 1.0, 0.8);
        let max_err = out[200..15_800].iter().zip(&want[200..15_800]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(max_err < 0.01, "max_err {max_err}");
    }

    #[test]
    fn downsample_rejects_aliasing_tone() {
        // 12 kHz is above the 8 kHz output Nyquist and must not fold back into the passband.
        let out = resample(&sine(12_000.0, 48_000, 1.0, 0.8), 48_000, 16_000);
        assert!(rms(&out[200..15_800]) < 0.01, "rms {}", rms(&out));
    }

    #[test]
    fn upsample_and_odd_ratio() {
        let out = resample(&sine(300.0, 8_000, 1.0, 0.5), 8_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!((rms(&out[200..15_800]) - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
        let out = resample(&sine(1_000.0, 44_100, 1.0, 0.5), 44_100, 16_000);
        assert_eq!(out.len(), 16_000);
        let want = sine(1_000.0, 16_000, 1.0, 0.5);
        let max_err = out[200..15_800].iter().zip(&want[200..15_800]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(max_err < 0.01, "max_err {max_err}");
    }

    #[test]
    fn identity_and_empty() {
        assert_eq!(resample(&[0.1, 0.2], 16_000, 16_000), vec![0.1, 0.2]);
        assert!(resample(&[], 48_000, 16_000).is_empty());
    }
}
