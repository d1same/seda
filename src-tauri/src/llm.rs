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
- Four or more people is still one meeting: each named person is a different person. Write what each of them did or was asked to do. Do not merge two people into one, and do not leave one of them out.
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
    fn meeting_extra(language: Option<&str>, user: &str, has_mine: bool, vocab: &[String], people: &[String]) -> String {
        let mut s = language.map(|l| format!("\n\nWrite the notes in {l}. Keep names and technical terms as they were said.")).unwrap_or_default();
        s.push_str(&who_is_you(user));
        s.push_str(&people_line(people));
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
        let people = meeting_people(transcript, user, vocab);
        let system = format!("{}{}", MEETING.replace("{RULES}", NOTE_RULES), Self::meeting_extra(language, user, !mine.is_empty(), vocab, &people));
        let mut body = if mine.is_empty() { transcript.to_string() } else { format!("My own notes:\n{mine}\n\n---\n\n{transcript}") };
        body = with_people(body, &people);
        // Lists are written first, then moved under the topics. 2400 tokens so a side topic
        // (a license, a price) is not cut off the end.
        Ok(lists_last(&self.chat_with(&system, &body, 2400, Duration::from_secs(600))?))
    }

    /// Detailed notes for one ~10-minute stretch of a long meeting (written while it is still going).
    pub fn meeting_part(&mut self, transcript: &str, language: Option<&str>, user: &str, vocab: &[String]) -> Result<String> {
        let people = meeting_people(transcript, user, vocab);
        let system = format!("{}{}", MEETING_PART.replace("{RULES}", NOTE_RULES), Self::meeting_extra(language, user, false, vocab, &people));
        self.chat_with(&system, &with_people(transcript.to_string(), &people), 1600, Duration::from_secs(600))
    }

    /// When four or more people were named and the notes missed some, ask once for just those
    /// people and merge the bullets in. The rest of the notes stay as they are.
    pub fn add_missing_people(&mut self, notes: &str, transcript: &str, people: &[String], user: &str) -> Result<String> {
        let missing = missing_people(notes, people);
        if !should_fill_people(people, notes) {
            return Ok(notes.to_string());
        }
        let clip = lines_about(transcript, &missing, user);
        let who = if user.trim().is_empty() { "the user".to_string() } else { user.trim().to_string() };
        let system = format!(
            "These people were in the meeting and are missing from the notes: {}.\n\
             \"You\" in the transcript is {who}.\n\
             From the transcript lines only, write one bullet per fact, as \"- Name: fact\". \
             One person per bullet. Only these people. If the lines do not say what someone did or will do, skip that person. \
             A task uses a verb such as send, approve, buy, or follow up. Never invent. Never write Them. Output bullets only.",
            missing.join(", ")
        );
        let reply = self.chat_with(&system, &format!("Transcript lines:\n{clip}"), 700, Duration::from_secs(180))?;
        Ok(merge_people_bullets(notes, &reply, &missing))
    }

    /// Final notes from the part notes of a long meeting. The AI only writes the title and topics;
    /// decisions, next steps and open questions are collected from the parts by code, so the merge
    /// can never drop one (a small model rewriting everything at once loses items).
    pub fn meeting_merge(&mut self, parts: &[String], language: Option<&str>, my_notes: &str, user: &str, _minutes: u32, vocab: &[String], people: &[String]) -> Result<String> {
        let mine = my_notes.trim();
        let system = format!("{}{}", MEETING_MERGE.replace("{RULES}", NOTE_RULES), Self::meeting_extra(language, user, !mine.is_empty(), vocab, people));
        let topics_only: Vec<String> = parts.iter().map(|p| strip_sections(p, &LIST_SECTIONS)).collect();
        let joined = topics_only.iter().enumerate().map(|(i, p)| format!("### Part {}\n{p}", i + 1)).collect::<Vec<_>>().join("\n\n");
        let body = with_people(if mine.is_empty() { joined } else { format!("My own notes:\n{mine}\n\n---\n\n{joined}") }, people);
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

/// Words that show up in meetings and are not people's names.
fn not_a_name(low: &str) -> bool {
    const STOP: &[&str] = &[
        "a", "about", "actually", "adobe", "after", "again", "all", "already", "also", "am", "an", "and", "another", "any", "anybody", "anyone", "anything", "april", "are", "ask", "asked", "assign", "at", "august", "back", "bandwidth", "be", "because", "been", "before", "big", "but", "buy", "call", "can", "cant", "check", "chf", "come", "comes", "confirm", "contract", "could", "current", "december", "dedicated", "dhcp", "dont", "down", "each", "email", "eu", "even", "everyone", "everything", "example", "february", "follow", "for", "free", "friday", "from", "full", "get", "gets", "give", "gives", "going", "good", "got", "great", "guest", "guests", "had", "has", "have", "he", "hear", "hearing", "hello", "her", "here", "hey", "hi", "high", "him", "his", "how", "i", "if", "im", "in", "internet", "into", "ip", "is", "it", "its", "ive", "january", "july", "june", "just", "know", "lan", "last", "let", "lets", "license", "like", "local", "long", "look", "looking", "low", "mac", "make", "makes", "many", "march", "may", "maybe", "mbps", "me", "mean", "means", "meeting", "monday", "more", "most", "much", "my", "need", "needs", "network", "new", "next", "no", "not", "nothing", "november", "now", "october", "of", "office", "ok", "okay", "old", "on", "only", "onto", "or", "other", "our", "out", "over", "part", "pdf", "per", "please", "price", "printer", "priority", "private", "provide", "put", "quote", "really", "remaining", "remote", "review", "right", "said", "same", "saturday", "say", "says", "see", "send", "september", "service", "shall", "share", "shared", "she", "short", "should", "so", "some", "somebody", "someone", "something", "speed", "ssid", "static", "still", "stuff", "such", "sunday", "sure", "take", "takes", "talk", "talking", "team", "tell", "than", "thank", "thanks", "that", "thats", "the", "their", "them", "then", "there", "these", "they", "theyre", "thing", "think", "this", "those", "thursday", "time", "to", "today", "told", "tomorrow", "too", "total", "tuesday", "uk", "up", "usa", "usd", "use", "user", "users", "using", "very", "via", "vlan", "wan", "want", "wants", "was", "way", "we", "wednesday", "week", "well", "were", "what", "when", "where", "which", "who", "why", "wifi", "will", "wireless", "with", "work", "working", "would", "yeah", "yep", "yes", "yesterday", "you", "your", "youre", "zoom",
    ];
    const ALSO: &[&str] = &["available", "center", "dynamic", "geneva", "london", "paris", "switzerland"];
    STOP.contains(&low) || ALSO.contains(&low)
}

fn name_case(raw: &str) -> String {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else { return String::new() };
    let rest = chars.as_str();
    if first.is_uppercase() && rest.chars().any(|c| c.is_lowercase()) {
        return raw.to_string();
    }
    format!("{}{}", first.to_uppercase(), rest.to_lowercase())
}

fn name_like(low: &str, _raw: &str) -> bool {
    low.chars().count() >= 3 && low.chars().all(|c| c.is_alphabetic()) && !not_a_name(low)
}

fn next_is_cue(low: &str) -> bool {
    matches!(low, "will" | "can" | "could" | "should" | "shall" | "need" | "needs" | "want" | "wants" | "is" | "was" | "has" | "had" | "said" | "says")
}

fn prev_is_address(low: &str) -> bool {
    matches!(low, "ask" | "asked" | "tell" | "told" | "thanks" | "hi" | "hey" | "email" | "ping")
}

fn transcript_tokens(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '\'' || c == '’' {
            cur.push(c);
        } else if !cur.is_empty() {
            let low = cur.to_lowercase();
            out.push((std::mem::take(&mut cur), low));
        }
    }
    if !cur.is_empty() {
        let low = cur.to_lowercase();
        out.push((cur, low));
    }
    out
}

