//! Meetings: you (the mic) and everyone else (what the PC plays) are recorded as two tracks and
//! transcribed while the meeting runs, so the transcript is ready when it ends. Then the local
//! writing AI turns it into notes. Nothing leaves the computer.
use crate::{Core, audio, log};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::Emitter;

#[derive(Serialize, Deserialize, Clone)]
pub struct Segment {
    /// "You" or "Them".
    pub who: String,
    /// Seconds from the start of the meeting.
    pub start: f32,
    #[serde(default)]
    pub end: f32,
    pub text: String,
}

pub struct Meeting {
    pub started: Instant,
    pub segments: Arc<Mutex<Vec<Segment>>>,
    /// What the user types in the notepad during the meeting.
    pub my_notes: Arc<Mutex<String>>,
    running: Arc<AtomicBool>,
    worker: std::thread::JoinHandle<()>,
    /// Writes the meeting to the database every few seconds, so a crash keeps it.
    checkpoint: std::thread::JoinHandle<()>,
    live: LiveNotes,
    /// The meeting's row. It exists from the start (marked live) and is checkpointed.
    pub id: i64,
    /// True when this continues an earlier meeting (Resume).
    pub resumed: bool,
    /// Where the clock starts (a resumed meeting continues from its earlier length).
    pub offset: f32,
    /// The earlier session's transcript (a resumed meeting adds to it).
    prev: Vec<Segment>,
    /// Outlook appointment that covers this call, filled in shortly after the mics open.
    pub appointment: Arc<Mutex<crate::smart::Appointment>>,
}

/// What a resumed meeting continues from.
pub struct Resume {
    pub id: i64,
    pub offset: f32,
    pub segments: Vec<Segment>,
    pub my_notes: String,
    /// Part notes saved with the meeting, and the meeting time they cover.
    pub parts: Vec<String>,
    pub until: f32,
}

/// Notes for long meetings are written in ~10-minute stretches while the meeting is still going,
/// so when it ends only the last stretch and a final merge are left.
pub const WINDOW_SECS: f32 = 600.0;
/// Up to this much transcript (about 20 minutes of talk) the notes are written in one pass.
const SINGLE_PASS_CHARS: usize = 14_000;
/// Persian takes about 1.5x more model tokens per character, so less of it fits in one pass.
const SINGLE_PASS_CHARS_PERSIAN: usize = 10_000;
/// How often a running meeting is saved to the database.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(15);

pub struct LiveNotes {
    parts: Arc<Mutex<Vec<String>>>,
    /// Meeting time (seconds) up to which part notes exist.
    done_until: Arc<Mutex<f32>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// The transcript lines of one stretch of the meeting, without echoes, in time order.
fn window(segs: &[Segment], from: f32, to: f32) -> Vec<Segment> {
    let mut w: Vec<Segment> = segs.iter().filter(|s| s.start >= from && s.start < to && (s.who != "You" || !is_echo(s, segs))).cloned().collect();
    w.sort_by(|a, b| a.start.total_cmp(&b.start));
    w
}

/// Longest piece transcribed at once when nobody pauses (Whisper works on up to 30 s).
const MAX_CHUNK_SECS: f32 = 25.0;

/// One audio source and the part of it that hasn't been transcribed yet.
struct Track {
    who: &'static str,
    rec: Option<audio::Recording>,
    pending: Vec<f32>,
    /// Meeting time (seconds) where `pending` starts.
    pending_start: f32,
    wav: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
}

impl Track {
    /// wav: where to save the audio, or None to keep nothing but the transcript (the default).
    fn open(who: &'static str, device: &str, wav: Option<std::path::PathBuf>, offset: f32) -> anyhow::Result<Self> {
        let rec = audio::start(device)?;
        let spec = hound::WavSpec { channels: 1, sample_rate: audio::RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let wav = wav.and_then(|p| hound::WavWriter::create(p, spec).ok());
        Ok(Self { who, rec: Some(rec), pending: Vec::new(), pending_start: offset, wav })
    }

    fn take(&mut self, new: Vec<f32>) {
        if let Some(w) = self.wav.as_mut() {
            for s in &new {
                let _ = w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16);
            }
        }
        self.pending.extend(new);
    }

