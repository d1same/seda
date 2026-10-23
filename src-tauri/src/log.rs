//! Tiny append-only log at %APPDATA%\Seda\seda.log, so problems are visible instead of silent.
use parking_lot::Mutex;
use std::io::Write;

static LOCK: Mutex<()> = Mutex::new(());
const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub fn log(msg: impl AsRef<str>) {
    let _g = LOCK.lock();
    let path = crate::settings::data_dir().join("seda.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), msg.as_ref());
    }
}
