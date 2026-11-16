//! Export: a meeting as a Markdown file in Documents\Seda\Meetings (one file per meeting, rewritten
//! on every export, so the folder works as an Obsidian vault), or on the clipboard as formatted
//! text plus Markdown.
use crate::store::Meeting;
use std::path::{Path, PathBuf};

pub struct What {
    pub notes: bool,
    pub mine: bool,
    pub transcript: bool,
}

impl What {
    pub const ALL: What = What { notes: true, mine: true, transcript: true };
}

/// Title, else calendar subject, else "Meeting".
fn name_of(m: &Meeting) -> String {
    [m.title.trim(), m.cal.trim()].into_iter().find(|t| !t.is_empty()).unwrap_or("Meeting").to_string()
}

fn people_of(m: &Meeting) -> Vec<&str> {
    m.people.split(" | ").map(str::trim).filter(|p| !p.is_empty()).collect()
}

fn minutes(m: &Meeting) -> i64 {
    if m.secs <= 0.0 { 0 } else { ((m.secs / 60.0).round() as i64).max(1) }
}

/// A YAML double-quoted string.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace(['\r', '\n'], " ").replace('\\', "\\\\").replace('"', "\\\""))
}

/// The notepad's headings nest under "## My notes": `#` becomes `###`, `##` becomes `####`.
fn demote(md: &str) -> String {
    md.trim()
        .lines()
        .map(|l| {
            let hashes = l.chars().take_while(|c| *c == '#').count();
            if hashes > 0 && l[hashes..].starts_with(' ') { format!("{}{}", "#".repeat((hashes + 2).min(6)), &l[hashes..]) } else { l.to_string() }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The meeting as Markdown. `me`: the user's name ("You" in the transcript is written as it);
/// `template`: the template's name for the front matter.
pub fn markdown(m: &Meeting, me: &str, template: &str, what: &What, front_matter: bool) -> String {
    let title = name_of(m);
    let ts = chrono::DateTime::parse_from_rfc3339(&m.ts).ok();
    let people = people_of(m);
    let mut out = String::new();
    if front_matter {
        // seda_id first: export_id reads only the head of the file, and a long list of people
        // could push it out of reach.
        out.push_str(&format!("---\nseda_id: {}\n", m.id));
        out.push_str(&format!("title: {}\n", quoted(&title)));
        if let Some(t) = ts {
            out.push_str(&format!("date: {}\n", t.format("%Y-%m-%dT%H:%M:%S%:z")));
        }
        out.push_str(&format!("duration_minutes: {}\n", minutes(m)));
        out.push_str(&format!("people: [{}]\n", people.iter().map(|p| quoted(p)).collect::<Vec<_>>().join(", ")));
        if !m.cal.trim().is_empty() {
            out.push_str(&format!("calendar: {}\n", quoted(m.cal.trim())));
        }
        if !template.is_empty() {
            out.push_str(&format!("template: {}\n", quoted(template)));
        }
        out.push_str("---\n");
    }
    out.push_str(&format!("# {title}\n\n"));
    let mut meta: Vec<String> = Vec::new();
    if let Some(t) = ts {
        meta.push(t.format("%A, %-d %B %Y").to_string());
        meta.push(t.format("%H:%M").to_string());
    }
    meta.push(format!("{} min", minutes(m)));
    if !people.is_empty() {
        meta.push(format!("with {}", people.join(", ")));
    }
    out.push_str(&format!("{}\n\n", meta.join(" · ")));
    if what.notes && !m.notes.trim().is_empty() {
        out.push_str(&format!("{}\n\n", m.notes.trim()));
    }
    if what.mine && !m.my_notes.trim().is_empty() {
        out.push_str(&format!("## My notes\n\n{}\n\n", demote(&m.my_notes)));
    }
    let segs: Vec<crate::meeting::Segment> = serde_json::from_str(&m.transcript).unwrap_or_default();
    if what.transcript && !segs.is_empty() {
        let me = me.trim();
        let lines: Vec<String> = segs
            .iter()
            .map(|s| {
                let who = if s.who == "You" && !me.is_empty() { me } else { s.who.as_str() };
                format!("[{:02}:{:02}] **{who}:** {}", s.start as u32 / 60, s.start as u32 % 60, s.text.trim())
            })
            .collect();
        out.push_str(&format!("## Transcript\n\n{}\n\n", lines.join("\n\n")));
    }
    format!("{}\n", out.trim_end())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Escaped text with **bold**.
fn inline(s: &str) -> String {
    let e = escape(s);
    let parts: Vec<&str> = e.split("**").collect();
    if parts.len() % 2 == 0 {
        return e; // unbalanced: leave the marks as they are
    }
    parts.iter().enumerate().map(|(i, p)| if i % 2 == 1 { format!("<b>{p}</b>") } else { p.to_string() }).collect()
}

/// ` dir="rtl"` when the first letter is Arabic script: Outlook's Word engine ignores dir="auto".
fn rtl(s: &str) -> &'static str {
    // A transcript line: the direction is the text's, not the speaker label's ("**Sam:** سلام").
    let s = s.split_once(":** ").map_or(s, |(_, rest)| rest);
    match s.chars().find(|c| c.is_alphabetic()) {
        Some(c) if ('\u{0600}'..='\u{06FF}').contains(&c) || ('\u{FB50}'..='\u{FEFF}').contains(&c) => " dir=\"rtl\"",
        _ => "",
    }
}

/// Exactly the Markdown Seda writes, as HTML for the clipboard: headings, "- " bullets nested by
/// two spaces, task boxes, **bold** and paragraphs. No classes; a gray meta line under the title.
pub fn html(md: &str) -> String {
    let mut out = String::new();
    // One entry per open <ul>: whether its last <li> is still open.
    let mut lists: Vec<bool> = Vec::new();
    let close_lists = |out: &mut String, lists: &mut Vec<bool>| {
        while let Some(open) = lists.pop() {
            if open {
                out.push_str("</li>");
            }
            out.push_str("</ul>");
        }
    };
    let mut after_title = false;
    for raw in md.lines() {
        let line = raw.trim_end();
        let text = line.trim_start();
        if text.is_empty() {
            continue;
        }
        let hashes = text.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && text[hashes..].starts_with(' ') {
            close_lists(&mut out, &mut lists);
            let t = text[hashes..].trim();
            out.push_str(&format!("<h{hashes}{}>{}</h{hashes}>", rtl(t), inline(t)));
            after_title = hashes == 1;
            continue;
        }
        if let Some(item) = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")) {
            let indent = line.len() - text.len();
            let depth = (indent / 2).min(lists.len());
            let item = match item.get(..4) {
                Some("[ ] ") => format!("☐ {}", &item[4..]),
                Some("[x] " | "[X] ") => format!("☑ {}", &item[4..]),
                _ => item.to_string(),
            };
            while lists.len() > depth + 1 {
                if lists.pop() == Some(true) {
                    out.push_str("</li>");
                }
                out.push_str("</ul>");
            }
            if lists.len() == depth + 1 {
                if lists[depth] {
                    out.push_str("</li>");
                }
            } else {
                out.push_str(&format!("<ul{}>", rtl(&item)));
                lists.push(false);
            }
            out.push_str(&format!("<li{}>{}", rtl(&item), inline(&item)));
            if let Some(last) = lists.last_mut() {
                *last = true;
            }
            after_title = false;
            continue;
        }
        close_lists(&mut out, &mut lists);
        let style = if after_title { " style=\"color:#666666\"" } else { "" };
        out.push_str(&format!("<p{}{style}>{}</p>", rtl(text), inline(text)));
        after_title = false;
    }
    close_lists(&mut out, &mut lists);
    out
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
    "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_direction_mark(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// "YYYY-MM-DD HHMM " of the meeting's start: the start of its file name.
fn stamp(m: &Meeting) -> String {
    chrono::DateTime::parse_from_rfc3339(&m.ts).map(|t| t.format("%Y-%m-%d %H%M ").to_string()).unwrap_or_else(|_| "0000-00-00 0000 ".into())
}

/// `2026-11-11 0944 Release plan and budget.md`. Characters Windows forbids, control characters,
/// commas (Explorer can't select a path with one) and direction marks become spaces; at most 80
/// characters of title; no trailing dots or spaces; a reserved device name gets a leading `_`.
pub fn file_name(m: &Meeting) -> String {
    let cleaned: String = name_of(m).chars().map(|c| if "<>:\"/\\|?*,".contains(c) || c.is_control() || is_direction_mark(c) { ' ' } else { c }).collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = cleaned.chars().take(80).collect();
    let mut title = cut.trim_end_matches(['.', ' ']).trim().to_string();
    if title.is_empty() {
        title = "Meeting".into();
    }
    let base = title.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    if RESERVED.contains(&base.as_str()) {
        title = format!("_{title}");
    }
    format!("{}{title}.md", stamp(m))
}

/// Documents\Seda\Meetings (Documents follows a move into OneDrive), or the data folder.
pub fn dir() -> PathBuf {
    match dirs::document_dir() {
        Some(d) => d.join("Seda").join("Meetings"),
        None => crate::settings::data_dir().join("exports"),
    }
}

/// True when the folder syncs to OneDrive (the window says so: nothing else leaves the PC).
pub fn is_cloud(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str().to_string_lossy().starts_with("OneDrive"))
}

/// The seda_id in an exported file's front matter (its first 2,000 bytes: a long calendar subject
/// comes before it).
fn export_id(path: &Path) -> Option<i64> {
    use std::io::Read;
    let mut head = [0u8; 2000];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).ok()?;
    String::from_utf8_lossy(&head[..n]).lines().find_map(|l| l.trim().strip_prefix("seda_id:").and_then(|v| v.trim().parse().ok()))
}

/// Write the meeting's file (notes, my notes, transcript, front matter) and return its path.
pub fn write(m: &Meeting, me: &str, template: &str) -> anyhow::Result<PathBuf> {
    write_in(&dir(), m, me, template)
}

fn write_in(dir: &Path, m: &Meeting, me: &str, template: &str) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = file_name(m);
    let stem = name.strip_suffix(".md").unwrap_or(&name).to_string();
    // A file with this name that belongs to another meeting (or the user) is left alone.
    let mut target = dir.join(&name);
    let mut k = 2;
    while target.exists() && export_id(&target) != Some(m.id) {
        target = dir.join(format!("{stem} ({k}).md"));
        k += 1;
    }
    // This meeting exported earlier under another title: removed, so the folder holds one copy.
    let prefix = stamp(m);
    if let Ok(files) = std::fs::read_dir(dir) {
        for f in files.flatten() {
            let p = f.path();
            let n = f.file_name().to_string_lossy().to_string();
            if p != target && n.starts_with(&prefix) && n.ends_with(".md") && export_id(&p) == Some(m.id) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    let tmp = PathBuf::from(format!("{}.tmp", target.display()));
    std::fs::write(&tmp, markdown(m, me, template, &What::ALL, true))?;
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::anyhow!("Couldn't save {}: {e}", target.display()));
    }
    Ok(target)
}

