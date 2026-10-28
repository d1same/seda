//! Local text AI: runs llama-server (Vulkan) with Gemma 4 E4B as a hidden child process.
//! Prompts are the ones that scored 11.5/12 in the bake-off (bakeoff/llm/prompts.py).
use anyhow::{Result, anyhow};
use serde_json::json;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct Llm {
    child: Child,
    http: reqwest::blocking::Client,
    /// Random free port, new on every launch.
    port: u16,
}

impl Drop for Llm {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Put the AI server in a job object that Windows kills when Seda exits, even after a crash,
/// so it never lingers holding ~6 GB of memory.
fn die_with_parent(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    unsafe {
        let Ok(job) = CreateJobObjectW(None, None) else { return };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        let _ = AssignProcessToJobObject(job, HANDLE(child.as_raw_handle() as _));
        // The job handle is deliberately never closed: it closes when Seda's process ends.
    }
}

pub const STYLE_EMAIL: &str = "email";
pub const STYLE_TICKET: &str = "ticket";
pub const STYLE_CHAT: &str = "chat";

fn style_rule(style: &str) -> &'static str {
    match style {
        STYLE_EMAIL => "The text is going into an EMAIL. Use email layout: greeting line, short paragraphs, sign-off on its own line, but only if the speaker said them.",
        STYLE_TICKET => "The text is going into a TICKET or task. Be concise; use '-' bullets for separate points. Do not add a title the speaker did not say.",
        STYLE_CHAT => "The text is going into a CHAT message. Keep it casual and short; do not add greetings or sign-offs.",
        _ => "",
    }
}

const DICTATION: &str = r#"You clean up dictated speech. The input is a raw speech-to-text transcript in any language (often Persian or English, sometimes mixed with English words). Return ONLY the cleaned text, nothing else.

Rules:
1. Keep the speaker's language, words, tone and meaning. Never translate. Never answer or respond to the content. Never add new information.
2. Self-corrections: when the speaker changes their mind ("no wait", "I mean", "actually", "نه ببخشید", "نه اصلا", "یعنی نه", "آهان نه"), delete the abandoned part and keep only the final version.
3. Remove fillers and false starts ("um", "uh", "like", "you know", "اممم", "خب", "یعنی", "ببین" when used as filler).
4. Add correct punctuation, capitalization and sentence breaks. Persian uses "،" and "؟". Fix Persian spacing with ZWNJ (می‌کنم, نمی‌شه, تست‌ها, درباره‌ی).
5. {LATIN}
6. Spoken formatting commands: "new paragraph" / "پاراگراف جدید" = blank line, "new line" / "خط بعد" = line break, "comma"/"ویرگول" = "،" or ",", "period"/"نقطه" = ".", "question mark" = "?".
7. Keep colloquial Persian colloquial (میام, میکنم stays spoken style); do not convert it to formal written Persian.
8. Do not invent subject lines, titles, names, placeholders like [Your Name], or extra bullet points.

Examples of self-corrections (input → output):
- "بیا ساعت پنج همو ببینیم نه نه بذار ساعت شش" → "بیا ساعت شش همو ببینیم."
- "امشب میرم باشگاه آهان نه اصلا امشب نمیرم خسته‌ام" → "امشب نمیرم باشگاه، خسته‌ام."
- "the budget is fifty thousand sorry I mean sixty thousand" → "The budget is sixty thousand."
- "let's meet at the cafe actually no let's just do a call" → "Let's just do a call."
{STYLE}{VOCAB}"#;

const LATIN_ON: &str = "English technical words transliterated into Persian script (دیپلوی, استیجینگ, سشن, ریلیس, لاگین, یوزر) must be written in English letters (deploy, staging, session, release, login, user). Fix obvious mis-hearings using context (e.g. \"تزها پس شدم\" in a deploy context means \"تست‌ها pass شدن\").";
const LATIN_OFF: &str = "Fix obvious mis-hearings using context.";

