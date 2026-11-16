//! Search normalisation for meetings: pure functions, no I/O.
//! One rule: whatever is indexed and whatever is queried goes through `fold()`. If `fold()` ever
//! changes, bump `SEARCH_VERSION` so the index is rebuilt on the next launch.
use chrono::{DateTime, Duration, Local};
use std::collections::HashSet;
use std::sync::LazyLock;

pub const SEARCH_VERSION: &str = "1";

/// The search form of a text: lower case, Persian and Arabic letter forms unified, harakat and
/// invisible marks removed, ZWNJ as a space, Persian and Arabic digits as 0-9.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // ZWNJ: می‌کنم is indexed as می + کنم (this also works as a stemmer: تصمیم‌ها finds تصمیم).
            '\u{200C}' => out.push(' '),
            // Harakat, superscript alef, tatweel: unicode61 would split the word at them.
            '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{0640}' => {}
            // Invisible joiners and direction marks.
            '\u{200B}' | '\u{200D}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{00AD}' => {}
            '\u{064A}' | '\u{0649}' | '\u{0626}' => out.push('\u{06CC}'),
            '\u{0643}' => out.push('\u{06A9}'),
            '\u{0629}' | '\u{06C0}' | '\u{06D5}' | '\u{06BE}' | '\u{06C1}' => out.push('\u{0647}'),
            '\u{0623}' | '\u{0625}' | '\u{0671}' | '\u{0622}' => out.push('\u{0627}'),
            '\u{0624}' => out.push('\u{0648}'),
            '\u{06F0}'..='\u{06F9}' => out.push(char::from(b'0' + (c as u32 - 0x06F0) as u8)),
            '\u{0660}'..='\u{0669}' => out.push(char::from(b'0' + (c as u32 - 0x0660) as u8)),
            _ => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// Every word that contains ZWNJ, with the ZWNJ removed, folded, space-separated. Indexed in the