/// Open Explorer with the file selected. Explorer can't select a path with a comma in it (the
/// user's profile folder could have one): then it opens the folder.
pub fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    let s = path.display().to_string();
    if s.contains(',') {
        if let Some(d) = path.parent() {
            open_folder(d);
        }
        return;
    }
    // .arg() would quote the whole "/select,…" and Explorer would open Documents instead.
    let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{s}\"")).spawn();
}

pub fn open_folder(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(id: i64, title: &str) -> Meeting {
        let segs = r#"[{"who":"Them","start":2.0,"end":5.0,"text":"سلام، خوبین؟ بریم سراغ ریلیز این هفته."},{"who":"You","start":9.0,"end":12.0,"text":"Staging is green, all tests passed."}]"#;
        Meeting {
            id,
            ts: "2026-11-11T09:44:02.123+03:30".into(),
            secs: 1860.0,
            title: title.into(),
            transcript: segs.into(),
            notes: "## Release\n- Ships on **Thursday**\n  - QA signs off <today>".into(),
            my_notes: "# Release\n- release thursday?".into(),
            chat: String::new(),
            people: "Sam".into(),
            cal: "Release plan".into(),
            template: "one_on_one".into(),
            parts: String::new(),
        }
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name(&meeting(1, "Release plan and budget")), "2026-11-11 0944 Release plan and budget.md");
        assert_eq!(file_name(&meeting(1, "برنامه\u{200C}ی انتشار")), "2026-11-11 0944 برنامه\u{200C}ی انتشار.md");
        assert_eq!(file_name(&meeting(1, "a<b>c:d\"e/f\\g|h?i*j,k\u{200F}l\tm.. ")), "2026-11-11 0944 a b c d e f g h i j k l m.md");
        assert_eq!(file_name(&meeting(1, "CON")), "2026-11-11 0944 _CON.md");
        assert_eq!(file_name(&meeting(1, "nul.txt")), "2026-11-11 0944 _nul.txt.md");
        let long = file_name(&meeting(1, &"word ".repeat(40)));
        assert_eq!(long.trim_end_matches(".md").chars().count(), 16 + 79, "{long}"); // the cut lands after a space
        let mut untitled = meeting(1, "");
        assert!(file_name(&untitled).ends_with(" Release plan.md"));
        untitled.cal.clear();
        assert!(file_name(&untitled).ends_with(" Meeting.md"));
    }

    #[test]
    fn markdown_file() {
        let md = markdown(&meeting(42, "Release \"plan\" and budget"), "Alex", "1:1", &What::ALL, true);
        let expected = "---
seda_id: 42
title: \"Release \\\"plan\\\" and budget\"
date: 2026-11-11T09:44:02+03:30
duration_minutes: 31
people: [\"Sam\"]
calendar: \"Release plan\"
template: \"1:1\"
---
# Release \"plan\" and budget

Wednesday, 11 November 2026 · 09:44 · 31 min · with Sam

## Release
- Ships on **Thursday**
  - QA signs off <today>

## My notes

### Release
- release thursday?

## Transcript

[00:02] **Sam:** سلام، خوبین؟ بریم سراغ ریلیز این هفته.

[00:09] **Alex:** Staging is green, all tests passed.
";
        let expected = expected.replace("**Sam:** سلام", "**Them:** سلام");
        assert_eq!(md, expected);
        // Front matter: every line between the fences is `key: value`.
        let fm: Vec<&str> = md.lines().skip(1).take_while(|l| *l != "---").collect();
        assert_eq!(fm.len(), 7);
        assert!(fm.iter().all(|l| l.split_once(": ").is_some_and(|(k, _)| k.chars().all(|c| c.is_ascii_lowercase() || c == '_'))));
        // The clipboard copy has no front matter; an empty notepad has no section.
        let mut m = meeting(42, "T");
        m.my_notes.clear();
        let notes = markdown(&m, "", "", &What { notes: true, mine: true, transcript: false }, false);
        assert!(notes.starts_with("# T\n\nWednesday"));
        assert!(!notes.contains("My notes") && !notes.contains("Transcript") && !notes.contains("seda_id"));
        assert!(markdown(&m, "", "", &What::ALL, false).contains("**You:** Staging"));
    }

    #[test]
    fn html_for_the_clipboard() {
        let h = html("# Title\n\nWednesday · 09:44\n\n## Release\n- Ships **Thursday**\n  - QA <today>\n  - [x] done\n- Next\n\nسلام دنیا\n\n- [ ] open");
        assert_eq!(
            h,
            "<h1>Title</h1><p style=\"color:#666666\">Wednesday · 09:44</p><h2>Release</h2><ul><li>Ships <b>Thursday</b><ul><li>QA &lt;today&gt;</li><li>☑ done</li></ul></li><li>Next</li></ul><p dir=\"rtl\">سلام دنیا</p><ul><li>☐ open</li></ul>"
        );
        assert_eq!(html("- تست\n- **bold"), "<ul dir=\"rtl\"><li dir=\"rtl\">تست</li><li>**bold</li></ul>");
        assert_eq!(html("[00:02] **Sam:** سلام"), "<p dir=\"rtl\">[00:02] <b>Sam:</b> سلام</p>");
    }

    #[test]
    fn writes_one_file_per_meeting() {
        let dir = std::env::temp_dir().join(format!("seda-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = write_in(&dir, &meeting(42, "Release plan"), "", "").unwrap();
        assert!(first.ends_with("2026-11-11 0944 Release plan.md"));
        // Exported again under a new title: still one file.
        let second = write_in(&dir, &meeting(42, "Release plan and budget"), "", "").unwrap();
        assert!(!first.exists() && second.exists());
        // Again with the same title: overwritten in place.
        assert_eq!(write_in(&dir, &meeting(42, "Release plan and budget"), "", "").unwrap(), second);
        // Another meeting in the same minute with the same title: " (2)".
        let other = write_in(&dir, &meeting(43, "Release plan and budget"), "", "").unwrap();
        assert!(other.ends_with("2026-11-11 0944 Release plan and budget (2).md"), "{}", other.display());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        assert_eq!(export_id(&other), Some(43));
        // Fifty people with long names: the id is still found, so the file is replaced, not copied.
        let mut crowded = meeting(44, "Planning");
        crowded.people = (0..50).map(|i| format!("Participant number {i} from the partner company")).collect::<Vec<_>>().join(" | ");
        let p1 = write_in(&dir, &crowded, "", "").unwrap();
        assert_eq!(write_in(&dir, &crowded, "", "").unwrap(), p1);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(is_cloud(Path::new("C:\\Users\\x\\OneDrive - Contoso\\Documents")));
        assert!(!is_cloud(Path::new("C:\\Users\\x\\Documents")));
    }
}