fn absorb_person(names: &mut Vec<String>, new_name: &str) {
    let low = new_name.to_lowercase();
    if names.iter().any(|n| n.eq_ignore_ascii_case(new_name)) {
        return;
    }
    // "Brad" is already covered by "Brad Hilder".
    if names.iter().any(|n| n.to_lowercase().starts_with(&format!("{low} "))) {
        return;
    }
    if let Some(i) = names.iter().position(|n| low.starts_with(&format!("{} ", n.to_lowercase()))) {
        names[i] = new_name.to_string();
        return;
    }
    if names.len() < 12 {
        names.push(new_name.to_string());
    }
}

fn people_line(people: &[String]) -> String {
    if people.is_empty() {
        return String::new();
    }
    let mut s = format!("\n\nPeople named in this meeting, each a different person: {}.", people.join(", "));
    if people.len() >= 4 {
        s.push_str(&format!(
            " There are {} people. Write what each one did or was asked to do when the transcript says it. Do not merge two people into one, and do not leave one out.",
            people.len()
        ));
    }
    s
}

fn with_people(body: String, people: &[String]) -> String {
    if people.is_empty() { body } else { format!("People in this meeting: {}.\n\n{body}", people.join(", ")) }
}

/// Names spoken in the meeting: the user, plus anyone addressed or about to do something.
/// "printer will" and "VLAN" are not people. Capped at 12.
pub fn meeting_people(transcript: &str, user: &str, vocab: &[String]) -> Vec<String> {
    let toks = transcript_tokens(transcript);
    let vocab_l: Vec<String> = vocab.iter().map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty()).collect();
    let mut caps: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (raw, low) in &toks {
        if raw.chars().next().is_some_and(|c| c.is_uppercase()) {
            *caps.entry(low.clone()).or_default() += 1;
        }
    }
    let mut names = Vec::new();
    let user_trim = user.trim();
    if !user_trim.is_empty() {
        names.push(user_trim.split_whitespace().map(name_case).collect::<Vec<_>>().join(" "));
    }
    let n = toks.len();
    let mut i = 0;
    while i < n {
        let (raw, low) = &toks[i];
        if !name_like(low, raw) {
            i += 1;
            continue;
        }
        let addr_before = i > 0 && prev_is_address(&toks[i - 1].1);
        if i + 1 < n && name_like(&toks[i + 1].1, &toks[i + 1].0) {
            let cue_after = i + 2 < n && next_is_cue(&toks[i + 2].1);
            if cue_after || addr_before {
                absorb_person(&mut names, &format!("{} {}", name_case(raw), name_case(&toks[i + 1].0)));
                i += 2;
                continue;
            }
        }
        let cue_after = i + 1 < n && next_is_cue(&toks[i + 1].1);
        let in_vocab = vocab_l.iter().any(|v| v == low || v.split_whitespace().any(|p| p == low));
        // A saved name that Whisper capitalized, even when nobody said "Alex will".
        let saved_and_capped = in_vocab && caps.get(low).copied().unwrap_or(0) >= 1;
        if cue_after || addr_before || saved_and_capped {
            absorb_person(&mut names, &name_case(raw));
        }
        i += 1;
    }
    names
}

