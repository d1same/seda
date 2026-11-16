import { useCallback, useEffect, useMemo, useState } from "react";
import { api, bindingShort, type HistoryItem, type Meeting, type Settings, type Status } from "../../lib/api";
import { useOnHistory } from "../../lib/hooks";
import { on } from "../../lib/events";
import type { SectionId } from "../Main";
import { timeAgo } from "./ui";
import HistoryRow from "./HistoryRow";
import CopyDiagnostics from "../CopyDiagnostics";
import Icon from "../Icons";

const greeting = () => {
  const h = new Date().getHours();
  return h < 5 ? "Good evening" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
};

export default function Overview({ settings, status, go, open }: { settings: Settings; status: Status; go: (s: SectionId) => void; open: (item: HistoryItem) => void }) {
  const startMeeting = async () => { await api.meetingStart().catch(() => {}); go("meetings"); };
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const load = useCallback(() => { api.history(500).then(setItems); }, []);
  useOnHistory(load);
  useEffect(() => {
    const m = () => { api.meetingsList().then(setMeetings); };
    m();
    return on("meetings-changed", m);
  }, []);

  const stats = useMemo(() => {
    const today = new Date().toDateString();
    const t = items.filter((i) => new Date(i.ts).toDateString() === today);
    const words = t.reduce((n, i) => n + (i.corrected ?? i.cleaned).split(/\s+/).filter(Boolean).length, 0);
    const recent = items.slice(0, 20);
    const avg = recent.length ? recent.reduce((n, i) => n + i.asr_ms, 0) / recent.length / 1000 : 0;
    return { today: t.length, words, avg };
  }, [items]);

  const verb = settings.dictate_mode === "toggle" ? "tap" : "hold";
  const fixKeys = bindingShort(settings.fix_binding).replace(/ \+ /g, "+");
  const problem = status.asr_error ?? status.llm_error;
  const summary = stats.today
    ? `${stats.today} dictation${stats.today === 1 ? "" : "s"} today · ${stats.words.toLocaleString()} words${stats.avg ? ` · ${stats.avg.toFixed(1)} s to paste` : ""}`
    : "Nothing dictated yet today.";

  return (
    <>
      <div className="home-head">
        <h1>{greeting()}{settings.user_name ? `, ${settings.user_name}` : ""}</h1>
        <p>{summary}</p>
      </div>

      {problem && (
        <div className="notice err">
          <span>{problem}</span>
          <span style={{ display: "flex", gap: 8 }}><button className="btn small" onClick={() => go("settings")}>Settings</button><CopyDiagnostics small /></span>
        </div>
      )}

      <div className="quick">
        <button className="btn accent" onClick={startMeeting}><Icon name="record" size={14} /> Start meeting</button>
        <span className="quick-hint">Talk anywhere: {verb} <span className="keycap small">{bindingShort(settings.dictate_binding)}</span></span>
        <span className="quick-hint">Fix text: select it, press <span className="keycap small">{fixKeys}</span></span>
        <button className="link" onClick={() => go("words")}>Teach a word</button>
      </div>

      <textarea
        className="tryit-box"
        dir="auto"
        aria-label="Try dictating here"
        placeholder={`Click here, ${verb} ${bindingShort(settings.dictate_binding)} and say: “send the report to Alex — no wait, to Sam — by Thursday”`}
      />

      <div className="home-section">
        <div className="home-section-head">
          <h2>Recent</h2>
          {items.length > 0 && <button className="link" onClick={() => go("history")}>See all</button>}
        </div>
        {items.slice(0, 6).map((i) => <HistoryRow key={i.id} item={i} open={open} />)}
        {items.length === 0 && <div className="faint home-empty">Your dictations will show up here.</div>}
      </div>

      <div className="home-section">
        <div className="home-section-head">
          <h2>Meetings</h2>
          <button className="link" onClick={() => go("meetings")}>{meetings.length ? "See all" : "Start one"}</button>
        </div>
        {meetings.slice(0, 3).map((m) => {
          const writing = !m.title && !m.notes;
          return (
            <button key={m.id} className="mrow" onClick={() => go("meetings")}>
              {writing
                ? <span className="ttl"><span className="writing-ttl"><span className="lamp busy" />Writing notes…</span></span>
                : <span className="ttl" dir="auto">{m.title || m.cal || "Untitled meeting"}</span>}
              <span className="faint">{timeAgo(m.ts)} · {Math.max(1, Math.round(m.secs / 60))} min</span>
            </button>
          );
        })}
        {meetings.length === 0 && <div className="faint home-empty">Seda offers to take notes when a Teams, Zoom or Meet call starts.</div>}
      </div>
    </>
  );
}
