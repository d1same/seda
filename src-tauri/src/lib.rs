mod asr;
mod audio;
mod detect;
mod hotkey;
mod llm;
mod meeting;
mod settings;
mod store;
mod log;
mod setup;
mod update;
mod win;

use hotkey::HotkeyEvent;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use settings::{Binding, Settings};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use store::Store;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WindowEvent};

#[derive(Serialize, Clone, Default)]
pub struct Status {
    asr: String, // "loading" | "ready" | "error" | "off"
    asr_error: Option<String>,
    llm: String,
    llm_error: Option<String>,
}

#[derive(Serialize, Clone)]
struct PillState {
    state: &'static str, // listening | transcribing | polishing | done | empty | error | cancelled | idle | hidden
    app: String,
    message: String,
    /// "dot" = the round button (click to dictate, drag to move). "pill" = the status capsule.
    shape: &'static str,
}

struct ActiveRec {
    rec: audio::Recording,
    started: Instant,
    exe: String,
    title: String,
    /// The window in front when recording started (where the text will be pasted).
    target: isize,
    live: Arc<Mutex<LiveText>>,
}

/// Text transcribed while you were still talking: everything up to sample `done` (16 kHz).
#[derive(Default)]
struct LiveText {
    done: usize,
    parts: Vec<String>,
}

struct FixSession {
    hwnd: isize,
    original: String,
}

/// Call detection (see detect.rs and watch_calls).
#[derive(Default)]
struct CallState {
    /// The call app the pop-up is asking about.
    asking: Option<&'static str>,
    /// Calls the user said "Not now" to, or stopped recording by hand: not offered again until they end.
    dismissed: Vec<&'static str>,
    /// The meeting was started for this call, so it stops by itself when the call ends.
    linked: Option<&'static str>,
    gone_since: Option<Instant>,
}

/// Pop-up sizes (logical px): working capsule, capsule with a message, idle circle.
const PILL: (f64, f64) = (148.0, 30.0);
const PILL_WIDE: (f64, f64) = (300.0, 30.0);
const PILL_IDLE: (f64, f64) = (48.0, 48.0);

/// The idle circle is a small square. The status capsule is a wide short bar, so a late clip
/// can tell them apart and will not turn the capsule into a disc.
fn small_square(width: i32, height: i32) -> bool {
    width >= 8 && height >= 8 && width < 220 && (width - height).abs() <= 4
}

pub struct Core {
    app: AppHandle,
    settings: RwLock<Settings>,
    store: Store,
    asr: RwLock<Option<asr::Asr>>,
    llm: Mutex<Option<llm::Llm>>,
    status: Mutex<Status>,
    rec: Mutex<Option<ActiveRec>>,
    recording: Arc<AtomicBool>,
    pill_gen: AtomicU64,
    /// This dictation was started from the on-screen circle, so the circle stays put and a second click stops it.
    from_dot: AtomicBool,
    fix: Mutex<Option<FixSession>>,
    mic_test: Mutex<Option<audio::Recording>>,
    meeting: Mutex<Option<meeting::Meeting>>,
    call: Mutex<CallState>,
}

type CoreState<'a> = State<'a, Arc<Core>>;

impl Core {
    /// Personal words to spell right: the user's list plus their own name.
    fn vocab(&self) -> Vec<String> {
        let mut v = self.store.vocab();
        let name = self.settings.read().user_name.trim().to_string();
        if !name.is_empty() && !v.contains(&name) {
            v.push(name);
        }
        v
    }

    fn emit_status(&self) {
        let s = self.status.lock().clone();
        log::log(format!("status {}", serde_json::to_string(&s).unwrap_or_default()));
        let _ = self.app.emit("status", s);
        self.tray_idle();
    }

    /// Tray icon = status light: grey starting, green ready, red recording, amber working.
    fn set_tray(&self, kind: &str) {
        const READY: &[u8] = include_bytes!("../icons/tray/tray-ready-32.png");
        const REC: &[u8] = include_bytes!("../icons/tray/tray-rec-32.png");
        const BUSY: &[u8] = include_bytes!("../icons/tray/tray-busy-32.png");
        const OFF: &[u8] = include_bytes!("../icons/tray/tray-off-32.png");
        let (bytes, tip) = match kind {
            "rec" => (REC, "Seda — listening"),
            "busy" => (BUSY, "Seda — writing…"),
            "ready" => (READY, "Seda — ready"),
            _ => (OFF, "Seda — starting up"),
        };
        if let (Some(tray), Ok(img)) = (self.app.tray_by_id("main"), tauri::image::Image::from_bytes(bytes)) {
            let _ = tray.set_icon(Some(img));
            let _ = tray.set_tooltip(Some(tip));
        }
    }

    fn tray_idle(&self) {
        if self.rec.lock().is_some() {
            return;
        }
        let ready = self.status.lock().asr == "ready";
        self.set_tray(if ready { "ready" } else { "off" });
    }