    /// Transcribe finished sentences (up to the last pause), or everything when `all`.
    fn transcribe(&mut self, core: &Core, language: &str, vocab: &[String], all: bool) -> Vec<Segment> {
        let rate = audio::RATE as f32;
        let mut out = Vec::new();
        loop {
            let max = (MAX_CHUNK_SECS * rate) as usize;
            let cut = if all && self.pending.len() <= max {
                self.pending.len()
            } else if self.pending.len() > max {
                core.asr.read().as_ref().and_then(|a| a.pause_cut(&self.pending[..max])).unwrap_or(max)
            } else {
                match core.asr.read().as_ref().and_then(|a| a.pause_cut(&self.pending)) {
                    Some(c) => c,
                    None => break,
                }
            };
            if cut == 0 {
                break;
            }
            let chunk: Vec<f32> = self.pending.drain(..cut).collect();
            let start = self.pending_start;
            self.pending_start += cut as f32 / rate;
            if audio::rms(&chunk) > 0.002 {
                let text = core.asr.read().as_ref().map(|a| a.transcribe(&chunk, language, vocab));
                if let Some(Ok(text)) = text {
                    if !text.trim().is_empty() {
                        // Corrections you taught Seda while dictating apply here too.
                        let text = core.store.apply_fixes(&text);
                        out.push(Segment { who: self.who.to_string(), start, end: self.pending_start, text });
                    }
                }
            }
            if self.pending.is_empty() {
                break;
            }
        }
        out
    }

    fn finish(mut self) {
        if let Some(w) = self.wav.take() {
            let _ = w.finalize();
        }
    }
}

/// Backstop for echo the canceller missed (very loud speakers, a laptop mic next to them): a "You"
/// line that repeats what "Them" said a moment earlier. Only clear repeats count, so a short "yes,
/// exactly" said while they talk is kept (the old time-overlap rule dropped those).
fn is_echo(seg: &Segment, others: &[Segment]) -> bool {
    let words = |t: &str| t.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).filter(|w| !w.is_empty()).collect::<std::collections::HashSet<_>>();
    let mine = words(&seg.text);
    mine.len() >= 4
        && others.iter().filter(|o| o.who != seg.who && (o.start - seg.start).abs() < 8.0).any(|o| {
            let shared = mine.intersection(&words(&o.text)).count();
            shared >= 4 && shared * 10 >= mine.len() * 6
        })
}

/// The transcript as it is saved: echoes removed, in time order.
pub fn clean_transcript(segs: &[Segment]) -> Vec<Segment> {
    let mut kept: Vec<Segment> = segs.iter().filter(|s| s.who != "You" || !is_echo(s, segs)).cloned().collect();
    kept.sort_by(|a, b| a.start.total_cmp(&b.start));
    kept
}

/// Echo cancellation for the mic (WebRTC AEC3): what the speakers play is the reference, and its
/// echo is removed from the mic before anything is transcribed. With headphones there is no echo
/// and the mic passes through unchanged.
struct Aec {
    apm: sonora::AudioProcessing,
    /// Speaker audio and mic audio not processed yet (16 kHz).
    render: Vec<f32>,
    capture: Vec<f32>,
    /// Speaker samples to drop before the first one is used: the speaker track opened earlier
    /// than the mic, and both must start at the same moment.
    skip_render: usize,
    /// Mic samples processed without speaker audio (the speaker recorder stalled); that much
    /// speaker audio is dropped when it arrives so the two stay in step.
    debt: usize,
    /// When the canceller's state was last written to the log.
    logged: Instant,
}

/// 10 ms at 16 kHz: the frame size AEC3 works on.
const FRAME: usize = 160;
/// Longest the mic waits for its speaker audio before going on without it (a dead speaker
/// recorder). Normally the speaker audio arrives in the same tick.
const AEC_MAX_WAIT: usize = audio::RATE as usize * 5;
/// Extra speaker audio dropped at the start so the speaker signal always leads the mic: AEC3 finds
/// an echo 0–500 ms after its reference, never before it, and each stream's start-up time is unknown.
const AEC_LEAD: usize = audio::RATE as usize / 8;

