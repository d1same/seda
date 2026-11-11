use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A trigger the user can bind: a mouse button (1 = back/X1, 2 = forward/X2, 3 = middle)
/// or a keyboard key (Windows virtual-key code) with modifiers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Binding {
    pub kind: String, // "mouse" | "key"
    pub code: u32,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub win: bool,
}

/// When the dictation contains `say`, that phrase is replaced with `paste` before it is typed.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Snippet {
    pub say: String,
    pub paste: String,
}

impl Binding {
    pub fn mouse(code: u32) -> Self {
        Self { kind: "mouse".into(), code, ctrl: false, shift: false, alt: false, win: false }
    }
    pub fn key(code: u32, ctrl: bool, shift: bool, alt: bool) -> Self {
        Self { kind: "key".into(), code, ctrl, shift, alt, win: false }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub dictate_binding: Binding,
    pub dictate_mode: String, // "both" | "hold" | "toggle"
    pub fix_binding: Binding,
    pub language: String, // "fa" | "en" | "auto"
    pub cleanup: bool,
    pub app_styles: bool,
    pub latin_tech_terms: bool,
    pub save_audio: bool,
    pub sounds: bool,
    pub pill_position: String, // "bottom" | "top"
    pub idle_dot: bool,        // show a circle while idle: click to dictate, drag to move
    /// Where the user left that circle, in physical pixels. Empty = bottom centre of the screen.
    #[serde(default)]
    pub dot_x: Option<i32>,
    #[serde(default)]
    pub dot_y: Option<i32>,
    pub autostart: bool,       // start with Windows (quietly, in the tray)
    pub input_device: String,  // microphone name; "" = Windows default
    pub polish: String,        // "smart" (writing AI only when needed) | "always"; `cleanup` = off switch
    pub whisper_model: String,
    pub llm_server: String,
    pub llm_model: String,
    pub onboarded: bool,
    /// When a call app starts using the mic: "ask" (pop-up), "auto" (record right away) or "off".
    pub meeting_detect: String,
    /// Ids from detect::APPS that count as calls.
    pub meeting_apps: Vec<String>,
    /// What Seda calls the user: greeting, "You" in transcripts, owner of tasks in meeting notes.
    pub user_name: String,
    /// Pasted when you say "my email".
    pub email: String,
    /// Spoken phrase → the text that replaces it. "my email" is covered by `email`.
    pub snippets: Vec<Snippet>,
    /// Save meeting audio as WAV files. Off: like Granola, audio is transcribed live and never stored.
    pub keep_meeting_audio: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dictate_binding: Binding::mouse(1),
            dictate_mode: "both".into(),
            fix_binding: Binding::key(0x20, true, true, false), // Ctrl+Shift+Space
            language: "en".into(), // English only until setup adds a language
            cleanup: true,
            app_styles: true,
            latin_tech_terms: true,
            save_audio: true,
            sounds: true,
            pill_position: "bottom".into(),
            idle_dot: false,
            dot_x: None,
            dot_y: None,
            autostart: true,
            input_device: String::new(),
            polish: "smart".into(),
            // Filled in by setup (or setup::reconcile) with the models actually on this PC.
            whisper_model: String::new(),
            llm_server: String::new(),
            llm_model: String::new(),
            onboarded: false,
            meeting_detect: "ask".into(),
            meeting_apps: crate::detect::APPS.iter().filter(|a| a.default_on).map(|a| a.id.to_string()).collect(),
            user_name: String::new(),
            email: String::new(),
            snippets: Vec::new(),
            keep_meeting_audio: false,
        }
    }
}

pub fn data_dir() -> PathBuf {
    let d = dirs::data_dir().unwrap_or_default().join("Seda");
    let _ = std::fs::create_dir_all(d.join("audio"));
    d
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(data_dir().join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::write(data_dir().join("settings.json"), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
