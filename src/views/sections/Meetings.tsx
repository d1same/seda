import { useCallback, useEffect, useRef, useState } from "react";
import { api, type ChatTurn, type Meeting, type MeetingStatus, type Segment } from "../../lib/api";
import { on } from "../../lib/events";
import Icon, { IconButton } from "../Icons";
import { Head } from "./ui";
import { dayOf } from "./HistoryRow";
import { SelectBar, Tick, useSelection } from "./Select";
import "./meetings.css";

const clock = (s: number) => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = Math.floor(s % 60);
  return `${h ? `${h}:` : ""}${String(m).padStart(h ? 2 : 1, "0")}:${String(sec).padStart(2, "0")}`;
};
const mins = (s: number) => (s < 60 ? "<1 min" : `${Math.round(s / 60)} min`);
const hhmm = (ts: string) => new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
const IDLE: MeetingStatus = { running: false, secs: 0, segments: [], my_notes: "" };
const SUGGESTIONS = ["What did I agree to do?", "Write a follow-up email", "Make it shorter", "Translate to English"];

/** Just enough Markdown for the notes the AI writes: ## headings, bullets, [ ] tasks, **bold**. */
function Notes({ md }: { md: string }) {
  const inline = (t: string) => t.split(/(\*\*[^*]+\*\*)/).map((p, i) => (p.startsWith("**") ? <b key={i}>{p.slice(2, -2)}</b> : p));
  return (
    <div className="notes">
      {md.split("\n").map((raw, i) => {
        const l = raw.trim();
        if (!l) return null;
        // Sub-points are indented by 2+ spaces (or a tab) under their bullet.
        const depth = Math.min(2, Math.floor((raw.match(/^[ \t]*/)?.[0].replace(/\t/g, "  ").length ?? 0) / 2));
        const style = depth ? { marginInlineStart: depth * 20 } : undefined;
        if (l.startsWith("#")) return <h3 key={i} dir="auto">{l.replace(/^#+\s*/, "")}</h3>;
        const task = l.match(/^[-*]\s*\[( |x)\]\s*(.*)$/i);
        if (task) return <div key={i} className="task" style={style} dir="auto"><span className={`box ${task[1] !== " " ? "on" : ""}`} />{inline(task[2])}</div>;
        if (/^[-*]\s/.test(l)) return <div key={i} className={`bullet ${depth ? "sub" : ""}`} style={style} dir="auto">{inline(l.slice(2))}</div>;
        return <p key={i} style={style} dir="auto">{inline(l)}</p>;
      })}
    </div>
  );
}

function Transcript({ segs, follow, me }: { segs: Segment[]; follow?: boolean; me: string }) {
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => { if (follow) end.current?.scrollIntoView({ block: "end" }); }, [segs.length, follow]);
  if (!segs.length) return <div className="faint pad">{follow ? "Listening… each line appears after a pause." : "Nothing was transcribed."}</div>;
  return (
    <div className="transcript">
      {segs.map((s, i) => (
        <div key={i} className={`line ${s.who === "You" ? "you" : "them"}`}>
          <span className="t">{clock(s.start)}</span>
          <span className="who">{s.who === "You" ? me || "You" : s.who}</span>
          <span className="say" dir="auto">{s.text}</span>
        </div>
      ))}
      <div ref={end} />
    </div>
  );
}

/** The bar docked at the bottom of a meeting: ask about it, or tell Seda how to change the notes.
 *  The conversation opens above the bar. */
function AskBar({ id, initial }: { id: number; initial: ChatTurn[] }) {
  const [chat, setChat] = useState<ChatTurn[]>(initial);
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => { end.current?.scrollIntoView({ block: "nearest" }); }, [chat.length, busy, open]);

  const send = async (text: string) => {
    const question = text.trim();
    if (!question || busy) return;
    setQ(""); setErr(""); setBusy(true); setOpen(true);
    setChat((c) => [...c, { role: "you", text: question }]);
    try { setChat(await api.meetingAsk(id, question)); }
    catch (e) { setErr(String(e)); setChat((c) => c.slice(0, -1)); setQ(question); }
    setBusy(false);
  };

  return (
    <div className="askbar-dock">
      {open && (chat.length > 0 || busy) && (
        <div className="ask-thread">
          <div className="ask-thread-head">
            <span>Ask Seda</span>
            <span>
              <button className="link" onClick={() => { api.meetingClearChat(id); setChat([]); setOpen(false); }}>Clear</button>
              <button className="link" onClick={() => setOpen(false)}>Hide</button>
            </span>
          </div>
          <div className="ask-log">
            {chat.map((t, i) => (
              <div key={i} className={`bubble ${t.role}`} dir="auto">
                {t.text}
                {t.edited && <span className="edited">Notes updated</span>}
              </div>
            ))}
            {busy && <div className="bubble ai thinking"><span /><span /><span /></div>}
            <div ref={end} />
          </div>
        </div>
      )}
      {!busy && chat.length === 0 && (
        <div className="sugg-row">{SUGGESTIONS.map((s) => <button key={s} className="sugg" onClick={() => send(s)}>{s}</button>)}</div>
      )}
      {err && <div className="error-text">{err}</div>}
      <form className="askbar" onSubmit={(e) => { e.preventDefault(); send(q); }}>
        <Icon name="spark" size={15} />
        <input dir="auto" value={q} onChange={(e) => setQ(e.target.value)} onFocus={() => chat.length && setOpen(true)} placeholder="Ask about this meeting, or tell Seda how to change the notes…" disabled={busy} />
        {chat.length > 0 && !open && <button type="button" className="link" onClick={() => setOpen(true)}>{chat.length / 2 === 1 ? "1 answer" : `${chat.length / 2} answers`}</button>}
        <button className="send" disabled={busy || !q.trim()} aria-label="Send"><Icon name="send" size={14} /></button>
      </form>
    </div>
  );
}

