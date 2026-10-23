//! In-app updates from the GitHub releases. Every update is signed with Seda's private key; the
//! updater refuses anything whose signature doesn't match the public key in tauri.conf.json, so
//! not even someone with access to the GitHub account could push a fake update.
use parking_lot::Mutex;
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

/// The newest version found by the last check (shown in the sidebar until installed).
static AVAILABLE: Mutex<Option<UpdateInfo>> = Mutex::new(None);

pub fn available() -> Option<UpdateInfo> {
    AVAILABLE.lock().clone()
}

fn updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    #[allow(unused_mut)]
    let mut b = app.updater_builder();
    // Test builds can point at a test release instead of the real one.
    #[cfg(feature = "test-hooks")]
    if let Ok(url) = std::env::var("SEDA_UPDATE_URL") {
        b = b.endpoints(vec![url.parse().map_err(|e| format!("{e}"))?]).map_err(|e| e.to_string())?;
    }
    b.build().map_err(|e| e.to_string())
}

/// Ask GitHub whether there's a newer version. Never downloads anything by itself.
pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let found = updater(app)?.check().await.map_err(|e| e.to_string())?;
    let info = found.map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone().unwrap_or_default() });
    if let Some(i) = &info {
        crate::log::log(format!("update available: {}", i.version));
        let _ = app.emit("update-available", i);
    }
    *AVAILABLE.lock() = info.clone();
    Ok(info)
}

/// Check now, and again every 6 hours while Seda runs.
pub fn spawn_checker(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20)); // let the models load first
        loop {
            if let Err(e) = tauri::async_runtime::block_on(check(&app)) {
                crate::log::log(format!("update check failed: {e}"));
            }
            std::thread::sleep(Duration::from_secs(6 * 3600));
        }
    });
}

/// Download, verify the signature, install and restart. Progress goes out as "update-progress".
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let Some(update) = updater(app)?.check().await.map_err(|e| e.to_string())? else {
        return Err("Seda is already up to date".into());
    };
    crate::log::log(format!("installing update {}", update.version));
    let progress = app.clone();
    let mut done = 0u64;
    update
        .download_and_install(
            move |chunk, total| {
                done += chunk as u64;
                let _ = progress.emit("update-progress", serde_json::json!({ "done": done, "total": total }));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // On Windows the installer takes over from here and starts the new version when it's done.
    app.restart();
}
