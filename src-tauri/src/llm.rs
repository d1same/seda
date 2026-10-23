//! Local text AI: runs llama-server (Vulkan) with Gemma 4 E4B as a hidden child process.
//! Prompts are the ones that scored 11.5/12 in the bake-off (bakeoff/llm/prompts.py).
use anyhow::{Result, anyhow};
use serde_json::json;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PORT: u16 = 8791;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct Llm {
    child: Child,
    http: reqwest::blocking::Client,
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

const MEETING: &str = "You write meeting notes from a transcript. Each line starts with [mm:ss] and the speaker: \"You\" is the user, \"Them\" is everyone else on the call. The transcript comes from speech recognition, so some words are wrong: infer the meaning, never quote obvious mistakes.
Write the notes in the language most of the meeting was in. Output Markdown only, in this order:
# <a short title, 3-7 words>
## Summary
3-6 bullets: what was discussed and concluded.
## Decisions
Bullets.
## Action items
- [ ] <who>: <task> (<deadline if one was said>)  (when nobody was named, write just the task)
## Open questions
Bullets.
Leave out any section that would be empty (except Summary). Never invent names, numbers, dates or tasks that were not said.";

const MEETING_PART: &str = "You take notes on one part of a longer meeting transcript. Lines start with [mm:ss] and the speaker (\"You\" is the user, \"Them\" is everyone else). Write short Markdown bullets of everything that matters: topics, conclusions, decisions, tasks with owners and deadlines, open questions. Same language as the transcript. Never invent anything.";

const MEETING_MERGE: &str = "You get notes from consecutive parts of one meeting. Merge them into final meeting notes. Output Markdown only, in this order:
# <a short title, 3-7 words>
## Summary
3-6 bullets.
## Decisions
## Action items
- [ ] <who>: <task> (<deadline if one was said>)
## Open questions
Leave out empty sections (except Summary). Remove duplicates. Keep the notes' language. Never invent anything.";

/// The model sometimes writes "## Open questions" followed by just "None". Remove such sections.
pub fn drop_empty_sections(md: &str) -> String {
    let empty = |l: &str| {
        let l = l.trim().trim_start_matches(['-', '*', ' ']).trim().trim_end_matches('.').to_lowercase();
        l.is_empty() || (l.starts_with('(') && l.ends_with(')')) || ["none", "n/a", "nothing", "ندارد", "هیچ", "هیچ‌کدام", "-"].contains(&l.as_str())
    };
    let mut out: Vec<&str> = Vec::new();
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with("## ") {
            let mut j = i + 1;
            while j < lines.len() && !lines[j].starts_with("#") {
                j += 1;
            }
            if lines[i + 1..j].iter().all(|l| empty(l)) {
                i = j;
                continue;
            }
        }
        out.push(lines[i]);
        i += 1;
    }
    out.join("\n")
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
        let child = Command::new(server)
            .args(["-m", model, "--port", &PORT.to_string(), "--host", "127.0.0.1", "-ngl", layers, "-c", "16384", "--jinja", "--reasoning", "off"])
            .stdout(Stdio::null())
            .stderr(Stdio::from(engine_log))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        die_with_parent(&child);
        crate::log::log(format!("writing AI starting on {} ({})", if gpu { "GPU" } else { "CPU" }, model));
        let http = reqwest::blocking::Client::builder().timeout(Duration::from_secs(60)).build()?;
        let mut me = Self { child, http };
        let t = Instant::now();
        let mut noted = false;
        loop {
            if let Ok(Some(status)) = me.child.try_wait() {
                return Err(anyhow!("Text AI exited early ({status}) — see llama.log"));
            }
            if let Ok(r) = me.http.get(format!("http://127.0.0.1:{PORT}/health")).send() {
                if r.status().is_success() {
                    break;
                }
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
        let body = json!({
            "messages": messages,
            "temperature": 0.2,
            "max_tokens": max_tokens,
            "cache_prompt": true,
            "chat_template_kwargs": {"enable_thinking": false},
        });
        let r: serde_json::Value = self
            .http
            .post(format!("http://127.0.0.1:{PORT}/v1/chat/completions"))
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

    /// Meeting notes in Markdown. Long meetings are summarised part by part first (each part fits
    /// the engine's context comfortably), then the part notes are merged. `language`: when set
    /// (e.g. "Persian"), the notes are written in it. `my_notes`: what the user typed during the meeting.
    pub fn meeting_notes(&mut self, transcript: &str, language: Option<&str>, my_notes: &str, user: &str) -> Result<String> {
        let mut lang = language.map(|l| format!("\n\nWrite the notes in {l}. Keep names and technical terms as they were said.")).unwrap_or_default();
        lang.push_str(&who_is_you(user));
        let mine = my_notes.trim();
        if !mine.is_empty() {
            lang.push_str("\n\nThe user also typed their own notes during the meeting (given before the transcript). Build on them: keep every point they wrote, in their words, and add the details from the transcript.");
        }
        let with_mine = |body: &str| if mine.is_empty() { body.to_string() } else { format!("My own notes:\n{mine}\n\n---\n\n{body}") };
        let (single, part, merge) = (format!("{MEETING}{lang}"), format!("{MEETING_PART}{lang}"), format!("{MEETING_MERGE}{lang}"));
        const PART: usize = 12_000; // characters, roughly 4k tokens
        let timeout = Duration::from_secs(600);
        if transcript.chars().count() <= PART {
            return Ok(drop_empty_sections(&self.chat_with(&single, &with_mine(transcript), 1500, timeout)?));
        }
        let mut parts = Vec::new();
        let mut cur = String::new();
        for line in transcript.lines() {
            if cur.chars().count() + line.chars().count() > PART && !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            cur.push_str(line);
            cur.push('\n');
        }
        if !cur.is_empty() {
            parts.push(cur);
        }
        let mut notes = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            crate::log::log(format!("meeting notes: part {} of {}", i + 1, parts.len()));
            notes.push(self.chat_with(&part, p, 900, timeout)?);
        }
        Ok(drop_empty_sections(&self.chat_with(&merge, &with_mine(&notes.join("\n\n---\n\n")), 1500, timeout)?))
    }
}