/** One finished meeting, full page. */
function MeetingDoc({ id, back, me }: { id: number; back: () => void; me: string }) {
  const [m, setM] = useState<Meeting | null>(null);
  const [tab, setTab] = useState<"notes" | "transcript" | "mine">("notes");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState({ title: "", notes: "" });
  const [copied, setCopied] = useState(false);
  const load = useCallback(() => { api.meetingGet(id).then(setM); }, [id]);
  useEffect(() => { load(); return on("meetings-changed", load); }, [load]);
  if (!m) return null;

  const segs: Segment[] = (() => { try { return JSON.parse(m.transcript || "[]"); } catch { return []; } })();
  const chat: ChatTurn[] = (() => { try { return JSON.parse(m.chat || "[]"); } catch { return []; } })();
  const writing = !m.title && !m.notes;
  const copy = async () => {
    await navigator.clipboard.writeText(
      tab === "transcript" ? segs.map((s) => `[${clock(s.start)}] ${s.who}: ${s.text}`).join("\n") : tab === "mine" ? m.my_notes : `# ${m.title}\n\n${m.notes}`,
    );
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };
  const save = async () => { await api.meetingSaveNotes(id, draft.title, draft.notes); setEditing(false); load(); };

  return (
    <div className="mdoc">
      <div className="mdoc-top">
        <IconButton icon="back" label="Back to meetings" iconOnly onClick={back} />
        <div className="mdoc-tools">
          {editing ? (
            <>
              <IconButton icon="check" label="Save" primary onClick={save} />
              <IconButton icon="back" label="Cancel" onClick={() => setEditing(false)} />
            </>
          ) : (
            <>
              <IconButton icon={copied ? "check" : "copy"} label={copied ? "Copied" : "Copy"} onClick={copy} />
              {tab === "notes" && !writing && <IconButton icon="edit" label="Edit" onClick={() => { setDraft({ title: m.title, notes: m.notes }); setEditing(true); }} />}
              {tab === "notes" && !writing && <IconButton icon="rewrite" label="Rewrite notes from the transcript" iconOnly onClick={() => api.meetingRenotes(id)} />}
              {!writing && <IconButton icon="record" label="Resume" onClick={async () => { try { await api.meetingResume(id); } catch (e) { alert(String(e)); } }} />}
              <IconButton icon="trash" label="Delete meeting" iconOnly danger onClick={async () => { if (confirm("Delete this meeting, its notes and recording?")) { await api.meetingDelete(id); back(); } }} />
            </>
          )}
        </div>
      </div>
      <div className="mdoc-meta">{dayOf(m.ts)} · {hhmm(m.ts)} · {mins(m.secs)}</div>
      {editing
        ? <input className="title-edit" dir="auto" value={draft.title} onChange={(e) => setDraft({ ...draft, title: e.target.value })} />
        : <h1 className="mdoc-title" dir="auto">{writing ? "Writing notes…" : m.title}</h1>}

      <div className="mtabs">
        <button className={tab === "notes" ? "on" : ""} onClick={() => setTab("notes")}>Notes</button>
        <button className={tab === "transcript" ? "on" : ""} onClick={() => { setTab("transcript"); setEditing(false); }}>Transcript</button>
        {m.my_notes.trim() && <button className={tab === "mine" ? "on" : ""} onClick={() => { setTab("mine"); setEditing(false); }}>My notes</button>}
      </div>

      <div className="mdoc-body">
        {tab === "notes" && (editing
          ? <textarea className="notes-edit" dir="auto" value={draft.notes} onChange={(e) => setDraft({ ...draft, notes: e.target.value })} />
          : writing ? <div className="writing"><span className="sweep-bar" />The local AI is reading the transcript — about a minute per half hour of meeting.</div>
          : <Notes md={m.notes} />)}
        {tab === "transcript" && <Transcript segs={segs} me={me} />}
        {tab === "mine" && <div className="mine" dir="auto">{m.my_notes}</div>}
      </div>

      {!writing && !editing && <AskBar key={id} id={id} initial={chat} />}
    </div>
  );
}

