// Sample data for previewing the UI in a normal browser (no Tauri backend).
import type { ChatTurn, HistoryItem, Meeting, SearchHit, Settings, Template } from "./api";
import { emitLocal } from "./events";

const now = Date.now();
const ago = (min: number) => new Date(now - min * 60_000).toISOString();

let settings: Settings = {
  dictate_binding: { kind: "mouse", code: 1, ctrl: false, shift: false, alt: false, win: false },
  dictate_mode: "both",
  fix_binding: { kind: "key", code: 0x20, ctrl: true, shift: true, alt: false, win: false },
  language: "fa",
  cleanup: true,
  app_styles: true,
  latin_tech_terms: true,
  save_audio: true,
  sounds: true,
  pill_position: "bottom",
  idle_dot: false,
  autostart: true,
  input_device: "",
  polish: "smart",
  whisper_model: "C:\\Users\\you\\.venvs\\whispercpp\\models\\ggml-fa-turbo-q5_0.bin",
  llm_server: "C:\\Users\\you\\.venvs\\llm\\bin\\llama-server.exe",
  llm_model: "C:\\Users\\you\\.venvs\\llm\\models\\gemma-4-E4B-it-Q4_K_M.gguf",
  meeting_detect: "ask",
  user_name: "",
  email: "",
  snippets: [],
  keep_meeting_audio: false,
  templates: [{
    id: "c_standup1", name: "Stand-up", about: "A short daily team stand-up.",
    sections: [
      { heading: "Done", hint: "finished since last time, by person" },
      { heading: "Doing", hint: "in progress now, by person" },
      { heading: "Blocked", hint: "what is stuck and what it waits on" },
      { heading: "Next steps", hint: "" },
    ],
  }],
  meeting_apps: ["teams", "zoom", "browser", "webex", "slack"],
  onboarded: !location.search.includes("setup"), // preview the setup flow with ?setup
};

const history: HistoryItem[] = [
  { id: 6, ts: ago(2), app: "Outlook", style: "email", raw: "سلام وقتتون بخیر خواستم بگم گزارش رو فرستادم لطفا یه نگاهی بندازید ممنون", cleaned: "سلام،\nوقتتون بخیر.\n\nخواستم بگم گزارش رو فرستادم. لطفاً یه نگاهی بندازید.\n\nممنون", corrected: null, audio: "a.wav", secs: 6.1, asr_ms: 1480, llm_ms: 2210 },
  { id: 5, ts: ago(14), app: "Jira", style: "ticket", raw: "من دیپلوی را روی سرور استیجینگ انجام دادم و همه‌ی تزها پس شدم", cleaned: "من deploy رو روی سرور staging انجام دادم و همه‌ی تست‌ها pass شدن.", corrected: null, audio: "b.wav", secs: 4.2, asr_ms: 1310, llm_ms: 1620 },
  { id: 4, ts: ago(40), app: "Teams", style: "chat", raw: "سلام، جلسه رو بذار برای سه شنبه نه ببخشید پنجشنبه ساعت ده", cleaned: "سلام، جلسه رو بذار برای پنجشنبه ساعت ده.", corrected: null, audio: "c.wav", secs: 3.9, asr_ms: 1190, llm_ms: 900 },
  { id: 3, ts: ago(95), app: "Gmail", style: "email", raw: "send the contract to john no wait to samantha by friday", cleaned: "Send the contract to Samantha by Friday.", corrected: "Send the contract to Sam by Friday.", audio: "d.wav", secs: 3.1, asr_ms: 980, llm_ms: 1100 },
];

