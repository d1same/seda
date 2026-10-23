//! SQLite storage: dictation history, meetings, personal vocabulary, learned fixes.
//! Corrected history entries that still have their audio double as voice-training data.
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::Serialize;

pub struct Store(Mutex<Connection>);

#[derive(Serialize, Clone)]
pub struct HistoryItem {
    pub id: i64,
    pub ts: String,
    pub app: String,
    pub style: String,
    pub raw: String,
    pub cleaned: String,
    pub corrected: Option<String>,
    pub audio: Option<String>,
    pub secs: f64,
    pub asr_ms: i64,
    pub llm_ms: i64,
}

/// A meeting: the transcript is JSON (a list of segments), the notes are Markdown written by the AI.
#[derive(Serialize)]
pub struct Meeting {
    pub id: i64,
    pub ts: String,
    pub secs: f64,
    pub title: String,
    pub transcript: String,
    pub notes: String,
    /// What the user typed during the meeting.
    pub my_notes: String,
    /// JSON list of {role: "you"|"ai", text} — the Ask conversation.
    pub chat: String,
}

#[derive(Serialize)]
pub struct Fix {
    pub wrong: String,
    pub right: String,
    pub hits: i64,
}

#[derive(Serialize)]
pub struct TrainingStats {
    pub samples: i64,
    pub seconds: f64,
    pub total_dictations: i64,
}

