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
    pub ts: String,
    pub segments: Arc<Mutex<Vec<Segment>>>,
    /// What the user types in the notepad during the meeting.
    pub my_notes: Mutex<String>,
    running: Arc<AtomicBool>,
    worker: std::thread::JoinHandle<()>,
    live: LiveNotes,
    /// Set when this continues an earlier meeting (Resume): its id and where its clock stopped.
    pub resume_id: Option<i64>,
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

/// Without headphones the mic also hears the other people, and Whisper turns that echo into
/// garbled "You" lines. Drop a "You" line when it repeats what "Them" said, or when they were
/// talking for most of it (people rarely talk over each other for long).
fn is_echo(seg: &Segment, others: &[Segment]) -> bool {
    let them = || others.iter().filter(|o| o.who == "Them");
    let len = (seg.end - seg.start).max(0.1);
    let overlap: f32 = them().map(|o| (seg.end.min(o.end) - seg.start.max(o.start)).max(0.0)).sum();
    if overlap / len >= 0.6 {
        return true;
    }
    let words = |t: &str| t.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).filter(|w| !w.is_empty()).collect::<std::collections::HashSet<_>>();
    let mine = words(&seg.text);
    !mine.is_empty()
        && them().filter(|o| (o.start - seg.start).abs() < 15.0).any(|o| mine.intersection(&words(&o.text)).count() * 2 >= mine.len())
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
        let mut tracks = vec![Track::open("You", &mic_dev, wav("you"), offset)?];
        // The call may play on any output (speakers, headset, dock), so listen to every one of them.
        // Silent ones cost nothing: quiet chunks are never transcribed.
        for (i, out) in audio::output_devices().iter().enumerate() {
            match Track::open("Them", &format!("{}:{out}", audio::SYSTEM), wav(&format!("them{i}")), offset) {
                Ok(t) => tracks.push(t),
                Err(e) => log::log(format!("meeting: can't record '{out}' ({e})")),
            }
        }
        if tracks.len() == 1 {
            match Track::open("Them", audio::SYSTEM, wav("them"), offset) {
                Ok(t) => tracks.push(t),
                Err(e) => log::log(format!("meeting: system audio unavailable ({e}); recording the mic only")),
            }
        }
        let segments = Arc::new(Mutex::new(Vec::<Segment>::new()));
        let running = Arc::new(AtomicBool::new(true));
        let appointment = Arc::new(Mutex::new(crate::smart::Appointment::default()));
        let (segs, run) = (segments.clone(), running.clone());
        let core_notes = core.clone();
        log::log(format!("meeting started ({} tracks)", tracks.len()));
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
            while run.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
                // "Them" first, so the echo check can see what they said.
                for t in tracks.iter_mut().rev() {
                    let new = t.rec.as_ref().map(|r| r.drain()).unwrap_or_default();
                    t.take(new);
                    let segs = t.transcribe(&core, &language, &vocab, false);
                    add(segs, &core);
                }
            }
            // The last words: stop each recorder and transcribe whatever is left.
            for mut t in tracks.into_iter().rev() {
                if let Some(Ok(rest)) = t.rec.take().map(|r| r.stop()) {
                    t.take(rest);
                }
                let segs = t.transcribe(&core, &language, &vocab, true);
                add(segs, &core);
                t.finish();
            }
        });
        let started = Instant::now();
        let (resume_id, prev, mine, old_parts, until0) = match resume {
            Some(r) => (Some(r.id), r.segments, r.my_notes, r.parts, r.until),
            None => (None, Vec::new(), String::new(), Vec::new(), 0.0),
        };
        let (parts, done_until) = (Arc::new(Mutex::new(old_parts)), Arc::new(Mutex::new(until0)));
        let handle = {
            let (segs, run, parts, until, prev) = (segments.clone(), running.clone(), parts.clone(), done_until.clone(), prev.clone());
            let appt_for_notes = appointment.clone();
            std::thread::spawn(move || {
                while run.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_secs(10));
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
        Ok(Self { started, ts: stamp.to_rfc3339(), segments, my_notes: Mutex::new(mine), running, worker, live, resume_id, offset, prev, appointment })
    }

    /// Stops recording; returns the full transcript in time order, and the notes written so far.
    pub fn stop(self) -> (Vec<Segment>, LiveNotes) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.worker.join();
        // A resumed meeting: the earlier transcript plus what was said now.
        let segs: Vec<Segment> = self.prev.iter().cloned().chain(self.segments.lock().iter().cloned()).collect();
        // Live, a "You" line can arrive before the "Them" line it echoes: check again with everything.
        let mut kept: Vec<Segment> = segs.iter().filter(|s| s.who != "You" || !is_echo(s, &segs)).cloned().collect();
        kept.sort_by(|a, b| a.start.total_cmp(&b.start));
        (kept, self.live)
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
    if full.chars().count() <= SINGLE_PASS_CHARS {
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