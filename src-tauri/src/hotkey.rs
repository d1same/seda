//! Global low-level mouse + keyboard hooks. Matching presses are swallowed so, for example,
//! mouse button 4 no longer makes the browser go "back" while it is bound to dictation.
//!
//! Rules that keep the system responsive: the callbacks only read a small config and push to a
//! channel (never block), and the hooks are paused entirely while the Settings window records
//! a new shortcut (the window reads that key itself). Windows silently drops hooks that stall or
//! after sleep, so they are re-installed every 30 seconds.
use crate::settings::Binding;
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, SetTimer,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_TIMER,
};

#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    DictateDown,
    DictateUp,
    Fix,
    Cancel,
    /// Diagnostic only: what the hook saw (logged by the controller, never in the hook itself).
    Seen(String),
}

struct Config {
    dictate: Binding,
    fix: Binding,
    dictate_held: bool,
    recording: bool,
    tx: Sender<HotkeyEvent>,
}

static CFG: OnceLock<Mutex<Config>> = OnceLock::new();
static PAUSED: AtomicBool = AtomicBool::new(false);
/// How many times Windows has called our hooks (diagnostics: proves the hook is alive).
pub static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Seda's own synthetic keystrokes (its Ctrl+V / Ctrl+C) carry this tag so the hook can ignore them.
/// All other synthetic input IS accepted: mouse software (Logitech G Hub, Razer Synapse, AutoHotkey)
/// delivers remapped buttons as synthetic input, and ignoring it broke shortcuts for those users.
pub const SEDA_MARK: usize = 0x5EDA_5EDA;

const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_XBUTTONDOWN: u32 = 0x020B;
const WM_XBUTTONUP: u32 = 0x020C;
const VK_ESCAPE: u32 = 0x1B;

pub fn start(dictate: Binding, fix: Binding, tx: Sender<HotkeyEvent>) {
    let _ = CFG.set(Mutex::new(Config { dictate, fix, dictate_held: false, recording: false, tx }));
    std::thread::spawn(|| unsafe {
        let install = || -> (Option<HHOOK>, Option<HHOOK>) {
            let m = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0).ok();
            let k = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0).ok();
            (m, k)
        };
        let (mut m, mut k) = install();
        crate::log::log(format!("hooks installed (mouse: {}, keyboard: {})", m.is_some(), k.is_some()));
        // Diagnostics: note how many key/click events the hook has seen, once a minute when it changes.
        std::thread::spawn(|| {
            let mut last = 0;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(if cfg!(debug_assertions) { 3 } else { 60 }));
                let n = CALLS.load(Ordering::Relaxed);
                if n != last {
                    crate::log::log(format!("hook alive: {n} key/click events so far"));
                    last = n;
                }
            }
        });
        SetTimer(None, 0, 30_000, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER {
                // Re-install: cheap, and recovers from Windows silently removing the hook.
                for h in [m, k].into_iter().flatten() {
                    let _ = UnhookWindowsHookEx(h);
                }
                (m, k) = install();
                if m.is_none() || k.is_none() {
                    crate::log::log("hook re-install failed");
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

pub fn set_bindings(dictate: Binding, fix: Binding) {
    if let Some(c) = CFG.get() {
        let mut c = c.lock();
        c.dictate = dictate;
        c.fix = fix;
        c.dictate_held = false;
    }
}

/// While paused every key and button goes straight through untouched.
pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::SeqCst);
    if let Some(c) = CFG.get() {
        c.lock().dictate_held = false;
    }
}

pub fn set_recording(on: bool) {
    if let Some(c) = CFG.get() {
        c.lock().recording = on;
    }
}

fn down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

fn mods() -> (bool, bool, bool, bool) {
    (down(0x11), down(0x10), down(0x12), down(0x5B) || down(0x5C))
}

fn key_matches(b: &Binding, vk: u32) -> bool {
    let (ctrl, shift, alt, win) = mods();
    b.kind == "key" && b.code == vk && b.ctrl == ctrl && b.shift == shift && b.alt == alt && b.win == win
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if wparam.0 as u32 != 0x0200 {
        CALLS.fetch_add(1, Ordering::Relaxed); // count clicks, not plain mouse moves
    }
    if code >= 0 && !PAUSED.load(Ordering::Relaxed) {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let msg = wparam.0 as u32;
        let button = match msg {
            WM_XBUTTONDOWN | WM_XBUTTONUP => (info.mouseData >> 16) & 0xFFFF, // 1 = X1, 2 = X2
            WM_MBUTTONDOWN | WM_MBUTTONUP => 3,
            _ => 0,
        };
        let injected = info.dwExtraInfo == SEDA_MARK;
        if button != 0 && !injected {
            // try_lock: never make the whole system's mouse wait on us.
            if let Some(mut c) = CFG.get().and_then(|m| m.try_lock()) {
                if msg == WM_XBUTTONDOWN || msg == WM_MBUTTONDOWN {
                    let _ = c.tx.send(HotkeyEvent::Seen(format!("mouse button {button} pressed (dictation is {} {})", c.dictate.kind, c.dictate.code)));
                }
                if c.dictate.kind == "mouse" && c.dictate.code == button {
                    let is_down = msg == WM_XBUTTONDOWN || msg == WM_MBUTTONDOWN;
                    if is_down && !c.dictate_held {
                        c.dictate_held = true;
                        let _ = c.tx.send(HotkeyEvent::DictateDown);
                    } else if !is_down && c.dictate_held {
                        c.dictate_held = false;
                        let _ = c.tx.send(HotkeyEvent::DictateUp);
                    }
                    return LRESULT(1);
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    CALLS.fetch_add(1, Ordering::Relaxed);
    if code >= 0 && !PAUSED.load(Ordering::Relaxed) {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let injected = info.dwExtraInfo == SEDA_MARK;
        let vk = info.vkCode;
        let msg = wparam.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        if !injected && (is_down || is_up) {
            if let Some(mut c) = CFG.get().and_then(|m| m.try_lock()) {
                // Log only shortcut-like presses (a modifier held), never ordinary typing.
                let (ctrl, shift, alt, win) = mods();
                if is_down && (ctrl || alt || win) && ((c.dictate.kind == "key" && c.dictate.code == vk) || (c.fix.kind == "key" && c.fix.code == vk)) {
                    let _ = c.tx.send(HotkeyEvent::Seen(format!(
                        "key {vk} down (ctrl {ctrl}, shift {shift}, alt {alt}, win {win}) → dictation match {}, fix match {}",
                        key_matches(&c.dictate, vk),
                        key_matches(&c.fix, vk)
                    )));
                }
                if c.dictate.kind == "key" && c.dictate.code == vk && (c.dictate_held || key_matches(&c.dictate, vk)) {
                    if is_down && !c.dictate_held {
                        c.dictate_held = true;
                        let _ = c.tx.send(HotkeyEvent::DictateDown);
                    } else if is_up && c.dictate_held {
                        c.dictate_held = false;
                        let _ = c.tx.send(HotkeyEvent::DictateUp);
                    }
                    return LRESULT(1); // also swallows auto-repeat while held
                } else if is_down && key_matches(&c.fix, vk) {
                    let _ = c.tx.send(HotkeyEvent::Fix);
                    return LRESULT(1);
                } else if is_down && vk == VK_ESCAPE && c.recording {
                    let _ = c.tx.send(HotkeyEvent::Cancel);
                    return LRESULT(1);
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