fn contains_word(hay: &str, word: &str) -> bool {
    hay.split(|c: char| !c.is_alphanumeric()).any(|w| w == word)
}

fn first_name(name: &str) -> String {
    name.split_whitespace().next().unwrap_or(name).to_lowercase()
}

/// People from `people` whose first name is not in the notes.
pub fn missing_people(notes: &str, people: &[String]) -> Vec<String> {
    let notes_l = notes.to_lowercase();
    people.iter().filter(|p| !contains_word(&notes_l, &first_name(p))).cloned().collect()
}

/// A second pass is worth it only when four or more people were named and at least one is missing.
fn should_fill_people(people: &[String], notes: &str) -> bool {
    people.len() >= 4 && !missing_people(notes, people).is_empty()
}

fn lines_about(transcript: &str, missing: &[String], user: &str) -> String {
    let user_l = user.trim().to_lowercase();
    let mut out = Vec::new();
    for line in transcript.lines() {
        let low = line.to_lowercase();
        let hit = missing.iter().any(|m| {
            let first = first_name(m);
            contains_word(&low, &first) || (!user_l.is_empty() && first == user_l && contains_word(&low, "you"))
        });
        if hit {
            out.push(line);
        }
        if out.len() >= 80 {
            break;
        }
    }
    out.join("\n")
}

fn is_task_fact(fact: &str) -> bool {
    let l = fact.to_lowercase();
    ["will ", "send", "follow", "approve", "buy", "provide", "email", "quote", "confirm", "procure", "share", "review", "assign", "schedule", "fix"].iter().any(|w| l.contains(w))
}

