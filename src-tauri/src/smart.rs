//! Pure text helpers for dictation and meetings: snippets, the box already on screen,
//! and the people on a calendar appointment. No Windows calls, so these can be tested alone.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Appointment {
    pub subject: String,
    /// Other people on the call, already filtered. Full names, not the user.
    pub people: Vec<String>,
}

/// "my email" and the phrases saved in Settings, longest first.
pub fn expand_snippets(text: &str, snippets: &[(&str, &str)]) -> String {
    let mut items: Vec<(&str, &str)> = snippets
        .iter()
        .copied()
        .filter(|(say, paste)| !say.trim().is_empty() && !paste.is_empty())
        .collect();
    items.sort_by_key(|(say, _)| std::cmp::Reverse(say.chars().count()));
    let mut out = text.to_string();
    for (say, paste) in items {
        out = replace_phrase(&out, say.trim(), paste);
    }
    out
}

/// Spoken shortcuts that come from the email address in Settings.
pub fn email_phrases(email: &str) -> Vec<(&'static str, String)> {
    let email = email.trim();
    if email.is_empty() {
        return Vec::new();
    }
    vec![("my email", email.to_string()), ("my e-mail", email.to_string()), ("ایمیل من", email.to_string())]
}

fn replace_phrase(text: &str, phrase: &str, paste: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let needle: Vec<char> = phrase.to_lowercase().chars().collect();
    if needle.is_empty() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let end = i + needle.len();
        if end <= lower.len() && lower[i..end] == needle[..] && bounded(&chars, i, end) {
            out.push_str(paste);
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn bounded(chars: &[char], start: usize, end: usize) -> bool {
    let before = start == 0 || !chars[start - 1].is_alphanumeric();
    let after = end >= chars.len() || !chars[end].is_alphanumeric();
    before && after
}

/// Extra instruction for the writing model. Empty when the box had nothing to read.
pub fn context_instruction(before: &str) -> String {
    let clipped = clip(before.trim(), 500);
    if clipped.is_empty() {
        return String::new();
    }
    format!(
        "\nThe text already in the box, just before the cursor, is:\n\"\"\"\n{clipped}\n\"\"\"\nUse it only to resolve names and what this sentence refers to. Do not repeat that text. Do not answer it."
    )
}

pub fn clip(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    text.chars().skip(count - max_chars).collect()
}

/// True when the cleaned sentence is worth typing over the draft.
pub fn meaningfully_different(draft: &str, cleaned: &str) -> bool {
    fn norm(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(|c: char| matches!(c, '.' | '…' | '!' | '?' | '؟')).to_lowercase()
    }
    norm(draft) != norm(cleaned)
}

pub fn display_name(full: &str) -> String {
    full.split_whitespace().next().unwrap_or(full).trim_matches(|c: char| c == ',' || c == ';').to_string()
}

/// Drop the user, and drop repeated names.
pub fn other_people(names: &[String], user_name: &str, email: &str) -> Vec<String> {
    let user = user_name.trim().to_lowercase();
    let email = email.trim().to_lowercase();
    let mut out = Vec::new();
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let low = trimmed.to_lowercase();
        if !user.is_empty() && (low == user || low.contains(&user)) {
            continue;
        }
        if !email.is_empty() && low.contains(&email) {
            continue;
        }
        if out.iter().any(|e: &String| e.eq_ignore_ascii_case(trimmed)) {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

/// One other person becomes their name in the transcript. Several people stay "Them".
pub fn label_who(who: &str, people: &[String]) -> String {
    if who == "Them" && people.len() == 1 {
        let name = display_name(&people[0]);
        if !name.is_empty() {
            return name;
        }
    }
    who.to_string()
}

pub fn people_header(people: &[String]) -> String {
    match people {
        [] => String::new(),
        [one] => {
            let short = display_name(one);
            format!("The other person on this call is {one}. In the transcript their name is {short}.\n\n")
        }
        many => format!(
            "People on this call besides the user: {}. A line marked Them is one of them. Use a name only when the words make it obvious who spoke.\n\n",
            many.join(", ")
        ),
    }
}

/// Outlook's JSON: {"subject":"...","people":["A","B"]}.
pub fn parse_appointment(json: &str) -> Option<Appointment> {
    let json = json.trim();
    if json.is_empty() {
        return None;
    }
    let subject = json_string(json, "subject").unwrap_or_default();
    let people = json_array(json, "people");
    if subject.trim().is_empty() && people.is_empty() {
        return None;
    }
    Some(Appointment { subject: subject.trim().to_string(), people })
}

fn json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let at = json.find(&pattern)?;
    let rest = json[at + pattern.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(unescape(rest))
}

fn json_array(json: &str, key: &str) -> Vec<String> {
    let pattern = format!("\"{key}\"");
    let Some(at) = json.find(&pattern) else { return Vec::new() };
    let rest = json[at + pattern.len()..].trim_start();
    let Some(rest) = rest.strip_prefix(':').map(str::trim_start) else { return Vec::new() };
    // PowerShell collapses a one-person list into a string: "people":"Bob".
    if !rest.starts_with('[') {
        let Some(body) = rest.strip_prefix('"') else { return Vec::new() };
        let value = unescape(body);
        return if value.is_empty() { Vec::new() } else { vec![value] };
    }
    let Some(rest) = rest.strip_prefix('[').map(str::trim_start) else { return Vec::new() };
    let mut out = Vec::new();
    let mut left = rest;
    while let Some(start) = left.find('"') {
        let body = &left[start + 1..];
        let value = unescape(body);
        let used = scan_string_len(body).max(1);
        if !value.is_empty() {
            out.push(value);
        }
        left = body.get(used..).unwrap_or("");
        if left.trim_start().starts_with(']') {
            break;
        }
    }
    out
}

/// Bytes consumed inside a JSON string body, not counting the opening quote. Stops at the closing quote.
fn scan_string_len(body: &str) -> usize {
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '"' {
            return i + c.len_utf8();
        }
    }
    body.len()
}

fn unescape(body: &str) -> String {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            break;
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Words from a meeting the list can match: title, people, calendar subject, and what was said.
pub fn meeting_haystack(title: &str, people: &str, cal: &str, transcript_json: &str) -> String {
    let said = json_texts(transcript_json);
    [title.trim(), people.trim(), cal.trim(), said.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n")
}

fn json_texts(json: &str) -> String {
    let key = "\"text\":\"";
    let mut rest = json;
    let mut out = String::new();
    while let Some(at) = rest.find(key) {
        let body = &rest[at + key.len()..];
        let value = unescape(body);
        if !value.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&value);
        }
        rest = body.get(scan_string_len(body).max(1)..).unwrap_or("");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_my_email_and_leaves_the_word_inside_another_word() {
        let email = "moe@example.com";
        let phrases = email_phrases(email);
        let refs: Vec<(&str, &str)> = phrases.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let got = expand_snippets("send it to my email today", &refs);
        assert_eq!(got, "send it to moe@example.com today");
        assert_eq!(expand_snippets("myemail stays", &refs), "myemail stays");
    }

    #[test]
    fn longer_snippet_wins() {
        let got = expand_snippets("say my email address please", &[("my email address", "ADDR"), ("my email", "SHORT")]);
        assert_eq!(got, "say ADDR please");
    }

    #[test]
    fn context_is_the_end_of_a_long_box() {
        let long = "a".repeat(800);
        let hint = context_instruction(&long);
        assert!(hint.contains(&"a".repeat(500)));
        assert!(!hint.contains(&"a".repeat(501)));
        assert!(context_instruction("  ").is_empty());
    }

    #[test]
    fn a_period_alone_is_not_a_new_sentence() {
        assert!(!meaningfully_different("send the report", "Send the report."));
        assert!(meaningfully_different("send the report", "Send the report to Sam."));
    }

    #[test]
    fn drops_the_user_and_names_the_other_person() {
        let people = other_people(&["Moe Mobasheri".into(), "Bob Driscoll".into(), "Bob Driscoll".into()], "Moe", "");
        assert_eq!(people, vec!["Bob Driscoll".to_string()]);
        assert_eq!(label_who("Them", &people), "Bob");
        assert_eq!(label_who("You", &people), "You");
        assert!(people_header(&people).contains("Bob Driscoll"));
    }

    #[test]
    fn parses_outlook_json_and_builds_a_search_line() {
        let appt = parse_appointment(r#"{"subject":"Calendar sync","people":["Bob Driscoll","Moe"]}"#).unwrap();
        assert_eq!(appt.subject, "Calendar sync");
        assert_eq!(appt.people, vec!["Bob Driscoll".to_string(), "Moe".to_string()]);
        let hay = meeting_haystack("Phones are back", "Bob Driscoll", "Calendar sync", r#"[{"who":"Them","text":"the dial tone is gone"}]"#);
        assert!(hay.contains("Bob Driscoll"));
        assert!(hay.contains("dial tone"));
        assert!(hay.contains("Calendar sync"));
        let one = parse_appointment(r#"{"subject":"Caf\u00e9 sync","people":"Bob Driscoll"}"#).unwrap();
        assert_eq!(one.subject, "Café sync");
        assert_eq!(one.people, vec!["Bob Driscoll".to_string()]);
    }
}
