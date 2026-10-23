import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { inTauri } from "./events";
import { mockInvoke } from "./mock";

/** The windows can load before the Rust core has finished starting; until then every command fails
 *  with "state not managed". Retry those for up to ~15 s instead of leaving the UI blank. */
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return mockInvoke(cmd, args) as T;
  for (let attempt = 0; ; attempt++) {
    try {
      return await tauriInvoke<T>(cmd, args);
    } catch (e) {
      if (attempt >= 60 || !String(e).includes("not managed")) throw e;
      await new Promise((r) => setTimeout(r, 250));
    }
  }
}

export type Binding = { kind: "mouse" | "key" | "none"; code: number; ctrl: boolean; shift: boolean; alt: boolean; win: boolean };

export type Settings = {
  dictate_binding: Binding;
  dictate_mode: "both" | "hold" | "toggle";
  fix_binding: Binding;
  language: string; // whisper code: "fa", "en", "es", …
  cleanup: boolean;
  app_styles: boolean;
  latin_tech_terms: boolean;
  save_audio: boolean;
  sounds: boolean;
  pill_position: "bottom" | "top";
  idle_dot: boolean;
  autostart: boolean;
  input_device: string; // "" = Windows default
  polish: "smart" | "always";
  whisper_model: string;
  llm_server: string;
  llm_model: string;
  onboarded: boolean;
  meeting_detect: "ask" | "auto" | "off";
  meeting_apps: string[];
  user_name: string;
  keep_meeting_audio: boolean;
};

export type CallApp = { id: string; name: string; installed: boolean };
export type UpdateInfo = { version: string; notes: string };

export type Status = { asr: string; asr_error: string | null; llm: string; llm_error: string | null };

export type HistoryItem = {
  id: number; ts: string; app: string; style: string; raw: string; cleaned: string;
  corrected: string | null; audio: string | null; secs: number; asr_ms: number; llm_ms: number;
};

export type Fix = { wrong: string; right: string; hits: number };

export type Segment = { who: "You" | "Them"; start: number; text: string };
/** In the list, transcript and notes are empty. 	itle is empty while the notes are being written. */
export type Meeting = { id: number; ts: string; secs: number; title: string; transcript: string; notes: string; my_notes: string; chat: string };
export type ChatTurn = { role: "you" | "ai"; text: string; edited?: boolean };
export type MeetingStatus = { running: boolean; secs: number; segments: Segment[]; my_notes: string };