fn append_to_section(md: &str, heading: &str, bullets: &[String]) -> String {
    if bullets.is_empty() {
        return md.to_string();
    }
    let header = format!("## {heading}");
    if let Some(pos) = md.find(&header) {
        let after = pos + header.len();
        let rest = &md[after..];
        let insert_at = rest.find("\n## ").map(|rel| after + rel).unwrap_or(md.len());
        let mut out = md[..insert_at].trim_end().to_string();
        out.push('\n');
        out.push_str(&bullets.join("\n"));
        out.push('\n');
        if insert_at < md.len() {
            out.push_str(md[insert_at..].trim_start_matches('\n'));
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        return out;
    }
    format!("{}\n\n## {heading}\n{}\n", md.trim_end(), bullets.join("\n"))
}

/// Bullets from the fill-in pass, kept only when they name someone who was actually missing.
fn merge_people_bullets(notes: &str, bullets: &str, missing: &[String]) -> String {
    let mut tasks = Vec::new();
    let mut facts = Vec::new();
    for line in bullets.lines() {
        let body = line.trim().trim_start_matches(['-', '*', ' ']).trim();
        let body = body.trim_start_matches("[ ]").trim_start_matches("[x]").trim_start_matches("[X]").trim();
        let Some((who, fact)) = body.split_once(':') else { continue };
        let who = who.trim();
        let fact = fact.trim();
        if who.is_empty() || fact.is_empty() {
            continue;
        }
        if !missing.iter().any(|m| first_name(m) == first_name(who) || m.eq_ignore_ascii_case(who)) {
            continue;
        }
        if is_task_fact(fact) {
            tasks.push(format!("- [ ] {who}: {fact}"));
        } else {
            facts.push(format!("- {who}: {fact}"));
        }
    }
    let notes = append_to_section(notes, "Next steps", &tasks);
    let notes = append_to_section(&notes, "People", &facts);
    drop_empty_sections(&notes)
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

    #[test]
    fn four_people_stay_four_people() {
        let transcript = "\
[00:01] You: I'll send the model
[00:20] Them: Jose will assign the private vlan
[00:40] Them: Luke will buy the printer in Switzerland
[01:00] Them: ask Natalie to approve the quote
[01:20] Them: Mike should send the admin email
[01:40] Them: the printer will sit on the vlan and the ssid is shared
";
        let people = meeting_people(transcript, "Moe", &[]);
        assert_eq!(people, vec!["Moe", "Jose", "Luke", "Natalie", "Mike"]);
        assert!(should_fill_people(&people, "Jose assigns a vlan. Luke buys the printer."));
        assert!(!should_fill_people(&people, "Moe, Jose, Luke, Natalie and Mike all have a task."));
        assert!(!should_fill_people(&["Moe".into(), "Jose".into(), "Luke".into()], "only Jose"));
    }

    #[test]
    fn a_full_name_is_one_person_and_lowercase_still_counts() {
        let people = meeting_people("brad hilder will send the pdf. JOSE will assign the vlan.", "", &[]);
        assert_eq!(people, vec!["Brad Hilder", "Jose"]);
        assert!(meeting_people("the vlan is private and the printer will use dhcp", "", &[]).is_empty());
    }

    #[test]
    fn a_saved_name_counts_when_it_was_capitalized() {
        let people = meeting_people("We looped in Seb about the license. Seb agreed.", "", &["Seb".into()]);
        assert_eq!(people, vec!["Seb"]);
    }

    #[test]
    fn missing_people_are_added_without_touching_the_rest() {
        let notes = "\
# Office network
## Bandwidth
- 130 Mbps remains
## Next steps
- [ ] Jose: assign the private VLAN
";
        let missing = missing_people(notes, &["Moe".into(), "Jose".into(), "Luke".into(), "Seb".into()]);
        assert_eq!(missing, vec!["Moe", "Luke", "Seb"]);
        let patched = "\
- Luke: will buy the printer locally
- Nobody: will steal the quote
- Seb: handled the Adobe license last week
";
        let got = merge_people_bullets(notes, patched, &missing);
        assert!(got.contains("Jose: assign the private VLAN"));
        assert!(got.contains("130 Mbps remains"));
        assert!(got.contains("[ ] Luke: will buy the printer locally"));
        assert!(got.contains("Seb: handled the Adobe license last week"));
        assert!(!got.contains("Nobody"));
    }
}