impl Aec {
    fn new(skip_render: usize) -> Self {
        let config = sonora::Config { echo_canceller: Some(sonora::config::EchoCanceller::default()), ..Default::default() };
        let apm = sonora::AudioProcessing::builder()
            .config(config)
            .capture_config(sonora::StreamConfig::new(audio::RATE, 1))
            .render_config(sonora::StreamConfig::new(audio::RATE, 1))
            .build();
        Self { apm, render: Vec::new(), capture: Vec::new(), skip_render, debt: 0, logged: Instant::now() }
    }

    /// Once a minute: whether AEC3 found the echo delay (None = not locked on, nothing cancelled).
    fn log_state(&mut self) {
        if self.logged.elapsed() < Duration::from_secs(60) {
            return;
        }
        self.logged = Instant::now();
        let st = self.apm.statistics();
        log::log(format!(
            "meeting: echo canceller delay {:?} ms, echo reduced {:.0?} dB",
            st.delay_ms,
            st.echo_return_loss_enhancement
        ));
    }

    /// New speaker and mic audio in; the mic audio with the echo removed out. `flush` (end of the
    /// meeting) returns every mic sample.
    fn process(&mut self, speakers: &[f32], mic: Vec<f32>, flush: bool) -> Vec<f32> {
        let skip = self.skip_render.min(speakers.len());
        self.skip_render -= skip;
        let speakers = &speakers[skip..];
        let late = self.debt.min(speakers.len());
        self.debt -= late;
        self.render.extend_from_slice(&speakers[late..]);
        self.capture.extend(mic);
        let (mut r, mut c) = (0usize, 0usize);
        let mut out = Vec::with_capacity(self.capture.len());
        let (mut clean, mut sink) = ([0f32; FRAME], [0f32; FRAME]);
        while self.capture.len() - c >= FRAME {
            if self.render.len() - r >= FRAME {
                let _ = self.apm.process_render_f32(&[&self.render[r..r + FRAME]], &mut [&mut sink[..]]);
                r += FRAME;
            } else if self.capture.len() - c < AEC_MAX_WAIT && !flush {
                // The speaker track is filled with silence up to the clock, so its audio for this
                // moment comes with the next tick. Running without it would let the echo through.
                break;
            } else {
                self.debt += FRAME; // the speaker recorder stalled: go on without it
            }
            let frame = &self.capture[c..c + FRAME];
            match self.apm.process_capture_f32(&[frame], &mut [&mut clean[..]]) {
                Ok(()) => out.extend_from_slice(&clean),
                Err(_) => out.extend_from_slice(frame),
            }
            c += FRAME;
        }
        if flush {
            out.extend_from_slice(&self.capture[c..]);
            c = self.capture.len();
        }
        self.capture.drain(..c);
        self.render.drain(..r);
        // Speaker audio far ahead of the mic (should not happen): keep memory bounded.
        let cap = audio::RATE as usize * 10;
        if self.render.len() > cap {
            let extra = self.render.len() - cap;
            self.render.drain(..extra);
        }
        self.log_state();
        out
    }
}