const meetingNotes = "## Release\n- Release Thursday?\n- Release moves to Thursday; staging is green.\n## Budget\n- Budget -10%\n- Marketing budget is cut by 10% next quarter.\n## Decisions\n- Sam owns the release checklist.\n## Action items\n- [ ] Sam: send the updated proposal (Friday)\n- [ ] You: book the review meeting with Alex\n## Open questions\n- Do we need a second QA pass?";
const meetingSegs = [
  { who: "Them", start: 2, text: "سلام، خوبین؟ بریم سراغ ریلیز این هفته." },
  { who: "You", start: 9, text: "Staging is green, all tests passed. I think we can ship Thursday." },
  { who: "Them", start: 21, text: "Sounds good. Sam, can you own the release checklist?" },
  { who: "You", start: 30, text: "باشه، من جلسه‌ی ریویو رو با Alex می‌ذارم." },
];
// A Persian meeting (no personal names in Persian script), so RTL titles and snippets can be seen.
const faSegs = [
  { who: "Them", start: 4, text: "سلام، امروز درباره‌ی بودجه‌ی تبلیغات و تاریخ انتشار نسخه‌ی موبایل صحبت می‌کنیم." },
  { who: "You", start: 15, text: "بودجه‌ی تبلیغات ده درصد کم می‌شه، ولی تاریخ انتشار تغییر نمی‌کنه." },
  { who: "Them", start: 31, text: "پس Sam تا پنجشنبه گزارش تست‌ها رو می‌فرسته؟" },
  { who: "You", start: 40, text: "آره، و من فهرست کارهای انتشار رو به‌روز می‌کنم." },
];
const faNotes = "## بودجه\n- بودجه‌ی تبلیغات ده درصد کم می‌شود.\n## انتشار\n- تاریخ انتشار نسخه‌ی موبایل تغییر نمی‌کند.\n## Next steps\n- [ ] **گزارش تست‌ها را بفرست** (Sam)\n- [ ] **فهرست کارهای انتشار را به‌روز کن** (You)";
const meetings: Meeting[] = [
  { id: 2, ts: ago(30), secs: 1860, title: "Release plan and budget", people: "Sam", cal: "Release plan", template: "", transcript: JSON.stringify(meetingSegs), notes: meetingNotes, my_notes: "# Release\n- release thursday?\n# Budget\n- budget -10%", chat: JSON.stringify([{ role: "you", text: "Who owns the release checklist?" }, { role: "ai", text: "Sam — she agreed at 0:21." }]) },
  { id: 1, ts: ago(60 * 26), secs: 920, title: "Weekly sync", people: "", cal: "", template: "", transcript: JSON.stringify(meetingSegs.slice(0, 2)), notes: "## Summary\n- Short status round.", my_notes: "", chat: "[]" },
  { id: 3, ts: ago(60 * 24 * 3), secs: 2400, title: "بودجه‌ی تبلیغات و انتشار نسخه‌ی موبایل", people: "Sam | Alex", cal: "", template: "", transcript: JSON.stringify(faSegs), notes: faNotes, my_notes: "", chat: "[]" },
];
/** The live recording's row id (apart from the finished meetings above, so a template pick while
 *  recording never touches them). */
const LIVE_ID = 4;
let liveTemplate = "";
let meetingOn = false;
let meetingStarted = 0;
let askLog: ChatTurn[] = [];

