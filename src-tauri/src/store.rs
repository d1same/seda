//! SQLite storage: dictation history, meetings, personal vocabulary, learned fixes.
//! Corrected history entries that still have their audio double as voice-training data.
use parking_lot::Mutex;
use rusqlite::{OptionalExtension, params, Connection};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Store {
    conn: Mutex<Connection>,
    /// The search index failed once: said in the log, then quiet (search just finds nothing).
    fts_warned: AtomicBool,
}

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
    /// Other people from the Outlook appointment, joined with " | ".
    #[serde(default)]
    pub people: String,
    /// Calendar subject, kept so a search still finds it after the notes rename the meeting.
    #[serde(default)]
    pub cal: String,
    /// The note template chosen for this meeting ('' = never chosen, behaves as Auto).
    #[serde(default)]
    pub template: String,
    /// JSON: {"parts": [...], "until": secs}. The part notes behind the final notes, kept so a
    /// resumed meeting only needs notes for its new stretch.
    #[serde(skip)]
    pub parts: String,
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
        Self::from_connection(Connection::open(crate::settings::data_dir().join("seda.db"))?)
    }

    /// Migrates the database to the current schema. Every step is idempotent, so a 0.3.0 database
    /// opens too; a failure to create the search index is logged and never fails the open.
    pub fn from_connection(conn: Connection) -> anyhow::Result<Self> {
        // Deleted dictations and meetings are overwritten in the file, not just unlinked.
        conn.execute_batch("PRAGMA secure_delete = ON;")?;
        // A meeting checkpoint and a dictation may write at the same moment: wait, don't drop one.
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
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
        for col in ["my_notes", "chat", "parts", "people", "cal", "template"] {
            let _ = conn.execute(&format!("ALTER TABLE meetings ADD COLUMN {col} TEXT DEFAULT ''"), []);
        }
        // While the meeting is recording: the time (Unix seconds) of its last checkpoint; 0 when
        // finished. A row whose checkpoints stopped more than a minute ago was left behind by a crash.
        let _ = conn.execute("ALTER TABLE meetings ADD COLUMN live INTEGER DEFAULT 0", []);
        // Full-text search over meetings: one row per meeting (rowid = meetings.id). Contentless:
        // the indexed text is the folded form (search::fold), and snippets come from the original.
        if let Err(e) = conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS meeting_fts USING fts5(
                title, people, notes, said, joined,
                content='', contentless_delete=1,
                tokenize=\"unicode61 remove_diacritics 2\");",
        ) {
            crate::log::log(format!("search index unavailable: {e}"));
        }
        if let Err(e) = conn.execute_batch("CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY, v TEXT NOT NULL);") {
            crate::log::log(format!("kv table unavailable: {e}"));
        }
        Ok(Self { conn: Mutex::new(conn), fts_warned: AtomicBool::new(false) })
    }

    /// A failed index write is said once in the log and swallowed: it must never fail a save.
    fn fts_failed(&self, what: &str, e: impl std::fmt::Display) {
        if !self.fts_warned.swap(true, Ordering::SeqCst) {
            crate::log::log(format!("search index: {what} failed ({e})"));
        }
    }

    /// Index one meeting again, in its own transaction. Called with the connection lock held.
    fn reindex(&self, c: &Connection, id: i64) {
        let r = c.unchecked_transaction().and_then(|tx| {
            index_row(&tx, id)?;
            tx.commit()
        });
        if let Err(e) = r {
            self.fts_failed("update", e);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_history(&self, app: &str, style: &str, raw: &str, cleaned: &str, audio: Option<&str>, secs: f64, asr_ms: i64, llm_ms: i64) -> i64 {
        let c = self.conn.lock();
        let _ = c.execute(
            "INSERT INTO history(ts,app,style,raw,cleaned,audio,secs,asr_ms,llm_ms) VALUES(?,?,?,?,?,?,?,?,?)",
            params![chrono::Local::now().to_rfc3339(), app, style, raw, cleaned, audio, secs, asr_ms, llm_ms],
        );
        c.last_insert_rowid()
    }

    pub fn history(&self, limit: i64) -> Vec<HistoryItem> {
        let c = self.conn.lock();
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
        let c = self.conn.lock();
        if let Ok(Some(path)) = c.query_row("SELECT audio FROM history WHERE id=?", [id], |r| r.get::<_, Option<String>>(0)) {
            let _ = std::fs::remove_file(path);
        }
        let _ = c.execute("DELETE FROM history WHERE id=?", [id]);
    }

    /// Save the user's corrected text and learn word-level substitutions from it.
    pub fn correct(&self, id: i64, text: &str) -> Vec<(String, String)> {
        let c = self.conn.lock();
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

    /// A meeting that just started (live until `set_meeting_live(id, false)`).
    pub fn begin_meeting(&self, ts: &str) -> i64 {
        let c = self.conn.lock();
        let _ = c.execute(
            "INSERT INTO meetings(ts,secs,title,transcript,notes,my_notes,chat,live) VALUES(?,0,'','[]','','','[]',strftime('%s','now'))",
            params![ts],
        );
        c.last_insert_rowid()
    }

    /// A resumed meeting records again.
    pub fn set_meeting_live(&self, id: i64, live: bool) {
        let sql = if live { "UPDATE meetings SET live=strftime('%s','now') WHERE id=?" } else { "UPDATE meetings SET live=0 WHERE id=?" };
        let _ = self.conn.lock().execute(sql, [id]);
    }

    /// Written every few seconds while recording, so a crash loses at most that much.
    pub fn checkpoint_meeting(&self, id: i64, secs: f64, transcript: &str, my_notes: &str, parts: &str) {
        let _ = self
            .conn
            .lock()
            .execute("UPDATE meetings SET secs=?, transcript=?, my_notes=?, parts=?, live=strftime('%s','now') WHERE id=?", params![secs, transcript, my_notes, parts, id]);
    }

    /// Meetings still marked as recording, with the seconds since their last checkpoint.
    pub fn live_meetings(&self) -> Vec<(i64, i64)> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT id, strftime('%s','now') - live FROM meetings WHERE live > 0").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect()
    }

    pub fn update_my_notes(&self, id: i64, my_notes: &str) {
        let c = self.conn.lock();
        let _ = c.execute("UPDATE meetings SET my_notes=? WHERE id=?", params![my_notes, id]);
        self.reindex(&c, id);
    }

    /// At Stop (also after resuming): the combined transcript, new length and notepad.
    pub fn update_meeting_recording(&self, id: i64, secs: f64, transcript: &str, my_notes: &str) {
        let c = self.conn.lock();
        let _ = c.execute("UPDATE meetings SET secs=?, transcript=?, my_notes=? WHERE id=?", params![secs, transcript, my_notes, id]);
        self.reindex(&c, id);
    }

    pub fn set_meeting_parts(&self, id: i64, parts: &str) {
        let _ = self.conn.lock().execute("UPDATE meetings SET parts=? WHERE id=?", params![parts, id]);
    }

    pub fn set_meeting_notes(&self, id: i64, title: &str, notes: &str) {
        let c = self.conn.lock();
        let _ = c.execute("UPDATE meetings SET title=?, notes=? WHERE id=?", params![title, notes, id]);
        self.reindex(&c, id);
    }

    pub fn set_meeting_meta(&self, id: i64, people: &str, cal: &str) {
        let c = self.conn.lock();
        let _ = c.execute("UPDATE meetings SET people=?, cal=? WHERE id=?", params![people, cal, id]);
        self.reindex(&c, id);
    }

    pub fn set_meeting_template(&self, id: i64, template: &str) {
        let _ = self.conn.lock().execute("UPDATE meetings SET template=? WHERE id=?", params![template, id]);
    }

    pub fn meeting_template(&self, id: i64) -> String {
        self.conn.lock().query_row("SELECT COALESCE(template,'') FROM meetings WHERE id=?", [id], |r| r.get(0)).unwrap_or_default()
    }

    /// The template of the latest other meeting with this calendar subject that chose one.
    pub fn template_for_subject(&self, cal: &str, not_id: i64) -> Option<String> {
        if cal.trim().is_empty() {
            return None;
        }
        self.conn
            .lock()
            .query_row(
                "SELECT template FROM meetings WHERE cal=?1 AND id<>?2 AND COALESCE(template,'')<>'' ORDER BY id DESC LIMIT 1",
                params![cal, not_id],
                |r| r.get(0),
            )
            .ok()
    }

    /// True while the row is marked as recording (here or in a copy of Seda that crashed).
    pub fn is_live(&self, id: i64) -> bool {
        self.conn.lock().query_row("SELECT COALESCE(live,0) FROM meetings WHERE id=?", [id], |r| r.get::<_, i64>(0)).is_ok_and(|l| l > 0)
    }

    /// Newest first. The list leaves out transcript and notes (they can be long).
    pub fn meetings(&self) -> Vec<Meeting> {
        let c = self.conn.lock();
        let mut st = c
            .prepare("SELECT id,ts,secs,title,COALESCE(people,''),COALESCE(cal,''),COALESCE(template,'') FROM meetings WHERE COALESCE(live,0)=0 ORDER BY id DESC")
            .unwrap();
        st.query_map([], |r| {
            Ok(Meeting {
                id: r.get(0)?, ts: r.get(1)?, secs: r.get(2)?, title: r.get(3)?, transcript: String::new(), notes: String::new(),
                my_notes: String::new(), chat: String::new(), parts: String::new(), people: r.get(4)?, cal: r.get(5)?, template: r.get(6)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    pub fn meeting(&self, id: i64) -> Option<Meeting> {
        self.conn
            .lock()
            .query_row("SELECT id,ts,secs,title,transcript,notes,COALESCE(my_notes,''),COALESCE(chat,'[]'),COALESCE(parts,''),COALESCE(people,''),COALESCE(cal,''),COALESCE(template,'') FROM meetings WHERE id=?", [id], |r| {
                Ok(Meeting {
                    id: r.get(0)?, ts: r.get(1)?, secs: r.get(2)?, title: r.get(3)?, transcript: r.get(4)?, notes: r.get(5)?,
                    my_notes: r.get(6)?, chat: r.get(7)?, parts: r.get(8)?, people: r.get(9)?, cal: r.get(10)?, template: r.get(11)?,
                })
            })
            .ok()
    }

    pub fn set_meeting_chat(&self, id: i64, chat: &str) {
        let _ = self.conn.lock().execute("UPDATE meetings SET chat=? WHERE id=?", params![chat, id]);
    }

    pub fn delete_meeting(&self, id: i64) {
        let c = self.conn.lock();
        let _ = c.execute("DELETE FROM meetings WHERE id=?", [id]);
        if let Err(e) = c.execute("DELETE FROM meeting_fts WHERE rowid=?", [id]) {
            self.fts_failed("delete", e);
        }
    }

    /// Rewrite the index without the deleted meetings' words (a delete leaves a tombstone until
    /// segments merge; with secure_delete the freed pages are zeroed).
    pub fn compact_search(&self) {
        if let Err(e) = self.conn.lock().execute("INSERT INTO meeting_fts(meeting_fts) VALUES('optimize')", []) {
            self.fts_failed("optimize", e);
        }
    }

    /// First launch after an update (or after drift): index every meeting. Runs on its own thread;
    /// the lock is released between batches of 5 so a dictation never waits long for the store.
    pub fn backfill_search(&self) {
        let t = std::time::Instant::now();
        let (version, meetings, indexed) = {
            let c = self.conn.lock();
            let version: Option<String> = c.query_row("SELECT v FROM kv WHERE k='fts'", [], |r| r.get(0)).optional().unwrap_or(None);
            let meetings: i64 = c.query_row("SELECT count(*) FROM meetings", [], |r| r.get(0)).unwrap_or(0);
            let indexed = c.query_row("SELECT count(*) FROM meeting_fts_docsize", [], |r| r.get::<_, i64>(0));
            (version, meetings, indexed)
        };
        let indexed = match indexed {
            Ok(n) => n,
            Err(e) => return self.fts_failed("backfill", e),
        };
        if version.as_deref() == Some(crate::search::SEARCH_VERSION) && meetings == indexed {
            return;
        }
        let ids: Vec<i64> = {
            let c = self.conn.lock();
            if let Err(e) = c.execute("INSERT INTO meeting_fts(meeting_fts) VALUES('delete-all')", []) {
                return self.fts_failed("backfill", e);
            }
            let mut st = c.prepare("SELECT id FROM meetings ORDER BY id").unwrap();
            st.query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect()
        };
        for batch in ids.chunks(5) {
            let c = self.conn.lock();
            let r = c.unchecked_transaction().and_then(|tx| {
                for id in batch {
                    index_row(&tx, *id)?;
                }
                tx.commit()
            });
            if let Err(e) = r {
                return self.fts_failed("backfill", e);
            }
        }
        self.kv_set("fts", crate::search::SEARCH_VERSION);
        crate::log::log(format!("search index: {} meetings in {} ms", ids.len(), t.elapsed().as_millis()));
    }

    /// (id, bm25) best first, finished meetings only. bm25 is smaller-is-better. Weights: title 6,
    /// people 3, notes 4, said 1, joined 0.5.
    pub fn search_meetings(&self, match_expr: &str, limit: usize) -> Vec<(i64, f64)> {
        let c = self.conn.lock();
        let hits: Vec<(i64, f64)> = match c.prepare(
            "SELECT rowid, bm25(meeting_fts, 6.0, 3.0, 4.0, 1.0, 0.5) FROM meeting_fts WHERE meeting_fts MATCH ?1 ORDER BY 2 LIMIT ?2",
        ) {
            Ok(mut st) => match st.query_map(params![match_expr, limit as i64], |r| Ok((r.get(0)?, r.get(1)?))) {
                Ok(rows) => rows.filter_map(Result::ok).collect(),
                Err(e) => {
                    self.fts_failed("query", e);
                    return Vec::new();
                }
            },
            Err(e) => {
                self.fts_failed("query", e);
                return Vec::new();
            }
        };
        if hits.is_empty() {
            return hits;
        }
        // Live meetings (still recording, or being resumed) and deleted ones are not results.
        let list = hits.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>().join(",");
        let finished: std::collections::HashSet<i64> = c
            .prepare(&format!("SELECT id FROM meetings WHERE COALESCE(live,0)=0 AND id IN ({list})"))
            .and_then(|mut st| st.query_map([], |r| r.get(0)).map(|rows| rows.filter_map(Result::ok).collect()))
            .unwrap_or_default();
        hits.into_iter().filter(|(id, _)| finished.contains(id)).collect()
    }

    pub fn kv_get(&self, k: &str) -> Option<String> {
        self.conn.lock().query_row("SELECT v FROM kv WHERE k=?", [k], |r| r.get(0)).optional().unwrap_or(None)
    }

    pub fn kv_set(&self, k: &str, v: &str) {
        let r = self.conn.lock().execute("INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v", params![k, v]);
        if let Err(e) = r {
            crate::log::log(format!("kv write failed ({e})"));
        }
    }

    pub fn vocab(&self) -> Vec<String> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT term FROM vocab ORDER BY term").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect()
    }
    pub fn add_vocab(&self, term: &str) {
        let _ = self.conn.lock().execute("INSERT OR IGNORE INTO vocab(term) VALUES(?)", [term.trim()]);
    }
    pub fn remove_vocab(&self, term: &str) {
        let _ = self.conn.lock().execute("DELETE FROM vocab WHERE term=?", [term]);
    }

    pub fn fixes(&self) -> Vec<Fix> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT wrong,right,hits FROM fixes ORDER BY hits DESC, wrong").unwrap();
        st.query_map([], |r| Ok(Fix { wrong: r.get(0)?, right: r.get(1)?, hits: r.get(2)? }))
            .unwrap()
            .filter_map(Result::ok)
            .collect()
    }
    pub fn add_fix(&self, wrong: &str, right: &str) {
        let _ = self.conn.lock().execute(
            "INSERT INTO fixes(wrong,right,hits) VALUES(?,?,0) ON CONFLICT(wrong) DO UPDATE SET right=excluded.right",
            params![wrong.trim(), right.trim()],
        );
    }
    pub fn remove_fix(&self, wrong: &str) {
        let _ = self.conn.lock().execute("DELETE FROM fixes WHERE wrong=?", [wrong]);
    }

    /// Replace every learned wrong→right phrase (whole words only) and count the hits.
    pub fn apply_fixes(&self, text: &str) -> String {
        let fixes = self.fixes();
        let mut out = text.to_string();
        let c = self.conn.lock();
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
        let c = self.conn.lock();
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

/// Index one meeting (no transaction of its own: the caller holds one). The indexed text is folded;
/// `said` is only what was said, without times or speaker labels.
fn index_row(c: &Connection, id: i64) -> rusqlite::Result<()> {
    use crate::search::{fold, joined_compounds};
    c.execute("DELETE FROM meeting_fts WHERE rowid=?1", [id])?;
    let row = c
        .query_row(
            "SELECT COALESCE(title,''),COALESCE(cal,''),COALESCE(people,''),COALESCE(notes,''),COALESCE(my_notes,''),COALESCE(transcript,'') FROM meetings WHERE id=?",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?)),
        )
        .optional()?;
    let Some((title, cal, people, notes, mine, transcript)) = row else { return Ok(()) };
    let segs: Vec<crate::meeting::Segment> = serde_json::from_str(&transcript).unwrap_or_default();
    let said = segs.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
    let (title, people, notes) = (format!("{title}\n{cal}"), people.replace(" | ", "\n"), format!("{notes}\n{mine}"));
    let joined = joined_compounds(&[title.as_str(), people.as_str(), notes.as_str(), said.as_str()].join("\n"));
    c.execute(
        "INSERT INTO meeting_fts(rowid,title,people,notes,said,joined) VALUES(?1,?2,?3,?4,?5,?6)",
        params![id, fold(&title), fold(&people), fold(&notes), fold(&said), joined],
    )?;
    Ok(())
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

    fn segs(lines: &[(&str, &str)]) -> String {
        let v: Vec<crate::meeting::Segment> = lines
            .iter()
            .enumerate()
            .map(|(i, (who, text))| crate::meeting::Segment { who: who.to_string(), start: i as f32 * 5.0, end: i as f32 * 5.0 + 4.0, text: text.to_string() })
            .collect();
        serde_json::to_string(&v).unwrap()
    }

    fn find(store: &Store, q: &str) -> Vec<i64> {
        let e = crate::search::match_expr(q, crate::search::Mode::All).unwrap();
        store.search_meetings(&e, 10).into_iter().map(|h| h.0).collect()
    }

    #[test]
    fn search_on_a_new_database() {
        use crate::search::{match_expr, Mode};
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let id = store.begin_meeting("2026-11-11T09:44:02+03:30");
        let transcript = segs(&[("Sam", "the staging server is green"), ("Them", "ما این کار را می\u{200C}کنیم")]);
        store.update_meeting_recording(id, 1860.0, &transcript, "- ask Sam about Q3");
        // Still recording: not a result yet.
        assert!(find(&store, "staging").is_empty());
        store.set_meeting_live(id, false);
        store.set_meeting_notes(id, "برنامه کیفیت", "## Release\n- Thursday");
        assert_eq!(find(&store, "staging"), vec![id], "a word that was said");
        assert_eq!(find(&store, "میکنیم"), vec![id], "typed without the ZWNJ");
        assert_eq!(find(&store, "می\u{200C}کنیم"), vec![id], "typed with the ZWNJ");
        assert_eq!(find(&store, "كيفيت"), vec![id], "Arabic kaf and yeh");
        assert_eq!(find(&store, "thurs"), vec![id], "notes, by prefix");
        assert_eq!(find(&store, "q3"), vec![id], "my notes");
        assert!(find(&store, "budget").is_empty());
        // Whatever the user types, SQLite accepts the expression.
        for q in ["\"x\" OR (", "NOT", "AND OR NEAR", "a*b", "(", "'", "-staging", "v2.1", "NEAR(staging green)", "title:staging", "^green", "می\u{200C}کنیم کتابها"] {
            for mode in [Mode::All, Mode::Any] {
                if let Some(e) = match_expr(q, mode) {
                    store.search_meetings(&e, 10);
                }
            }
        }
        assert!(!store.fts_warned.load(Ordering::SeqCst), "a query was rejected");
        assert_eq!(store.search_meetings(&match_expr("what did we say about the staging server", Mode::Any).unwrap(), 10).len(), 1);
        // Renamed: the new title is found.
        store.set_meeting_notes(id, "Launch checklist", "");
        assert_eq!(find(&store, "checklist"), vec![id]);
        assert!(find(&store, "كيفيت").is_empty());
        store.delete_meeting(id);
        assert!(find(&store, "staging").is_empty());
        store.compact_search();
        assert!(!store.fts_warned.load(Ordering::SeqCst));
    }

    #[test]
    fn migrates_a_0_3_0_database_and_backfills() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE history(id INTEGER PRIMARY KEY, ts TEXT NOT NULL, app TEXT, style TEXT, raw TEXT, cleaned TEXT,
                corrected TEXT, audio TEXT, secs REAL, asr_ms INTEGER, llm_ms INTEGER);
             CREATE TABLE vocab(term TEXT PRIMARY KEY);
             CREATE TABLE fixes(wrong TEXT PRIMARY KEY, right TEXT NOT NULL, hits INTEGER DEFAULT 0);
             CREATE TABLE meetings(id INTEGER PRIMARY KEY, ts TEXT NOT NULL, secs REAL, title TEXT, transcript TEXT, notes TEXT,
                my_notes TEXT DEFAULT '', chat TEXT DEFAULT '', parts TEXT DEFAULT '', people TEXT DEFAULT '', cal TEXT DEFAULT '',
                live INTEGER DEFAULT 0);",
        )
        .unwrap();
        let t1 = segs(&[("Them", "the invoice is late again")]);
        let t2 = segs(&[("You", "nothing about money")]);
        conn.execute(
            "INSERT INTO meetings(ts,secs,title,transcript,notes,people,cal) VALUES('2026-11-10T10:00:00+03:30',600,'Budget review',?1,'## Budget\n- cut by 10%','Sam | Alex','Weekly sync')",
            [&t1],
        )
        .unwrap();
        conn.execute("INSERT INTO meetings(ts,secs,title,transcript,notes) VALUES('2026-11-11T10:00:00+03:30',300,'Invoice process',?1,'')", [&t2]).unwrap();
        let store = Store::from_connection(conn).unwrap();
        assert!(find(&store, "invoice").is_empty(), "nothing indexed before the backfill");
        store.backfill_search();
        assert_eq!(store.kv_get("fts").as_deref(), Some(crate::search::SEARCH_VERSION));
        // The title outweighs a word that was only said.
        assert_eq!(find(&store, "invoice"), vec![2, 1]);
        assert_eq!(find(&store, "alex"), vec![1]);
        assert_eq!(find(&store, "weekly"), vec![1]);
        let list = store.meetings();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|m| m.template.is_empty() && m.transcript.is_empty()));
        // A chosen template is kept, and a later meeting with the same subject finds it.
        store.set_meeting_template(1, "one_on_one");
        assert_eq!(store.meeting_template(1), "one_on_one");
        assert_eq!(store.meeting(1).unwrap().template, "one_on_one");
        assert_eq!(store.template_for_subject("Weekly sync", 3).as_deref(), Some("one_on_one"));
        assert_eq!(store.template_for_subject("Weekly sync", 1), None);
        assert_eq!(store.template_for_subject("", 3), None);
        // The ask thread is stored in kv.
        store.kv_set("ask_all", "[]");
        store.kv_set("ask_all", "[1]");
        assert_eq!(store.kv_get("ask_all").as_deref(), Some("[1]"));
        assert!(!store.fts_warned.load(Ordering::SeqCst));
    }

    #[test]
    fn a_broken_search_index_never_fails_open_or_a_save() {
        let conn = Connection::open_in_memory().unwrap();
        // A plain table under the index's name: the virtual table is never created, every index write fails.
        conn.execute_batch("CREATE TABLE meeting_fts(x);").unwrap();
        let store = Store::from_connection(conn).unwrap();
        let id = store.begin_meeting("2026-11-11T09:44:02+03:30");
        store.update_meeting_recording(id, 60.0, &segs(&[("Them", "the staging server is green")]), "- my line");
        store.set_meeting_live(id, false);
        store.set_meeting_notes(id, "Launch", "## Release\n- Thursday");
        store.update_my_notes(id, "- my line, edited");
        store.set_meeting_meta(id, "Sam", "Weekly sync");
        let m = store.meeting(id).unwrap();
        assert_eq!((m.title.as_str(), m.notes.as_str(), m.my_notes.as_str(), m.cal.as_str()), ("Launch", "## Release\n- Thursday", "- my line, edited", "Weekly sync"));
        store.backfill_search();
        assert!(find(&store, "staging").is_empty());
        store.compact_search();
        store.delete_meeting(id);
        assert!(store.meeting(id).is_none());
        assert!(store.fts_warned.load(Ordering::SeqCst));
        assert_eq!(store.kv_get("fts"), None);
    }

    #[test]
    fn replaces_whole_words_only() {
        assert_eq!(replace_words("cat category cat.", "cat", "dog"), ("dog category dog.".to_string(), 2));
    }
}