impl Meeting {
    pub fn start(core: Arc<Core>, resume: Option<Resume>) -> anyhow::Result<Self> {
        let offset = resume.as_ref().map_or(0.0, |r| r.offset);
        let stamp = chrono::Local::now();
        let name = stamp.format("%Y%m%d-%H%M%S").to_string();
        let (mic_dev, keep) = {
            let s = core.settings.read();
            (s.input_device.clone(), s.keep_meeting_audio)
        };
        // Audio files only when the user asked to keep recordings; normally only the transcript stays.
        let dir = crate::settings::data_dir().join("meetings");
        if keep {
            std::fs::create_dir_all(&dir)?;
        }
        let wav = |suffix: &str| keep.then(|| dir.join(format!("{name}-{suffix}.wav")));
        // The call may play on any output (speakers, headset, dock), so listen to every one of them.
        // Silent ones cost nothing: quiet chunks are never transcribed. The default speakers go
        // first: they are the echo reference and must be recording before the mic starts.
        let default_out = audio::default_output_name();
        let mut outs: Vec<String> = audio::output_devices().into_iter().map(|o| format!("{}:{o}", audio::SYSTEM)).collect();
        let is_default = |dev: &String| default_out.as_deref().is_some_and(|d| dev.strip_prefix(audio::SYSTEM).and_then(|r| r.strip_prefix(':')) == Some(d));
        match outs.iter().position(is_default) {
            Some(p) => {
                let d = outs.remove(p);
                outs.insert(0, d);
            }
            // The default speakers by name aren't in the list: record "@system" (the default) only
            // when there is nothing else, or it records the same speakers twice.
            None if outs.is_empty() || default_out.is_none() => outs.insert(0, audio::SYSTEM.to_string()),
            None => {}
        }
        let open_them = |i: usize, dev: &str| match Track::open("Them", dev, wav(&format!("them{i}")), offset) {
            Ok(t) => Some(t),
            Err(e) => {
                log::log(format!("meeting: can't record '{dev}' ({e})"));
                None
            }
        };
        let mut them: Vec<Track> = Vec::new();
        let mut reference: Option<usize> = None;
        // The first one is the echo reference only when it is the default speakers.
        let first_is_default = outs.first().is_some_and(|d| d == audio::SYSTEM || is_default(d));
        let mut opened = 0;
        if first_is_default {
            if let Some(t) = open_them(0, &outs[0]) {
                them.push(t);
                reference = Some(0);
            }
            opened = 1;
        }
        let mic = Track::open("You", &mic_dev, wav("you"), offset)?;
        for (i, dev) in outs.iter().enumerate().skip(opened) {
            if let Some(t) = open_them(i, dev) {
                them.push(t);
            }
        }
        if them.is_empty() {
            log::log("meeting: system audio unavailable; recording the mic only");
        }
        // The speaker track started a moment before the mic: that much of it has no mic audio.
        let skip_render = match (reference, mic.rec.as_ref()) {
            (Some(i), Some(m)) => them[i]
                .rec
                .as_ref()
                .map_or(0, |r| (m.started.saturating_duration_since(r.started).as_secs_f64() * audio::RATE as f64) as usize),
            _ => 0,
        };
        let reference_name = reference.map(|_| default_out.clone().unwrap_or_else(|| "default speakers".into()));

        // The row exists from now on, so nothing said is lost if Seda or Windows stops unexpectedly.
        let (id, resumed) = match &resume {
            Some(r) => {
                core.store.set_meeting_live(r.id, true);
                (r.id, true)
            }
            None => (core.store.begin_meeting(&stamp.to_rfc3339()), false),
        };

        let segments = Arc::new(Mutex::new(Vec::<Segment>::new()));
        let running = Arc::new(AtomicBool::new(true));
        let appointment = Arc::new(Mutex::new(crate::smart::Appointment::default()));
        let (segs, run) = (segments.clone(), running.clone());
        let core_notes = core.clone();
        log::log(format!("meeting started ({} tracks)", them.len() + 1));
        {
            let (appt, named, core_ol) = (appointment.clone(), segments.clone(), core.clone());
            let (user_name, email) = {
                let s = core.settings.read();
                (s.user_name.clone(), s.email.clone())
            };
            std::thread::spawn(move || {
                let Some(found) = crate::outlook::current(&user_name, &email) else { return };
                *appt.lock() = found.clone();
                {
                    let mut lines = named.lock();
                    label_segments(&mut lines, &found.people);
                }
                // Saved now, so a recovered meeting keeps its name and people too.
                core_ol.store.set_meeting_meta(id, &found.people.join(" | "), &found.subject);
                let who = if found.people.len() == 1 { crate::smart::display_name(&found.people[0]) } else { String::new() };
                log::log(format!("meeting named from Outlook ({} other people)", found.people.len()));
                let _ = core_ol.app.emit(
                    "meeting-named",
                    serde_json::json!({ "title": found.subject, "people": found.people.join(", "), "who": who }),
                );
            });
        }
        let appt_for_lines = appointment.clone();
        let worker = std::thread::spawn(move || {
            let (language, vocab) = (core.settings.read().language.clone(), core.vocab());
            let (mut mic, them) = (mic, them);
            let mut aec = reference.map(|_| Aec::new(skip_render + AEC_LEAD));
            if let Some(dev) = &reference_name {
                log::log(format!("meeting: echo cancellation on (reference: {dev})"));
            }
            let add = |new: Vec<Segment>, core: &Core| {
                let people = appt_for_lines.lock().people.clone();
                let mut all = segs.lock();
                for mut s in new {
                    if s.who == "You" && is_echo(&s, &all) {
                        continue;
                    }
                    s.who = crate::smart::label_who(&s.who, &people);
                    let _ = core.app.emit("meeting-segment", &s);
                    all.push(s);
                }
            };
            let mut them = them;
            while run.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
                // Take every track's new audio at the same moment, before any transcribing (which
                // takes seconds): the echo canceller needs the speakers' audio for the same time
                // as the mic's.
                let fresh: Vec<Vec<f32>> = them.iter().map(|t| t.rec.as_ref().map(|r| r.drain()).unwrap_or_default()).collect();
                let new = mic.rec.as_ref().map(|r| r.drain()).unwrap_or_default();
                let new = match (aec.as_mut(), reference) {
                    (Some(a), Some(i)) => a.process(&fresh[i], new, false),
                    _ => new,
                };
                // "Them" first, so the echo check can see what they said.
                for (t, f) in them.iter_mut().zip(fresh) {
                    t.take(f);
                    let segs = t.transcribe(&core, &language, &vocab, false);
                    add(segs, &core);
                }
                mic.take(new);
                let segs = mic.transcribe(&core, &language, &vocab, false);
                add(segs, &core);
            }
            // The last words: stop each recorder and transcribe whatever is left.
            let mut speakers = Vec::new();
            for (i, mut t) in them.into_iter().enumerate() {
                if let Some(Ok(rest)) = t.rec.take().map(|r| r.stop()) {
                    if Some(i) == reference {
                        speakers = rest.clone();
                    }
                    t.take(rest);
                }
                let segs = t.transcribe(&core, &language, &vocab, true);
                add(segs, &core);
                t.finish();
            }
            let rest = mic.rec.take().map(|r| r.stop()).and_then(Result::ok).unwrap_or_default();
            let rest = match aec.as_mut() {
                Some(a) => a.process(&speakers, rest, true),
                None => rest,
            };
            mic.take(rest);
            let segs = mic.transcribe(&core, &language, &vocab, true);
            add(segs, &core);
            mic.finish();
        });
        let started = Instant::now();
        let (prev, mine, old_parts, until0) = match resume {
            Some(r) => (r.segments, r.my_notes, r.parts, r.until),
            None => (Vec::new(), String::new(), Vec::new(), 0.0),
        };
        let my_notes = Arc::new(Mutex::new(mine));
        let (parts, done_until) = (Arc::new(Mutex::new(old_parts)), Arc::new(Mutex::new(until0)));
        let checkpoint = {
            let (core, segs, run, parts, until, prev, mine) =
                (core_notes.clone(), segments.clone(), running.clone(), parts.clone(), done_until.clone(), prev.clone(), my_notes.clone());
            std::thread::spawn(move || {
                let mut last = Instant::now();
                while run.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(250));
                    if last.elapsed() < CHECKPOINT_EVERY {
                        continue;
                    }
                    last = Instant::now();
                    let all: Vec<Segment> = prev.iter().cloned().chain(segs.lock().iter().cloned()).collect();
                    let transcript = serde_json::to_string(&clean_transcript(&all)).unwrap_or_default();
                    let parts = serde_json::json!({ "parts": *parts.lock(), "until": *until.lock() }).to_string();
                    let secs = offset as f64 + started.elapsed().as_secs_f64();
                    let notepad = mine.lock().clone();
                    core.store.checkpoint_meeting(id, secs, &transcript, &notepad, &parts);
                }
            })
        };
        let handle = {
            let (segs, run, parts, until, prev) = (segments.clone(), running.clone(), parts.clone(), done_until.clone(), prev.clone());
            let appt_for_notes = appointment.clone();
            std::thread::spawn(move || {
                while run.load(Ordering::SeqCst) {
                    // Short sleeps: Stop waits for this thread.
                    std::thread::sleep(Duration::from_secs(2));
                    let from = *until.lock();
                    // A minute of slack so the stretch's last sentences are transcribed first.
                    if offset + started.elapsed().as_secs_f32() < from + WINDOW_SECS + 60.0 || !run.load(Ordering::SeqCst) {
                        continue;
                    }
                    // Earlier session (if resumed) + what's been said now: stretches not covered yet get notes.
                    let all: Vec<Segment> = prev.iter().cloned().chain(segs.lock().iter().cloned()).collect();
                    let chunk = window(&all, from, from + WINDOW_SECS);
                    if !chunk.is_empty() {
                        let t = Instant::now();
                        let user = core_notes.settings.read().user_name.clone();
                        let others = appt_for_notes.lock().people.clone();
                        let body = with_people(&transcript_text(&chunk), &others);
                        let note = core_notes.llm.lock().as_mut().map(|l| l.meeting_part(&body, main_language(&chunk), &user));
                        match note {
                            Some(Ok(n)) => {
                                log::log(format!("meeting notes: part {} written during the meeting in {:.0}s", parts.lock().len() + 1, t.elapsed().as_secs_f32()));
                                parts.lock().push(n);
                            }
                            Some(Err(e)) => {
                                log::log(format!("meeting notes: part failed ({e}); will redo it at the end"));
                                continue;
                            }
                            None => continue,
                        }
                    }
                    *until.lock() = from + WINDOW_SECS;
                }
            })
        };
        let live = LiveNotes { parts, done_until, handle: Some(handle) };
        Ok(Self { started, segments, my_notes, running, worker, checkpoint, live, id, resumed, offset, prev, appointment })
    }

    /// Stops recording; returns the full transcript in time order, and the notes written so far.
    pub fn stop(self) -> (Vec<Segment>, LiveNotes) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.worker.join();
        // The final save comes after this, so an older checkpoint must not land on top of it.
        let _ = self.checkpoint.join();
        // A resumed meeting: the earlier transcript plus what was said now.
        let segs: Vec<Segment> = self.prev.iter().cloned().chain(self.segments.lock().iter().cloned()).collect();
        // Live, a "You" line can arrive before the "Them" line it echoes: check again with everything.
        (clean_transcript(&segs), self.live)
    }
}