const BUILTINS: Template[] = [
  { id: "auto", name: "Auto", about: "", sections: [], builtin: true },
  { id: "phone_call", name: "Phone call", about: "A phone call or quick voice call. Capture concrete facts, numbers, dates, agreements, and follow-ups cleanly.", builtin: true,
    sections: [{ heading: "Context", hint: "who was on the call, why they spoke, and any account, order, or reference details" }, { heading: "Key points", hint: "main facts, updates, numbers, dates, and details discussed, grouped with sub-bullets" }, { heading: "Outcomes & agreements", hint: "what was resolved, confirmed, or decided on the call" }, { heading: "Next steps", hint: "" }] },
  { id: "ticket", name: "Troubleshooting ticket", about: "A support or troubleshooting call. The notes are pasted into a ticket.", builtin: true,
    sections: [{ heading: "Problem", hint: "what is wrong" }, { heading: "What we tried", hint: "each real step, in order" }, { heading: "Solution", hint: "what fixed it, or what is still broken" }, { heading: "Next steps", hint: "" }] },
  { id: "one_on_one", name: "1:1", about: "A one-to-one between the user and one other person.", builtin: true,
    sections: [{ heading: "Updates", hint: "what each person reported since last time: progress, results, numbers" }, { heading: "Feedback", hint: "feedback given in either direction, with the example that was mentioned" }, { heading: "Blockers", hint: "what is in the way, and who can unblock it" }, { heading: "Decisions", hint: "" }, { heading: "Next steps", hint: "" }] },
  { id: "customer", name: "Customer call", about: "A call with a customer or a prospect.", builtin: true,
    sections: [{ heading: "Context", hint: "who they are, their company, what they use today" }, { heading: "Needs", hint: "what they want and why, in their words; numbers and deadlines" }, { heading: "Concerns", hint: "objections, risks, and competitors they mentioned" }, { heading: "Decisions", hint: "" }, { heading: "Next steps", hint: "" }] },
  { id: "interview", name: "Interview", about: "A job interview. The user is the interviewer unless the transcript shows otherwise.", builtin: true,
    sections: [{ heading: "Background", hint: "experience and current role, as the candidate described it" }, { heading: "Strengths", hint: "what they did well, each with the evidence from their answers" }, { heading: "Concerns", hint: "gaps or weak answers, each with the evidence" }, { heading: "Candidate's questions", hint: "what they asked and what they were told" }, { heading: "Next steps", hint: "" }] },
];

const EXPORT_DIR = "C:\\Users\\you\\Documents\\Seda\\Meetings";
const later = <T,>(ms: number, v: () => T) => new Promise<T>((res) => setTimeout(() => res(v()), ms));
const segsOf = (m: Meeting): { who: string; text: string }[] => { try { return JSON.parse(m.transcript || "[]"); } catch { return []; } };

/** Mock search: every word must appear somewhere in the meeting; the snippet comes from the first
 *  line holding the first word. (The core uses FTS5 with Persian folding; this is just enough to see rows.) */
