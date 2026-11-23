//! Note templates: the sections Seda fills in when it writes a meeting's notes.
//! Built-ins are constants here; custom ones live in settings.json; a meeting stores only the id.
//! A template changes only the final step of the notes (the single pass, or the merge of the part
//! notes): the part notes written during a meeting never depend on it.
use crate::search::fold;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Section {
    pub heading: String,
    /// What belongs in the section (shown to the model, never copied into the notes).
    pub hint: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub about: String,
    pub sections: Vec<Section>,
}

#[derive(Serialize)]
pub struct TemplateInfo {
    #[serde(flatten)]
    pub t: Template,
    pub builtin: bool,
}

/// A section the code fills from the part notes of a long meeting.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Problem,
    Tried,
    Solution,
    Decisions,
    NextSteps,
    OpenQuestions,
}

pub enum Shape<'a> {
    /// Today's notes, unchanged.
    Auto,
    /// Auto, told to write a troubleshooting ticket.
    Ticket,
    /// The template's sections are the skeleton.
    Sections(&'a Template),
}

const MAX_TEMPLATES: usize = 20;
const MAX_SECTIONS: usize = 8;

fn tpl(id: &str, name: &str, about: &str, sections: &[(&str, &str)]) -> Template {
    Template {
        id: id.into(),
        name: name.into(),
        about: about.into(),
        sections: sections.iter().map(|(h, d)| Section { heading: (*h).into(), hint: (*d).into() }).collect(),
    }
}

pub fn builtins() -> Vec<Template> {
    vec![
        tpl("auto", "Auto", "", &[]),
        tpl(
            "phone_call",
            "Phone call",
            "A phone call or quick voice call. Capture concrete facts, numbers, dates, agreements, and follow-ups cleanly.",
            &[
                ("Context", "who was on the call, why they spoke, and any account, order, or reference details"),
                ("Key points", "main facts, updates, numbers, dates, and details discussed, grouped with sub-bullets"),
                ("Outcomes & agreements", "what was resolved, confirmed, or decided on the call"),
                ("Next steps", ""),
            ],
        ),
        tpl(
            "ticket",
            "Troubleshooting ticket",
            "A support or troubleshooting call. The notes are pasted into a ticket.",
            &[("Problem", "what is wrong"), ("What we tried", "each real step, in order"), ("Solution", "what fixed it, or what is still broken"), ("Next steps", "")],
        ),
        tpl(
            "one_on_one",
            "1:1",
            "A one-to-one between the user and one other person.",
            &[
                ("Updates", "what each person reported since last time: progress, results, numbers"),
                ("Feedback", "feedback given in either direction, with the example that was mentioned"),
                ("Blockers", "what is in the way, and who can unblock it"),
                ("Decisions", ""),
                ("Next steps", ""),
            ],
        ),
        tpl(
            "customer",
            "Customer call",
            "A call with a customer or a prospect.",
            &[
                ("Context", "who they are, their company, what they use today"),
                ("Needs", "what they want and why, in their words; numbers and deadlines"),
                ("Concerns", "objections, risks, and competitors they mentioned"),
                ("Decisions", ""),
                ("Next steps", ""),
            ],
        ),
        tpl(
            "interview",
            "Interview",
            "A job interview. The user is the interviewer unless the transcript shows otherwise.",
            &[
                ("Background", "experience and current role, as the candidate described it"),
                ("Strengths", "what they did well, each with the evidence from their answers"),
                ("Concerns", "gaps or weak answers, each with the evidence"),
                ("Candidate's questions", "what they asked and what they were told"),
                ("Next steps", ""),
            ],
        ),
    ]
}

/// Built-ins first, then the user's own.
pub fn all(custom: &[Template]) -> Vec<TemplateInfo> {
    builtins()
        .into_iter()
        .map(|t| TemplateInfo { t, builtin: true })
        .chain(custom.iter().cloned().map(|t| TemplateInfo { t, builtin: false }))
        .collect()
}