fn label_segments(segments: &mut [Segment], people: &[String]) {
    if people.len() != 1 {
        return;
    }
    for seg in segments {
        seg.who = crate::smart::label_who(&seg.who, people);
    }
}

fn with_people(transcript: &str, others: &[String]) -> String {
    let header = crate::smart::people_header(others);
    if header.is_empty() { transcript.to_string() } else { format!("{header}{transcript}") }
}

/// The finished notes: short meetings in one pass; long ones from the part notes written during the
/// meeting (live) plus notes for whatever is left, merged. Without live (Rewrite) every part is
/// written now.
/// Returns the notes and the part notes behind them (with the meeting time they cover), which are
/// saved so a resumed meeting can reuse them.
pub fn write_notes(core: &Core, segs: &[Segment], live: Option<LiveNotes>, my_notes: &str, others: &[String]) -> anyhow::Result<(String, Vec<String>, f32)> {
    let (mut parts, from) = match live {
        Some(mut l) => {
            if let Some(h) = l.handle.take() {
                let _ = h.join(); // a part may still be in progress
            }
            let p = l.parts.lock().clone();
            (p, *l.done_until.lock())
        }
        None => (Vec::new(), 0.0),
    };
    let user = core.settings.read().user_name.clone();
    let language = main_language(segs);
    let end = segs.iter().map(|s| s.end.max(s.start)).fold(0.0f32, f32::max);
    let minutes = (end / 60.0).round() as u32;
    let mut guard = core.llm.lock();
    let llm = guard.as_mut().ok_or_else(|| anyhow::anyhow!("the writing AI isn't running"))?;
    let full = with_people(&transcript_text(segs), others);
    // One note from the whole call, including a resume. Saved part notes are only for a long call
    // that does not fit in one pass. A short resume must not keep the thin notes from the first half.
    let single = if language == Some("Persian") { SINGLE_PASS_CHARS_PERSIAN } else { SINGLE_PASS_CHARS };
    if full.chars().count() <= single {
        return Ok((llm.meeting_single(&full, language, my_notes, &user, minutes)?, Vec::new(), 0.0));
    }
    let mut a = from;
    while a <= end {
        let chunk = window(segs, a, a + WINDOW_SECS);
        if !chunk.is_empty() {
            log::log(format!("meeting notes: part {}", parts.len() + 1));
            parts.push(llm.meeting_part(&with_people(&transcript_text(&chunk), others), language, &user)?);
        }
        a += WINDOW_SECS;
    }
    log::log(format!("meeting notes: merging {} parts", parts.len()));
    let notes = llm.meeting_merge(&parts, language, my_notes, &user, minutes)?;
    // Covered up to where the talk actually ended, so a resumed meeting's new words start a new stretch.
    Ok((notes, parts, end))
}