function mockSearch(query: string): SearchHit[] {
  const words = query.toLowerCase().split(/\s+/).filter((w) => w.length > 1);
  if (!words.length) return [];
  const snip = (line: string, w: string): [string, string, string] | null => {
    const at = line.toLowerCase().indexOf(w);
    if (at < 0) return null;
    let a = at, b = at + w.length;
    while (a > 0 && !/\s/.test(line[a - 1])) a--;
    while (b < line.length && !/\s/.test(line[b])) b++;
    return [line.slice(Math.max(0, a - 60), a), line.slice(a, b), line.slice(b, b + 90)];
  };
  const hits: SearchHit[] = [];
  for (const m of meetings) {
    const said = segsOf(m).map((s) => `${s.who}: ${s.text}`);
    const notes = `${m.notes}\n${m.my_notes}`.split("\n").filter(Boolean);
    const all = [m.title, m.cal ?? "", m.people ?? "", ...notes, ...said].join("\n").toLowerCase();
    if (!words.every((w) => all.includes(w))) continue;
    const w = words[0];
    let hit: SearchHit | null = null;
    const t = snip(`${m.title} ${m.cal ?? ""}`, w);
    if (t) hit = { id: m.id, field: "title", snippet: t };
    else if ((m.people ?? "").toLowerCase().includes(w)) hit = { id: m.id, field: "people", snippet: ["", "", ""] };
    for (const l of notes) { if (hit) break; const s = snip(l.replace(/^[#\-*\s]+(\[[ x]\]\s*)?/i, ""), w); if (s) hit = { id: m.id, field: "notes", snippet: s }; }
    for (const l of said) { if (hit) break; const s = snip(l, w); if (s) hit = { id: m.id, field: "said", snippet: s }; }
    hits.push(hit ?? { id: m.id, field: "said", snippet: ["", "", ""] });
  }
  return hits;
}

export function mockInvoke(cmd: string, args: any): any {
  switch (cmd) {
    case "meeting_start": meetingOn = true; meetingStarted = Date.now(); liveTemplate = ""; setTimeout(() => emitLocal("meeting-state", true), 50); return;
    case "meeting_resume": meetingOn = true; meetingStarted = Date.now() - 1860_000; setTimeout(() => emitLocal("meeting-state", true), 50); return;
    case "meeting_stop": meetingOn = false; return 2;
    case "meeting_status": return { running: meetingOn, secs: (Date.now() - meetingStarted) / 1000, segments: meetingOn ? meetingSegs : [], my_notes: "", title: "", people: "", id: meetingOn ? LIVE_ID : 0, template: meetingOn ? liveTemplate : "" };
    // Like the core, every turn carries `sources` (empty for a single meeting); the answer here also edited the notes.
    case "meeting_ask": return later(700, () => [...JSON.parse(meetings[0].chat), { role: "you", text: args.question }, { role: "ai", text: "Mock answer: Sam sends the proposal by Friday.", edited: true }]
      .map((t: ChatTurn) => ({ ...t, sources: t.sources ?? [] })));
    case "meetings_search": return mockSearch(args.query);
    case "meetings_ask": {
      if (args.ids && args.ids.length === 0) return Promise.reject("There are no meetings to ask about yet.");
      // [1] and [2] cited, [3] read but not cited. A deleted meeting keeps its source (and a disabled button).
      const gone = (id: number, title: string, ts: string) => meetings.find((m) => m.id === id) ?? { id, title, ts };
      const read = [gone(2, "Release plan and budget", ago(30)), gone(1, "Weekly sync", ago(60 * 26)), gone(3, "بودجه‌ی تبلیغات و انتشار نسخه‌ی موبایل", ago(60 * 24 * 3))];
      const sources = read.map((m, i) => ({ n: i + 1, id: m.id, title: m.title, ts: m.ts, cited: i < 2 }));
      askLog = [...askLog, { role: "you" as const, text: args.question },
        { role: "ai" as const, text: "Sam owns the release checklist [1]. The budget cut was agreed a day earlier [2].", sources }].slice(-20);
      return later(900, () => [...askLog]);
    }
    case "meetings_ask_log": return [...askLog];
    case "meetings_ask_clear": askLog = []; return;
    case "meeting_templates": return [...BUILTINS, ...settings.templates.map((t) => ({ ...t, builtin: false }))];
    case "meeting_set_template": {
      if (args.id === LIVE_ID && meetingOn) { liveTemplate = args.template; return; }
      const m = meetings.find((x) => x.id === args.id);
      if (!m) return Promise.reject("No such meeting.");
      if ((m.template || "auto") === args.template) return;
      const { title, notes } = m;
      Object.assign(m, { template: args.template, title: "", notes: "" });
      emitLocal("meetings-changed", null);
      setTimeout(() => { Object.assign(m, { title, notes }); emitLocal("meetings-changed", null); }, 1500);
      return;
    }
    case "meeting_rebuild": return;
    case "meeting_copy": {
      const m = meetings.find((x) => x.id === args.id);
      if (!m) return Promise.reject("No such meeting.");
      const lines = segsOf(m).map((s) => `${s.who}: ${s.text}`).join("\n");
      const text = args.what === "transcript" ? lines : args.what === "mine" ? m.my_notes
        : args.what === "all" ? `# ${m.title}\n\n${m.notes}\n\n## Transcript\n\n${lines}` : `# ${m.title}\n\n${m.notes}`;
      // A preview tab may deny clipboard access; log instead so the "Copied" state can still be seen.
      return navigator.clipboard.writeText(text).catch(() => console.info("[mock] meeting_copy:", args.what, text));
    }
    case "meeting_export": return `${EXPORT_DIR}\\2026-11-11 0944 Release plan and budget.md`;
    case "meetings_export_all": return later(600, () => ({ count: meetings.length, dir: EXPORT_DIR }));
    case "exports_info": return { dir: EXPORT_DIR, cloud: false };
    case "open_exports_folder": return;
    case "call_apps": return [{ id: "teams", name: "Microsoft Teams", installed: true }, { id: "zoom", name: "Zoom", installed: true }, { id: "browser", name: "Google Meet & browser calls", installed: true }, { id: "webex", name: "Webex", installed: false }, { id: "slack", name: "Slack huddles", installed: false }, { id: "discord", name: "Discord", installed: true }, { id: "whatsapp", name: "WhatsApp", installed: false }];
    case "app_version": return "0.1.3";
    case "update_status": case "update_check": return { version: "0.1.4", notes: "Fixes and improvements." };
    case "update_install": return;
    case "call_start": case "call_dismiss": case "call_hide": case "call_open": return;
    case "meeting_save_notes": { const m = meetings.find((x) => x.id === args.id); if (m) { m.title = args.title; m.notes = args.notes; } return; }
    case "meeting_set_my_notes": case "meeting_clear_chat": return;
    case "meeting_update_my_notes": { const m = meetings.find((x) => x.id === args.id); if (m) m.my_notes = args.text; return; }
    case "meetings_list": return meetings.map((m) => ({ ...m, transcript: "", notes: "" }));
    case "meeting_get": { const m = meetings.find((x) => x.id === args.id); return m ? { ...m } : null; }
    case "meeting_delete": { const i = meetings.findIndex((x) => x.id === args.id); if (i >= 0) meetings.splice(i, 1); return; }
    case "meeting_renotes": return;
    case "get_settings": return settings;
    case "update_setting": settings = { ...settings, [args.key]: args.value }; return settings;
    case "get_status": return { asr: "ready", asr_error: null, llm: "ready", llm_error: null };
    case "history": return history;
    case "vocab_list": return ["Alex Morgan", "Jira", "staging", "Kubernetes"];
    case "fixes_list": return [{ wrong: "تزها", right: "تست‌ها", hits: 7 }, { wrong: "Samh", right: "Sam", hits: 3 }];
    case "training_stats": return { samples: 14, seconds: 13 * 60 + 20, total_dictations: 212 };
    case "correct_history": return [["Samh", "Sam"]];
    case "test_file": return { raw: "سلام امروز میخوام درباره برنامه هفته آینده با تیم صحبت کنم", text: "سلام، امروز می‌خوام درباره‌ی برنامه‌ی هفته‌ی آینده با تیم صحبت کنم.", asr_ms: 1230, llm_ms: 1400, secs: 6.3 };
    case "setup_info": {
      const item = (id: string, name: string, size_mb: number, state = "missing") =>
        ({ id, name, size_mb, state, path: `C:\\Users\\you\\AppData\\Local\\Seda\\models\\${id}` });
      const base = [item("speech", "Persian speech model", 548, "ready"), item("vad", "Voice detector", 1, "ready")];
      return {
        hardware: { gpu: "AMD Radeon(TM) Graphics", vram_gb: 3.2, ram_gb: 30.9, integrated: true, recommended: "full" },
        packs: {
          full: [...base, item("server", "AI engine (llama.cpp, GPU)", 35), item("llm", "Writing AI (Gemma 4 E4B)", 4600)],
          light: [...base, item("server", "AI engine (llama.cpp, GPU)", 35), item("llm", "Writing AI (Gemma 4 E2B, lighter)", 2900)],
          speech: base,
        },
        models_dir: "C:\\Users\\you\\AppData\\Local\\Seda\\models",
      };
    }
    case "list_mics": return ["Microphone Array (Realtek Audio)", "Headset (Jabra Evolve2)"];
    default: return null;
  }
}