    fn load_asr(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            {
                let mut s = me.status.lock();
                s.asr = "loading".into();
                s.asr_error = None;
            }
            me.emit_status();
            *me.asr.write() = None;
            let path = me.settings.read().whisper_model.clone();
            // Warm-up: the first GPU run compiles shaders, so do it now instead of on the first dictation.
            let warm = |a: asr::Asr| a.transcribe(&vec![0.0; audio::RATE as usize], "fa", &[]).map(|_| a);
            let result = asr::Asr::load(&path, true).and_then(warm).or_else(|e| {
                // Some graphics drivers lack what the GPU path needs: fall back to the CPU (slower, but works).
                log::log(format!("speech model on GPU failed ({e:#}); trying CPU"));
                asr::Asr::load(&path, false).and_then(warm)
            });
            match result {
                Ok(a) => {
                    *me.asr.write() = Some(a);
                    me.status.lock().asr = "ready".into();
                }
                Err(e) => {
                    let mut s = me.status.lock();
                    s.asr = "error".into();
                    s.asr_error = Some(e.to_string());
                }
            }
            me.emit_status();
        });
    }

    fn load_llm(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            *me.llm.lock() = None;
            let (enabled, server, model) = {
                let s = me.settings.read();
                (s.cleanup, s.llm_server.clone(), s.llm_model.clone())
            };
            {
                let mut s = me.status.lock();
                s.llm = if enabled { "loading".into() } else { "off".into() };
                s.llm_error = None;
            }
            me.emit_status();
            if !enabled {
                return;
            }
            match llm::Llm::start(&server, &model) {
                Ok(l) => {
                    *me.llm.lock() = Some(l);
                    me.status.lock().llm = "ready".into();
                }
                Err(e) => {
                    let mut s = me.status.lock();
                    s.llm = "error".into();
                    s.llm_error = Some(e.to_string());
                }
            }
            me.emit_status();
        });
    }

    // ---------- pill ----------

    fn pill(&self, state: &'static str, app: &str, message: impl Into<String>) {
        let message = message.into();
        // Never log what was said (the log ends up in "Copy diagnostics"): Seda's own error and
        // status messages are fine, dictated text is logged as a length only.
        let note = if matches!(state, "error" | "empty" | "cancelled") { message.clone() } else { format!("{} chars", message.chars().count()) };
        log::log(format!("pill {state} {note}"));
        let generation = self.pill_gen.fetch_add(1, Ordering::SeqCst) + 1;
        match state {
            "listening" => self.set_tray("rec"),
            "transcribing" | "polishing" => self.set_tray("busy"),
            _ => self.tray_idle(),
        }
        // A click on the desktop icon starts the same capsule the shortcut uses.
        // from_dot only remembers that the click started it, so the shortcut can still stop it.
        let _ = self.app.emit_to("pill", "pill", PillState { state, app: app.to_string(), message, shape: "pill" });
        if let Some(w) = self.app.get_webview_window("pill") {
            self.present_pill(&w, state, false);
        }
        let linger = match state {
            "done" => Some(1600),
            "empty" | "cancelled" => Some(1300),
            "error" => Some(3500),
            _ => None,
        };
        if let Some(ms) = linger {
            let app = self.app.clone();
            let core = app.state::<Arc<Core>>().inner().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(ms));
                if core.pill_gen.load(Ordering::SeqCst) == generation {
                    core.rest_pill(generation);
                }
            });
        }
    }

    /// Back to idle: either the circle (if the user wants one) or nothing at all.
    /// `generation` is the pill update this rest belongs to. A newer update (a click that
    /// already started dictation) wins, so a late rest cannot cover the listening circle.
    fn rest_pill(&self, generation: u64) {
        if self.pill_gen.load(Ordering::SeqCst) != generation {
            return;
        }
        let Some(w) = self.app.get_webview_window("pill") else { return };
        let dot = self.settings.read().idle_dot;
        let state = if dot { "idle" } else { "hidden" };
        let _ = self.app.emit_to("pill", "pill", PillState { state, app: String::new(), message: String::new(), shape: "dot" });
        if dot {
            self.present_pill(&w, "idle", true);
            return;
        }
        std::thread::sleep(Duration::from_millis(220)); // let the exit animation play
        if self.pill_gen.load(Ordering::SeqCst) == generation {
            if let Ok(h) = w.hwnd() {
                win::hide(h.0 as isize);
            }
        }
    }

    /// Show the pill window as either the round button (`dot`) or the status capsule.
    /// The circle accepts clicks and stays where the user dragged it. The capsule lets clicks through.
    fn present_pill(&self, w: &tauri::WebviewWindow, state: &str, dot: bool) {
        let wide = matches!(state, "error" | "empty");
        let size = if dot { PILL_IDLE } else if wide { PILL_WIDE } else { PILL };
        let generation = self.pill_gen.load(Ordering::SeqCst);
        let resized = self.shape_pill(w, size, dot);
        if dot {
            self.place_idle_dot(w);
        } else if resized || state == "listening" {
            self.place_pill(w, size);
        }
        if let Ok(h) = w.hwnd() {
            let hwnd = h.0 as isize;
            win::make_overlay(hwnd, !dot);
            // set_size finishes on the next window message, so the size we asked for may not be
            // the size on screen yet. Clip to the real window once it is a small square.
            // No hard window outline. A GDI clip is a jagged edge; the icon's own
            // anti-aliased corners are what should show, with the desktop in the rest.
            win::clip_icon(hwnd, false, 0, 0);
            win::show_no_activate(hwnd);
        }
        if dot {
            // A resize can clear the clip and leave the light square frame. Put the disc back
            // once the new size has landed, and skip it if a newer update already showed the capsule.
            let app = self.app.clone();
            std::thread::spawn(move || {
                for delay in [50u64, 150] {
                    std::thread::sleep(Duration::from_millis(delay));
                    let core = app.state::<Arc<Core>>().inner().clone();
                    if core.pill_gen.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    let Some(win) = app.get_webview_window("pill") else { return };
                    let Ok(outer) = win.outer_size() else { return };
                    let width = outer.width as i32;
                    let height = outer.height as i32;
                    if !small_square(width, height) {
                        continue;
                    }
                    let Ok(h) = win.hwnd() else { return };
                    let hwnd = h.0 as isize;
                    win::round_corners(hwnd, false);
                    win::clip_icon(hwnd, false, 0, 0);
                }
            });
        }
    }

    /// The circle's saved spot, or the bottom centre the first time.
    fn place_idle_dot(&self, w: &tauri::WebviewWindow) {
        let (x, y) = {
            let s = self.settings.read();
            (s.dot_x, s.dot_y)
        };
        if let (Some(x), Some(y)) = (x, y) {
            if self.point_on_screen(x, y) {
                let _ = w.set_position(PhysicalPosition::new(x, y));
                return;
            }
        }
        self.place_pill(w, PILL_IDLE);
    }

    fn point_on_screen(&self, x: i32, y: i32) -> bool {
        let Ok(monitors) = self.app.available_monitors() else { return false };
        monitors.into_iter().any(|m| {
            // Full screen, not the work area: the taskbar is a normal place to leave the circle.
            // 200px of slack keeps a circle that hangs off an edge.
            let p = m.position();
            let s = m.size();
            x > p.x - 200 && y > p.y - 200 && x < p.x + s.width as i32 + 200 && y < p.y + s.height as i32 + 200
        })
    }

    /// The pill window is a plain, solid window exactly the size of what it shows (the capsule, or
    /// the idle circle). The capsule gets native rounded corners. The circle does not: those
    /// corners are a light rounded square, and the disc is clipped on top instead.
    /// Returns true when the size changed (the caller then re-centres it).
    fn shape_pill(&self, w: &tauri::WebviewWindow, (width, height): (f64, f64), dot: bool) -> bool {
        let changed = w
            .inner_size()
            .ok()
            .zip(w.scale_factor().ok())
            .is_none_or(|(s, f)| ((s.width as f64 / f) - width).abs() > 1.0);
        // Always (re)set: it also makes the page fill the window after its frame was removed,
        // otherwise a light strip of bare window shows around the pill.
        let _ = w.set_size(tauri::LogicalSize::new(width, height));
        // Alpha 0 lets the desktop show through the icon's corners. The capsule stays solid.
        let _ = w.set_background_color(Some(tauri::window::Color(
            if dot { 0 } else { 0x14 },
            if dot { 0 } else { 0x12 },
            if dot { 0 } else { 0x10 },
            if dot { 0 } else { 255 },
        )));
        if let Ok(h) = w.hwnd() {
            win::round_corners(h.0 as isize, !dot);
        }
        changed
    }

    /// Bottom centre of the screen you're typing on: the one holding the active window (where the
    /// text will be pasted), falling back to the mouse's screen. Sized for that screen's scaling.
    fn place_pill(&self, w: &tauri::WebviewWindow, (lw, lh): (f64, f64)) {
        let monitor = win::foreground_center()
            .or_else(|| self.app.cursor_position().ok().map(|p| (p.x as i32, p.y as i32)))
            .and_then(|(x, y)| self.app.monitor_from_point(x as f64, y as f64).ok().flatten())
            .or_else(|| self.app.primary_monitor().ok().flatten());
        let Some(m) = monitor else { return };
        let size = tauri::PhysicalSize::new((lw * m.scale_factor()).round() as u32, (lh * m.scale_factor()).round() as u32);
        let area = m.work_area();
        let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
        let margin = (28.0 * m.scale_factor()) as i32;
        let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
        let _ = w.set_position(PhysicalPosition::new(x, y));
        let _ = w.set_size(size);
        let _ = w.set_position(PhysicalPosition::new(x, y)); // again: a DPI change on the move can shift it
    }

    // ---------- dictation ----------

    /// Modes: "hold" (talk while held), "toggle" (press to start, press to stop),
    /// "both" (a quick tap starts hands-free recording; holding works like push-to-talk).
    fn on_hotkey(self: &Arc<Self>, ev: HotkeyEvent) {
        let mode = self.settings.read().dictate_mode.clone();
        if !matches!(ev, HotkeyEvent::Seen(_)) {
            log::log(format!("hotkey {ev:?} (mode {mode})"));
        }
        match ev {
            HotkeyEvent::DictateDown => {
                if self.rec.lock().is_some() {
                    // Hold mode ignores a second press. A circle-started session is click-to-toggle,
                    // so the shortcut stops it too.
                    if mode != "hold" || self.from_dot.load(Ordering::SeqCst) {
                        self.finish();
                    }
                } else {
                    self.from_dot.store(false, Ordering::SeqCst);
                    self.begin();
                }
            }
            HotkeyEvent::DictateUp => {
                let held_ms = self.rec.lock().as_ref().map(|r| r.started.elapsed().as_millis());
                match (mode.as_str(), held_ms) {
                    ("hold", Some(_)) => self.finish(),
                    ("both", Some(ms)) if ms >= 400 => self.finish(),
                    _ => {} // toggle, or a quick tap in "both": keep recording until the next press
                }
            }
            HotkeyEvent::Cancel => {
                self.from_dot.store(false, Ordering::SeqCst);
                let taken = self.rec.lock().take(); // release the lock before pill() reads it again
                if let Some(r) = taken {
                    self.recording.store(false, Ordering::SeqCst);
                    hotkey::set_recording(false);
                    let _ = r.rec.stop();
                    self.pill("cancelled", "", "Cancelled");
                }
            }
            HotkeyEvent::Fix => self.fix_begin(),
            HotkeyEvent::Seen(what) => log::log(format!("input: {what}")),
        }
    }

    /// Click on the desktop icon: start the regular dictation capsule, or stop one this icon started.
    fn dot_click(self: &Arc<Self>) {
        log::log("dot click");
        if self.rec.lock().is_some() {
            self.finish();
        } else {
            self.from_dot.store(true, Ordering::SeqCst);
            self.begin();
        }
    }

    fn begin(self: &Arc<Self>) {
        if self.asr.read().is_none() {
            self.from_dot.store(false, Ordering::SeqCst);
            let msg = match self.status.lock().asr.as_str() {
                "error" => "Speech model failed to load — open Seda",
                _ => "Speech model is still loading…",
            };
            self.pill("error", "", msg);
            return;
        }
        let (exe, title) = win::foreground_app();
        let target = win::foreground();
        let mic = self.settings.read().input_device.clone();
        match audio::start(&mic) {
            Ok(rec) => {
                let app_name = win::friendly_app_name(&exe, &title);
                let live = Arc::new(Mutex::new(LiveText::default()));
                *self.rec.lock() = Some(ActiveRec { rec, started: Instant::now(), exe, title, target, live: live.clone() });
                self.recording.store(true, Ordering::SeqCst);
                hotkey::set_recording(true);
                self.pill("listening", &app_name, "");
                self.spawn_level_meter();
                self.spawn_live_transcriber(live);
            }
            Err(e) => {
                self.from_dot.store(false, Ordering::SeqCst);
                self.pill("error", "", format!("Microphone: {e}"));
            }
        }
    }

    fn spawn_level_meter(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            while me.recording.load(Ordering::SeqCst) {
                let payload = me.rec.lock().as_ref().map(|r| (r.rec.level(), r.started.elapsed().as_secs_f32()));
                if let Some((level, secs)) = payload {
                    let _ = me.app.emit_to("pill", "level", serde_json::json!({ "level": level, "secs": secs }));
                }
                std::thread::sleep(Duration::from_millis(33));
            }
        });
    }

    /// While you talk: whenever you pause, transcribe everything up to that pause in the
    /// background. When you stop, only the words after the last pause are left to do.
    fn spawn_live_transcriber(self: &Arc<Self>, live: Arc<Mutex<LiveText>>) {
        let me = self.clone();
        std::thread::spawn(move || {
            let (language, vocab) = (me.settings.read().language.clone(), me.vocab());
            while me.recording.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(500));
                let Some(snap) = me.rec.lock().as_ref().map(|r| r.rec.snapshot()) else { break };
                // Held while transcribing, so finishing waits for a chunk that is in progress.
                let mut l = live.lock();
                if l.done >= snap.len() {
                    continue;
                }
                let tail = &snap[l.done..];
                let asr = me.asr.read();
                let Some(asr) = asr.as_ref() else { break };
                if let Some(cut) = asr.pause_cut(tail) {
                    let t = Instant::now();
                    if let Ok(text) = asr.transcribe(&tail[..cut], &language, &vocab) {
                        log::log(format!("live chunk {:.1}s → {} ms", cut as f32 / audio::RATE as f32, t.elapsed().as_millis()));
                        if !text.trim().is_empty() {
                            l.parts.push(text);
                        }
                    }
                    l.done += cut;
                }
            }
        });
    }

    fn finish(self: &Arc<Self>) {
        self.from_dot.store(false, Ordering::SeqCst);
        let Some(active) = self.rec.lock().take() else { return };
        self.recording.store(false, Ordering::SeqCst);
        hotkey::set_recording(false);
        let app_name = win::friendly_app_name(&active.exe, &active.title);
        let samples = match active.rec.stop() {
            Ok(s) => s,
            Err(e) => return self.pill("error", "", format!("Microphone: {e}")),
        };
        let secs = samples.len() as f64 / audio::RATE as f64;
        if secs < 0.35 {
            return self.pill("empty", &app_name, "Too short — hold while you talk");
        }
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        log::log(format!("recorded {secs:.1}s, level rms {:.4} peak {peak:.4}", audio::rms(&samples)));
        if let Some(why) = audio::silence_reason(&samples) {
            return self.pill("error", &app_name, why);
        }
        self.pill("transcribing", &app_name, "");
        let me = self.clone();
        let live = active.live.clone();
        std::thread::spawn(move || me.process(samples, secs, active.exe, active.title, app_name, Some(active.target), Some(live)));
    }

    /// Speech → text → learned fixes → AI cleanup. Returns (raw, final text, asr ms, llm ms).
    /// `live` holds text already transcribed during recording; only the rest is transcribed here.
    /// `on_polish` is called right before the AI cleanup starts.
    fn transcribe_and_clean(
        &self,
        samples: &[f32],
        style: &str,
        live: Option<Arc<Mutex<LiveText>>>,
        on_polish: impl FnOnce(),
    ) -> Result<(String, String, i64, i64), String> {
        let s = self.settings.read().clone();
        let vocab = self.vocab();
        let t = Instant::now();
        // Waits here if a live chunk is still being transcribed.
        let (start, mut parts) = match &live {
            Some(l) => {
                let l = l.lock();
                (l.done.min(samples.len()), l.parts.clone())
            }
            None => (0, Vec::new()),
        };
        let tail = match self.asr.read().as_ref().map(|a| a.transcribe(&samples[start..], &s.language, &vocab)) {
            Some(Ok(t)) => t,
            Some(Err(e)) => return Err(e.to_string()),
            None => return Err("Speech model not loaded".into()),
        };
        parts.push(tail);
        let raw = parts.into_iter().filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join(" ");
        if start > 0 {
            log::log(format!("live: {:.1}s already done while talking, {:.1}s left", start as f32 / audio::RATE as f32, (samples.len() - start) as f32 / audio::RATE as f32));
        }
        let asr_ms = t.elapsed().as_millis() as i64;
        if raw.trim().is_empty() {
            return Ok((raw, String::new(), asr_ms, 0));
        }
        let mut text = self.store.apply_fixes(&raw);
        let t = Instant::now();
        if s.cleanup && (s.polish == "always" || needs_polish(&text, style)) {
            // The writing AI may be busy for minutes (notes for a long meeting): then don't wait for it.
            match self.llm.try_lock_for(Duration::from_millis(300)) {
                Some(mut guard) => {
                    if let Some(llm) = guard.as_mut() {
                        on_polish();
                        match llm.cleanup(&text, style, &vocab, s.latin_tech_terms) {
                            Ok(clean) if !clean.is_empty() => text = self.store.apply_fixes(&clean),
                            Ok(_) => {}
                            Err(e) => log::log(format!("cleanup skipped, pasting unpolished text: {e:#}")),
                        }
                    }
                }
                None => {
                    log::log("writing AI busy (meeting notes); pasting without polish");
                    text = finish_sentence(&text);
                }
            }
        } else {
            text = finish_sentence(&text);
        }
        Ok((raw, text, asr_ms, t.elapsed().as_millis() as i64))
    }

    /// 	arget: the window that was in front when you started talking; text is pasted only there.
    #[allow(clippy::too_many_arguments)]
    fn process(self: &Arc<Self>, samples: Vec<f32>, secs: f64, exe: String, title: String, app_name: String, target: Option<isize>, live: Option<Arc<Mutex<LiveText>>>) {
        let s = self.settings.read().clone();
        let style = if s.app_styles { win::style_for(&exe, &title) } else { "" };
        let (raw, text, asr_ms, llm_ms) = match self.transcribe_and_clean(&samples, style, live, || self.pill("polishing", &app_name, "")) {
            Ok(r) => r,
            Err(e) => return self.pill("error", &app_name, e),
        };
        log::log(format!("dictation {secs:.1}s audio, speech {asr_ms} ms, cleanup {llm_ms} ms, style '{style}'"));
        if text.trim().is_empty() {
            return self.pill("empty", &app_name, "Didn't catch that");
        }

        // A trailing space, so the next dictation doesn't glue onto this one ("…today.Please").
        let mut pasted = if text.ends_with(char::is_whitespace) { text.clone() } else { format!("{text} ") };
        // In a terminal a line break runs a command: paste dictation there as one line.
        if win::is_console(win::foreground()) {
            pasted = pasted.replace("\r\n", " ").replace(['\r', '\n'], " ");
        }
        if target.is_some_and(|h| h != win::foreground()) {
            // You moved to another window while Seda was writing: don't type into it.
            let _ = win::copy_private(&text);
            self.pill("error", &app_name, "You switched windows, so Seda didn't paste. Press Ctrl+V where you want it.");
        } else if let Err(e) = win::paste(&pasted) {
            self.pill("error", &app_name, format!("Paste failed: {e}"));
        } else {
            self.pill("done", &app_name, text.clone());
        }

        let audio_path = if s.save_audio {
            let p = settings::data_dir().join("audio").join(format!("{}.wav", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")));
            audio::save_wav(&p, &samples).ok().map(|_| p.to_string_lossy().into_owned())
        } else {
            None
        };
        self.store.add_history(&app_name, style, &raw, &text, audio_path.as_deref(), secs, asr_ms, llm_ms);
        let _ = self.app.emit("history-changed", ());
    }

    // ---------- fix my text ----------

    fn fix_begin(self: &Arc<Self>) {
        let hwnd = win::foreground();
        if let Some(w) = self.app.get_webview_window("fix") {
            if w.hwnd().map(|h| h.0 as isize).ok() == Some(hwnd) {
                return; // already open and focused
            }
        }
        let Some(text) = win::copy_selection() else {
            return self.pill("error", "", "Select some text first, then press the Fix shortcut");
        };
        *self.fix.lock() = Some(FixSession { hwnd, original: text.clone() });
        if let Some(w) = self.app.get_webview_window("fix") {
            let _ = w.center();
            let _ = w.show();
            let _ = w.set_focus();
        }
        let _ = self.app.emit_to("fix", "fix-open", serde_json::json!({ "original": text }));
        self.fix_run("grammar".into());
    }

    fn fix_run(self: &Arc<Self>, mode: String) {
        let Some(original) = self.fix.lock().as_ref().map(|f| f.original.clone()) else { return };
        let me = self.clone();
        std::thread::spawn(move || {
            let result = match me.llm.lock().as_mut() {
                Some(l) => l.rewrite(&original, &mode).map_err(|e| e.to_string()),
                None => Err(match me.status.lock().llm.as_str() {
                    "loading" => "The text AI is still starting — try again in a moment.".to_string(),
                    "off" => "The text AI is turned off in Settings → Writing.".to_string(),
                    _ => "The text AI isn't running. Check Settings → Engine.".to_string(),
                }),
            };
            let payload = match result {
                Ok(t) => serde_json::json!({ "mode": mode, "text": t }),
                Err(e) => serde_json::json!({ "mode": mode, "error": e }),
            };
            let _ = me.app.emit_to("fix", "fix-result", payload);
        });
    }

    fn fix_close(&self, paste: Option<String>) {
        if let Some(w) = self.app.get_webview_window("fix") {
            let _ = w.hide();
        }
        if let Some(f) = self.fix.lock().take() {
            win::focus(f.hwnd);
            if let Some(text) = paste {
                std::thread::sleep(Duration::from_millis(150));
                let _ = win::paste(&text);
            }
        }
    }
}

/// Words that mean the speaker corrected themselves or hesitated: these always go through the AI.
const CORRECTION_CUES: &[&str] = &[
    "no wait", "wait no", "i mean", "actually", "sorry", "scratch that", "um", "uh",
    "نه ببخشید", "ببخشید", "نه نه", "یعنی نه", "اصلا", "اصلاً", "آهان", "نه صبر کن", "اممم", "یعنی",
    // Other supported languages: the most common "I mean / no wait / sorry" words.
    "no espera", "perdón", "quiero decir", "non attends", "pardon", "je veux dire", "nein warte", "ich meine",
    "não espera", "desculpa", "quer dizer", "нет подожди", "то есть", "извините", "لا انتظر", "أقصد", "عفوا",
    "नहीं रुको", "मतलब", "不对", "我是说", "いや", "じゃなくて",
];

/// Short, clean phrases don't need the writing AI: Whisper already punctuates them, and skipping
/// the AI makes quick messages paste in about a second.
/// "smart" polish: the writing AI runs only when it changes something that matters — a
/// self-correction or filler words, or a long email/ticket that needs layout. Everything else
/// (most dictations) pastes straight after transcription: the speech model already punctuates.
fn needs_polish(text: &str, style: &str) -> bool {
    let words = text.split_whitespace().count();
    let long_formatted = (style == llm::STYLE_EMAIL || style == llm::STYLE_TICKET) && (words > 30 || text.chars().count() > 160);
    long_formatted || has_correction_cue(text)
}

fn has_correction_cue(text: &str) -> bool {
    // Whole-word match: " um " must not match "summer".
    let padded: String = format!(" {} ", text.to_lowercase())
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\u{200c}' { c } else { ' ' })
        .collect();
    let padded = padded.split_whitespace().collect::<Vec<_>>().join(" ");
    let padded = format!(" {padded} ");
    // Chinese and Japanese don't put spaces between words: plain substring match there.
    CORRECTION_CUES.iter().any(|c| padded.contains(&format!(" {c} ")) || (!c.is_ascii() && !c.contains(' ') && text.contains(c) && !text.contains(' ')))
}

/// Light touch without the AI: capital first letter (English) and a closing full stop.
fn finish_sentence(text: &str) -> String {
    let t = text.trim();
    let mut out: String = match t.chars().next() {
        Some(c) if c.is_ascii_lowercase() => c.to_ascii_uppercase().to_string() + &t[1..],
        _ => t.to_string(),
    };
    if !out.ends_with(['.', '!', '?', '؟', '…', ':']) {
        out.push('.');
    }
    out
}

// ---------- commands ----------

#[tauri::command]
fn get_settings(core: CoreState) -> Settings {
    core.settings.read().clone()
}

/// Change ONE setting. The window sends only the field that changed, and it's merged into the
/// backend's current settings: a stale copy in the window can never overwrite model paths that
/// setup filled in meanwhile (that bug made both models "not downloaded" right after setup).
#[tauri::command]
fn update_setting(core: CoreState, key: String, value: serde_json::Value) -> Result<Settings, String> {
    let mut json = serde_json::to_value(core.settings.read().clone()).map_err(|e| e.to_string())?;
    let obj = json.as_object_mut().ok_or("settings are not an object")?;
    if !obj.contains_key(&key) {
        return Err(format!("unknown setting {key}"));
    }
    // Engine and model paths decide what program Seda runs: only files inside Seda's own models
    // folder are accepted (never a network path or a program elsewhere).
    if matches!(key.as_str(), "whisper_model" | "llm_model" | "llm_server") {
        let p = value.as_str().unwrap_or_default();
        if !p.is_empty() && !setup::is_model_path(p, key == "llm_server") {
            return Err(format!("Choose a file inside {}", setup::models_dir().display()));
        }
    }
    obj.insert(key.clone(), value);
    let next: Settings = serde_json::from_value(json).map_err(|e| format!("bad value for {key}: {e}"))?;
    log::log(format!("setting {key} changed"));
    apply_settings(&core, next)?;
    Ok(core.settings.read().clone())
}

fn apply_settings(core: &CoreState, mut settings: Settings) -> Result<(), String> {
    // A new language may need the other speech model (Persian vs multilingual).
    let language_changed = core.settings.read().language != settings.language;
    if language_changed {
        match setup::speech_model_for(&settings.language) {
            Some(p) => settings.whisper_model = p,
            None => settings.onboarded = false, // setup downloads it, with this language preselected
        }
    }
    let (reload_asr, reload_llm, idle_changed) = {
        let old = core.settings.read();
        (
            old.whisper_model != settings.whisper_model,
            old.llm_model != settings.llm_model || old.llm_server != settings.llm_server || old.cleanup != settings.cleanup,
            old.idle_dot != settings.idle_dot || old.pill_position != settings.pill_position,
        )
    };
    settings.save().map_err(|e| e.to_string())?;
    hotkey::set_bindings(settings.dictate_binding.clone(), settings.fix_binding.clone());
    sync_autostart(&core.app, settings.autostart);
    *core.settings.write() = settings;
    let _ = core.app.emit("settings-changed", ());
    if reload_asr {
        core.load_asr();
    }
    if reload_llm {
        core.load_llm();
    }
    if idle_changed && core.rec.lock().is_none() {
        let generation = core.pill_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let c = core.inner().clone();
        std::thread::spawn(move || c.rest_pill(generation));
    }
    Ok(())
}

/// Settings is recording a new shortcut: let every key and button through untouched meanwhile.
/// Shortcut pressed while a Seda window has focus: "down", "up" or "cancel".
#[tauri::command]
fn local_hotkey(window: tauri::WebviewWindow, event: String) -> Result<(), String> {
    only(&window, "main")?;
    hotkey::local(&event);
    Ok(())
}

/// Click the on-screen icon: start the regular dictation capsule, or stop one that icon started.
#[tauri::command]
fn dot_click(core: CoreState, window: tauri::WebviewWindow) -> Result<(), String> {
    only(&window, "pill")?;
    core.inner().dot_click();
    Ok(())
}

/// Drag the circle. `dx` and `dy` are CSS pixels since the last move (the webview's own pixels).
#[tauri::command]
fn dot_nudge(window: tauri::WebviewWindow, dx: f64, dy: f64) -> Result<(), String> {
    only(&window, "pill")?;
    if dx == 0.0 && dy == 0.0 {
        return Ok(());
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let x = pos.x + (dx * scale).round() as i32;
    let y = pos.y + (dy * scale).round() as i32;
    window.set_position(tauri::PhysicalPosition::new(x, y)).map_err(|e| e.to_string())
}

/// Remember where the circle was dropped, so the next launch puts it back.
#[tauri::command]
fn dot_save(core: CoreState, window: tauri::WebviewWindow) -> Result<(), String> {
    only(&window, "pill")?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let mut s = core.settings.write();
    s.dot_x = Some(pos.x);
    s.dot_y = Some(pos.y);
    s.save().map_err(|e| e.to_string())
}

/// Commands that act on your screen or microphone may only come from the window they belong to.
fn only(window: &tauri::WebviewWindow, label: &str) -> Result<(), String> {
    if window.label() == label { Ok(()) } else { Err(format!("not allowed from the {} window", window.label())) }
}

// ---------- updates ----------

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn update_status() -> Option<update::UpdateInfo> {
    update::available()
}

#[tauri::command(async)]
async fn update_check(app: AppHandle) -> Result<Option<update::UpdateInfo>, String> {
    update::check(&app).await
}

#[tauri::command(async)]
async fn update_install(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    only(&window, "main")?;
    update::install(&app).await
}

#[tauri::command]
fn set_hotkeys_paused(paused: bool) {
    log::log(format!("hotkeys paused: {paused}"));
    hotkey::set_paused(paused);
}

#[tauri::command]
fn get_status(core: CoreState) -> Status {
    core.status.lock().clone()
}

#[tauri::command]
fn reload_models(core: CoreState) {
    core.load_asr();
    core.load_llm();
}

#[tauri::command]
fn history(core: CoreState, limit: i64) -> Vec<store::HistoryItem> {
    core.store.history(limit)
}

#[tauri::command]
fn correct_history(core: CoreState, id: i64, text: String) -> Vec<(String, String)> {
    core.store.correct(id, &text)
}

#[tauri::command]
fn delete_history(core: CoreState, id: i64) {
    core.store.delete_history(id);
}

#[tauri::command]
fn vocab_list(core: CoreState) -> Vec<String> {
    core.store.vocab()
}

#[tauri::command]
fn vocab_add(core: CoreState, term: String) {
    if !term.trim().is_empty() {
        core.store.add_vocab(&term);
    }
}

#[tauri::command]
fn vocab_remove(core: CoreState, term: String) {
    core.store.remove_vocab(&term);
}

#[tauri::command]
fn fixes_list(core: CoreState) -> Vec<store::Fix> {
    core.store.fixes()
}

#[tauri::command]
fn fix_add(core: CoreState, wrong: String, right: String) {
    if !wrong.trim().is_empty() {
        core.store.add_fix(&wrong, &right);
    }
}

#[tauri::command]
fn fix_remove(core: CoreState, wrong: String) {
    core.store.remove_fix(&wrong);
}

#[tauri::command]
fn training_stats(core: CoreState) -> store::TrainingStats {
    core.store.training_stats()
}

#[tauri::command]
fn open_data_folder() {
    let _ = std::process::Command::new("explorer").arg(settings::data_dir()).spawn();
}

#[derive(Serialize)]
struct TestResult {
    raw: String,
    text: String,
    asr_ms: i64,
    llm_ms: i64,
    secs: f64,
}

/// Run a WAV file through the full dictation pipeline without pasting (Engine → Test).
#[tauri::command]
async fn test_file(core: State<'_, Arc<Core>>, window: tauri::WebviewWindow, path: String, style: String) -> Result<TestResult, String> {
    only(&window, "main")?;
    let path = path.trim().trim_matches('"').to_string();
    if path.starts_with(r"\\") || path.starts_with("//") {
        return Err("Pick a file on this PC (network paths aren't allowed)".into());
    }
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let samples = audio::load_wav(path.trim().trim_matches('"')).map_err(|e| format!("Couldn't read the file: {e}"))?;
        let secs = samples.len() as f64 / audio::RATE as f64;
        let (raw, text, asr_ms, llm_ms) = core.transcribe_and_clean(&samples, &style, None, || {})?;
        Ok(TestResult { raw, text, asr_ms, llm_ms, secs })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn fix_rewrite(core: CoreState, window: tauri::WebviewWindow, mode: String) {
    if only(&window, "fix").is_err() {
        return;
    }
    core.inner().fix_run(mode);
}

#[tauri::command]
fn fix_accept(core: CoreState, window: tauri::WebviewWindow, text: String) {
    if only(&window, "fix").is_err() {
        return;
    }
    core.fix_close(Some(text));
}

#[tauri::command]
fn fix_cancel(core: CoreState) {
    core.fix_close(None);
}

/// Builds run straight from cargo's target folder (it has a `build` dir next to the exe).
fn is_dev_build() -> bool {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("build").is_dir())).unwrap_or(true)
}

fn sync_autostart(app: &AppHandle, on: bool) {
    use tauri_plugin_autostart::ManagerExt;
    if is_dev_build() {
        return; // only the installed app may register itself to start with Windows
    }
    let launcher = app.autolaunch();
    let _ = if on { launcher.enable() } else { launcher.disable() };
}

// ---------- first-run setup ----------

#[derive(Serialize)]
struct SetupInfo {
    hardware: setup::Hardware,
    packs: std::collections::BTreeMap<&'static str, Vec<setup::ItemStatus>>,
    models_dir: String,
}

#[tauri::command]
async fn setup_info(language: String) -> SetupInfo {
    tauri::async_runtime::spawn_blocking(move || SetupInfo {
        hardware: setup::hardware(),
        packs: ["full", "light", "speech"].into_iter().map(|p| (p, setup::status(p, &language))).collect(),
        models_dir: setup::models_dir().to_string_lossy().into_owned(),
    })
    .await
    .expect("setup info")
}

/// Download what's missing for `pack`, then point Seda at the models and (re)start them.
/// Emits "setup-progress" {id, done, total} and finally "setup-done" {ok, error}.
#[tauri::command]
fn setup_download(core: CoreState, pack: String) {
    let core = core.inner().clone();
    std::thread::spawn(move || {
        let app = core.app.clone();
        let language = core.settings.read().language.clone();
        let result = setup::download_missing(&pack, &language, &|id, done, total| {
            let _ = app.emit("setup-progress", serde_json::json!({ "id": id, "done": done, "total": total }));
        });
        if let Err(e) = &result {
            log::log(format!("setup failed: {e:#}"));
            let _ = app.emit("setup-done", serde_json::json!({ "ok": false, "error": format!("{e:#}") }));
            return;
        }
        let (speech, server, llm) = setup::resolved_paths(&pack, &language);
        let settings = {
            let mut s = core.settings.write();
            s.whisper_model = speech;
            s.cleanup = pack != "speech";
            if pack != "speech" {
                s.llm_server = server;
                s.llm_model = llm;
            }
            let _ = s.save();
            s.clone()
        };
        log::log(format!("setup complete ({pack}, language {})", settings.language));
        let _ = app.emit("settings-changed", ()); // the window must pick up the new model paths
        core.load_asr();
        core.load_llm();
        let _ = app.emit("setup-done", serde_json::json!({ "ok": true }));
    });
}

#[tauri::command]
fn setup_cancel() {
    setup::CANCEL.store(true, Ordering::SeqCst);
}

/// Microphone check during setup: emits "mic-level" while running.
#[tauri::command]
fn mic_test_start(core: CoreState, window: tauri::WebviewWindow) -> Result<(), String> {
    only(&window, "main")?;
    let mic = core.settings.read().input_device.clone();
    let rec = audio::start(&mic).map_err(|e| e.to_string())?;
    *core.mic_test.lock() = Some(rec);
    let core = core.inner().clone();
    std::thread::spawn(move || loop {
        let level = match core.mic_test.lock().as_ref() {
            Some(r) => r.level(),
            None => break,
        };
        let _ = core.app.emit("mic-level", level);
        std::thread::sleep(Duration::from_millis(50));
    });
    Ok(())
}

/// Test mode only (SEDA_TEST_INPUT=1): show the pill in a given state without a key press.
#[tauri::command]
fn test_pill(core: CoreState, state: String) {
    // Only in builds made with `--features test-hooks`, never in the public installer.
    if !cfg!(feature = "test-hooks") || std::env::var("SEDA_TEST_INPUT").as_deref() != Ok("1") {
        return;
    }
    let s: &'static str = match state.as_str() {
        "listening" => "listening",
        "transcribing" => "transcribing",
        "done" => "done",
        _ => "error",
    };
    core.pill(s, "Test", "پیام آزمایشی");
}

