//! Windows helpers: paste/copy via simulated keystrokes, foreground-app detection,
//! and showing a window without stealing focus.
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW,
    SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_BORDER, WS_CAPTION, WS_DLGFRAME,
    WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};
use windows::core::PWSTR;

const VK_CONTROL: u16 = 0x11;
const VK_SHIFT: u16 = 0x10;
const VK_MENU: u16 = 0x12;
const VK_LWIN: u16 = 0x5B;

fn key(vk: u16, up: bool) -> INPUT {
    // Include the hardware scan code: some apps (games, remote desktops, some Electron apps)
    // ignore keystrokes that only carry a virtual-key code.
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: crate::hotkey::SEDA_MARK,
            },
        },
    }
}

/// True when the focused window belongs to a program running as administrator. Windows blocks
/// a normal app from sending keystrokes to those, so pasting there cannot work.
pub fn foreground_is_elevated() -> bool {
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::OpenProcessToken;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut token = Default::default();
        let mut elevated = false;
        if OpenProcessToken(proc_handle, TOKEN_QUERY, &mut token).is_ok() {
            let mut info = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            if GetTokenInformation(token, TokenElevation, Some(&mut info as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok() {
                elevated = info.TokenIsElevated != 0;
            }
            let _ = CloseHandle(token);
        } else {
            // Can't even read its token: that is what an elevated process looks like from here.
            elevated = true;
        }
        let _ = CloseHandle(proc_handle);
        elevated && !self_is_elevated()
    }
}

fn self_is_elevated() -> bool {
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = Default::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut info = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenElevation, Some(&mut info as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok();
        let _ = CloseHandle(token);
        ok && info.TokenIsElevated != 0
    }
}

/// Send Ctrl+<letter>, first releasing any modifiers the user may still be holding.
fn ctrl_combo(letter: u16) {
    let inputs = [
        key(VK_SHIFT, true),
        key(VK_MENU, true),
        key(VK_LWIN, true),
        key(VK_CONTROL, false),
        key(letter, false),
        key(letter, true),
        key(VK_CONTROL, true),
    ];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Paste `text` at the cursor of the focused app.
///
/// The previous clipboard comes back only after 1.5 s, and only if the clipboard still holds our
/// text: browsers and Electron apps (Teams, Slack, Discord…) read the clipboard a moment after
/// Ctrl+V, and restoring sooner made them paste the OLD clipboard instead.
pub fn paste(text: &str) -> anyhow::Result<()> {
    if foreground_is_elevated() {
        let mut cb = arboard::Clipboard::new()?;
        cb.set_text(text.to_string())?;
        anyhow::bail!("This window runs as administrator, so Windows blocks typing into it. The text is on your clipboard — press Ctrl+V.");
    }
    let mut cb = arboard::Clipboard::new()?;
    let previous = cb.get_text().ok();
    cb.set_text(text.to_string())?;
    std::thread::sleep(Duration::from_millis(60));
    ctrl_combo(0x56); // V
    let ours = text.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        let Ok(mut cb) = arboard::Clipboard::new() else { return };
        if cb.get_text().ok().as_deref() == Some(ours.as_str()) {
            if let Some(p) = previous {
                let _ = cb.set_text(p);
            }
        }
    });
    Ok(())
}

/// Copy the current selection of the focused app. Returns None if nothing is selected.
pub fn copy_selection() -> Option<String> {
    let mut cb = arboard::Clipboard::new().ok()?;
    let previous = cb.get_text().ok();
    let marker = "\u{2063}seda-empty\u{2063}";
    cb.set_text(marker.to_string()).ok()?;
    ctrl_combo(0x43); // C
    let t = Instant::now();
    let mut got = None;
    while t.elapsed() < Duration::from_millis(600) {
        std::thread::sleep(Duration::from_millis(30));
        if let Ok(s) = cb.get_text() {
            if s != marker {
                got = Some(s);
                break;
            }
        }
    }
    match previous {
        Some(p) => {
            let _ = cb.set_text(p);
        }
        None => {
            let _ = cb.clear();
        }
    }
    got.filter(|s| !s.trim().is_empty())
}

pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

