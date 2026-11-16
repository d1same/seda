//! Microphone and system-audio capture. The mic is opened only while recording so Windows' "mic in use"
//! indicator is not permanently on. Output is 16 kHz mono f32, the format Whisper expects.
use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, bounded};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

pub const RATE: u32 = 16_000;
/// Device name meaning "what this PC is playing" (loopback of the default speakers): the other
/// people in a call.
pub const SYSTEM: &str = "@system";

pub struct Recording {
    stop_tx: Sender<()>,
    done_rx: Receiver<Result<Vec<f32>>>,
    level: Arc<AtomicU32>,
    /// Captured audio so far at the device rate, shared so it can be transcribed while you talk.
    buf: Arc<Mutex<Buf>>,
    rate: u32,
    loopback: bool,
    /// When the stream started: sample 0 of this recording. Lines a loopback track up with the mic.
    pub started: Instant,
}

/// How far a loopback track may fall behind the clock before the gap is filled with silence. The
/// same in the audio callback and in `drain`, so audio after a pause lands where it was played.
#[allow(non_snake_case)]
fn GAP_SLACK(rate: u32) -> usize {
    rate as usize / 20
}

/// The recorder's buffer and how many device-rate samples it has produced in total.
struct Buf {
    data: Vec<f32>,
    written: usize,
}

impl Buf {
    /// Loopback only: Windows sends nothing while nothing plays. Count that time as silence (up to
    /// 50 ms ago, so audio still on its way isn't overtaken) so the track keeps time with the clock.
    fn fill_gap(&mut self, started: Instant, rate: u32) {
        let expected = (started.elapsed().as_secs_f64() * rate as f64) as usize;
        let gap = expected.saturating_sub(self.written);
        if gap > GAP_SLACK(rate) {
            let pad = gap - GAP_SLACK(rate);
            let n = self.data.len();
            self.data.resize(n + pad, 0.0);
            self.written += pad;
        }
    }
}

impl Recording {
    /// Latest input level, roughly 0.0–1.0.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Takes everything recorded since the last call, as 16 kHz mono (recording continues).
    /// For long recordings (meetings), so the buffer never grows.
    pub fn drain(&self) -> Vec<f32> {
        let raw = {
            let mut b = self.buf.lock();
            if self.loopback {
                b.fill_gap(self.started, self.rate);
            }
            // Only whole output samples: the device samples left over stay for the next drain, so
            // nothing is lost between drains and the mic and speaker tracks never drift apart.
            let step = self.rate as f64 / RATE as f64;
            let out = (b.data.len() as f64 / step) as usize;
            let take = ((out as f64 * step).ceil() as usize).min(b.data.len());
            b.data.drain(..take).collect::<Vec<f32>>()
        };
        resample(&raw, self.rate, RATE)
    }

    pub fn stop(self) -> Result<Vec<f32>> {
        let _ = self.stop_tx.send(());
        self.done_rx.recv().map_err(|_| anyhow!("recorder thread died"))?
    }
}

/// Names of the microphones Windows offers (for the picker in Settings / setup).
pub fn input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|ds| ds.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// The speakers Windows plays to by default (what a call usually comes out of).
pub fn default_output_name() -> Option<String> {
    cpal::default_host().default_output_device().and_then(|d| d.name().ok())
}

/// Names of the speakers/headsets Windows offers (all of them are recorded in a meeting).
/// Each name once: several monitors can report the same name, and opening by name would just
/// record that first device again.
pub fn output_devices() -> Vec<String> {
    let mut names: Vec<String> = cpal::default_host()
        .output_devices()
        .map(|ds| ds.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    names
}

/// The microphone to use: the one named in settings if it's still connected, else Windows' default.
fn pick_device(name: &str) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if !name.is_empty() {
        if let Ok(mut ds) = host.input_devices() {
            if let Some(d) = ds.find(|d| d.name().map(|n| n == name).unwrap_or(false)) {
                return Ok(d);
            }
        }
        crate::log::log(format!("microphone '{name}' not found, using the Windows default"));
    }
    host.default_input_device().context("No microphone found")
}

/// Why a recording came out silent, in words the user can act on. None = it's fine.
pub fn silence_reason(samples: &[f32]) -> Option<&'static str> {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak == 0.0 {
        // Exactly zero: Windows hands apps silence when "Let desktop apps access your microphone" is off.
        Some("Windows is blocking the mic: Settings → Privacy → Microphone → allow desktop apps")
    } else if rms(samples) < 0.003 {
        Some("Mic is almost silent — pick another microphone in Seda's Settings")
    } else {
        None
    }
}

