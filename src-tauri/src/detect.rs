//! Call detection. Windows keeps a list of every app that has used the microphone, with
//! "started using" / "stopped using" times (the same data behind the mic icon in the taskbar).
//! An app whose stop time is 0 is using the mic right now. If that app is a call app, a call
//! has probably started.
use serde::Serialize;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_QWORD, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::core::{PCWSTR, PWSTR};

pub struct CallApp {
    pub id: &'static str,
    pub name: &'static str,
    /// Executable names (lowercase) for classic desktop apps.
    exes: &'static [&'static str],
    /// Package family name prefixes for Store apps.
    packages: &'static [&'static str],
    /// Install locations (with %ENV% variables) used to tell whether it's installed.
    paths: &'static [&'static str],
    /// Counts as a call app until the user says otherwise.
    pub default_on: bool,
}

pub const APPS: &[CallApp] = &[
    CallApp { id: "teams", name: "Microsoft Teams", exes: &["ms-teams.exe", "teams.exe"], packages: &["msteams_"], paths: &["%LOCALAPPDATA%\\Microsoft\\Teams", "%LOCALAPPDATA%\\Packages\\MSTeams_8wekyb3d8bbwe"], default_on: true },
    CallApp { id: "zoom", name: "Zoom", exes: &["zoom.exe"], packages: &[], paths: &["%APPDATA%\\Zoom\\bin\\Zoom.exe", "%PROGRAMFILES%\\Zoom\\bin\\Zoom.exe"], default_on: true },
    CallApp { id: "browser", name: "Google Meet & browser calls", exes: &["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe", "arc.exe", "vivaldi.exe"], packages: &["thebrowsercompany.arc_"], paths: &["%PROGRAMFILES%\\Google\\Chrome\\Application\\chrome.exe", "%PROGRAMFILES(X86)%\\Microsoft\\Edge\\Application\\msedge.exe", "%PROGRAMFILES%\\Mozilla Firefox\\firefox.exe"], default_on: true },
    CallApp { id: "webex", name: "Webex", exes: &["webex.exe", "ciscocollabhost.exe", "webexmta.exe", "atmgr.exe"], packages: &[], paths: &["%LOCALAPPDATA%\\CiscoSparkLauncher", "%PROGRAMFILES%\\Cisco Spark"], default_on: true },
    CallApp { id: "slack", name: "Slack huddles", exes: &["slack.exe"], packages: &["91750d7e.slack_"], paths: &["%LOCALAPPDATA%\\slack"], default_on: true },
    CallApp { id: "gotomeeting", name: "GoTo Meeting", exes: &["g2mcomm.exe", "g2mstart.exe", "goto.exe"], packages: &[], paths: &["%LOCALAPPDATA%\\GoTo Meeting", "%LOCALAPPDATA%\\Programs\\GoTo"], default_on: true },
    CallApp { id: "ringcentral", name: "RingCentral", exes: &["ringcentral.exe"], packages: &[], paths: &["%LOCALAPPDATA%\\Programs\\RingCentral"], default_on: true },
    CallApp { id: "discord", name: "Discord", exes: &["discord.exe"], packages: &[], paths: &["%LOCALAPPDATA%\\Discord"], default_on: false },
    CallApp { id: "whatsapp", name: "WhatsApp", exes: &["whatsapp.exe"], packages: &["5319275a.whatsappdesktop_"], paths: &["%LOCALAPPDATA%\\Packages\\5319275A.WhatsAppDesktop_cv1g1gvanyjgm"], default_on: false },
    CallApp { id: "telegram", name: "Telegram", exes: &["telegram.exe"], packages: &["telegrammessengerllp.telegramdesktop_"], paths: &["%APPDATA%\\Telegram Desktop"], default_on: false },
];

const STORE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open(parent: HKEY, path: &str) -> Option<HKEY> {
    let mut h = HKEY::default();
    let p = wide(path);
    (unsafe { RegOpenKeyExW(parent, PCWSTR(p.as_ptr()), Some(0), KEY_READ, &mut h) } == ERROR_SUCCESS).then_some(h)
}