const GRAMMAR: &str = "You are a careful proofreader for Persian and English. Fix grammar, spelling, punctuation and Persian ZWNJ spacing in the user's text. Keep the meaning, language, tone and wording as close as possible; change only what is wrong. Never translate. Return ONLY the corrected text.";

pub fn rewrite_prompt(mode: &str) -> &'static str {
    match mode {
        "formal" => "Rewrite the user's text to sound more professional and polite. Keep the same language and meaning. Return ONLY the rewritten text.",
        "shorter" => "Rewrite the user's text to be shorter and clearer, keeping all key facts. Keep the same language. Return ONLY the rewritten text.",
        "friendly" => "Rewrite the user's text to sound warmer and more friendly. Keep the same language and meaning. Return ONLY the rewritten text.",
        "to_en" => "Translate the user's text into natural, fluent English. Return ONLY the translation.",
        "to_fa" => "Translate the user's text into natural, fluent Persian (Farsi). Keep English technical terms in English letters. Return ONLY the translation.",
        _ => GRAMMAR,
    }
}

/// Shared rules for all meeting notes (single pass, parts and merge): Granola-style notes, organised
/// by topic, specific, and with only real follow-up work as tasks.
const NOTE_RULES: &str = "Rules:
- Organise by topic: one ## section per subject that was discussed, named after it (e.g. \"## Zoom audio setup\"), in the order it came up. A short side topic (a license, another product, a status update) still gets its own section. Never use generic headings like \"Summary\" or \"Discussion\".
- Be specific: names, places, prices, speeds, counts, dates, settings, versions, and what was decided. A rough figure (\"about 9 dollars a user\") still counts — write it. Never turn a figure that was said into an open question, and never write vague lines like \"participants discussed issues\".
- A person, company or number stays on the topic it was said about. Do not move it onto a different topic. If Sam renewed a license, do not write that Sam bought the printer.
- Put supporting details as indented sub-bullets (two spaces, then \"- \") under their point.
- Next steps are only follow-up work for AFTER the meeting. Every commitment counts, whoever made it: \"I'll …\", \"I can …\", \"we'll …\" and requests like \"Sam, can you …\" are all next steps. Never list things said during the call (\"wait a moment\", \"share your screen\", \"can you hear me\", \"let me check\").
- \"You\" is the user. \"Them\" is everyone else, often several people — never one person, and never an owner. Write the owner's name only when the transcript says that person will do it. If you cannot tell who, leave the name off. The wrong name is worse than no name.
- Skip greetings and small talk. Never invent anything that was not said. The transcript comes from speech recognition, so some words are wrong: infer the meaning, never quote obvious mistakes, and never replace a garbled word with a different name from elsewhere in the call.
- Open questions are only questions someone asked that nobody answered. If there are none, omit the section. Never write \"None\" or \"N/A\".";

const MEETING: &str = "You are an excellent note-taker. Turn this meeting transcript (lines start with [mm:ss] and the speaker) into clear notes. Output Markdown only, in THIS order (lists first, so nothing important is cut off):
# <a specific title naming what the meeting was about, 4-9 words>
## Decisions
- <decision>   (only if decisions were made)
## Next steps
- [ ] <name, only if the transcript says who>: <follow-up task> (<when, if said>)
## Open questions
- <question left unanswered>   (only if any)
## <topic>
- <specific point>
  - <supporting detail>
(one ## section per topic; a short side topic still gets its own section)
List EVERY decision, follow-up task and open question that was mentioned; missing one is worse than including a minor one.
{RULES}";

const MEETING_PART: &str = "You take detailed notes on one part (about 10 minutes) of a longer meeting (lines start with [mm:ss] and the speaker). Output Markdown only, in THIS order (lists first, so nothing important is cut off):
## Decisions
- <decision>
## Next steps
- [ ] <name, only if the transcript says who>: <follow-up task> (<when>)
## Open questions
- <question>
## <topic>
- <specific point>
  - <supporting detail>
(one ## section per topic discussed in this part, including a short side topic)
Leave out empty sections. Capture EVERY decision, follow-up task and open question in this part.
{RULES}";
const MEETING_MERGE: &str = "You get the notes of consecutive parts of one meeting, in order. Combine their topic sections into the final notes. Output Markdown only:
# <a specific title naming what the meeting was about, 4-9 words>
## <topic>
- <specific point>
  - <supporting detail>
Merge sections about the same subject from different parts into ONE section, keeping all their specific details (names, numbers, places). A short side topic stays its own section; do not drop a subject to keep the notes short. Write ONLY the title and topic sections: do NOT write Decisions, Next steps or Open questions sections (they are added separately).
{RULES}";

/// A bullet that carries no information ("None", "N/A", "(Them)").
fn is_blank_item(line: &str) -> bool {
    let l = line.trim().trim_start_matches(['-', '*', ' ']).trim().trim_end_matches(['.', ':']).to_lowercase();
    l.is_empty() || (l.starts_with('(') && l.ends_with(')')) || ["none", "n/a", "na", "nothing", "ندارد", "هیچ", "هیچ‌کدام", "-"].contains(&l.as_str())
}

/// The model labels every remote person "Them". Drop that fake owner; keep a real name.
fn clean_line(line: &str) -> String {
    let s = line.trim_end();
    let lower = s.to_ascii_lowercase();
    let s = if let Some(head) = lower.strip_suffix("(them)") {
        if head.ends_with(' ') || head.ends_with(':') {
            s[..head.len()].trim_end().trim_end_matches(':').trim_end()
        } else {
            s
        }
    } else {
        s
    };
    let Some(body) = s.trim_start().strip_prefix("- ").or_else(|| s.trim_start().strip_prefix("* ")) else {
        return s.to_string();
    };
    let indent_len = s.len() - s.trim_start().len();
    let indent = &s[..indent_len];
    let body = body.trim_start();
    let (mark, rest) = if let Some(r) = body.strip_prefix("[ ]") {
        ("[ ] ", r.trim_start())
    } else if let Some(r) = body.strip_prefix("[x]") {
        ("[x] ", r.trim_start())
    } else if let Some(r) = body.strip_prefix("[X]") {
        ("[x] ", r.trim_start())
    } else {
        ("", body)
    };
    if rest.len() >= 5 && rest.as_bytes()[..4].eq_ignore_ascii_case(b"them") && rest.as_bytes()[4] == b':' {
        return format!("{indent}- {mark}{}", rest[5..].trim_start());
    }
    s.to_string()
}

/// The model sometimes writes "## Open questions" followed by just "None", or a real
/// bullet plus a leftover "None". Remove empty sections and those placeholder lines.
pub fn drop_empty_sections(md: &str) -> String {
    let owned: Vec<String> = md.lines().map(clean_line).collect();
    let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with("## ") {
            let mut j = i + 1;
            while j < lines.len() && !lines[j].starts_with('#') {
                j += 1;
            }
            let kept: Vec<&str> = lines[i + 1..j].iter().copied().filter(|l| l.trim().is_empty() || !is_blank_item(l)).collect();
            if kept.iter().all(|l| l.trim().is_empty()) {
                i = j;
                continue;
            }
            out.push(lines[i]);
            out.extend(kept);
            i = j;
            continue;
        }
        out.push(lines[i]);
        i += 1;
    }
    out.join("\n")
}

/// Lists first in the model output (so a token limit can't cut them off), then moved back
/// under the topics for the reader. Decisions, next steps and open questions are copied
/// out by code, same as a long meeting's merge.
fn lists_last(md: &str) -> String {
    let topics = strip_sections(md, &LIST_SECTIONS);
    let mut notes = topics;
    let parts = [md.to_string()];
    for (heading, aliases) in [("Decisions", &["decisions"][..]), ("Next steps", &["next steps", "action items"][..]), ("Open questions", &["open questions"][..])] {
        let items = collect_items(&parts, aliases);
        if !items.is_empty() {
            if !notes.is_empty() && !notes.ends_with('\n') {
                notes.push('\n');
            }
            notes.push_str(&format!("\n## {heading}\n{}\n", items.join("\n")));
        }
    }
    drop_empty_sections(notes.trim())
}

const ASK: &str = "You are Seda's meeting assistant. Below are the notes and the transcript of one meeting (\"You\" is the user, \"Them\" the other people; the transcript comes from speech recognition and has some wrong words).
Answer the user's questions about this meeting: short and direct, in the language they ask in, using only what is in the notes and transcript. If it isn't there, say so.
If the user asks you to change, add to, shorten, translate or rewrite the notes, reply with one short sentence saying what you changed, then the COMPLETE updated notes in Markdown (first line \"# <title>\") between <notes> and </notes>. Never invent facts.";

/// Tells the AI who "You" is, so "Alex, can you send…" becomes Alex's task.
fn who_is_you(user: &str) -> String {
    let n = user.trim();
    if n.is_empty() {
        return String::new();
    }
    format!("\n\nThe user (\"You\" in the transcript) is called {n}. When someone addresses {n} (for example \"{n}, can you…\"), that is a task for {n}. Write the user's own tasks as \"{n}: …\" and refer to the user as {n}, never as \"You\".")
}

/// A free port on this PC, picked by Windows.
fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

/// 32 random bytes as hex, from the operating system's secure random source.
fn random_key() -> Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| anyhow!("random: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

impl Llm {
    pub fn start(server: &str, model: &str) -> Result<Self> {
        if !std::path::Path::new(server).exists() {
            return Err(anyhow!("Writing AI isn't downloaded yet — run Setup (Settings → Advanced)"));
        }
        if !std::path::Path::new(model).exists() {
            return Err(anyhow!("Writing AI model isn't downloaded yet — run Setup (Settings → Advanced)"));
        }
        // Only fall back to the CPU when the GPU engine actually crashes — not when it's merely slow.
        Self::launch(server, model, true).or_else(|e| {
            crate::log::log(format!("writing AI on GPU failed ({e:#}); trying CPU"));
            Self::launch(server, model, false)
        })
    }

    fn launch(server: &str, model: &str, gpu: bool) -> Result<Self> {
        let layers = if gpu { "99" } else { "0" };
        // The engine's own output goes to llama.log next to seda.log: it says why a start was slow or failed.
        let engine_log = std::fs::File::create(crate::settings::data_dir().join("llama.log"))?;
        // Locked down: reachable only from this PC, on a port that is new on every launch, only with
        // a password that is new on every launch and known only to Seda (passed privately in the
        // environment, not on the command line), no status page (it can show what the AI is working
        // on), and no browser access (a web page could otherwise call it from your browser).
        let key = random_key()?;
        let port = free_port()?;
        let child = Command::new(server)
            .args(["-m", model, "--port", &port.to_string(), "--host", "127.0.0.1", "-ngl", layers, "-c", "16384", "--jinja", "--reasoning", "off"])
            .args(["--no-slots", "--cors-origins", "http://127.0.0.1:1"])
            .env("LLAMA_API_KEY", &key)
            .stdout(Stdio::null())
            .stderr(Stdio::from(engine_log))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        die_with_parent(&child);
        crate::log::log(format!("writing AI starting on {}", if gpu { "GPU" } else { "CPU" }));
        let mut auth = reqwest::header::HeaderMap::new();
        auth.insert(reqwest::header::AUTHORIZATION, format!("Bearer {key}").parse()?);
        let http = reqwest::blocking::Client::builder().timeout(Duration::from_secs(60)).default_headers(auth).build()?;
        let mut me = Self { child, http, port };
        let t = Instant::now();
        let mut noted = false;
        loop {
            if let Ok(Some(status)) = me.child.try_wait() {
                return Err(anyhow!("Text AI exited early ({status}) — see llama.log"));
            }
            // Talk to the port only once our own engine is the one listening on it: another program
            // grabbing it first must never receive the password or your text.
            match crate::win::tcp_listener_pid(port) {
                Some(pid) if pid == me.child.id() => {
                    if let Ok(r) = me.http.get(format!("http://127.0.0.1:{port}/health")).send() {
                        if r.status().is_success() {
                            break;
                        }
                    }
                }
                Some(_) => {
                    let _ = me.child.kill();
                    return Err(anyhow!("Another program took the writing AI's port — restart Seda"));
                }
                None => {}
            }
            // The very first start on a PC compiles GPU shaders for that graphics card, which can take
            // several minutes (only once). Keep waiting while the engine is alive.
            if !noted && t.elapsed() > Duration::from_secs(60) {
                crate::log::log("writing AI still starting (first start prepares the graphics card; this happens once)");
                noted = true;
            }
            if t.elapsed() > Duration::from_secs(15 * 60) {
                return Err(anyhow!("Text AI did not start within 15 minutes — see llama.log"));
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        // Warm-up so the first real request is fast.
        let _ = me.chat(GRAMMAR, "سلام");
        crate::log::log(format!("writing AI started on {} in {:.1}s", if gpu { "GPU" } else { "CPU" }, t.elapsed().as_secs_f32()));
        Ok(me)
    }

    fn chat(&mut self, system: &str, user: &str) -> Result<String> {
        self.chat_limited(system, user, Duration::from_secs(60))
    }

    /// The answer is capped to roughly the input's length (a rewrite never needs much more),
    /// and the request gives up after `timeout` so the caller can fall back to the raw text.
    fn chat_limited(&mut self, system: &str, user: &str, timeout: Duration) -> Result<String> {
        self.chat_with(system, user, (user.chars().count() / 2 + 80).clamp(80, 1200), timeout)
    }

    fn chat_with(&mut self, system: &str, user: &str, max_tokens: usize, timeout: Duration) -> Result<String> {
        self.chat_messages(vec![json!({"role": "system", "content": system}), json!({"role": "user", "content": user})], max_tokens, timeout)
    }

    fn chat_messages(&mut self, messages: Vec<serde_json::Value>, max_tokens: usize, timeout: Duration) -> Result<String> {
        // If the engine has died, its port could be taken by something else: never send it text.
        if !matches!(self.child.try_wait(), Ok(None)) {
            return Err(anyhow!("the writing AI stopped — restart Seda"));
        }
        let port = self.port;
        let body = json!({
            "messages": messages,
            "temperature": 0.2,
            "max_tokens": max_tokens,
            "cache_prompt": true,
            "chat_template_kwargs": {"enable_thinking": false},
        });
        let r: serde_json::Value = self
            .http
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .timeout(timeout)
            .json(&body)
            .send()?
            .json()?;
        let text = r["choices"][0]["message"]["content"].as_str().ok_or_else(|| anyhow!("empty AI reply"))?;
        Ok(text.trim().trim_matches('"').trim().to_string())
    }

    pub fn cleanup(&mut self, raw: &str, style: &str, vocab: &[String], latin_terms: bool) -> Result<String> {
        let style_line = match style_rule(style) {
            "" => String::new(),
            s => format!("\n{s}"),
        };
        let vocab_line = if vocab.is_empty() {
            String::new()
        } else {
            format!("\nSpell these names and terms exactly like this when they appear: {}.", vocab.join(", "))
        };
        let system = DICTATION
            .replace("{LATIN}", if latin_terms { LATIN_ON } else { LATIN_OFF })
            .replace("{STYLE}", &style_line)
            .replace("{VOCAB}", &vocab_line);
        // Never keep you waiting: past 8 s the caller pastes the unpolished text instead.
        self.chat_limited(&system, raw, Duration::from_secs(8))
    }

    pub fn rewrite(&mut self, text: &str, mode: &str) -> Result<String> {
        self.chat(rewrite_prompt(mode), text)
    }

    /// One turn of the Ask box on a meeting. `context` holds the notes and (the relevant part of) the
    /// transcript; `history` the earlier turns as (role, text) with role "you" or "ai". When the user
    /// asks for a change to the notes, the reply carries the complete new notes inside <notes>…</notes>.
    pub fn ask_meeting(&mut self, context: &str, history: &[(String, String)], question: &str, user: &str) -> Result<String> {
        let mut messages = vec![json!({"role": "system", "content": format!("{ASK}{}\n\n{context}", who_is_you(user))})];
        for (role, text) in history.iter().rev().take(6).rev() {
            messages.push(json!({"role": if role == "you" { "user" } else { "assistant" }, "content": text}));
        }
        // The notes may be in Persian while the question is in English: answer in the question's language.
        let hint = if question.is_ascii() { "Answer in English." } else { "Reply in the same language as this message." };
        messages.push(json!({"role": "user", "content": format!("{question}\n\n({hint})")}));
        self.chat_messages(messages, 1800, Duration::from_secs(300))
    }

    /// Shared instructions for meeting notes: language, who "You" is, the user's own notes.
    fn meeting_extra(language: Option<&str>, user: &str, has_mine: bool, vocab: &[String]) -> String {
        let mut s = language.map(|l| format!("\n\nWrite the notes in {l}. Keep names and technical terms as they were said.")).unwrap_or_default();
        s.push_str(&who_is_you(user));
        if !vocab.is_empty() {
            let terms = vocab.iter().take(40).map(String::as_str).collect::<Vec<_>>().join(", ");
            s.push_str(&format!("\n\nSpell these names and terms exactly like this when they were said: {terms}."));
        }
        if has_mine {
            s.push_str("\n\nThe user also typed their own notes during the meeting (given first). Build on them: keep every point they wrote, in their words, and add the details from the meeting.");
        }
        s
    }

    /// Notes for a short meeting in one pass.
    pub fn meeting_single(&mut self, transcript: &str, language: Option<&str>, my_notes: &str, user: &str, _minutes: u32, vocab: &[String]) -> Result<String> {
        let mine = my_notes.trim();
        let system = format!("{}{}", MEETING.replace("{RULES}", NOTE_RULES), Self::meeting_extra(language, user, !mine.is_empty(), vocab));
        let body = if mine.is_empty() { transcript.to_string() } else { format!("My own notes:\n{mine}\n\n---\n\n{transcript}") };
        // Lists are written first, then moved under the topics. 2400 tokens so a side topic
        // (a license, a price) is not cut off the end.
        Ok(lists_last(&self.chat_with(&system, &body, 2400, Duration::from_secs(600))?))
    }

    /// Detailed notes for one ~10-minute stretch of a long meeting (written while it is still going).
    pub fn meeting_part(&mut self, transcript: &str, language: Option<&str>, user: &str, vocab: &[String]) -> Result<String> {
        let system = format!("{}{}", MEETING_PART.replace("{RULES}", NOTE_RULES), Self::meeting_extra(language, user, false, vocab));
        self.chat_with(&system, transcript, 1600, Duration::from_secs(600))
    }

    /// Final notes from the part notes of a long meeting. The AI only writes the title and topics;
    /// decisions, next steps and open questions are collected from the parts by code, so the merge
    /// can never drop one (a small model rewriting everything at once loses items).
    pub fn meeting_merge(&mut self, parts: &[String], language: Option<&str>, my_notes: &str, user: &str, _minutes: u32, vocab: &[String]) -> Result<String> {
        let mine = my_notes.trim();
        let system = format!("{}{}", MEETING_MERGE.replace("{RULES}", NOTE_RULES), Self::meeting_extra(language, user, !mine.is_empty(), vocab));
        let topics_only: Vec<String> = parts.iter().map(|p| strip_sections(p, &LIST_SECTIONS)).collect();
        let joined = topics_only.iter().enumerate().map(|(i, p)| format!("### Part {}\n{p}", i + 1)).collect::<Vec<_>>().join("\n\n");
        let body = if mine.is_empty() { joined } else { format!("My own notes:\n{mine}\n\n---\n\n{joined}") };
        let mut notes = strip_sections(&self.chat_with(&system, &body, 2200, Duration::from_secs(600))?, &LIST_SECTIONS);
        for (heading, aliases) in [("Decisions", &["decisions"][..]), ("Next steps", &["next steps", "action items"][..]), ("Open questions", &["open questions"][..])] {
            let items = collect_items(parts, aliases);
            if !items.is_empty() {
                notes.push_str(&format!("\n\n## {heading}\n{}", items.join("\n")));
            }
        }
        Ok(drop_empty_sections(notes.trim()))
    }
}

/// The list sections that are gathered by code rather than rewritten by the merge.
const LIST_SECTIONS: [&str; 4] = ["decisions", "next steps", "action items", "open questions"];

fn heading_is(line: &str, names: &[&str]) -> bool {
    line.strip_prefix("## ").is_some_and(|h| names.iter().any(|n| h.trim().trim_end_matches(':').eq_ignore_ascii_case(n)))
}

/// `md` without the `## <name>` sections listed in `names`.
fn strip_sections(md: &str, names: &[&str]) -> String {
    let mut out = Vec::new();
    let mut skipping = false;
    for line in md.lines() {
        if line.starts_with('#') {
            skipping = heading_is(line, names);
        }
        if !skipping {
            out.push(line);
        }
    }
    out.join("\n").trim().to_string()
}

/// Every bullet under the given section names across all parts, in order, without duplicates.
fn collect_items(parts: &[String], names: &[&str]) -> Vec<String> {
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for p in parts {
        let mut inside = false;
        for line in p.lines() {
            if line.starts_with('#') {
                inside = heading_is(line, names);
                continue;
            }
            let l = clean_line(line.trim_end());
            if inside && (l.trim_start().starts_with("- ") || l.trim_start().starts_with("* ")) && !is_blank_item(&l) && seen.insert(norm(&l)) {
                items.push(l);
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_placeholder_lines_and_fake_them_owner() {
        let md = "\
# Switzerland office network
## Open questions
- What is the exact price for the services?
- None

## Next steps
- **Share the wireless options** (Them)
- [ ] Them: send the admin email
- [ ] Jose: assign the private VLAN
";
        let got = drop_empty_sections(md);
        assert!(got.contains("What is the exact price"));
        assert!(!got.to_lowercase().contains("none"));
        assert!(!got.contains("(Them)"));
        assert!(!got.contains("Them:"));
        assert!(got.contains("send the admin email"));
        assert!(got.contains("Jose: assign the private VLAN"));
    }

    #[test]
    fn drops_a_section_that_is_only_none() {
        let md = "## Decisions\n- None\n\n## Printer\n- Luke buys it locally\n";
        let got = drop_empty_sections(md);
        assert!(!got.contains("Decisions"));
        assert!(got.contains("Luke buys it locally"));
    }

    #[test]
    fn moves_lists_under_the_topics() {
        let md = "\
# Office network
## Decisions
- Use the shared speed
## Next steps
- [ ] Moe: send Luke a printer model
## Open questions
- None
## Bandwidth
- 130 Mbps is still free on the 500 Mbps circuit
";
        let got = lists_last(md);
        let bandwidth = got.find("## Bandwidth").unwrap();
        let decisions = got.find("## Decisions").unwrap();
        let steps = got.find("## Next steps").unwrap();
        assert!(bandwidth < decisions);
        assert!(decisions < steps);
        assert!(got.contains("Moe: send Luke a printer model"));
        assert!(!got.to_lowercase().contains("none"));
        assert!(!got.contains("Open questions"));
    }
}