pub fn focus(hwnd: isize) {
    unsafe {
        let _ = SetForegroundWindow(HWND(hwnd as _));
    }
}

/// (exe name in lowercase, window title) of the focused window.
pub fn foreground_app() -> (String, String) {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title);
        let title = String::from_utf16_lossy(&title[..n.max(0) as usize]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut exe = String::new();
        if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size).is_ok() {
                let path = String::from_utf16_lossy(&buf[..size as usize]);
                exe = path.rsplit('\\').next().unwrap_or_default().to_lowercase();
            }
            let _ = CloseHandle(h);
        }
        (exe, title)
    }
}

/// Pick a writing style from the focused app.
pub fn style_for(exe: &str, title: &str) -> &'static str {
    let t = title.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    if matches!(exe, "outlook.exe" | "olk.exe" | "thunderbird.exe" | "hxoutlook.exe")
        || has(&["gmail", "outlook", "inbox", "- mail", "compose", "proton mail"])
    {
        crate::llm::STYLE_EMAIL
    } else if has(&["jira", "linear", "asana", "trello", "clickup", "github", "gitlab", "azure devops", "monday.com", "notion"]) {
        crate::llm::STYLE_TICKET
    } else if matches!(exe, "ms-teams.exe" | "teams.exe" | "slack.exe" | "whatsapp.exe" | "telegram.exe" | "discord.exe" | "signal.exe")
        || has(&["whatsapp", "telegram", "slack", "microsoft teams", "discord", "messenger"])
    {
        crate::llm::STYLE_CHAT
    } else {
        ""
    }
}

pub fn friendly_app_name(exe: &str, title: &str) -> String {
    let base = exe.trim_end_matches(".exe");
    let name = match base {
        "olk" | "outlook" | "hxoutlook" => "Outlook",
        "ms-teams" | "teams" => "Teams",
        "chrome" => "Chrome",
        "msedge" => "Edge",
        "firefox" => "Firefox",
        "winword" => "Word",
        "code" => "VS Code",
        "slack" => "Slack",
        "whatsapp" => "WhatsApp",
        "telegram" => "Telegram",
        "notepad" => "Notepad",
        "" => "",
        other => other,
    };
    // For browsers, the site in the title is more useful than the browser name.
    if matches!(base, "chrome" | "msedge" | "firefox" | "brave" | "arc") {
        if let Some(site) = title.rsplit(" - ").nth(1) {
            return site.chars().take(28).collect();
        }
    }
    name.to_string()
}

/// Make an overlay window never take focus and let clicks pass through it.
/// Turn the pill window into a bare overlay: no title bar or frame (some systems drew a "Seda pill"
/// caption and a grey box), never takes focus, clicks pass through.
/// Frameless, never takes focus, not in Alt+Tab. click_through: clicks pass to the window below.
pub fn make_overlay(hwnd: isize, click_through: bool) {
    unsafe {
        let h = HWND(hwnd as _);
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        let strip = (WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0 | WS_BORDER.0 | WS_DLGFRAME.0) as isize;
        SetWindowLongPtrW(h, GWL_STYLE, (style & !strip) | WS_POPUP.0 as isize);
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let flags = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | if click_through { WS_EX_TRANSPARENT.0 } else { 0 }) as isize;
        let drop = (WS_EX_WINDOWEDGE.0 | WS_EX_CLIENTEDGE.0 | WS_EX_DLGMODALFRAME.0) as isize;
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex & !drop) | flags);
        let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Native rounded corners (Windows 11). Used instead of a clip region: clipping a WebView2 window
/// can stop it from appearing on screen at all.
pub fn round_corners(hwnd: isize) {
    use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            HWND(hwnd as _),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&pref) as u32,
        );
        // No 1px light window border around the pill (DWMWA_COLOR_NONE).
        let none: u32 = 0xFFFF_FFFE;
        let _ = DwmSetWindowAttribute(HWND(hwnd as _), DWMWA_BORDER_COLOR, &none as *const _ as *const std::ffi::c_void, 4);
    }
}

pub fn show_no_activate(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_SHOWNOACTIVATE);
    }
}

pub fn hide(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_HIDE);
    }
}
