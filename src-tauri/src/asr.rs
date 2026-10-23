//! Speech-to-text with whisper.cpp (Vulkan). Settings come from the bake-off:
//! language fixed (auto-detect breaks with a trimmed window), audio_ctx = clip length + 5 s.
use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext, WhisperVadContextParams,
    WhisperVadParams,
};

pub struct Asr {
    ctx: WhisperContext,
    /// Running on the graphics card. On CPU we use greedy decoding (about 2x faster than beam search).
    pub gpu: bool,
    /// Silero voice-activity detector, if its model sits next to the speech model.
    vad: Option<Mutex<WhisperVadContext>>,
}

/// Less real speech than this (after the detector) is treated as "nothing said".
const MIN_SPEECH_SECS: f32 = 0.3;

/// Phrases Whisper tends to invent on silence or noise.
const HALLUCINATIONS: &[&str] = &[
    "thanks for watching", "thank you for watching", "subscribe", "زیرنویس", "ممنون که تماشا", "خداحافظ",
    "amara.org", "مشترک شوید",
];

/// Below this Farsi-decode confidence, also try English. Tuned on the bake-off clips:
/// Farsi/mixed speech scored 0.94–0.99, pure English decoded as Farsi scored 0.72–0.80.
const FALLBACK_BELOW: f32 = 0.90;
/// Noise filter (see `transcribe`); tuned on recorded room noise vs. real speech.
const NO_SPEECH_ABOVE: f32 = 0.6;
const KEEP_ABOVE: f32 = 0.5;

impl Asr {
    pub fn load(path: &str, gpu: bool) -> Result<Self> {
        if path.is_empty() || !std::path::Path::new(path).exists() {
            return Err(anyhow!("Speech model isn't downloaded yet — run Setup (Settings → Advanced)"));
        }
        whisper_rs::install_logging_hooks(); // routes whisper.cpp's very chatty log away from stderr
        let mut p = WhisperContextParameters::default();
        p.use_gpu(gpu);
        // Flash attention stays off: on an AMD Radeon 680M it was slower and less accurate.
        p.flash_attn(false);
        let ctx = WhisperContext::new_with_params(path, p).map_err(|e| anyhow!("Could not load speech model: {e:?}"))?;
        let vad_path = crate::setup::vad_path(path);
        let vad = WhisperVadContext::new(&vad_path.to_string_lossy(), WhisperVadContextParams::default()).ok().map(Mutex::new);
        crate::log::log(format!(
            "speech model loaded on {}; voice detector: {}",
            if gpu { "GPU" } else { "CPU" },
            if vad.is_some() { "on" } else { "missing, noise filter weaker" }
        ));
        Ok(Self { ctx, gpu, vad })
    }

    /// For transcribing while you talk: if `audio` contains finished speech followed by a pause of
    /// at least 0.7 s, return the sample index just after that pause, so everything before it can
    /// be transcribed now. None while you're mid-sentence (or there's too little audio yet).
    pub fn pause_cut(&self, audio: &[f32]) -> Option<usize> {
        let vad = self.vad.as_ref()?;
        let rate = crate::audio::RATE as f32;
        if (audio.len() as f32) < rate * 2.5 {
            return None;
        }
        let mut params = WhisperVadParams::new();
        params.set_threshold(0.5);
        params.set_min_speech_duration(200);
        params.set_min_silence_duration(500);
        let segments: Vec<_> = vad.lock().segments_from_samples(params, audio).ok()?.collect();
        let len_s = audio.len() as f32 / rate;
        // Last segment that ended well before "now" (the user paused after it).
        let done = segments.iter().rev().find(|s| len_s - s.end / 100.0 >= 0.7)?;
        let cut = ((done.end / 100.0 + 0.35) * rate) as usize;
        (cut as f32 >= rate * 1.5 && cut < audio.len()).then_some(cut)
    }

    /// Cut the clip down to where someone is actually talking (keeping pauses in between).
    /// None = no real speech, e.g. only room noise, a click, or breathing.
    fn speech_only(&self, audio: &[f32]) -> Option<Vec<f32>> {
        let Some(vad) = &self.vad else { return Some(audio.to_vec()) };
        let mut params = WhisperVadParams::new();
        params.set_threshold(0.5);
        params.set_min_speech_duration(200);
        let segments: Vec<_> = vad.lock().segments_from_samples(params, audio).ok()?.collect();
        let rate = crate::audio::RATE as f32;
        let speech: f32 = segments.iter().map(|s| (s.end - s.start) / 100.0).sum();
        if segments.is_empty() || speech < MIN_SPEECH_SECS {
            crate::log::log(format!("voice detector: no speech ({speech:.2}s)"));
            return None;
        }
        // Timestamps are centiseconds. Only trim long silence: a tight cut measurably hurt
        // accuracy (Whisper uses the quiet lead-in), so keep 0.6 s on both sides.
        let start = (((segments[0].start / 100.0) - 0.6).max(0.0) * rate) as usize;
        let end = ((((segments[segments.len() - 1].end / 100.0) + 0.6) * rate) as usize).min(audio.len());
        Some(audio[start..end.max(start)].to_vec())
    }