/** While recording: a notepad as the page, a small recording chip, the live transcript docked below. */
function LiveDoc({ st, stop, busy, me }: { st: MeetingStatus; stop: () => void; busy: boolean; me: string }) {
  const [mine, setMine] = useState(st.my_notes);
  const [showAll, setShowAll] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  const change = (v: string) => {
    setMine(v);
    clearTimeout(timer.current);
    timer.current = window.setTimeout(() => api.meetingSetMyNotes(v), 400);
  };
  const last = st.segments.slice(-2);

  return (
    <div className="mdoc live">
      <div className="mdoc-top">
        <span className="rec-chip"><span className="rec-dot" />{clock(st.secs)}</span>
        <IconButton icon="check" label={busy ? "Finishing…" : "Stop & write notes"} primary disabled={busy} onClick={stop} />
      </div>
      <div className="mdoc-meta">{st.resuming ? "Continuing · " : `Today · ${new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })} · `}recording you and the call</div>
      <h1 className="mdoc-title" dir="auto">{st.resuming || "New meeting"}</h1>
      <textarea className="notepad" dir="auto" autoFocus value={mine} onChange={(e) => change(e.target.value)} placeholder="Write notes… (optional — Seda builds the final notes on them)" />

      <div className="live-dock">
        {showAll ? (
          <div className="live-full scroll"><Transcript segs={st.segments} follow me={me} /></div>
        ) : (
          <div className="live-peek">
            {last.length === 0 && <span className="faint">Listening… lines appear after each pause.</span>}
            {last.map((s, i) => <div key={i} className={s.who === "You" ? "you" : "them"} dir="auto"><b>{s.who === "You" ? me || "You" : s.who}</b> {s.text}</div>)}
          </div>
        )}
        <button className="link" onClick={() => setShowAll(!showAll)}>{showAll ? "Hide transcript" : `Transcript · ${st.segments.length}`}</button>
      </div>
    </div>
  );
}

