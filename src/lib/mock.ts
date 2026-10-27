// Sample data for previewing the UI in a normal browser (no Tauri backend).
import type { HistoryItem, Settings } from "./api";

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
  meeting_apps: ["teams", "zoom", "browser", "webex", "slack"],
  onboarded: !location.search.includes("setup"), // preview the setup flow with ?setup
};

const history: HistoryItem[] = [
  { id: 6, ts: ago(2), app: "Outlook", style: "email", raw: "سلام وقتتون بخیر خواستم بگم گزارش رو فرستادم لطفا یه نگاهی بندازید ممنون", cleaned: "سلام،\nوقتتون بخیر.\n\nخواستم بگم گزارش رو فرستادم. لطفاً یه نگاهی بندازید.\n\nممنون", corrected: null, audio: "a.wav", secs: 6.1, asr_ms: 1480, llm_ms: 2210 },
  { id: 5, ts: ago(14), app: "Jira", style: "ticket", raw: "من دیپلوی را روی سرور استیجینگ انجام دادم و همه‌ی تزها پس شدم", cleaned: "من deploy رو روی سرور staging انجام دادم و همه‌ی تست‌ها pass شدن.", corrected: null, audio: "b.wav", secs: 4.2, asr_ms: 1310, llm_ms: 1620 },
  { id: 4, ts: ago(40), app: "Teams", style: "chat", raw: "سلام، جلسه رو بذار برای سه شنبه نه ببخشید پنجشنبه ساعت ده", cleaned: "سلام، جلسه رو بذار برای پنجشنبه ساعت ده.", corrected: null, audio: "c.wav", secs: 3.9, asr_ms: 1190, llm_ms: 900 },
  { id: 3, ts: ago(95), app: "Gmail", style: "email", raw: "send the contract to john no wait to samantha by friday", cleaned: "Send the contract to Samantha by Friday.", corrected: "Send the contract to Sam by Friday.", audio: "d.wav", secs: 3.1, asr_ms: 980, llm_ms: 1100 },
];

const meetingNotes = "## Summary\n- Release moves to Thursday; staging is green.\n- Marketing budget is cut by 10% next quarter.\n## Decisions\n- Sam owns the release checklist.\n## Action items\n- [ ] Sam: send the updated proposal (Friday)\n- [ ] You: book the review meeting with John\n## Open questions\n- Do we need a second QA pass?";
const meetingSegs = [
  { who: "Them", start: 2, text: "سلام، خوبین؟ بریم سراغ ریلیز این هفته." },
  { who: "You", start: 9, text: "Staging is green, all tests passed. I think we can ship Thursday." },
  { who: "Them", start: 21, text: "Sounds good. Sam, can you own the release checklist?" },
  { who: "You", start: 30, text: "باشه، من جلسه‌ی ریویو رو با جان می‌ذارم." },
];
const meetings = [
  { id: 2, ts: ago(30), secs: 1860, title: "Release plan and budget", people: "Sam", cal: "Release plan", search: "Release plan and budget\nSam\nRelease plan\nstaging Thursday checklist", transcript: JSON.stringify(meetingSegs), notes: meetingNotes, my_notes: "release thu?\nbudget -10%", chat: JSON.stringify([{ role: "you", text: "Who owns the release checklist?" }, { role: "ai", text: "Sam — she agreed at 0:21." }]) },
  { id: 1, ts: ago(60 * 26), secs: 920, title: "Weekly sync", people: "", cal: "", search: "Weekly sync", transcript: JSON.stringify(meetingSegs.slice(0, 2)), notes: "## Summary\n- Short status round.", my_notes: "", chat: "[]" },
];
let meetingOn = false;
let meetingStarted = 0;

export function mockInvoke(cmd: string, args: any): any {
  switch (cmd) {
    case "meeting_start": meetingOn = true; meetingStarted = Date.now(); return;
    case "meeting_resume": meetingOn = true; meetingStarted = Date.now() - 1860_000; return;
    case "meeting_stop": meetingOn = false; return 2;
    case "meeting_status": return { running: meetingOn, secs: (Date.now() - meetingStarted) / 1000, segments: meetingOn ? meetingSegs : [], my_notes: "", title: "", people: "" };
    case "meeting_ask": return [...JSON.parse(meetings[0].chat), { role: "you", text: args.question }, { role: "ai", text: "Mock answer: Sam sends the proposal by Friday." }];
    case "call_apps": return [{ id: "teams", name: "Microsoft Teams", installed: true }, { id: "zoom", name: "Zoom", installed: true }, { id: "browser", name: "Google Meet & browser calls", installed: true }, { id: "webex", name: "Webex", installed: false }, { id: "slack", name: "Slack huddles", installed: false }, { id: "discord", name: "Discord", installed: true }, { id: "whatsapp", name: "WhatsApp", installed: false }];
    case "app_version": return "0.1.3";
    case "update_status": case "update_check": return { version: "0.1.4", notes: "Fixes and improvements." };
    case "update_install": return;
    case "call_start": case "call_dismiss": case "call_hide": case "call_open": return;
    case "meeting_set_my_notes": case "meeting_save_notes": case "meeting_clear_chat": return;
    case "meetings_list": return meetings.map((m) => ({ ...m, transcript: "", notes: "" }));
    case "meeting_get": return meetings.find((m) => m.id === args.id) ?? null;
    case "meeting_delete": case "meeting_renotes": return;
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