#[tauri::command]
fn list_mics() -> Vec<String> {
    audio::input_devices()
}

/// Windows' microphone privacy page ("Let desktop apps access your microphone").
#[tauri::command]
fn open_mic_privacy() {
    let _ = std::process::Command::new("explorer").arg("ms-settings:privacy-microphone").spawn();
}

#[tauri::command]
fn mic_test_stop(core: CoreState) {
    if let Some(r) = core.mic_test.lock().take() {
        let _ = r.stop();
    }
}

/// Everything needed to debug a problem on someone else's PC, as text to paste.
#[tauri::command]
fn diagnostics(core: CoreState) -> String {
    let s = core.settings.read().clone();
    let hw = setup::hardware();
    let status = core.status.lock().clone();
    let gpu = core.asr.read().as_ref().map(|a| if a.gpu { "GPU" } else { "CPU" }).unwrap_or("not loaded");
    let log = std::fs::read_to_string(settings::data_dir().join("seda.log")).unwrap_or_default();
    let tail: Vec<&str> = log.lines().rev().take(80).collect::<Vec<_>>().into_iter().rev().collect();
    let monitors = core
        .app
        .available_monitors()
        .map(|ms| ms.iter().map(|m| format!("{}x{}@{:.2}", m.size().width, m.size().height, m.scale_factor())).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let report = format!(
        "Seda {} diagnostics\nexe: {}\ngpu: {} (vram {:.1} GB, integrated {}) · ram {:.0} GB · monitors: {}\n\
         speech: {} on {} {}\nwriting AI: {} {}\nbindings: dictate {:?} / fix {:?} · mode {} · language {} · cleanup {}\n\
         models: {} | {} | {}\nonboarded: {}\n--- log ---\n{}",
        env!("CARGO_PKG_VERSION"),
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
        hw.gpu, hw.vram_gb, hw.integrated, hw.ram_gb, monitors,
        status.asr, gpu, status.asr_error.unwrap_or_default(),
        status.llm, status.llm_error.unwrap_or_default(),
        s.dictate_binding, s.fix_binding, s.dictate_mode, s.language, s.cleanup,
        s.whisper_model, s.llm_server, s.llm_model, s.onboarded,
        tail.join("\n")
    );
    // Shared with whoever helps: hide the Windows user name in paths (C:\Users\<name>\… → ~\…).
    match dirs::home_dir() {
        Some(home) => report.replace(&home.display().to_string(), "~"),
        None => report,
    }
}

// ---------- meetings ----------

#[tauri::command]
fn meeting_start(core: CoreState, window: tauri::WebviewWindow) -> Result<(), String> {
    only(&window, "main")?;
    start_meeting(core.inner())
}

/// Continue a stopped meeting: same transcript, clock and notepad; notes are rewritten at the end.
#[tauri::command]
fn meeting_resume(core: CoreState, window: tauri::WebviewWindow, id: i64) -> Result<(), String> {
    only(&window, "main")?;
    let m = core.store.meeting(id).ok_or("Meeting not found")?;
    let segments: Vec<meeting::Segment> = serde_json::from_str(&m.transcript).unwrap_or_default();
    let saved: serde_json::Value = serde_json::from_str(&m.parts).unwrap_or_default();
    let parts: Vec<String> = serde_json::from_value(saved["parts"].clone()).unwrap_or_default();
    let until = if parts.is_empty() { 0.0 } else { saved["until"].as_f64().unwrap_or(0.0) as f32 };
    let resume = meeting::Resume { id, offset: m.secs as f32, segments, my_notes: m.my_notes, parts, until };
    start_meeting_with(core.inner(), Some(resume))
}

fn start_meeting(core: &Arc<Core>) -> Result<(), String> {
    start_meeting_with(core, None)
}

fn start_meeting_with(core: &Arc<Core>, resume: Option<meeting::Resume>) -> Result<(), String> {
    let mut m = core.meeting.lock();
    if m.is_some() {
        return Ok(());
    }
    if core.asr.read().is_none() {
        return Err("The speech model isn't ready yet".into());
    }
    *m = Some(meeting::Meeting::start(core.clone(), resume).map_err(|e| e.to_string())?);
    let _ = core.app.emit("meeting-state", true);
    Ok(())
}

/// Stops the meeting, saves the transcript and starts writing the notes in the background.
/// Async: finishing the last transcription takes a few seconds and must not freeze the window.
#[tauri::command(async)]
fn meeting_stop(core: CoreState) -> Option<i64> {
    stop_meeting(core.inner())
}

fn stop_meeting(core: &Arc<Core>) -> Option<i64> {
    let m = core.meeting.lock().take()?;
    {
        // Stopped while the call goes on: don't offer to record that same call again.
        let mut c = core.call.lock();
        c.linked = None;
        c.dismissed = detect::active().iter().map(|a| a.id).collect();
    }
    let (secs, ts, mine) = (m.offset as f64 + m.started.elapsed().as_secs_f64(), m.ts.clone(), m.my_notes.lock().clone());
    let resumed = m.resume_id;
    let _ = core.app.emit("meeting-state", false);
    let (segs, live) = m.stop();
    let transcript = serde_json::to_string(&segs).unwrap_or_default();
    let id = match resumed {
        // Resumed: the same meeting grows; its notes are rewritten below.
        Some(id) => {
            core.store.update_meeting_recording(id, secs, &transcript, &mine);
            core.store.set_meeting_notes(id, "", "");
            id
        }
        None => core.store.add_meeting(&ts, secs, &transcript, &mine),
    };
    log::log(format!("meeting saved: {:.0} min, {} lines", secs / 60.0, segs.len()));
    let _ = core.app.emit("meetings-changed", ());
    let c = core.clone();
    std::thread::spawn(move || write_meeting_notes(&c, id, Some(live)));
    Some(id)
}

// ---------- call detection ----------

impl Core {
    /// The small "call detected" card in the bottom-right corner. `state`: detected | recording |
    /// ended | hidden.
    fn call_card(&self, state: &str, app: &str) {
        let _ = self.app.emit_to("call", "call", serde_json::json!({ "state": state, "app": app }));
        let Some(w) = self.app.get_webview_window("call") else { return };
        let Ok(h) = w.hwnd() else { return };
        if state == "hidden" {
            return win::hide(h.0 as isize);
        }
        let _ = w.set_size(tauri::LogicalSize::new(340.0, 84.0));
        win::round_corners(h.0 as isize, true);
        if let (Some(m), Ok(size)) = (self.app.primary_monitor().ok().flatten(), w.outer_size()) {
            let area = m.work_area();
            let margin = (16.0 * m.scale_factor()) as i32;
            let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
            let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
        win::show_no_activate(h.0 as isize);
    }
}

/// Every 2 s: is a call app using the microphone? Then ask (or just start), and stop a meeting
/// that was started for a call once that call has ended.
fn watch_calls(core: Arc<Core>) {
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let (mode, enabled) = {
            let s = core.settings.read();
            (s.meeting_detect.clone(), s.meeting_apps.clone())
        };
        let active: Vec<&detect::CallApp> =
            if mode == "off" { Vec::new() } else { detect::active().into_iter().filter(|a| enabled.iter().any(|e| e == a.id)).collect() };
        let is_active = |id: &str| active.iter().any(|a| a.id == id);
        let running = core.meeting.lock().is_some();
        let mut c = core.call.lock();
        c.dismissed.retain(|d| is_active(d));
        if let Some(a) = c.asking {
            if !is_active(a) || running {
                c.asking = None;
                core.call_card("hidden", "");
            }
        }
        if running {
            let Some(id) = c.linked else { continue };
            if is_active(id) {
                c.gone_since = None;
            } else if c.gone_since.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(12) {
                drop(c);
                let name = detect::by_id(id).map(|a| a.name).unwrap_or("call");
                log::log(format!("call ended ({name}); stopping the meeting"));
                stop_meeting(&core);
                core.call_card("ended", name);
            }
            continue;
        }
        c.linked = None;
        if c.asking.is_some() {
            continue;
        }
        let Some(app) = active.iter().find(|a| !c.dismissed.contains(&a.id)) else { continue };
        log::log(format!("call detected: {}", app.name));
        if mode == "auto" {
            c.linked = Some(app.id);
            c.gone_since = None;
            drop(c);
            match start_meeting(&core) {
                Ok(()) => core.call_card("recording", app.name),
                Err(e) => log::log(format!("couldn't start meeting: {e}")),
            }
        } else {
            c.asking = Some(app.id);
            core.call_card("detected", app.name);
        }
    }
}

/// "Start notes" on the call card.
#[tauri::command]
fn call_start(core: CoreState, window: tauri::WebviewWindow) -> Result<(), String> {
    only(&window, "call")?;
    call_start_now(core.inner())
}

fn call_start_now(core: &Arc<Core>) -> Result<(), String> {
    let app = core.call.lock().asking.take();
    let name = app.and_then(detect::by_id).map(|a| a.name).unwrap_or("call");
    start_meeting(core)?;
    {
        let mut c = core.call.lock();
        c.linked = app;
        c.gone_since = None;
    }
    core.call_card("recording", name);
    Ok(())
}

/// "Not now" (or the card timed out).
#[tauri::command]
fn call_dismiss(core: CoreState) {
    let mut c = core.call.lock();
    if let Some(a) = c.asking.take() {
        c.dismissed.push(a);
    }
    drop(c);
    core.call_card("hidden", "");
}

#[tauri::command]
fn call_hide(core: CoreState) {
    core.call_card("hidden", "");
}

/// "Open notes" on the card: show the main window on Meetings.
#[tauri::command]
fn call_open(core: CoreState) {
    core.call_card("hidden", "");
    show_main(&core.app);
    let _ = core.app.emit_to("main", "navigate", "meetings");
}

#[tauri::command]
fn call_apps() -> Vec<detect::AppInfo> {
    detect::apps()
}

/// live: the part notes written during the meeting (None when rewriting later).
fn write_meeting_notes(core: &Core, id: i64, live: Option<meeting::LiveNotes>) {
    let Some(m) = core.store.meeting(id) else { return };
    let segs: Vec<meeting::Segment> = serde_json::from_str(&m.transcript).unwrap_or_default();
    if segs.is_empty() && m.my_notes.trim().is_empty() {
        core.store.set_meeting_notes(id, "Empty meeting", "Nothing was said.");
        let _ = core.app.emit("meetings-changed", ());
        return;
    }
    let t = Instant::now();
    let notes = meeting::write_notes(core, &segs, live, &m.my_notes);
    let (title, notes) = match notes {
        Ok((n, parts, until)) => {
            // Kept so Resume only needs notes for the new stretch.
            core.store.set_meeting_parts(id, &serde_json::json!({ "parts": parts, "until": until }).to_string());
            let title = n.lines().find_map(|l| l.strip_prefix("# ")).unwrap_or("Meeting").trim().to_string();
            let body = n.lines().filter(|l| !l.starts_with("# ")).collect::<Vec<_>>().join("\n").trim().to_string();
            (title, body)
        }
        Err(e) => ("Meeting".to_string(), format!("Couldn't write notes: {e}. The transcript is saved — try “Write notes again”.")),
    };
    log::log(format!("meeting notes written in {:.1}s", t.elapsed().as_secs_f32()));
    core.store.set_meeting_notes(id, &title, &notes);
    let _ = core.app.emit("meetings-changed", ());
}

#[derive(Serialize)]
struct MeetingStatus {
    running: bool,
    secs: f64,
    /// Title of the meeting being continued (Resume), else empty.
    resuming: String,
    segments: Vec<meeting::Segment>,
    my_notes: String,
}

#[tauri::command]
fn meeting_status(core: CoreState) -> MeetingStatus {
    match core.meeting.lock().as_ref() {
        Some(m) => MeetingStatus {
            running: true,
            secs: m.offset as f64 + m.started.elapsed().as_secs_f64(),
            resuming: m.resume_id.and_then(|id| core.store.meeting(id)).map(|x| x.title).unwrap_or_default(),
            segments: m.segments.lock().clone(),
            my_notes: m.my_notes.lock().clone(),
        },
        None => MeetingStatus { running: false, secs: 0.0, resuming: String::new(), segments: Vec::new(), my_notes: String::new() },
    }
}

/// The notepad during a meeting, sent as you type (debounced by the window).
#[tauri::command]
fn meeting_set_my_notes(core: CoreState, text: String) {
    if let Some(m) = core.meeting.lock().as_ref() {
        *m.my_notes.lock() = text;
    }
}

/// Notes edited by hand.
#[tauri::command]
fn meeting_save_notes(core: CoreState, id: i64, title: String, notes: String) {
    core.store.set_meeting_notes(id, title.trim(), notes.trim());
    let _ = core.app.emit("meetings-changed", ());
}

#[derive(Serialize, serde::Deserialize, Clone)]
struct ChatTurn {
    role: String, // "you" | "ai"
    text: String,
    /// True when this answer rewrote the notes.
    #[serde(default)]
    edited: bool,
}

/// Ask about a meeting, or tell the AI to change its notes. Returns the updated chat.
#[tauri::command(async)]
fn meeting_ask(core: CoreState, id: i64, question: String) -> Result<Vec<ChatTurn>, String> {
    let m = core.store.meeting(id).ok_or("Meeting not found")?;
    let segs: Vec<meeting::Segment> = serde_json::from_str(&m.transcript).unwrap_or_default();
    let mut chat: Vec<ChatTurn> = serde_json::from_str(&m.chat).unwrap_or_default();
    let history: Vec<(String, String)> = chat.iter().map(|t| (t.role.clone(), t.text.clone())).collect();
    let context = meeting::ask_context(&m.title, &m.notes, &segs, &question);
    let t = Instant::now();
    let reply = {
        let mut guard = core.llm.lock();
        let llm = guard.as_mut().ok_or("The writing AI isn't running yet")?;
        let user = core.settings.read().user_name.clone();
        llm.ask_meeting(&context, &history, &question, &user).map_err(|e| e.to_string())?
    };
    log::log(format!("meeting ask answered in {:.1}s", t.elapsed().as_secs_f32()));
    // A change to the notes comes back as complete notes: save them, show only the sentence before them.
    let (answer, new_notes) = split_notes_reply(&reply);
    let edited = new_notes.is_some();
    if let Some(new) = new_notes {
        let title = new.lines().find_map(|l| l.strip_prefix("# ")).unwrap_or(&m.title).trim();
        let title = title.strip_prefix("Meeting notes:").unwrap_or(title).trim(); // echoed from the context header
        let body = new.lines().filter(|l| !l.starts_with("# ")).collect::<Vec<_>>().join("\n");
        core.store.set_meeting_notes(id, title, llm::drop_empty_sections(body.trim()).trim());
    }
    let answer = if answer.is_empty() && edited { "Updated the notes.".to_string() } else { answer };
    chat.push(ChatTurn { role: "you".into(), text: question, edited: false });
    chat.push(ChatTurn { role: "ai".into(), text: answer, edited });
    core.store.set_meeting_chat(id, &serde_json::to_string(&chat).unwrap_or_default());
    let _ = core.app.emit("meetings-changed", ());
    Ok(chat)
}

/// Splits an Ask reply into (the sentence for the chat, the new notes if the reply rewrote them).
/// The notes should come inside <notes>…</notes>, but the small model sometimes drops the tags:
/// then two or more "## " sections mean the reply is a full set of notes.
fn split_notes_reply(reply: &str) -> (String, Option<String>) {
    if let (Some(a), Some(b)) = (reply.find("<notes>"), reply.rfind("</notes>")) {
        if b > a {
            let said = format!("{}{}", &reply[..a], &reply[b + 8..]);
            return (said.trim().to_string(), Some(reply[a + 7..b].trim().to_string()));
        }
    }
    let lines: Vec<&str> = reply.lines().collect();
    if lines.iter().filter(|l| l.starts_with("## ")).count() >= 2 {
        let first = lines.iter().position(|l| l.starts_with('#')).unwrap_or(0);
        let said = lines[..first].join("\n").replace("```markdown", "").replace("```", "");
        let notes = lines[first..].join("\n").replace("```", "");
        return (said.trim().trim_end_matches(':').trim().to_string(), Some(notes.trim().to_string()));
    }
    (reply.trim().to_string(), None)
}

#[tauri::command]
fn meeting_clear_chat(core: CoreState, id: i64) {
    core.store.set_meeting_chat(id, "[]");
}

#[tauri::command]
fn meetings_list(core: CoreState) -> Vec<store::Meeting> {
    core.store.meetings()
}

#[tauri::command]
fn meeting_get(core: CoreState, id: i64) -> Option<store::Meeting> {
    core.store.meeting(id)
}

#[tauri::command]
fn meeting_delete(core: CoreState, id: i64) {
    if let Some(m) = core.store.meeting(id) {
        // The audio files are named after the start time.
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&m.ts) {
            let name = t.format("%Y%m%d-%H%M%S").to_string();
            let dir = settings::data_dir().join("meetings");
            // "-you.wav", "-them.wav", "-them0.wav", …
            if let Ok(files) = std::fs::read_dir(&dir) {
                for f in files.flatten().filter(|f| f.file_name().to_string_lossy().starts_with(&name)) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
    }
    core.store.delete_meeting(id);
    let _ = core.app.emit("meetings-changed", ());
}

#[tauri::command]
fn meeting_renotes(core: CoreState, id: i64) {
    core.store.set_meeting_notes(id, "", "");
    let _ = core.app.emit("meetings-changed", ());
    let c = core.inner().clone();
    std::thread::spawn(move || write_meeting_notes(&c, id, None));
}

#[tauri::command]
fn finish_onboarding(core: CoreState) {
    {
        let mut s = core.settings.write();
        s.onboarded = true;
        let _ = s.save();
    }
    let _ = core.app.emit("settings-changed", ());
}

/// Open seda.log in Notepad (the folder also holds llama.log, the writing AI engine's own log).
#[tauri::command]
fn open_log() {
    open_log_file();
}

fn open_log_file() {
    let path = settings::data_dir().join("seda.log");
    if !path.exists() {
        log::log("log opened");
    }
    let _ = std::process::Command::new("notepad").arg(path).spawn();
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// `seda --selftest [style] a.wav b.wav …` — run clips through speech + cleanup without the UI.
fn selftest(args: &[String]) {
    let (style, files) = match args.first().map(String::as_str) {
        Some(s @ ("email" | "ticket" | "chat")) => (s, &args[1..]),
        _ => ("", args),
    };
    let mut s = Settings::load();
    // Benchmarks can point at model files directly.
    for (var, field) in [("SEDA_WHISPER", &mut s.whisper_model), ("SEDA_SERVER", &mut s.llm_server), ("SEDA_LLM", &mut s.llm_model)] {
        if let Ok(v) = std::env::var(var) {
            *field = v;
        }
    }
    let t = Instant::now();
    let asr = asr::Asr::load(&s.whisper_model, true).expect("speech model");
    println!("speech model loaded in {:.1}s", t.elapsed().as_secs_f32());
    if args.first().map(String::as_str) == Some("asr") {
        // Speech step only: warm up once, then time each clip.
        let _ = asr.transcribe(&vec![0.0; audio::RATE as usize * 2], &s.language, &[]);
        for f in &args[1..] {
            let samples = audio::load_wav(f).expect("wav");
            let t = Instant::now();
            let text = asr.transcribe(&samples, &s.language, &[]).unwrap_or_default();
            println!("{f} speech {} ms\n  heard: {text}", t.elapsed().as_millis());
        }
        return;
    }
    if args.first().map(String::as_str) == Some("live") {
        // Simulate talking: clips joined with 1 s pauses, revealed 0.5 s at a time like a live mic.
        let mut audio = Vec::new();
        for f in &args[1..] {
            audio.extend(audio::load_wav(f).expect("wav"));
            audio.extend(std::iter::repeat_n(0.0f32, audio::RATE as usize));
        }
        let t = Instant::now();
        let all = asr.transcribe(&audio, &s.language, &[]).unwrap_or_default();
        println!("all at the end: {} ms\n  {all}\n", t.elapsed().as_millis());
        let (mut done, mut parts, step) = (0usize, Vec::new(), audio::RATE as usize / 2);
        let mut heard = step;
        while heard < audio.len() {
            if let Some(cut) = asr.pause_cut(&audio[done..heard]) {
                parts.push(asr.transcribe(&audio[done..done + cut], &s.language, &[]).unwrap_or_default());
                done += cut;
                println!("  live chunk done at {:.1}s of {:.1}s", heard as f32 / 16000.0, audio.len() as f32 / 16000.0);
            }
            heard += step;
        }
        let t = Instant::now();
        parts.push(asr.transcribe(&audio[done..], &s.language, &[]).unwrap_or_default());
        println!("\nlive: only {:.1}s left after you stop → {} ms\n  {}", (audio.len() - done) as f32 / 16000.0, t.elapsed().as_millis(), parts.join(" "));
        return;
    }
    if style == "scores" || args.first().map(String::as_str) == Some("scores") {
        for f in &args[1..] {
            let samples = audio::load_wav(f).expect("wav");
            let [fa, en] = asr.scores(&samples).expect("decode");
            println!("{f}\n  fa conf {:.3} no_speech {:.3}  {}\n  en conf {:.3} no_speech {:.3}  {}",
                fa.conf, fa.no_speech, fa.text, en.conf, en.no_speech, en.text);
        }
        return;
    }
    let t = Instant::now();
    let mut llm = llm::Llm::start(&s.llm_server, &s.llm_model).expect("text AI");
    println!("text AI started in {:.1}s\n", t.elapsed().as_secs_f32());
    for f in files {
        let samples = audio::load_wav(f).expect("wav");
        let t = Instant::now();
        let raw = asr.transcribe(&samples, &s.language, &[]).unwrap_or_default();
        let asr_s = t.elapsed().as_secs_f32();
        let t = Instant::now();
        let clean = llm.cleanup(&raw, style, &[], s.latin_tech_terms).unwrap_or_default();
        println!("{f}  [{:.1}s audio | speech {asr_s:.2}s | cleanup {:.2}s]\n  heard: {raw}\n  final: {clean}\n",
            samples.len() as f32 / audio::RATE as f32, t.elapsed().as_secs_f32());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Developer command-line tools: only in builds made with `--features test-hooks`; the public
    // installer has none (the compiler drops this code entirely when the feature is off).
    if cfg!(feature = "test-hooks") && dev_cli() {
        return;
    }
    tauri_main();
}

/// Runs a developer command (--selftest, --download-test, --calls, --setup-status). True if it did.
fn dev_cli() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--selftest") {
        selftest(&args[2..]);
        return true;
    }
    if let (Some("--download-test"), Some(dir)) = (args.get(1).map(String::as_str), args.get(2)) {
        if let Err(e) = setup::download_test(std::path::Path::new(dir)) {
            println!("download test failed: {e:#}");
        }
        return true;
    }
    if args.get(1).map(String::as_str) == Some("--calls") {
        print!("{}", detect::report());
        return true;
    }
    if args.get(1).map(String::as_str) == Some("--setup-status") {
        let hw = setup::hardware();
        println!("{}", serde_json::to_string_pretty(&hw).unwrap_or_default());
        for p in ["full", "light", "speech"] {
            println!("{p}: {}", serde_json::to_string(&setup::status(p, "fa")).unwrap_or_default());
        }
        return true;
    }
    false
}

fn tauri_main() {
    log::scrub_old_logs();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .invoke_handler(tauri::generate_handler![
            get_settings, update_setting, set_hotkeys_paused, get_status, reload_models, history, correct_history,
            delete_history, vocab_list, vocab_add, vocab_remove, fixes_list, fix_add, fix_remove, training_stats,
            open_data_folder, test_file, fix_rewrite, fix_accept, fix_cancel, setup_info, setup_download, setup_cancel,
            mic_test_start, mic_test_stop, list_mics, open_mic_privacy, test_pill, finish_onboarding, diagnostics, open_log,
            meeting_start, meeting_stop, meeting_status, meetings_list, meeting_get, meeting_delete, meeting_renotes,
            meeting_set_my_notes, meeting_save_notes, meeting_ask, meeting_clear_chat,
            call_start, call_dismiss, call_hide, call_open, call_apps, local_hotkey, dot_click, dot_nudge, dot_save,
            app_version, update_status, update_check, update_install, meeting_resume
        ])
        .setup(|app| {
            let mut settings = Settings::load();
            // Paths may come from another PC or a half-finished setup: fix them up, or send the
            // user back through setup rather than showing two red "error" lights.
            if !setup::reconcile(&mut settings) && settings.onboarded {
                log::log("models missing on this PC; reopening setup");
                settings.onboarded = false;
            }
            let _ = settings.save();
            let store = Store::open()?;
            let core = Arc::new(Core {
                app: app.handle().clone(),
                settings: RwLock::new(settings.clone()),
                store,
                asr: RwLock::new(None),
                llm: Mutex::new(None),
                status: Mutex::new(Status { asr: "loading".into(), llm: "loading".into(), ..Default::default() }),
                rec: Mutex::new(None),
                recording: Arc::new(AtomicBool::new(false)),
                pill_gen: AtomicU64::new(0),
                from_dot: AtomicBool::new(false),
            meeting: Mutex::new(None),
                call: Mutex::new(CallState::default()),
                fix: Mutex::new(None),
                mic_test: Mutex::new(None),
            });
            app.manage(core.clone());

            if let Some(pill) = app.get_webview_window("pill") {
                if let Ok(h) = pill.hwnd() {
                    win::make_overlay(h.0 as isize, true);
                }
            }
            if let Some(w) = app.get_webview_window("call") {
                if let Ok(h) = w.hwnd() {
                    win::make_overlay(h.0 as isize, false); // clickable, but never steals focus
                }
            }
            {
                let c = core.clone();
                std::thread::spawn(move || watch_calls(c));
            }
            update::spawn_checker(app.handle().clone());

            core.load_asr();
            core.load_llm();
            sync_autostart(app.handle(), settings.autostart);
            if std::env::args().any(|a| a == "--hidden") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide(); // started with Windows: stay in the tray
                }
            }
            if settings.idle_dot {
                let c = core.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(800)); // let the pill page load first
                    c.rest_pill(0);
                });
            }

            let (tx, rx) = crossbeam_channel::unbounded();
            hotkey::start(settings.dictate_binding.clone(), settings.fix_binding.clone(), tx.clone());
            // Test hooks exist only in builds made with `--features test-hooks` (never the public installer).
            if cfg!(feature = "test-hooks") && std::env::var("SEDA_TEST_INPUT").as_deref() == Ok("1") {
                // Test mode: drive the shortcut from a file, for machines where synthetic key
                // presses can't reach the hook. Write "down", "up", "fix", "paste:<text>" or "wav:<file>".
                let c = core.clone();
                std::thread::spawn(move || {
                    let f = std::env::temp_dir().join("seda-test.txt");
                    loop {
                        std::thread::sleep(Duration::from_millis(100));
                        let Ok(cmd) = std::fs::read_to_string(&f) else { continue };
                        let _ = std::fs::remove_file(&f);
                        let cmd = cmd.trim().to_string();
                        log::log(format!("test input: {cmd}"));
                        match cmd.as_str() {
                            "down" => { let _ = tx.send(HotkeyEvent::DictateDown); }
                            "up" => { let _ = tx.send(HotkeyEvent::DictateUp); }
                            "fix" => { let _ = tx.send(HotkeyEvent::Fix); }
                            "call-off" => *detect::TEST_ACTIVE.lock() = None,
                            s if s.starts_with("call-on:") => *detect::TEST_ACTIVE.lock() = Some(s[8..].to_string()),
                            "call-start" => log::log(format!("test call start: {:?}", call_start_now(&c))),
                            "show" => show_main(&c.app),
                            s if s.starts_with("say:") => {
                                // Add a transcript line to the running meeting, as if it was just heard: "say:Them|text".
                                if let Some(m) = c.meeting.lock().as_ref() {
                                    let (who, text) = s[4..].split_once('|').unwrap_or(("Them", &s[4..]));
                                    let at = m.offset + m.started.elapsed().as_secs_f32();
                                    m.segments.lock().push(meeting::Segment { who: who.into(), start: at, end: at + 3.0, text: text.into() });
                                }
                            }
                            s if s.starts_with("livenotes:") => {
                                // As if during a meeting: earlier stretches done first, then time only what's left at the end.
                                let path = s[10..].to_string();
                                let segs: Vec<meeting::Segment> = std::fs::read_to_string(&path).ok().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
                                let live = meeting::simulate_live(&c, &segs);
                                let t = Instant::now();
                                let text = match meeting::write_notes(&c, &segs, Some(live), "") { Ok((n, _, _)) => n, Err(e) => format!("ERROR {e}") };
                                let _ = std::fs::write(format!("{path}.live.md"), &text);
                                log::log(format!("test livenotes: end-of-meeting wait {:.1}s", t.elapsed().as_secs_f32()));
                            }
                            s if s.starts_with("notes:") => {
                                // Run a saved transcript (segments JSON) through the meeting-notes pipeline; output beside it.
                                let path = s[6..].to_string();
                                let segs: Vec<meeting::Segment> = std::fs::read_to_string(&path).ok().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
                                let t = Instant::now();
                                let text = match meeting::write_notes(&c, &segs, None, "") { Ok((n, _, _)) => n, Err(e) => format!("ERROR {e}") };
                                let _ = std::fs::write(format!("{path}.notes.md"), &text);
                                log::log(format!("test notes: {} lines in {:.1}s", segs.len(), t.elapsed().as_secs_f32()));
                            }
                            s if s.starts_with("eval:") => {
                                // Run script in the main window (e.g. focus a field, copy its value into the title).
                                if let Some(w) = c.app.get_webview_window("main") {
                                    log::log(format!("test eval: {:?}", w.eval(&s[5..])));
                                }
                            }
                            "meeting-start" => log::log(format!("test meeting start: {:?}", start_meeting(&c))),
                            "meeting-stop" => log::log(format!("test meeting stop: {:?}", meeting_stop(c.app.state()))),
                            _ => if let Some(t) = cmd.strip_prefix("paste:") {
                                log::log(format!("test paste: {:?}", win::paste(t)));
                            } else if let Some((id, q)) = cmd.strip_prefix("ask:").and_then(|r| r.split_once(':')) {
                                let r = meeting_ask(c.app.state(), id.parse().unwrap_or(0), q.to_string());
                                log::log(format!("test ask: {}", r.map(|t| t.last().map(|x| format!("[edited {}] {}", x.edited, x.text)).unwrap_or_default()).unwrap_or_else(|e| e)));
                            } else if let Some(p) = cmd.strip_prefix("wav:") {
                                // A recorded clip through the real dictation path (pill, speech, AI, paste).
                                match audio::load_wav(p) {
                                    Ok(samples) => {
                                        let secs = samples.len() as f64 / audio::RATE as f64;
                                        c.pill("transcribing", "Test", "");
                                        c.process(samples, secs, "notepad.exe".into(), "Test".into(), "Test".into(), None, None);
                                    }
                                    Err(e) => log::log(format!("test wav: {e}")),
                                }
                            },
                        }
                    }
                });
            }
            let c = core.clone();
            std::thread::spawn(move || {
                for ev in rx {
                    c.on_hotkey(ev);
                }
            });

            let open = MenuItem::with_id(app, "open", "Open Seda", true, None::<&str>)?;
            let log_item = MenuItem::with_id(app, "log", "Open log", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &log_item, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray/tray-off-32.png"))?)
                .tooltip("Seda — starting up")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_main(app),
                    "log" => open_log_file(),
                    "quit" => {
                        if let Some(c) = app.try_state::<Arc<Core>>() {
                            *c.llm.lock() = None; // stops llama-server
                        }
                        app.exit(0)
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Seda");
}

#[allow(dead_code)]
fn _assert_send_sync() {
    fn is<T: Send + Sync>() {}
    is::<Core>();
    let _ = Binding::mouse(1);
}