/// The template with this id; unknown or "" is Auto (a removed custom template reads as Auto).
pub fn resolve(custom: &[Template], id: &str) -> Template {
    let mut list = builtins();
    let auto = list.remove(0);
    if id.is_empty() {
        return auto;
    }
    list.into_iter().chain(custom.iter().cloned()).find(|t| t.id == id).unwrap_or(auto)
}

pub fn shape(t: &Template) -> Shape<'_> {
    match t.id.as_str() {
        "auto" => Shape::Auto,
        "ticket" => Shape::Ticket,
        _ => Shape::Sections(t),
    }
}

/// One line, without leading Markdown marks, at most `max` characters.
fn one_line(s: &str, max: usize) -> String {
    let s = s.replace(['\r', '\n', '\t'], " ");
    let s = s.trim().trim_start_matches(['#', '-', '*']).trim();
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(max).collect::<String>().trim().to_string()
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix("c_").is_some_and(|r| (1..=20).contains(&r.len()) && r.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

fn new_id(taken: &HashSet<String>) -> String {
    loop {
        let mut b = [0u8; 4];
        if getrandom::fill(&mut b).is_err() {
            let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7);
            b = n.to_le_bytes();
        }
        let id = format!("c_{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
        if !taken.contains(&id) {
            return id;
        }
    }
}

/// Custom templates as the window sent them, within the limits that protect the context budget:
/// at most 20 templates, 8 sections each; name 1–40 characters, about up to 300, heading 1–60,
/// hint up to 200; no newlines or leading Markdown marks; no duplicate headings; a valid unique id.
/// A template without a name or without a section is dropped.
pub fn clean(custom: Vec<Template>) -> Vec<Template> {
    let mut out: Vec<Template> = Vec::new();
    let mut ids: HashSet<String> = HashSet::new();
    for t in custom {
        if out.len() >= MAX_TEMPLATES {
            break;
        }
        let name = one_line(&t.name, 40);
        if name.is_empty() {
            continue;
        }
        let mut seen = HashSet::new();
        let mut sections = Vec::new();
        for s in t.sections {
            let heading = one_line(&s.heading, 60);
            if heading.is_empty() || !seen.insert(key(&heading)) {
                continue;
            }
            sections.push(Section { heading, hint: one_line(&s.hint, 200) });
            if sections.len() == MAX_SECTIONS {
                break;
            }
        }
        if sections.is_empty() {
            continue;
        }
        let id = if valid_id(&t.id) && !ids.contains(&t.id) { t.id } else { new_id(&ids) };
        ids.insert(id.clone());
        out.push(Template { id, name, about: one_line(&t.about, 300), sections });
    }
    out
}

/// How a heading is compared: cut at " — " or " – " (a copied hint), without `*`, `#` or a
/// trailing colon, folded, spaces collapsed.
pub fn key(heading: &str) -> String {
    let h = heading.split(" — ").next().unwrap_or("");
    let h = h.split(" – ").next().unwrap_or("");
    let h: String = h.chars().filter(|c| *c != '*' && *c != '#').collect();
    let h = h.trim().trim_end_matches([':', '：']).trim();
    fold(h).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Only names that cannot mean anything else: a custom "Steps" or "Questions" section stays the
/// template's own, written by the model, whatever the meeting's length.
const KINDS: &[(Kind, &[&str])] = &[
    (Kind::Problem, &["problem", "مشکل"]),
    (Kind::Tried, &["what we tried"]),
    (Kind::Solution, &["solution", "راه حل"]),
    (Kind::Decisions, &["decisions", "تصمیم ها", "تصمیمات"]),
    (Kind::NextSteps, &["next steps", "action items", "to-dos", "todos", "follow-ups", "اقدام ها", "کارهای بعدی", "قدم های بعدی"]),
    (Kind::OpenQuestions, &["open questions", "سوال های باز"]),
];

fn kind_of_key(k: &str) -> Option<Kind> {
    KINDS.iter().find(|(_, names)| names.iter().any(|n| key(n) == k)).map(|(kind, _)| *kind)
}

/// A section the code fills from the part notes, by its heading.
pub fn kind_of(heading: &str) -> Option<Kind> {
    kind_of_key(&key(heading))
}

/// The part-note headings to collect a kind's bullets from.
pub fn aliases(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Problem => &["problem"],
        Kind::Tried => &["what we tried", "steps"],
        Kind::Solution => &["solution"],
        Kind::Decisions => &["decisions"],
        Kind::NextSteps => &["next steps", "action items"],
        Kind::OpenQuestions => &["open questions"],
    }
}

/// The three list sections every template's notes may end with, and their default headings.
pub const DEFAULT_LISTS: [(Kind, &str); 3] = [(Kind::Decisions, "Decisions"), (Kind::NextSteps, "Next steps"), (Kind::OpenQuestions, "Open questions")];

/// The kinds the code places on the long path: the three lists, plus every kind the template has.
pub fn placed_kinds(t: &Template) -> Vec<Kind> {
    let mut v: Vec<Kind> = DEFAULT_LISTS.iter().map(|(k, _)| *k).collect();
    for s in &t.sections {
        if let Some(k) = kind_of(&s.heading) {
            if !v.contains(&k) {
                v.push(k);
            }
        }
    }
    v
}

/// The headings the code places on the long path (the model is told not to write them).
pub fn placed_headings(t: &Template) -> Vec<String> {
    let mut v: Vec<String> = t.sections.iter().filter(|s| kind_of(&s.heading).is_some()).map(|s| s.heading.clone()).collect();
    for (kind, heading) in DEFAULT_LISTS {
        if !t.sections.iter().any(|s| kind_of(&s.heading) == Some(kind)) {
            v.push(heading.to_string());
        }
    }
    v
}

pub const OTHER_NOTES: &str = "Other notes";

struct Part {
    key: String,
    heading: String,
    body: String,
}

/// The model's notes as (title, text before the first section, sections).
fn split_sections(md: &str) -> (Option<String>, String, Vec<Part>) {
    let (mut title, mut pre, mut parts) = (None, Vec::new(), Vec::<(String, Vec<&str>)>::new());
    for line in md.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            parts.push((h.to_string(), Vec::new()));
        } else if let Some(last) = parts.last_mut() {
            last.1.push(line);
        } else if let (None, Some(t)) = (&title, line.strip_prefix("# ")) {
            title = Some(t.trim().to_string());
        } else if !line.trim().is_empty() && !line.trim_start().starts_with("```") && !line.trim_end().ends_with(':') {
            // Text before the first section, without a lead-in such as "Here are the notes:".
            pre.push(line);
        }
    }
    let parts = parts
        .into_iter()
        .map(|(h, body)| {
            let shown = h.split(" — ").next().unwrap_or("").split(" – ").next().unwrap_or("").replace('*', "");
            let shown = shown.trim().trim_end_matches(':').trim().to_string();
            let body = body.into_iter().filter(|l| !l.trim_start().starts_with("```")).collect::<Vec<_>>().join("\n");
            Part { key: key(&h), heading: shown, body: body.trim_end().trim_start_matches(['\n', '\r']).to_string() }
        })
        .collect();
    (title, pre.join("\n"), parts)
}