fn subkeys(h: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0.. {
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let r = unsafe { RegEnumKeyExW(h, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) };
        if r != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

fn qword(h: HKEY, sub: &str, value: &str) -> Option<u64> {
    let (s, v) = (wide(sub), wide(value));
    let mut data = 0u64;
    let mut size = 8u32;
    let r = unsafe {
        RegGetValueW(h, PCWSTR(s.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_QWORD, None, Some(&mut data as *mut u64 as *mut _), Some(&mut size))
    };
    (r == ERROR_SUCCESS).then_some(data)
}

/// Every app Windows has seen using the mic: (key name, using it right now).
/// Key names are package family names, or exe paths with '#' for '\' under "NonPackaged".
fn mic_users() -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let Some(root) = open(HKEY_CURRENT_USER, STORE) else { return out };
    let mut scan = |h: HKEY, names: Vec<String>| {
        for n in names {
            if n == "NonPackaged" {
                continue;
            }
            let start = qword(h, &n, "LastUsedTimeStart").unwrap_or(0);
            let stop = qword(h, &n, "LastUsedTimeStop").unwrap_or(1);
            out.push((n.to_lowercase(), start != 0 && stop == 0));
        }
    };
    scan(root, subkeys(root));
    if let Some(np) = open(root, "NonPackaged") {
        scan(np, subkeys(np));
        unsafe { let _ = RegCloseKey(np); }
    }
    unsafe { let _ = RegCloseKey(root); }
    out
}

fn matches(app: &CallApp, key: &str) -> bool {
    match key.rsplit_once('#') {
        Some((_, exe)) => app.exes.contains(&exe),
        None => app.packages.iter().any(|p| key.starts_with(p)),
    }
}

/// Test mode only: pretend this app (an id from APPS) is on a call.
pub static TEST_ACTIVE: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

/// Call apps using the microphone right now (Seda itself never counts).
pub fn active() -> Vec<&'static CallApp> {
    let users: Vec<String> = mic_users().into_iter().filter(|(k, on)| *on && !k.ends_with("#seda.exe")).map(|(k, _)| k).collect();
    let test = TEST_ACTIVE.lock().clone();
    APPS.iter().filter(|a| users.iter().any(|k| matches(a, k)) || test.as_deref() == Some(a.id)).collect()
}

/// For "seda --calls": what Windows reports, to check detection on a PC.
pub fn report() -> String {
    let mut s = String::from("Call apps:\n");
    for a in apps() {
        s += &format!("  {:<28} {}\n", a.name, if a.installed { "installed" } else { "-" });
    }
    s += "Apps Windows has seen using the mic (* = right now):\n";
    for (k, on) in mic_users() {
        s += &format!("  {} {}\n", if on { "*" } else { " " }, k);
    }
    s
}

pub fn by_id(id: &str) -> Option<&'static CallApp> {
    APPS.iter().find(|a| a.id == id)
}

#[derive(Serialize)]
pub struct AppInfo {
    id: &'static str,
    name: &'static str,
    installed: bool,
}

/// The call apps, and whether each is on this PC (installed, or seen using the mic before).
pub fn apps() -> Vec<AppInfo> {
    let seen: Vec<String> = mic_users().into_iter().map(|(k, _)| k).collect();
    let expand = |p: &str| {
        let mut s = p.to_string();
        for var in ["LOCALAPPDATA", "APPDATA", "PROGRAMFILES(X86)", "PROGRAMFILES"] {
            s = s.replace(&format!("%{var}%"), &std::env::var(var).unwrap_or_default());
        }
        s
    };
    APPS.iter()
        .map(|a| AppInfo {
            id: a.id,
            name: a.name,
            installed: seen.iter().any(|k| matches(a, k)) || a.paths.iter().any(|p| std::path::Path::new(&expand(p)).exists()),
        })
        .collect()
}