    /// Every language means "<language> + English": decode in the chosen language, and if Whisper is
    /// unsure, also as English and keep the more confident one. (Decoding pure English speech as,
    /// say, Farsi gives phonetic nonsense.)
    pub fn transcribe(&self, audio: &[f32], language: &str, vocab: &[String]) -> Result<String> {
        let Some(trimmed) = self.speech_only(audio) else { return Ok(String::new()) };
        let audio = &trimmed[..];
        let best = if language == "en" || language == "auto" {
            self.decode(audio, language, vocab)?
        } else {
            let first = self.decode(audio, language, vocab)?;
            if first.conf >= FALLBACK_BELOW || looping(&first.text) {
                first
            } else {
                let en = self.decode(audio, "en", vocab)?;
                if en.conf > first.conf { en } else { first }
            }
        };
        if looping(&best.text) {
            crate::log::log("dropped as noise: decoder looped");
            return Ok(String::new());
        }
        // Background noise makes Whisper invent short phrases ("I don't know.", "Thank you.").
        // Drop results Whisper itself flags as non-speech, or that it is unsure of in every language.
        if best.no_speech > NO_SPEECH_ABOVE || best.conf < KEEP_ABOVE {
            crate::log::log(format!("dropped as noise: conf {:.2}, no_speech {:.2}: {}", best.conf, best.no_speech, best.text));
            return Ok(String::new());
        }
        Ok(best.text)
    }

    /// Both decodes with their scores, for the self-test.
    pub fn scores(&self, audio: &[f32]) -> Result<[Decoded; 2]> {
        Ok([self.decode(audio, "fa", &[])?, self.decode(audio, "en", &[])?])
    }

    /// One decode: the text, its confidence (mean token probability, 0–1) and Whisper's
    /// probability that the clip holds no speech at all.
    fn decode(&self, audio: &[f32], language: &str, vocab: &[String]) -> Result<Decoded> {
        let secs = audio.len() as f32 / crate::audio::RATE as f32;
        // Greedy decoding: measured on the 12 test clips it was ~2x faster than beam search
        // (avg 1.1 s vs 1.9 s, worst 1.6 s vs 4.9 s) with the same text.
        let strategy = SamplingStrategy::Greedy { best_of: 1 };
        let mut params = FullParams::new(strategy);
        let lang = if language == "auto" { None } else { Some(language) };
        params.set_language(lang);
        params.set_n_threads(8);
        params.set_translate(false);
        params.set_no_context(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        // No temperature fallback: on unclear audio (noise, accents) whisper.cpp otherwise re-decodes
        // up to 5 more times at higher temperatures. That was most of the multi-second waits.
        params.set_temperature_inc(0.0);
        // Hard cap on output length: fast Persian speech makes ~9 tokens a second. A decode stuck in a loop
        // ("... ... ...") would otherwise run to 224 tokens, seconds of GPU time on noise.
        params.set_max_tokens((secs * 12.0) as i32 + 32);
        if lang.is_some() && secs < 25.0 {
            params.set_audio_ctx(((secs * 50.0) as i32 + 250).min(1500));
        }
        let prompt = vocab.join("، ");
        if !prompt.is_empty() {
            params.set_initial_prompt(&prompt);
        }

        // Pad very short clips so the encoder has something to work with.
        let mut buf = audio.to_vec();
        if buf.len() < crate::audio::RATE as usize {
            buf.resize(crate::audio::RATE as usize, 0.0);
        }
        let mut state = self.ctx.create_state().map_err(|e| anyhow!("{e:?}"))?;
        state.full(params, &buf).map_err(|e| anyhow!("Transcription failed: {e:?}"))?;
        let (mut sum, mut n) = (0.0f32, 0u32);
        let mut parts = Vec::new();
        let mut no_speech: f32 = 0.0;
        for (si, seg) in state.as_iter().enumerate() {
            if si == 0 {
                no_speech = seg.no_speech_probability();
            }
            parts.push(seg.to_string());
            for i in 0..seg.n_tokens() {
                if let Some(tok) = seg.get_token(i) {
                    let special = tok.to_str_lossy().map(|s| s.starts_with("[_") || s.starts_with("<|")).unwrap_or(true);
                    if !special {
                        sum += tok.token_probability();
                        n += 1;
                    }
                }
            }
        }
        let conf = if n == 0 { 0.0 } else { sum / n as f32 };
        let text = parts.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
        let text = collapse_repeats(&text);
        let lower = text.to_lowercase();
        if HALLUCINATIONS.iter().any(|h| lower.contains(h)) && text.chars().count() < 60 {
            return Ok(Decoded { text: String::new(), conf: 0.0, no_speech: 1.0 });
        }
        Ok(Decoded { text, conf, no_speech })
    }
}

/// A decode stuck repeating itself ("... ... ...", the same word over and over): noise, not speech.
fn looping(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() >= 8 {
        let mut uniq = words.clone();
        uniq.sort_unstable();
        uniq.dedup();
        if uniq.len() * 4 < words.len() {
            return true;
        }
    }
    // Repeats inside one "word" ("هایتهایتهایت…"): few distinct 3-letter pieces in a long text.
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.len() < 24 {
        return false;
    }
    let mut tri: Vec<&[char]> = chars.windows(3).collect();
    let total = tri.len();
    tri.sort_unstable();
    tri.dedup();
    tri.len() * 4 < total
}

/// Whisper sometimes gets stuck mid-sentence ("هایتهایتهایت…"). Keep one copy of any piece of 2–12
/// characters that repeats 4+ times in a row.
fn collapse_repeats(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::with_capacity(c.len());
    let mut i = 0;
    'next: while i < c.len() {
        for unit in 2..=12 {
            let mut reps = 1;
            while i + (reps + 1) * unit <= c.len() && c[i..i + unit] == c[i + reps * unit..i + (reps + 1) * unit] {
                reps += 1;
            }
            if reps >= 4 {
                out.extend_from_slice(&c[i..i + unit]);
                i += reps * unit;
                continue 'next;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out.into_iter().collect()
}

pub struct Decoded {
    pub text: String,
    pub conf: f32,
    pub no_speech: f32,
}
