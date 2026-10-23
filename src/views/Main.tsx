import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri, on } from "../lib/events";
import { bindingParts, type Binding, type HistoryItem } from "../lib/api";
import { useLocalShortcut, useSettings, useStatus } from "../lib/hooks";
import Overview from "./sections/Overview";
import History from "./sections/History";
import Meetings from "./sections/Meetings";
import Vocabulary from "./sections/Vocabulary";
import Settings from "./sections/Settings";
import HistoryDetail from "./sections/HistoryDetail";
import Onboarding from "./Onboarding";
import Logo from "./Logo";
import Icon from "./Icons";
import UpdateBanner from "./UpdateBanner";
import "./main.css";

const SECTIONS = [
  { id: "home", label: "Home" },
  { id: "meetings", label: "Meetings" },
  { id: "history", label: "History" },
  { id: "words", label: "Words" },
  { id: "settings", label: "Settings" },
] as const;
/** "Ctrl+Shift+Space", "Mouse 4": short enough for the sidebar. */
const compact = (b: Binding) => bindingParts(b).map((p) => p.replace(/ \(.*\)$/, "")).join("+");

export type SectionId = (typeof SECTIONS)[number]["id"];

export default function Main() {
  const [section, setSectionRaw] = useState<SectionId>("home");
  // A dictation opened full-page; Back returns to the section it was opened from.
  const [opened, setOpened] = useState<HistoryItem | null>(null);
  const setSection = (s: SectionId) => { setOpened(null); setSectionRaw(s); };
  const { settings, update } = useSettings();
  const status = useStatus();
  useLocalShortcut(settings?.dictate_binding);
  // The call card's "Open notes" jumps straight to Meetings.
  useEffect(() => on<SectionId>("navigate", setSection), []);
  const win = inTauri ? getCurrentWindow() : null;
  const lamp = status.asr === "error" || status.llm === "error" ? "err" : status.asr === "ready" && status.llm !== "loading" ? "ok" : "busy";
  const lampText = lamp === "ok" ? "Ready" : lamp === "err" ? "Needs attention" : "Starting up…";

  const windowButtons = (
    <div className="win-buttons">
      <button aria-label="Minimize" onClick={() => win?.minimize()}><svg width="10" height="10"><path d="M0 5h10" stroke="currentColor" /></svg></button>
      <button aria-label="Close to tray" className="close" onClick={() => win?.hide()}><svg width="10" height="10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" /></svg></button>
    </div>
  );

  if (settings && !settings.onboarded) {
    return (
      <div className="shell setup">
        <header className="titlebar" data-tauri-drag-region>
          <div className="brand" data-tauri-drag-region><Logo size={20} /><span className="mark">Seda</span></div>
          {windowButtons}
        </header>
        <Onboarding settings={settings} update={update} status={status} onDone={() => update("onboarded", true)} />
      </div>
    );
  }

  return (
    <div className="shell">
      <aside className="rail">
        <div className="brand" data-tauri-drag-region>
          <Logo size={24} />
          <span className="mark">Seda</span>
        </div>
        <nav>
          {SECTIONS.map((s) => (
            <button key={s.id} className={section === s.id && !opened ? "on" : ""} onClick={() => setSection(s.id)}>
              <Icon name={s.id} />
              {s.label}
            </button>
          ))}
        </nav>
        <UpdateBanner />
        <div className="rail-foot">
          <button className="status-line" onClick={() => setSection(lamp === "err" ? "settings" : "home")} title={status.asr_error ?? status.llm_error ?? lampText}>
            <span className={`lamp ${lamp}`} />
            {lampText}
          </button>
          <div className="shortcut-line"><span>Talk</span><span className="keycap">{settings ? compact(settings.dictate_binding) : "…"}</span></div>
          <div className="shortcut-line"><span>Fix text</span><span className="keycap">{settings ? compact(settings.fix_binding) : "…"}</span></div>
        </div>
      </aside>

      <section className="main-col">
        <header className="titlebar" data-tauri-drag-region>{windowButtons}</header>
        <main className="content scroll">
          <div className="page">
            {settings && opened && <HistoryDetail key={opened.id} item={opened} back={() => setOpened(null)} />}
            {settings && !opened && (
              <>
                {section === "home" && <Overview settings={settings} status={status} go={setSection} open={setOpened} />}
                {section === "meetings" && <Meetings me={settings.user_name} />}
                {section === "history" && <History open={setOpened} />}
                {section === "words" && <Vocabulary />}
                {section === "settings" && <Settings settings={settings} update={update} status={status} go={setSection} />}
              </>
            )}
          </div>
        </main>
      </section>
    </div>
  );
}