/// The final notes of a template: its sections in its order with its own headings, then what the
/// model wrote that fits none of them, Other notes last. On the long path (`parts` not empty) the
/// list sections come from the part notes, so the merge cannot drop one.
pub fn assemble(t: &Template, llm_out: &str, parts: &[String]) -> String {
    let (title, pre, sections) = split_sections(llm_out);
    let long = !parts.is_empty();
    let placed = if long { placed_kinds(t) } else { Vec::new() };
    let mut used = vec![false; sections.len()];
    let mut out: Vec<String> = vec![format!("# {}", title.filter(|t| !t.is_empty()).unwrap_or_else(|| "Meeting".into()))];
    if !pre.trim().is_empty() {
        out.push(pre);
    }
    let push = |out: &mut Vec<String>, heading: &str, body: &str| {
        if !body.trim().is_empty() {
            out.push(format!("## {heading}\n{body}"));
        }
    };
    for s in &t.sections {
        let kind = kind_of(&s.heading);
        if let (Some(kind), true) = (kind, long) {
            push(&mut out, &s.heading, &crate::llm::collect_items(parts, aliases(kind)).join("\n"));
            continue;
        }
        let k = key(&s.heading);
        let found = (0..sections.len())
            .find(|&i| !used[i] && sections[i].key == k)
            .or_else(|| kind.and_then(|kd| (0..sections.len()).find(|&i| !used[i] && kind_of_key(&sections[i].key) == Some(kd))));
        if let Some(i) = found {
            used[i] = true;
            push(&mut out, &s.heading, &sections[i].body);
        }
    }
    let other = key(OTHER_NOTES);
    for (i, s) in sections.iter().enumerate() {
        let is_placed = kind_of_key(&s.key).is_some_and(|k| placed.contains(&k));
        if used[i] || is_placed || s.key == other {
            continue;
        }
        push(&mut out, &s.heading, &s.body);
    }
    if long {
        for (kind, heading) in DEFAULT_LISTS {
            if !t.sections.iter().any(|s| kind_of(&s.heading) == Some(kind)) {
                push(&mut out, heading, &crate::llm::collect_items(parts, aliases(kind)).join("\n"));
            }
        }
    }
    let rest: Vec<&str> = sections.iter().enumerate().filter(|(i, s)| !used[*i] && s.key == other).map(|(_, s)| s.body.as_str()).collect();
    push(&mut out, OTHER_NOTES, &rest.join("\n"));
    crate::llm::drop_empty_sections(&out.join("\n\n"))
}