export type Pack = "full" | "light" | "speech";
export type SetupItem = { id: string; name: string; size_mb: number; state: "ready" | "missing"; path: string };
export type SetupInfo = {
  hardware: { gpu: string; vram_gb: number; ram_gb: number; integrated: boolean; recommended: Pack };
  packs: Record<Pack, SetupItem[]>;
  models_dir: string;
};
export type TrainingStats = { samples: number; seconds: number; total_dictations: number };

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  updateSetting: (key: string, value: unknown) => invoke<Settings>("update_setting", { key, value }),
  setHotkeysPaused: (paused: boolean) => invoke<void>("set_hotkeys_paused", { paused }),
  localHotkey: (event: "down" | "up" | "cancel") => invoke<void>("local_hotkey", { event }),
  appVersion: () => invoke<string>("app_version"),
  updateStatus: () => invoke<UpdateInfo | null>("update_status"),
  updateCheck: () => invoke<UpdateInfo | null>("update_check"),
  updateInstall: () => invoke<void>("update_install"),
  getStatus: () => invoke<Status>("get_status"),
  reloadModels: () => invoke<void>("reload_models"),
  history: (limit = 200) => invoke<HistoryItem[]>("history", { limit }),
  correctHistory: (id: number, text: string) => invoke<[string, string][]>("correct_history", { id, text }),
  deleteHistory: (id: number) => invoke<void>("delete_history", { id }),
  vocabList: () => invoke<string[]>("vocab_list"),
  vocabAdd: (term: string) => invoke<void>("vocab_add", { term }),
  vocabRemove: (term: string) => invoke<void>("vocab_remove", { term }),
  fixesList: () => invoke<Fix[]>("fixes_list"),
  fixAdd: (wrong: string, right: string) => invoke<void>("fix_add", { wrong, right }),
  fixRemove: (wrong: string) => invoke<void>("fix_remove", { wrong }),
  testFile: (path: string, style: string) =>
    invoke<{ raw: string; text: string; asr_ms: number; llm_ms: number; secs: number }>("test_file", { path, style }),
  setupInfo: (language: string) => invoke<SetupInfo>("setup_info", { language }),
  setupDownload: (pack: Pack) => invoke<void>("setup_download", { pack }),
  setupCancel: () => invoke<void>("setup_cancel"),
  micTestStart: () => invoke<void>("mic_test_start"),
  micTestStop: () => invoke<void>("mic_test_stop"),
  finishOnboarding: () => invoke<void>("finish_onboarding"),
  diagnostics: () => invoke<string>("diagnostics"),
  openLog: () => invoke<void>("open_log"),
  listMics: () => invoke<string[]>("list_mics"),
  openMicPrivacy: () => invoke<void>("open_mic_privacy"),
  trainingStats: () => invoke<TrainingStats>("training_stats"),
  openDataFolder: () => invoke<void>("open_data_folder"),
  fixRewrite: (mode: string) => invoke<void>("fix_rewrite", { mode }),
  fixAccept: (text: string) => invoke<void>("fix_accept", { text }),
  fixCancel: () => invoke<void>("fix_cancel"),
  meetingStart: () => invoke<void>("meeting_start"),
  meetingStop: () => invoke<number | null>("meeting_stop"),
  meetingStatus: () => invoke<MeetingStatus>("meeting_status"),
  meetingsList: () => invoke<Meeting[]>("meetings_list"),
  meetingGet: (id: number) => invoke<Meeting | null>("meeting_get", { id }),
  meetingDelete: (id: number) => invoke<void>("meeting_delete", { id }),
  meetingRenotes: (id: number) => invoke<void>("meeting_renotes", { id }),
  meetingSetMyNotes: (text: string) => invoke<void>("meeting_set_my_notes", { text }),
  meetingSaveNotes: (id: number, title: string, notes: string) => invoke<void>("meeting_save_notes", { id, title, notes }),
  meetingAsk: (id: number, question: string) => invoke<ChatTurn[]>("meeting_ask", { id, question }),
  meetingClearChat: (id: number) => invoke<void>("meeting_clear_chat", { id }),
  callStart: () => invoke<void>("call_start"),
  callDismiss: () => invoke<void>("call_dismiss"),
  callHide: () => invoke<void>("call_hide"),
  callOpen: () => invoke<void>("call_open"),
  callApps: () => invoke<CallApp[]>("call_apps"),
};

const MOUSE: Record<number, string> = { 1: "Mouse 4 (Back)", 2: "Mouse 5 (Forward)", 3: "Middle click" };
const KEYS: Record<number, string> = {
  0x08: "Backspace", 0x09: "Tab", 0x0d: "Enter", 0x13: "Pause", 0x14: "Caps Lock", 0x1b: "Esc", 0x20: "Space",
  0x21: "Page Up", 0x22: "Page Down", 0x23: "End", 0x24: "Home", 0x25: "←", 0x26: "↑", 0x27: "→", 0x28: "↓",
  0x2c: "Print Screen", 0x2d: "Insert", 0x2e: "Delete", 0x91: "Scroll Lock", 0xa5: "Right Alt", 0xa3: "Right Ctrl",
  0xc0: "`", 0xbd: "-", 0xbb: "=", 0xdb: "[", 0xdd: "]", 0xdc: "\\", 0xba: ";", 0xde: "'", 0xbc: ",", 0xbe: ".", 0xbf: "/",
};

export function keyName(code: number): string {
  if (KEYS[code]) return KEYS[code];
  if ((code >= 0x30 && code <= 0x39) || (code >= 0x41 && code <= 0x5a)) return String.fromCharCode(code);
  if (code >= 0x70 && code <= 0x87) return `F${code - 0x6f}`;
  if (code >= 0x60 && code <= 0x69) return `Num ${code - 0x60}`;
  return `Key ${code}`;
}

/** Parts to render as keycaps, e.g. ["Ctrl", "Shift", "Space"]. */
export function bindingParts(b: Binding): string[] {
  if (b.kind === "mouse") return [MOUSE[b.code] ?? `Mouse ${b.code}`];
  if (b.kind !== "key") return ["Not set"];
  return [b.ctrl && "Ctrl", b.alt && "Alt", b.shift && "Shift", b.win && "Win", keyName(b.code)].filter(Boolean) as string[];
}

export function bindingShort(b: Binding): string {
  if (b.kind === "mouse") return ({ 1: "Mouse 4", 2: "Mouse 5", 3: "Middle click" } as Record<number, string>)[b.code] ?? "Mouse";
  return bindingParts(b).join(" + ");
}