export default function Meetings({ me }: { me: string }) {
  const [list, setList] = useState<Meeting[]>([]);
  const [openId, setOpenId] = useState<number | null>(null);
  const [st, setSt] = useState<MeetingStatus>(IDLE);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [q, setQ] = useState("");
  const sel = useSelection();

  const loadList = useCallback(() => { api.meetingsList().then(setList); }, []);
  useEffect(() => {
    loadList();
    api.meetingStatus().then(setSt);
    const un = [
      on("meetings-changed", loadList),
      on<Segment>("meeting-segment", (s) => setSt((p) => ({ ...p, segments: [...p.segments, s] }))),
      on<boolean>("meeting-state", () => api.meetingStatus().then(setSt)),
    ];
    return () => un.forEach((f) => f());
  }, [loadList]);
  useEffect(() => {
    if (!st.running) return;
    const t = setInterval(() => setSt((p) => ({ ...p, secs: p.secs + 1 })), 1000);
    return () => clearInterval(t);
  }, [st.running]);

  const start = async () => {
    setErr("");
    try { await api.meetingStart(); setSt({ ...IDLE, running: true }); } catch (e) { setErr(String(e)); }
  };
  const stop = async () => {
    setBusy(true);
    const id = await api.meetingStop();
    setBusy(false);
    setSt(IDLE);
    loadList();
    if (id) setOpenId(id);
  };

  if (st.running) return <LiveDoc st={st} stop={stop} busy={busy} me={me} />;
  if (openId !== null) return <MeetingDoc key={openId} id={openId} me={me} back={() => { setOpenId(null); loadList(); }} />;

  const needle = q.trim().toLowerCase();
  const shown = needle ? list.filter((m) => m.title.toLowerCase().includes(needle)) : list;
  const days: [string, Meeting[]][] = [];
  for (const m of shown) {
    const d = dayOf(m.ts);
    if (days.length && days[days.length - 1][0] === d) days[days.length - 1][1].push(m);
    else days.push([d, [m]]);
  }

  return (
    <>
      <Head n="Meetings" title="Meetings" right={
        <div className="head-actions">
          {list.length > 0 && <SelectBar sel={sel} all={shown.map((m) => m.id)} what="meeting" onDelete={async (ids) => { for (const id of ids) await api.meetingDelete(id); loadList(); }} />}
          {!sel.on && <button className="btn primary" onClick={start}>Start meeting</button>}
        </div>
      }>
        Seda records you and the call, transcribes as it goes, and writes the notes when it ends.
      </Head>
      {err && <div className="notice err">{err}</div>}
      {list.length > 3 && <input className="field m-search" dir="auto" placeholder="Search meetings…" value={q} onChange={(e) => setQ(e.target.value)} />}
      {days.map(([day, ms]) => (
        <div key={day}>
          <div className="label day-label">{day}</div>
          {ms.map((m) => (
            <button key={m.id} className={`mrow ${sel.on && sel.has(m.id) ? "picked" : ""}`} onClick={() => (sel.on ? sel.toggle(m.id) : setOpenId(m.id))}>
              {sel.on && <Tick on={sel.has(m.id)} />}
              <span className="ttl" dir="auto">{m.title || "Writing notes…"}</span>
              <span className="faint">{hhmm(m.ts)} · {mins(m.secs)}</span>
            </button>
          ))}
        </div>
      ))}
      {list.length === 0 && (
        <div className="empty">
          <div className="big">No meetings yet.</div>
          When a Teams, Zoom or Meet call starts, Seda offers to take notes. Or press <b>Start meeting</b>.
        </div>
      )}
    </>
  );
}