impl Store {
    pub fn open() -> anyhow::Result<Self> {
        let conn = Connection::open(crate::settings::data_dir().join("seda.db"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history(
                id INTEGER PRIMARY KEY, ts TEXT NOT NULL, app TEXT, style TEXT, raw TEXT, cleaned TEXT,
                corrected TEXT, audio TEXT, secs REAL, asr_ms INTEGER, llm_ms INTEGER);
             CREATE TABLE IF NOT EXISTS vocab(term TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS fixes(wrong TEXT PRIMARY KEY, right TEXT NOT NULL, hits INTEGER DEFAULT 0);
             CREATE TABLE IF NOT EXISTS meetings(
                id INTEGER PRIMARY KEY, ts TEXT NOT NULL, secs REAL, title TEXT, transcript TEXT, notes TEXT);",
        )?;
        // Added after the first meetings release: older databases get the columns here.
        for col in ["my_notes", "chat"] {
            let _ = conn.execute(&format!("ALTER TABLE meetings ADD COLUMN {col} TEXT DEFAULT ''"), []);
        }
        Ok(Self(Mutex::new(conn)))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_history(&self, app: &str, style: &str, raw: &str, cleaned: &str, audio: Option<&str>, secs: f64, asr_ms: i64, llm_ms: i64) -> i64 {
        let c = self.0.lock();
        let _ = c.execute(
            "INSERT INTO history(ts,app,style,raw,cleaned,audio,secs,asr_ms,llm_ms) VALUES(?,?,?,?,?,?,?,?,?)",
            params![chrono::Local::now().to_rfc3339(), app, style, raw, cleaned, audio, secs, asr_ms, llm_ms],
        );
        c.last_insert_rowid()
    }

    pub fn history(&self, limit: i64) -> Vec<HistoryItem> {
        let c = self.0.lock();
        let mut st = c
            .prepare("SELECT id,ts,app,style,raw,cleaned,corrected,audio,secs,asr_ms,llm_ms FROM history ORDER BY id DESC LIMIT ?")
            .unwrap();
        st.query_map([limit], |r| {
            Ok(HistoryItem {
                id: r.get(0)?, ts: r.get(1)?, app: r.get(2)?, style: r.get(3)?, raw: r.get(4)?, cleaned: r.get(5)?,
                corrected: r.get(6)?, audio: r.get(7)?, secs: r.get(8)?, asr_ms: r.get(9)?, llm_ms: r.get(10)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    pub fn delete_history(&self, id: i64) {
        let c = self.0.lock();
        if let Ok(Some(path)) = c.query_row("SELECT audio FROM history WHERE id=?", [id], |r| r.get::<_, Option<String>>(0)) {
            let _ = std::fs::remove_file(path);
        }
        let _ = c.execute("DELETE FROM history WHERE id=?", [id]);
    }

    /// Save the user's corrected text and learn word-level substitutions from it.
    pub fn correct(&self, id: i64, text: &str) -> Vec<(String, String)> {
        let c = self.0.lock();
        let before: String = c.query_row("SELECT cleaned FROM history WHERE id=?", [id], |r| r.get(0)).unwrap_or_default();
        let _ = c.execute("UPDATE history SET corrected=? WHERE id=?", params![text, id]);
        let learned = diff_substitutions(&before, text);
        for (w, r) in &learned {
            let _ = c.execute(
                "INSERT INTO fixes(wrong,right,hits) VALUES(?,?,0) ON CONFLICT(wrong) DO UPDATE SET right=excluded.right",
                params![w, r],
            );
        }
        learned
    }

    pub fn add_meeting(&self, ts: &str, secs: f64, transcript: &str, my_notes: &str) -> i64 {
        let c = self.0.lock();
        let _ = c.execute(
            "INSERT INTO meetings(ts,secs,title,transcript,notes,my_notes,chat) VALUES(?,?,'',?,'',?,'[]')",
            params![ts, secs, transcript, my_notes],
        );
        c.last_insert_rowid()
    }

    pub fn set_meeting_notes(&self, id: i64, title: &str, notes: &str) {
        let _ = self.0.lock().execute("UPDATE meetings SET title=?, notes=? WHERE id=?", params![title, notes, id]);
    }

    /// Newest first. The list leaves out transcript and notes (they can be long).
    pub fn meetings(&self) -> Vec<Meeting> {
        let c = self.0.lock();
        let mut st = c.prepare("SELECT id,ts,secs,title FROM meetings ORDER BY id DESC").unwrap();
        st.query_map([], |r| {
            Ok(Meeting { id: r.get(0)?, ts: r.get(1)?, secs: r.get(2)?, title: r.get(3)?, transcript: String::new(), notes: String::new(), my_notes: String::new(), chat: String::new() })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    pub fn meeting(&self, id: i64) -> Option<Meeting> {
        self.0
            .lock()
            .query_row("SELECT id,ts,secs,title,transcript,notes,COALESCE(my_notes,''),COALESCE(chat,'[]') FROM meetings WHERE id=?", [id], |r| {
                Ok(Meeting {
                    id: r.get(0)?, ts: r.get(1)?, secs: r.get(2)?, title: r.get(3)?, transcript: r.get(4)?, notes: r.get(5)?,
                    my_notes: r.get(6)?, chat: r.get(7)?,
                })
            })
            .ok()
    }

    pub fn set_meeting_chat(&self, id: i64, chat: &str) {
        let _ = self.0.lock().execute("UPDATE meetings SET chat=? WHERE id=?", params![chat, id]);
    }

    pub fn delete_meeting(&self, id: i64) {
        let _ = self.0.lock().execute("DELETE FROM meetings WHERE id=?", [id]);
    }

    pub fn vocab(&self) -> Vec<String> {
        let c = self.0.lock();
        let mut st = c.prepare("SELECT term FROM vocab ORDER BY term").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect()
    }
    pub fn add_vocab(&self, term: &str) {
        let _ = self.0.lock().execute("INSERT OR IGNORE INTO vocab(term) VALUES(?)", [term.trim()]);
    }
    pub fn remove_vocab(&self, term: &str) {
        let _ = self.0.lock().execute("DELETE FROM vocab WHERE term=?", [term]);
    }

    pub fn fixes(&self) -> Vec<Fix> {
        let c = self.0.lock();
        let mut st = c.prepare("SELECT wrong,right,hits FROM fixes ORDER BY hits DESC, wrong").unwrap();
        st.query_map([], |r| Ok(Fix { wrong: r.get(0)?, right: r.get(1)?, hits: r.get(2)? }))
            .unwrap()
            .filter_map(Result::ok)
            .collect()
    }
    pub fn add_fix(&self, wrong: &str, right: &str) {
        let _ = self.0.lock().execute(
            "INSERT INTO fixes(wrong,right,hits) VALUES(?,?,0) ON CONFLICT(wrong) DO UPDATE SET right=excluded.right",
            params![wrong.trim(), right.trim()],
        );
    }
    pub fn remove_fix(&self, wrong: &str) {
        let _ = self.0.lock().execute("DELETE FROM fixes WHERE wrong=?", [wrong]);
    }

    /// Replace every learned wrong→right phrase (whole words only) and count the hits.
    pub fn apply_fixes(&self, text: &str) -> String {
        let fixes = self.fixes();
        let mut out = text.to_string();
        let c = self.0.lock();
        for f in fixes {
            let (next, n) = replace_words(&out, &f.wrong, &f.right);
            if n > 0 {
                out = next;
                let _ = c.execute("UPDATE fixes SET hits=hits+? WHERE wrong=?", params![n as i64, f.wrong]);
            }
        }
        out
    }

    pub fn training_stats(&self) -> TrainingStats {
        let c = self.0.lock();
        let (samples, seconds): (i64, f64) = c
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(secs),0) FROM history WHERE corrected IS NOT NULL AND audio IS NOT NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0.0));
        let total_dictations = c.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap_or(0);
        TrainingStats { samples, seconds, total_dictations }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\u{200c}' || c == '\''
}

/// Whole-word, case-sensitive replacement that works for Persian and English.
fn replace_words(text: &str, wrong: &str, right: &str) -> (String, usize) {
    if wrong.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut n = 0;
    let mut i = 0;
    while let Some(pos) = text[i..].find(wrong) {
        let start = i + pos;
        let end = start + wrong.len();
        let before_ok = text[..start].chars().next_back().is_none_or(|c| !is_word_char(c));
        let after_ok = text[end..].chars().next().is_none_or(|c| !is_word_char(c));
        out.push_str(&text[i..start]);
        if before_ok && after_ok {
            out.push_str(right);
            n += 1;
        } else {
            out.push_str(wrong);
        }
        i = end;
    }
    out.push_str(&text[i..]);
    (out, n)
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !is_word_char(c)).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Word-level LCS diff; every replaced run of 1–3 words becomes a (wrong, right) pair.
fn diff_substitutions(a: &str, b: &str) -> Vec<(String, String)> {
    let (a, b) = (words(a), words(b));
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    let (mut da, mut db): (Vec<&str>, Vec<&str>) = (vec![], vec![]);
    let flush = |da: &mut Vec<&str>, db: &mut Vec<&str>, out: &mut Vec<(String, String)>| {
        if !da.is_empty() && !db.is_empty() && da.len() <= 3 && db.len() <= 3 {
            out.push((da.join(" "), db.join(" ")));
        }
        da.clear();
        db.clear();
    };
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            flush(&mut da, &mut db, &mut out);
            i += 1;
            j += 1;
        } else if j < m && (i == n || dp[i][j + 1] >= dp[i + 1][j]) {
            db.push(&b[j]);
            j += 1;
        } else {
            da.push(&a[i]);
            i += 1;
        }
    }
    flush(&mut da, &mut db, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_substitutions() {
        let got = diff_substitutions("همه‌ی تزها پس شدن", "همه‌ی تست‌ها pass شدن");
        assert_eq!(got, vec![("تزها پس".to_string(), "تست‌ها pass".to_string())]);
    }

    #[test]
    fn replaces_whole_words_only() {
        assert_eq!(replace_words("cat category cat.", "cat", "dog"), ("dog category dog.".to_string(), 2));
    }
}