pub fn start(device: &str) -> Result<Recording> {
    let device = device.to_string();
    let (stop_tx, stop_rx) = bounded::<()>(1);
    let (done_tx, done_rx) = bounded(1);
    let (ready_tx, ready_rx) = bounded::<Result<(u32, Instant)>>(1);
    let level = Arc::new(AtomicU32::new(0));
    let lvl = level.clone();
    let buf = Arc::new(Mutex::new(Buf { data: Vec::with_capacity(48_000 * 30), written: 0 }));
    let loopback = device.starts_with(SYSTEM);
    let shared = buf.clone();

    std::thread::spawn(move || {
        let built = build_stream(&device, buf.clone(), lvl);
        let (stream, rate, started) = match built {
            Ok(v) => v,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        let _ = ready_tx.send(Ok((rate, started)));
        let _ = stop_rx.recv();
        drop(stream);
        let samples = std::mem::take(&mut buf.lock().data);
        let _ = done_tx.send(Ok(resample(&samples, rate, RATE)));
    });

    let (rate, started) = ready_rx.recv().map_err(|_| anyhow!("recorder thread died"))??;
    Ok(Recording { stop_tx, done_rx, level, buf: shared, rate, loopback, started })
}

fn build_stream(name: &str, buf: Arc<Mutex<Buf>>, level: Arc<AtomicU32>) -> Result<(cpal::Stream, u32, Instant)> {
    // "@system" = the default speakers; "@system:<name>" = that output device (headset, dock, …).
    let loopback = name.starts_with(SYSTEM);
    let device = if loopback {
        let host = cpal::default_host();
        match name.strip_prefix(SYSTEM).and_then(|r| r.strip_prefix(':')) {
            Some(n) => host.output_devices()?.find(|d| d.name().map(|x| x == n).unwrap_or(false)).context("Output device not found")?,
            None => host.default_output_device().context("No speakers found")?,
        }
    } else {
        pick_device(name)?
    };
    let config = if loopback { device.default_output_config() } else { device.default_input_config() }
        .context("Audio device config unavailable")?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate().0;
    crate::log::log(format!(
        "{}: {} ({rate} Hz, {channels} ch, {:?})",
        if loopback { "system audio" } else { "mic" },
        device.name().unwrap_or_default(),
        config.sample_format()
    ));
    let err = |e| eprintln!("audio stream error: {e}");

    // Windows sends loopback audio only while something is playing. Fill the gaps with silence so
    // the track stays in step with the clock (and with the mic track).
    let started = Instant::now();
    let push = move |mono: Vec<f32>| {
        let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len().max(1) as f32).sqrt();
        level.store((rms * 6.0).min(1.0).to_bits(), Ordering::Relaxed);
        let mut b = buf.lock();
        if loopback {
            let expected = (started.elapsed().as_secs_f64() * rate as f64) as usize;
            let gap = expected.saturating_sub(b.written + mono.len());
            if gap > GAP_SLACK(rate) {
                let n = b.data.len();
                b.data.resize(n + gap, 0.0);
                b.written += gap;
            }
        }
        b.written += mono.len();
        b.data.extend_from_slice(&mono);
    };
    let to_mono = move |frame_iter: &mut dyn Iterator<Item = f32>, len: usize| -> Vec<f32> {
        let mut out = Vec::with_capacity(len / channels);
        let mut acc = 0.0;
        for (i, s) in frame_iter.enumerate() {
            acc += s;
            if (i + 1) % channels == 0 {
                out.push(acc / channels as f32);
                acc = 0.0;
            }
        }
        out
    };

    let cfg: cpal::StreamConfig = config.clone().into();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &cfg,
            move |d: &[f32], _: &_| push(to_mono(&mut d.iter().copied(), d.len())),
            err,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &cfg,
            move |d: &[i16], _: &_| push(to_mono(&mut d.iter().map(|s| *s as f32 / 32768.0), d.len())),
            err,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            &cfg,
            move |d: &[u16], _: &_| push(to_mono(&mut d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0), d.len())),
            err,
            None,
        )?,
        f => return Err(anyhow!("Unsupported microphone format {f:?}")),
    };
    stream.play()?;
    Ok((stream, rate, started))
}

/// Read any WAV file as 16 kHz mono f32.
pub fn load_wav(path: &str) -> Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().filter_map(Result::ok).map(|s| s as f32 / scale).collect()
        }
    };
    let mono: Vec<f32> = raw.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    Ok(resample(&mono, spec.sample_rate, RATE))
}

pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = (input.len() as f64 / ratio) as usize;
    // Average over the source span of each output sample (cheap anti-aliasing for downsampling).
    (0..n)
        .map(|i| {
            let start = (i as f64 * ratio) as usize;
            let end = (((i + 1) as f64 * ratio) as usize).clamp(start + 1, input.len());
            input[start..end].iter().sum::<f32>() / (end - start) as f32
        })
        .collect()
}

pub fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

pub fn save_wav(path: &std::path::Path, samples: &[f32]) -> Result<()> {
    let spec = hound::WavSpec { channels: 1, sample_rate: RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec)?;
    for s in samples {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)?;
    }
    w.finalize()?;
    Ok(())
}