/// Test builds: the part notes a real meeting would have written while it was going (every full
/// 10-minute stretch except the last), so the end-of-meeting wait can be measured on its own.
#[allow(dead_code)] // used by test builds only
pub fn simulate_live(core: &Core, segs: &[Segment]) -> LiveNotes {
    let end = segs.iter().map(|s| s.end.max(s.start)).fold(0.0f32, f32::max);
    let (mut parts, mut from) = (Vec::new(), 0.0f32);
    while from + WINDOW_SECS + 60.0 <= end {
        let chunk = window(segs, from, from + WINDOW_SECS);
        if let Some(Ok(n)) = core.llm.lock().as_mut().map(|l| l.meeting_part(&transcript_text(&chunk), main_language(&chunk), "")) {
            parts.push(n);
        }
        from += WINDOW_SECS;
    }
    LiveNotes { parts: Arc::new(Mutex::new(parts)), done_until: Arc::new(Mutex::new(from)), handle: None }
}

impl LiveNotes {
    /// Part notes saved with a meeting (a recovered one), so they aren't written again.
    pub fn from_saved(parts: Vec<String>, until: f32) -> Self {
        Self { parts: Arc::new(Mutex::new(parts)), done_until: Arc::new(Mutex::new(until)), handle: None }
    }
}