/// `joined` column so `میکنم` typed without the ZWNJ still finds `می‌کنم`.
pub fn joined_compounds(s: &str) -> String {
    s.split_whitespace()
        .filter(|w| w.contains('\u{200C}'))
        .map(|w| fold(&w.replace('\u{200C}', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// The list: every word must match, as a prefix.
    All,
    /// Ask: any word may match; stopwords dropped, English words stemmed.
    Any,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Intent {
    NextSteps,
    Decisions,
    Open,
}

const MAX_WORDS: usize = 12;
/// Raw words read from a question before its stopwords are dropped.
const MAX_ASK_WORDS: usize = 40;

const STOP_EN: &[&str] = &[
    "the", "a", "an", "of", "to", "in", "on", "at", "by", "for", "from", "with", "about", "and", "or", "is", "are", "was", "were", "be",
    "been", "do", "does", "did", "have", "has", "had", "what", "when", "where", "who", "whom", "which", "how", "why", "we", "i", "you",
    "they", "he", "she", "it", "me", "us", "my", "our", "your", "their", "that", "this", "there", "any", "all", "said", "say", "tell",
    "talk", "talked", "discuss", "discussed", "meeting", "meetings",
];
const STOP_FA: &[&str] = &[
    "و", "در", "به", "از", "که", "را", "رو", "با", "این", "آن", "اون", "برای", "تا", "چه", "چی", "کی", "کجا", "چرا", "چطور", "من", "ما",
    "تو", "شما", "او", "هم", "یا", "اما", "ولی", "است", "هست", "بود", "شد", "شده", "کرد", "کردیم", "گفت", "گفتیم", "درباره", "مورد",
    "جلسه", "آیا", "هر", "همه", "یک",
];
/// Not stopwords for search, but they carry no topic in a question to Ask ("What is still open
/// this week?"): left out of `content_words` so such a question reads the recent meetings.
const FILLER: &[&str] = &[
    "still", "yet", "again", "also", "just", "now", "left", "please", "ever", "so", "far", "up", "can", "could", "would", "should",
    "will", "need", "today", "yesterday", "week", "weeks", "month", "last", "past", "recent", "recently", "lately", "latest",
    "هنوز", "الان", "امروز", "دیروز", "هفته", "ماه", "پیش", "قبل", "گذشته", "اخیر", "اخیرا", "تازگی",
];
const INTENT_NEXT: &[&str] = &[
    "agree", "agreed", "promise", "promised", "todo", "to-do", "task", "tasks", "action", "actions", "owe", "follow", "باید", "قول", "کار",
    "وظیفه",
];
const INTENT_DECISIONS: &[&str] = &["decide", "decided", "decision", "decisions", "تصمیم"];
const INTENT_OPEN: &[&str] = &["open", "unresolved", "pending", "unanswered", "باز", "مانده"];

fn folded_set(lists: &[&[&str]]) -> HashSet<String> {
    lists.iter().flat_map(|l| l.iter()).map(|w| fold(w)).collect()
}

static STOPWORDS: LazyLock<HashSet<String>> = LazyLock::new(|| folded_set(&[STOP_EN, STOP_FA]));
static NOT_CONTENT: LazyLock<HashSet<String>> =
    LazyLock::new(|| folded_set(&[STOP_EN, STOP_FA, FILLER, INTENT_NEXT, INTENT_DECISIONS, INTENT_OPEN]));

fn is_arabic_letter(c: char) -> bool {
    ('\u{0600}'..='\u{06FF}').contains(&c) && c.is_alphabetic()
}

fn alnum_count(s: &str) -> usize {
    s.chars().filter(|c| c.is_alphanumeric()).count()
}

/// More Arabic-script letters than ASCII letters.
pub fn mostly_persian(s: &str) -> bool {
    let (mut arabic, mut latin) = (0usize, 0usize);
    for c in s.chars() {
        if is_arabic_letter(c) {
            arabic += 1;
        } else if c.is_ascii_alphabetic() {
            latin += 1;
        }
    }
    arabic > latin
}

/// The raw words of a query: whitespace-split (ZWNJ stays inside its word), quotes removed,
/// punctuation at both ends trimmed, at most `max`.
fn raw_words(query: &str, max: usize) -> Vec<String> {
    query
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|w| !w.is_empty())
        .take(max)
        .collect()
}

/// A Persian word split at one common affix: `میکنم` → `می کنم`, `کتابها` → `کتاب ها`. Only when at
/// least two letters remain. The table is deliberately this small.
fn affix_split(w: &str) -> Option<String> {
    let letters = |s: &str| s.chars().filter(|c| c.is_alphabetic()).count();
    for p in ["نمی", "می"] {
        if let Some(rest) = w.strip_prefix(p) {
            if letters(rest) >= 2 {
                return Some(format!("{p} {rest}"));
            }
        }
    }
    for s in ["هایی", "های", "ها", "ترین", "تر"] {
        if let Some(rest) = w.strip_suffix(s) {
            if letters(rest) >= 2 {
                return Some(format!("{rest} {s}"));
            }
        }
    }
    None
}

/// English: one trailing ing, ed, es or s, then one trailing e, each only when 4+ letters remain.
/// `decide`, `decided` and `deciding` all become `decid`.
fn stem(w: &str) -> String {
    let mut s = w.to_string();
    for suf in ["ing", "ed", "es", "s"] {
        if s.ends_with(suf) && s.len() - suf.len() >= 4 {
            s.truncate(s.len() - suf.len());
            break;
        }
    }
    if s.ends_with('e') && s.len() > 4 {
        s.pop();
    }
    s
}

/// Folded alternatives of one raw word (at most two).
fn alternatives(raw: &str) -> Vec<String> {
    if raw.contains('\u{200C}') {
        return dedup(vec![fold(raw), fold(&raw.replace('\u{200C}', ""))]);
    }
    let w = fold(raw);
    if w.chars().any(is_arabic_letter) {
        if let Some(split) = affix_split(&w) {
            return vec![w, split];
        }
    }
    vec![w]
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Each query word as its list of folded alternatives, after the mode's filters.
fn query_terms(query: &str, mode: Mode) -> Vec<Vec<String>> {
    let words: Vec<String> = match mode {
        Mode::All => raw_words(query, MAX_WORDS),
        Mode::Any => {
            // Questions put the topic last: drop the stopwords before the cap, not after.
            let words = raw_words(query, MAX_ASK_WORDS);
            let kept: Vec<String> = words.iter().filter(|w| !STOPWORDS.contains(&fold(w))).cloned().collect();
            // A question of nothing but stopwords keeps them all.
            let kept = if kept.is_empty() { words } else { kept };
            kept.into_iter().take(MAX_WORDS).collect()
        }
    };
    let mut out = Vec::new();
    for w in words {
        let mut alts: Vec<String> = alternatives(&w);
        if mode == Mode::Any {
            alts = dedup(alts.into_iter().map(|a| if a.chars().all(|c| c.is_ascii_alphabetic()) { stem(&a) } else { a }).collect());
        }
        // Nothing searchable left ("(", "-") or a 1-character word: dropped. An empty phrase in
        // an AND would match nothing at all.
        alts.retain(|a| alnum_count(a) >= 2);
        if !alts.is_empty() {
            out.push(alts);
        }
    }
    out
}

/// An FTS5 MATCH string, or None when nothing is searchable. Every alternative is a double-quoted
/// FTS5 string, so no query can be a syntax error.
pub fn match_expr(query: &str, mode: Mode) -> Option<String> {
    let groups: Vec<String> = query_terms(query, mode)
        .into_iter()
        .map(|alts| {
            let quoted: Vec<String> = alts
                .iter()
                .map(|a| {
                    let star = match mode {
                        Mode::All => alnum_count(a) >= 2,
                        Mode::Any => a.chars().count() >= 4,
                    };
                    format!("\"{a}\"{}", if star { "*" } else { "" })
                })
                .collect();
            if quoted.len() == 1 { quoted.into_iter().next().unwrap_or_default() } else { format!("({})", quoted.join(" OR ")) }
        })
        .collect();
    if groups.is_empty() {
        return None;
    }
    Some(groups.join(match mode {
        Mode::All => " AND ",
        Mode::Any => " OR ",
    }))
}

/// Folded words and their variants, for snippets and scoring.
pub fn terms(query: &str) -> Vec<String> {
    dedup(query_terms(query, Mode::All).into_iter().flatten().collect())
}

/// The last `n` characters of `s`.
fn tail(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

/// [words before, the matched word as written, up to 90 characters after], for the first word of
/// `line` that starts with a term (or the first word of a phrase variant the line contains).
pub fn snippet(line: &str, terms: &[String]) -> Option<[String; 3]> {
    if terms.is_empty() || line.trim().is_empty() {
        return None;
    }
    let folded_line = fold(line);
    let singles: Vec<&str> = terms.iter().filter(|t| !t.contains(' ')).map(String::as_str).collect();
    // A phrase variant ("می کنم") the line contains: its first word marks the spot.
    let heads: Vec<&str> =
        terms.iter().filter(|t| t.contains(' ') && folded_line.contains(t.as_str())).filter_map(|t| t.split(' ').next()).collect();
    let mut start = None;
    for (i, c) in line.char_indices() {
        let space = c.is_whitespace();
        match (start, space) {
            (None, false) => start = Some(i),
            (Some(s), true) => {
                if let Some(hit) = word_hit(line, s, i, &singles, &heads) {
                    return Some(hit);
                }
                start = None;
            }
            _ => {}
        }
    }
    start.and_then(|s| word_hit(line, s, line.len(), &singles, &heads))
}

fn word_hit(line: &str, s: usize, e: usize, singles: &[&str], heads: &[&str]) -> Option<[String; 3]> {
    let word = &line[s..e];
    let keep = |c: char| c.is_alphanumeric() || c == '\u{200C}';
    let lead = word.len() - word.trim_start_matches(|c: char| !keep(c)).len();
    let core = word.trim_matches(|c: char| !keep(c));
    if core.is_empty() {
        return None;
    }
    let (s, e) = (s + lead, s + lead + core.len());
    let fw = fold(core);
    let subs: Vec<&str> = fw.split(|c: char| !c.is_alphanumeric()).filter(|x| !x.is_empty()).collect();
    let joined = fold(&core.replace('\u{200C}', ""));
    let hit = singles.iter().any(|t| fw.starts_with(t) || joined.starts_with(t) || subs.iter().any(|x| x.starts_with(t)))
        || heads.iter().any(|h| subs.iter().any(|x| x.starts_with(h)));
    if !hit {
        return None;
    }
    let after: String = line[e..].chars().take(90).collect();
    Some([tail(&line[..s], 60), line[s..e].to_string(), after])
}

/// Folded words of a text (split at anything that is not a letter or digit).
fn folded_words(s: &str) -> Vec<String> {
    fold(s).split(|c: char| !(c.is_alphanumeric() || c == '-')).map(|w| w.trim_matches('-')).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

fn local_midnight(now: DateTime<Local>) -> DateTime<Local> {
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| d.and_local_timezone(Local).earliest())
        .unwrap_or_else(|| now - Duration::hours(24))
}

/// The time range a question names ("this week", "دیروز"…), both ends inclusive.
pub fn date_cue(question: &str, now: DateTime<Local>) -> Option<(DateTime<Local>, DateTime<Local>)> {
    let q = format!(" {} ", folded_words(question).join(" "));
    let has = |cues: &[&str]| cues.iter().any(|c| q.contains(&format!(" {} ", fold(c))));
    let midnight = local_midnight(now);
    let days = Duration::days;
    if has(&["last week", "هفته پیش", "هفته قبل", "هفته گذشته"]) {
        return Some((now - days(14), now - days(7)));
    }
    if has(&["this week", "این هفته"]) {
        return Some((now - days(7), now));
    }
    if has(&["yesterday", "دیروز"]) {
        return Some((midnight - days(1), midnight));
    }
    if has(&["today", "امروز"]) {
        return Some((midnight, now));
    }
    if has(&["this month", "این ماه"]) {
        return Some((now - days(31), now));
    }
    if has(&["recent", "recently", "lately", "latest", "اخیر", "اخیرا", "تازگی"]) {
        return Some((now - days(14), now));
    }
    None
}

/// What kind of answer the question wants, from its first intent word. Used to prefer the
/// matching section of the notes.
pub fn intent(question: &str) -> Option<Intent> {
    static TABLE: LazyLock<Vec<(HashSet<String>, Intent)>> = LazyLock::new(|| {
        vec![
            (folded_set(&[INTENT_NEXT]), Intent::NextSteps),
            (folded_set(&[INTENT_DECISIONS]), Intent::Decisions),
            (folded_set(&[INTENT_OPEN]), Intent::Open),
        ]
    });
    folded_words(question).iter().find_map(|w| TABLE.iter().find(|(set, _)| set.contains(w)).map(|(_, i)| *i))
}

/// The words of a question that name a topic: folded, without stopwords, intent words and time
/// words, in order, without repeats.
pub fn content_words(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in folded_words(question) {
        for part in w.split('-').filter(|p| p.chars().count() >= 2) {
            if !NOT_CONTENT.contains(&w) && !NOT_CONTENT.contains(part) && !out.iter().any(|o| o == part) {
                out.push(part.to_string());
            }
        }
    }
    out
}

fn digit(c: char) -> Option<u32> {
    match c {
        '0'..='9' => Some(c as u32 - '0' as u32),
        '\u{06F0}'..='\u{06F9}' => Some(c as u32 - 0x06F0),
        '\u{0660}'..='\u{0669}' => Some(c as u32 - 0x0660),
        _ => None,
    }
}

/// Numbers inside one citation bracket starting after `open`: "1, 3" or "۲". Returns them and the
/// index of the closing bracket.
fn parse_cite(chars: &[char], mut i: usize, close: char) -> Option<(Vec<usize>, usize)> {
    let mut nums = Vec::new();
    loop {
        while chars.get(i).is_some_and(|c| *c == ' ') {
            i += 1;
        }
        let (mut n, mut len) = (0usize, 0);
        while let Some(d) = chars.get(i).and_then(|c| digit(*c)) {
            n = n * 10 + d as usize;
            len += 1;
            i += 1;
            if len > 3 {
                return None;
            }
        }
        if len == 0 {
            return None;
        }
        nums.push(n);
        while chars.get(i).is_some_and(|c| *c == ' ') {
            i += 1;
        }
        match chars.get(i) {
            Some(c) if *c == close => return Some((nums, i)),
            Some(',' | '،') => i += 1,
            _ => return None,
        }
    }
}

/// An answer's citations in canonical form: `[1, 3]`, `[۱،۳]` and `【2】` become `[1][3]` and
/// `[2]`; numbers outside 1..=n are removed. Round brackets are left alone ("(2)" can be text).
/// Returns the text and the cited numbers in order of first use.
pub fn citations(text: &str, n: usize) -> (String, Vec<usize>) {
    let chars: Vec<char> = text.chars().collect();
    let (mut out, mut cited) = (String::with_capacity(text.len()), Vec::new());
    let mut i = 0;
    while i < chars.len() {
        let close = match chars[i] {
            '[' => Some(']'),
            '\u{3010}' => Some('\u{3011}'),
            _ => None,
        };
        if let Some((nums, end)) = close.and_then(|cl| parse_cite(&chars, i + 1, cl)) {
            let valid: Vec<usize> = nums.into_iter().filter(|k| (1..=n).contains(k)).collect();
            if valid.is_empty() {
                // A dropped citation takes the space before it along.
                if out.ends_with(' ') {
                    out.pop();
                }
            }
            for k in valid {
                out.push_str(&format!("[{k}]"));
                if !cited.contains(&k) {
                    cited.push(k);
                }
            }
            i = end + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, cited)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_unifies_persian_forms() {
        // Arabic kaf, ZWNJ: the same words as with a space.
        assert_eq!(fold("می\u{200C}كنم"), fold("می کنم"));
        // Harakat and tanwin go.
        assert_eq!(fold("لطفاً"), "لطفا");
        let m = fold("معلّم");
        assert_eq!(m, "معلم");
        assert!(!m.contains(' '));
        assert_eq!(fold("۱۴۰۴"), "1404");
        assert_eq!(fold("٢٠٢٦"), "2026");
        // Arabic yeh and kaf become Persian ones.
        assert_eq!(fold("كيفيت"), "کیفیت");
        assert_eq!(fold("Release\u{200F}"), "release");
        assert_eq!(fold("آب"), "اب");
        assert_eq!(fold("مؤسسه"), "موسسه");
    }

    #[test]
    fn joined_compounds_lists_zwnj_words_only() {
        assert_eq!(joined_compounds("ما می\u{200C}کنیم و تصمیم\u{200C}ها"), "میکنیم تصمیمها");
        assert_eq!(joined_compounds("no compounds here"), "");
    }

    #[test]
    fn match_expr_list_mode() {
        let e = match_expr("میکنم", Mode::All).unwrap();
        assert!(e.contains("\"میکنم\"*"), "{e}");
        assert!(e.contains("\"می کنم\"*"), "{e}");
        let e = match_expr("کتابها", Mode::All).unwrap();
        assert!(e.contains("\"کتاب ها\"*"), "{e}");
        let e = match_expr("می\u{200C}کنم", Mode::All).unwrap();
        assert_eq!(e, "(\"می کنم\"* OR \"میکنم\"*)");
        assert_eq!(match_expr("rel budget", Mode::All).unwrap(), "\"rel\"* AND \"budget\"*");
        // Quotes and operators are only text.
        let e = match_expr("\"x\" OR (", Mode::All).unwrap();
        assert_eq!(e, "\"or\"*");
        assert_eq!(match_expr("a ( - ?", Mode::All), None);
        assert_eq!(match_expr("", Mode::All), None);
        assert_eq!(match_expr("v2.1", Mode::All).unwrap(), "\"v2.1\"*");
    }

    #[test]
    fn match_expr_ask_mode() {
        assert_eq!(match_expr("what did we decide", Mode::Any).unwrap(), "\"decid\"*");
        assert_eq!(match_expr("deciding", Mode::Any).unwrap(), "\"decid\"*");
        assert_eq!(match_expr("the release notes", Mode::Any).unwrap(), "\"releas\"* OR \"note\"*");
        // A long question keeps its topic, which comes last.
        let e = match_expr("Can you tell me what was said in the meeting with Sam about the migration to the new billing provider", Mode::Any).unwrap();
        assert!(e.contains("\"migration\"*") && e.contains("\"bill\"*") && e.contains("\"provider\"*"), "{e}");
        // Only stopwords: they are kept.
        assert!(match_expr("what was it", Mode::Any).is_some());
        // Persian stopwords go, with or without the folded alef.
        assert_eq!(match_expr("آیا درباره بودجه", Mode::Any).unwrap(), "\"بودجه\"*");
    }

    #[test]
    fn terms_and_snippets() {
        let t = terms("rel");
        assert_eq!(t, vec!["rel".to_string()]);
        let s = snippet("Sam: the release moves to Thursday, says Alex.", &t).unwrap();
        assert_eq!(s, ["Sam: the ".to_string(), "release".to_string(), " moves to Thursday, says Alex.".to_string()]);
        // Punctuation stays outside the mark.
        let s = snippet("We ship (Thursday).", &terms("thurs")).unwrap();
        assert_eq!(s[1], "Thursday");
        assert_eq!(s[2], ").");
        // Persian with ZWNJ, typed without it and with Arabic yeh.
        let line = "ما این کار را می\u{200C}کنیم فردا";
        assert_eq!(snippet(line, &terms("میکنیم")).unwrap()[1], "می\u{200C}کنیم");
        assert_eq!(snippet(line, &terms("كنيم")).unwrap()[1], "می\u{200C}کنیم");
        assert!(snippet(line, &terms("budget")).is_none());
        // Long text is cut: 60 before, 90 after.
        let long = format!("{} needle {}", "x".repeat(100), "y".repeat(200));
        let s = snippet(&long, &terms("needle")).unwrap();
        assert_eq!(s[0].chars().count(), 60);
        assert_eq!(s[2].chars().count(), 90);
    }

    #[test]
    fn date_cues() {
        let now = Local::now();
        let (a, b) = date_cue("what did I promise this week", now).unwrap();
        assert_eq!(b, now);
        assert_eq!(b - a, Duration::days(7));
        let (a, b) = date_cue("What happened last week?", now).unwrap();
        assert_eq!((now - a, now - b), (Duration::days(14), Duration::days(7)));
        let (a, b) = date_cue("دیروز چی گفتیم؟", now).unwrap();
        assert_eq!(b - a, Duration::days(1));
        assert!(b <= now);
        assert!(date_cue("what about the release", now).is_none());
        assert!(date_cue("اخیراً چه تصمیمی گرفتیم", now).is_some());
    }

    #[test]
    fn citations_are_canonical() {
        assert_eq!(citations("Sam owns it [1, 3]. Cut agreed [۲].", 3), ("Sam owns it [1][3]. Cut agreed [2].".to_string(), vec![1, 3, 2]));
        assert_eq!(citations("Done \u{3010}2\u{3011} and [1،2].", 2).0, "Done [2] and [1][2].");
        assert_eq!(citations("Made up [7]. Mixed [1, 9].", 2), ("Made up. Mixed [1].".to_string(), vec![1]));
        assert_eq!(citations("Keep (2) and [x] and [ 2 ]", 2).0, "Keep (2) and [x] and [2]");
        assert_eq!(citations("No citations.", 2), ("No citations.".to_string(), vec![]));
    }

    #[test]
    fn intents_and_content_words() {
        assert_eq!(intent("What did I agree to do this week?"), Some(Intent::NextSteps));
        assert_eq!(intent("What was decided about the budget?"), Some(Intent::Decisions));
        assert_eq!(intent("What is still open?"), Some(Intent::Open));
        assert_eq!(intent("چه تصمیم\u{200C}هایی گرفتیم"), Some(Intent::Decisions));
        assert_eq!(intent("Who is Sam?"), None);
        assert!(content_words("What is still open?").is_empty());
        assert!(content_words("What did I agree to do this week?").is_empty());
        assert_eq!(content_words("What did we decide about the release?"), vec!["release".to_string()]);
        assert_eq!(content_words("درباره بودجه چه تصمیمی گرفتیم"), vec!["بودجه".to_string(), "تصمیمی".to_string(), "گرفتیم".to_string()]);
        assert!(mostly_persian("بودجه و برنامه release"));
        assert!(!mostly_persian("Release plan بودجه"));
    }
}