/// Words of a note line, for matching: no Markdown marks, punctuation or case (as the window's
/// `tokens` in Meetings.tsx).
fn tokens(line: &str) -> Vec<String> {
    let l = line.to_lowercase();
    let l = l.trim_start_matches([' ', '\t', '#', '*', '-']);
    let l = ["[ ]", "[x]"].iter().find_map(|b| l.strip_prefix(b)).unwrap_or(l).trim_start();
    let l: String = l.chars().map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' }).collect();
    l.split_whitespace().map(str::to_string).collect()
}

/// The window's `fromMine`: `notes_line` is one of the user's lines, kept (typos fixed at most).
fn is_kept(notes_line: &[String], mine: &[String]) -> bool {
    let shared = mine.iter().filter(|w| notes_line.contains(w)).count();
    shared >= ((mine.len() as f64 * 0.8).ceil() as usize).max(1) && notes_line.len() as f64 <= mine.len() as f64 * 1.5 + 2.0
}

/// Safety net for the template path: every line of the notepad (not a heading) that no line of
/// the notes keeps is added under "## Other notes", so the user's words are never lost.
pub fn keep_my_lines(notes: String, my_notes: &str) -> String {
    let result: Vec<Vec<String>> = notes.lines().map(tokens).filter(|t| !t.is_empty()).collect();
    let missing: Vec<String> = my_notes
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| {
            let mine = tokens(l);
            !mine.is_empty() && !result.iter().any(|t| is_kept(t, &mine))
        })
        .map(|l| match l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
            Some(rest) => format!("- {rest}"),
            None => format!("- {l}"),
        })
        .collect();
    if missing.is_empty() {
        return notes;
    }
    let mut lines: Vec<String> = notes.lines().map(str::to_string).collect();
    let other = key(OTHER_NOTES);
    match lines.iter().position(|l| l.strip_prefix("## ").is_some_and(|h| key(h) == other)) {
        Some(at) => {
            let mut end = (at + 1..lines.len()).find(|&i| lines[i].starts_with('#')).unwrap_or(lines.len());
            while end > at + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            lines.splice(end..end, missing);
            lines.join("\n")
        }
        None => format!("{}\n\n## {OTHER_NOTES}\n{}", notes.trim_end(), missing.join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, name: &str, sections: &[(&str, &str)]) -> Template {
        tpl(id, name, "", sections)
    }

    #[test]
    fn builtins_and_resolve() {
        let custom = vec![t("c_standup", "Stand-up", &[("Done", ""), ("Doing", "")])];
        let all = all(&custom);
        assert_eq!(all.iter().map(|i| i.t.id.as_str()).collect::<Vec<_>>(), ["auto", "phone_call", "ticket", "one_on_one", "customer", "interview", "c_standup"]);
        assert!(all[0].builtin && all[1].builtin && !all[6].builtin);
        let json = serde_json::to_value(&all[6]).unwrap();
        assert_eq!(json["builtin"], false);
        assert_eq!(json["sections"][0]["heading"], "Done");
        assert_eq!(resolve(&custom, "").id, "auto");
        assert_eq!(resolve(&custom, "c_gone").id, "auto");
        assert_eq!(resolve(&custom, "c_standup").name, "Stand-up");
        assert_eq!(resolve(&[], "phone_call").sections.len(), 4);
        assert_eq!(resolve(&[], "one_on_one").sections.len(), 5);
        assert!(matches!(shape(&resolve(&[], "auto")), Shape::Auto));
        assert!(matches!(shape(&resolve(&[], "ticket")), Shape::Ticket));
        assert!(matches!(shape(&resolve(&[], "phone_call")), Shape::Sections(_)));
        assert!(matches!(shape(&resolve(&[], "interview")), Shape::Sections(_)));
        // No built-in invents a verdict.
        assert!(!builtins().iter().flat_map(|t| t.sections.iter()).any(|s| s.heading.to_lowercase().contains("verdict")));
    }

    #[test]
    fn clean_enforces_every_limit() {
        let many: Vec<Template> = (0..25).map(|i| t(&format!("c_{i}"), &format!("T{i}"), &[("A", "")])).collect();
        assert_eq!(clean(many).len(), 20);

        let long = |n: usize| "x".repeat(n);
        let sections: Vec<(String, String)> = (0..10).map(|i| (format!("S{i}"), String::new())).collect();
        let big = Template {
            id: "Bad ID".into(),
            name: long(50),
            about: format!("line one\nline two {}", long(400)),
            sections: sections.iter().map(|(h, d)| Section { heading: h.clone(), hint: d.clone() }).collect(),
        };
        let messy = Template {
            id: "c_ok1".into(),
            name: "  ## Weekly\nsync ".into(),
            about: String::new(),
            sections: vec![
                Section { heading: "## Updates".into(), hint: "- what\nchanged".into() },
                Section { heading: "updates".into(), hint: "duplicate".into() },
                Section { heading: long(70), hint: long(250) },
                Section { heading: "  ".into(), hint: "no heading".into() },
            ],
        };
        let dup_id = t("c_ok1", "Second", &[("A", "")]);
        let empty = t("c_empty", "No sections", &[]);
        let nameless = t("c_noname", "  ", &[("A", "")]);
        let got = clean(vec![big, messy, dup_id, empty, nameless]);
        assert_eq!(got.len(), 3);
        let (big, messy, dup) = (&got[0], &got[1], &got[2]);
        assert!(valid_id(&big.id) && big.id.len() == 10, "{}", big.id);
        assert_eq!(big.name.chars().count(), 40);
        assert_eq!(big.about.chars().count(), 300);
        assert!(!big.about.contains('\n'));
        assert_eq!(big.sections.len(), 8);
        assert_eq!(messy.id, "c_ok1");
        assert_eq!(messy.name, "Weekly sync");
        assert_eq!(messy.sections.len(), 2);
        assert_eq!(messy.sections[0], Section { heading: "Updates".into(), hint: "what changed".into() });
        assert_eq!(messy.sections[1].heading.chars().count(), 60);
        assert_eq!(messy.sections[1].hint.chars().count(), 200);
        assert_ne!(dup.id, "c_ok1");
        assert!(valid_id(&dup.id));
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of("Action items"), Some(Kind::NextSteps));
        assert_eq!(kind_of("Updates"), None);
        assert_eq!(kind_of("**Next steps:**"), Some(Kind::NextSteps));
        assert_eq!(kind_of("Next steps — **<imperative task>** (<who>)"), Some(Kind::NextSteps));
        assert_eq!(kind_of("تصمیم‌ها"), Some(Kind::Decisions));
        assert_eq!(kind_of("سؤال‌های باز"), Some(Kind::OpenQuestions));
        assert_eq!(kind_of("راه‌حل"), Some(Kind::Solution));
        assert_eq!(kind_of("To-dos"), Some(Kind::NextSteps));
        assert_eq!(kind_of("What we tried"), Some(Kind::Tried));
        // Ambiguous names are the template's own sections.
        for h in ["Steps", "Questions", "Issue", "Fix", "Tasks", "Actions", "Agreed"] {
            assert_eq!(kind_of(h), None, "{h}");
        }
    }

    #[test]
    fn assemble_single_pass_reorders_and_cuts_hints() {
        let one = resolve(&[], "one_on_one");
        let out = "# Q3 report is ready\n## Next steps\n- **Send the deck** (Sam)\n## **Updates — what each person reported**\n- Report sent to finance\n## Invented section\n- something else\n## Blockers\n## Decisions\n- Launch moves a week";
        let got = assemble(&one, out, &[]);
        assert_eq!(
            got,
            "# Q3 report is ready\n\n## Updates\n- Report sent to finance\n\n## Decisions\n- Launch moves a week\n\n## Next steps\n- **Send the deck** (Sam)\n\n## Invented section\n- something else"
        );
        // A lead-in before the first section is dropped.
        let got = assemble(&one, "# T\nHere are the notes:\n## Feedback\n- good demo", &[]);
        assert_eq!(got, "# T\n\n## Feedback\n- good demo");
        // No title: "Meeting". Other notes stay last.
        let got = assemble(&one, "## Other notes\n- misc\n## Feedback\n- good demo", &[]);
        assert_eq!(got, "# Meeting\n\n## Feedback\n- good demo\n\n## Other notes\n- misc");
    }

    #[test]
    fn assemble_long_path_places_collected_lists() {
        let custom = t("c_x", "Customer-ish", &[("Needs", "what they want"), ("Action items", ""), ("Context", "")]);
        let parts = vec![
            "## Problem\n- the export is slow\n## Next steps\n- **Send pricing** (Sam)\n## Decisions\n- Go with plan B".to_string(),
            "## Next steps\n- **Send pricing** (Sam)\n- **Book a demo** (Alex)\n## Open questions\n- Who signs?".to_string(),
        ];
        // The model wrote a list section anyway, out of order: it is replaced by the collected one.
        let out = "# Plan B for the export\n## Context\n- They use spreadsheets today\n## Needs\n- Faster export\n## Next steps\n- **Wrong** (Nobody)\n## Other notes\n- Price came up";
        let got = assemble(&custom, out, &parts);
        assert_eq!(
            got,
            "# Plan B for the export\n\n## Needs\n- Faster export\n\n## Action items\n- **Send pricing** (Sam)\n- **Book a demo** (Alex)\n\n## Context\n- They use spreadsheets today\n\n## Decisions\n- Go with plan B\n\n## Open questions\n- Who signs?\n\n## Other notes\n- Price came up"
        );
    }

    #[test]
    fn keeps_the_users_lines() {
        let notes = "# Budget\n\n## Updates\n- Ask Sam about Q3 numbers\n\n## Other notes\n- misc\n\n## Next steps\n- **Call** (Alex)".to_string();
        // Kept (typo fixed, a word added): unchanged.
        assert_eq!(keep_my_lines(notes.clone(), "# Budget\n- ask Sam about Q3"), notes);
        // Dropped: added under Other notes, before the next section.
        let got = keep_my_lines(notes.clone(), "- ask Sam about Q3\n- renew the domain\nbuy milk");
        assert_eq!(got, "# Budget\n\n## Updates\n- Ask Sam about Q3 numbers\n\n## Other notes\n- misc\n- renew the domain\n- buy milk\n\n## Next steps\n- **Call** (Alex)");
        // No Other notes yet: the section is added at the end.
        let got = keep_my_lines("# T\n\n## Updates\n- x".to_string(), "- [ ] renew the domain");
        assert_eq!(got, "# T\n\n## Updates\n- x\n\n## Other notes\n- [ ] renew the domain");
        // A line kept in a much longer sentence does not count as kept.
        let got = keep_my_lines("# T\n\n## A\n- renew the domain before the end of the month and also call the bank".to_string(), "- renew domain");
        assert!(got.ends_with("## Other notes\n- renew domain"), "{got}");
    }
}