/// "[03:12] Them: …" lines, the form the AI gets.
pub fn transcript_text(segs: &[Segment]) -> String {
    segs.iter()
        .map(|s| format!("[{:02}:{:02}] {}: {}", s.start as u32 / 60, s.start as u32 % 60, s.who, s.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Persian when most letters are in Persian/Arabic script. Otherwise None: the model picks the
/// language itself (it does that well for Latin-script meetings).
pub fn main_language(segs: &[Segment]) -> Option<&'static str> {
    let (mut arabic, mut latin) = (0usize, 0usize);
    for c in segs.iter().flat_map(|s| s.text.chars()) {
        if ('\u{0600}'..='\u{06FF}').contains(&c) {
            arabic += 1;
        } else if c.is_ascii_alphabetic() {
            latin += 1;
        }
    }
    (arabic > latin).then_some("Persian")
}
/// What the Ask box sends the AI: the notes plus the transcript. A long transcript doesn't fit,
/// so then only the stretches that share the most words with the question go in (in time order).
pub fn ask_context(title: &str, notes: &str, segs: &[Segment], question: &str) -> String {
    const BUDGET: usize = 9_000; // characters of transcript
    let lines: Vec<String> = transcript_text(segs).lines().map(str::to_string).collect();
    let total: usize = lines.iter().map(|l| l.chars().count() + 1).sum();
    let picked = if total <= BUDGET {
        lines.join("\n")
    } else {
        let words = |t: &str| t.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).filter(|w| w.chars().count() > 2).collect::<std::collections::HashSet<_>>();
        let q = words(question);
        // Windows of 12 lines, scored by shared words.
        let mut windows: Vec<(usize, usize)> = (0..lines.len()).step_by(12).map(|i| {
            let w = words(&lines[i..(i + 12).min(lines.len())].join(" "));
            (i, q.intersection(&w).count())
        }).collect();
        windows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut keep = Vec::new();
        let mut used = 0;
        for (i, _) in windows {
            let chunk = lines[i..(i + 12).min(lines.len())].join("\n");
            if used + chunk.chars().count() > BUDGET {
                continue;
            }
            used += chunk.chars().count();
            keep.push((i, chunk));
        }
        keep.sort();
        keep.into_iter().map(|(_, c)| c).collect::<Vec<_>>().join("\n…\n")
    };
    format!("# Meeting notes: {title}\n{notes}\n\n# Transcript\n{picked}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech-like test signal: noise shaped into syllables (4 Hz on/off), deterministic.
    fn babble(seed: u32, secs: f32) -> Vec<f32> {
        let mut x = seed;
        let n = (secs * audio::RATE as f32) as usize;
        let mut prev = 0.0f32;
        (0..n)
            .map(|i| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = (x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                prev = 0.6 * prev + 0.4 * white; // a little low-pass, like a voice
                let env = ((i as f32 / audio::RATE as f32) * std::f32::consts::TAU * 4.0).sin().max(0.0);
                prev * env * 0.5
            })
            .collect()
    }

    fn energy(s: &[f32]) -> f32 {
        s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32
    }

    /// The far end plays for 8 s (the mic hears only its echo), then the user talks alone for 3 s.
    /// The echo must drop a lot once AEC3 has converged, and the user's own speech must pass.
    #[test]
    fn aec_removes_echo_and_keeps_the_user() {
        let rate = audio::RATE as usize;
        let lead = rate / 10; // the speaker track opened 100 ms before the mic
        let far: Vec<f32> = [babble(7, 8.0), vec![0.0; rate * 3]].concat();
        let near: Vec<f32> = [vec![0.0; rate * 8], babble(99, 3.0)].concat();
        // Echo: 60 ms later, quieter, a bit smeared (room + speaker).
        let delay = rate * 60 / 1000;
        let mic: Vec<f32> = (0..far.len())
            .map(|i| {
                let e = |k: usize| if i >= delay + k { far[i - delay - k] } else { 0.0 };
                0.45 * e(0) + 0.15 * e(1) + 0.05 * e(2) + near[i]
            })
            .collect();
        let speakers: Vec<f32> = [vec![0.3; lead], far.clone()].concat(); // the lead is dropped

        // Like the meeting worker: about a second of each per tick, not exactly equal.
        let out = run(Aec::new(lead + AEC_LEAD), &speakers, &mic, |_| (rate + 37, rate));
        check(&mic, &near, &out, "balanced ticks");
        // A tick after a long transcription: 4 s of both at once.
        let out = run(Aec::new(lead + AEC_LEAD), &speakers, &mic, |i| if i % 3 == 2 { (rate * 4, rate * 4) } else { (rate, rate) });
        check(&mic, &near, &out, "uneven ticks");
    }

    fn run(mut aec: Aec, speakers: &[f32], mic: &[f32], tick: impl Fn(usize) -> (usize, usize)) -> Vec<f32> {
        let mut out = Vec::new();
        let (mut s, mut m, mut i) = (0, 0, 0);
        while m < mic.len() {
            let (ds, dm) = tick(i);
            let sn = (s + ds).min(speakers.len());
            let mn = (m + dm).min(mic.len());
            out.extend(aec.process(&speakers[s..sn], mic[m..mn].to_vec(), false));
            (s, m, i) = (sn, mn, i + 1);
        }
        out.extend(aec.process(&[], Vec::new(), true));
        assert_eq!(out.len(), mic.len(), "every mic sample comes back");
        out
    }

    fn check(mic: &[f32], near: &[f32], out: &[f32], what: &str) {
        let rate = audio::RATE as usize;

        let (a, b) = (rate * 4, rate * 8); // echo only, after convergence
        let erle = 10.0 * (energy(&mic[a..b]) / energy(&out[a..b]).max(1e-12)).log10();
        let kept = energy(&out[rate * 9..rate * 11]) / energy(&near[rate * 9..rate * 11]);
        eprintln!("{what}: echo reduced by {erle:.1} dB; user speech kept at {:.0}% energy", kept * 100.0);
        assert!(erle > 15.0, "{what}: echo only reduced by {erle:.1} dB");
        assert!(kept > 0.5, "{what}: user speech dropped to {:.0}% energy", kept * 100.0);
    }

    #[test]
    fn echo_backstop_keeps_short_interjections() {
        let them = Segment { who: "Them".into(), start: 10.0, end: 16.0, text: "we should move the release to Thursday then".into() };
        let yes = Segment { who: "You".into(), start: 12.0, end: 13.0, text: "yes exactly".into() };
        let echo = Segment { who: "You".into(), start: 10.5, end: 16.0, text: "move the release to Thursday".into() };
        let all = vec![them, yes.clone(), echo.clone()];
        assert!(!is_echo(&yes, &all));
        assert!(is_echo(&echo, &all));
        assert_eq!(clean_transcript(&all).len(), 2);
    }
}
