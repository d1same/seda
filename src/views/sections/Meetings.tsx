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
const hhmm = (ts: string | number) => new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
const IDLE: MeetingStatus = { running: false, secs: 0, segments: [], my_notes: "" };
const peopleLabel = (people?: string) => (people || "").split(" | ").map((s) => s.trim()).filter(Boolean).join(", ");
const meetingName = (m: { title: string; cal?: string }) => m.title.trim() || m.cal?.trim() || "Writing notes…";
const speaker = (who: string, me: string) => (who === "You" ? me || "You" : who);
const SUGGESTIONS = ["What did I agree to do?", "Write a follow-up email", "Make it shorter", "Translate to English"];

/** Words of a note line, for matching: no Markdown marks, punctuation or case. */
const tokens = (line: string) =>
  line.toLowerCase().replace(/^[\s#*\-]+(\[[ x]\]\s*)?/i, "").replace(/[^\p{L}\p{N}\s]/gu, " ").split(/\s+/).filter(Boolean);
/** The user's own note lines, tokenised (empty lines dropped). */
const ownLines = (mine: string) => mine.split("\n").map(tokens).filter((t) => t.length > 0);

/** True when a line of the notes is one the user typed (the AI keeps their lines, fixing typos
 *  only). Granola-style: your words in ink, what Seda added in gray. */
export function fromMine(line: string, mine: string[][]): boolean {
  const t = tokens(line);
  if (!t.length) return false;
  return mine.some((m) => {
    const shared = m.filter((w) => t.includes(w)).length;
    return shared >= Math.max(1, Math.ceil(m.length * 0.8)) && t.length <= m.length * 1.5 + 2;
  });
}

/** Just enough Markdown for the notes the AI writes: ## headings, bullets, [ ] tasks, **bold**.
 *  With `mine` (the user's own notes), their lines are marked so they read darker than the AI's. */
function Notes({ md, mine = "" }: { md: string; mine?: string }) {
  const inline = (t: string) => t.split(/(\*\*[^*]+\*\*)/).map((p, i) => (p.startsWith("**") ? <b key={i}>{p.slice(2, -2)}</b> : p));
  const own = ownLines(mine);
  const who = (l: string) => (own.length ? (fromMine(l, own) ? " mine" : " ai") : "");
  return (
    <div className={`notes ${own.length ? "has-mine" : ""}`}>
      {md.split("\n").map((raw, i) => {
        const l = raw.trim();
        if (!l) return null;
        // Sub-points are indented by 2+ spaces (or a tab) under their bullet.
        const depth = Math.min(2, Math.floor((raw.match(/^[ \t]*/)?.[0].replace(/\t/g, "  ").length ?? 0) / 2));
        const style = depth ? { marginInlineStart: depth * 20 } : undefined;
        if (l.startsWith("#")) return <h3 key={i} className={who(l).trim()} dir="auto">{l.replace(/^#+\s*/, "")}</h3>;
        const task = l.match(/^[-*]\s*\[( |x)\]\s*(.*)$/i);
        if (task) return <div key={i} className={`task${who(l)}`} style={style} dir="auto"><span className={`box ${task[1] !== " " ? "on" : ""}`} />{inline(task[2])}</div>;
        if (/^[-*]\s/.test(l)) return <div key={i} className={`bullet ${depth ? "sub" : ""}${who(l)}`} style={style} dir="auto">{inline(l.slice(2))}</div>;
        return <p key={i} className={who(l).trim()} style={style} dir="auto">{inline(l)}</p>;
      })}
    </div>
  );
}

/** "My notes" after the meeting: edit them, then have Seda rebuild the notes around them. */
function MyNotes({ id, initial, onSaved }: { id: number; initial: string; onSaved: (text: string) => void }) {
  const [text, setText] = useState(initial);
  const timer = useRef<number | undefined>(undefined);
  const pending = useRef<string | null>(null);
  // Leaving the tab must not drop the last few keystrokes: flush instead of cancelling.
  useEffect(() => () => {
    clearTimeout(timer.current);
    if (pending.current !== null) { api.meetingUpdateMyNotes(id, pending.current); onSaved(pending.current); }
  }, [id]);
  const change = (v: string) => {
    setText(v);
    pending.current = v;
    clearTimeout(timer.current);
    timer.current = window.setTimeout(() => { pending.current = null; api.meetingUpdateMyNotes(id, v); onSaved(v); }, 400);
  };
  const reenhance = async () => {
    clearTimeout(timer.current);
    pending.current = null;
    await api.meetingUpdateMyNotes(id, text);
    onSaved(text);
    await api.meetingRenotes(id);
  };
  return (
    <div className="mine-edit">
      <textarea className="mine-pad" dir="auto" value={text} onChange={(e) => change(e.target.value)} placeholder="Write notes… (# heading, - bullet)" aria-label="My notes" />
      <div className="mine-actions">
        <span className="faint">Seda keeps your lines word for word and fills in the rest from the transcript.</span>
        <IconButton icon="rewrite" label="Rebuild notes around these" onClick={reenhance} />
      </div>
    </div>
  );
}

/** Left/right bubbles per speaker. Direction is set on the text only, so a Persian line stays on
 *  its speaker's side and reads right-to-left inside its bubble. */
function Transcript({ segs, me, live }: { segs: Segment[]; me: string; live?: boolean }) {
  if (!segs.length) {
    return live
      ? <div className="faint pad">Listening… lines appear after each pause.</div>
      : <div className="empty"><div className="big">Nothing was transcribed.</div></div>;
  }
  return (
    <div className="transcript">
      {segs.map((s, i) => (
        <div key={i} className={`line ${s.who === "You" ? "you" : "them"}`}>
          <span className="line-head">
            <span className="who" dir="auto">{speaker(s.who, me)}</span>
            <span className="t">{clock(s.start)}</span>
          </span>
          <span className="say" dir="auto">{s.text}</span>
        </div>
      ))}
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
              <span className="sep" aria-hidden>·</span>
              <button className="link" onClick={() => setOpen(false)}>Hide</button>
            </span>
          </div>
          <div className="ask-log">
            {chat.map((t, i) => (
              <div key={i} className={`bubble ${t.role}`} dir="auto">
                {t.text}
                {t.edited && <span className="edited"><Icon name="check" size={12} />Notes updated</span>}
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
        <input dir="auto" value={q} onChange={(e) => setQ(e.target.value)} onFocus={() => chat.length && setOpen(true)} placeholder="Ask anything about this meeting…" aria-label="Ask about this meeting" disabled={busy} />
        {chat.length > 0 && !open && <button type="button" className="link" onClick={() => setOpen(true)}>{chat.length / 2 === 1 ? "1 answer" : `${chat.length / 2} answers`}</button>}
        <button className="send" disabled={busy || !q.trim()} aria-label="Send"><Icon name="send" size={14} /></button>
      </form>
    </div>
  );
}

type MenuItem = { icon: string; label: string; run: () => void; danger?: boolean } | "rule";

/** The ⋯ button and its small menu. Closes on an outside click, on Esc, and after a pick. */
function MoreMenu({ items }: { items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => { if (!wrap.current?.contains(e.target as Node)) setOpen(false); };
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") { setOpen(false); trigger.current?.focus(); } };
    document.addEventListener("mousedown", down);
    document.addEventListener("keydown", key);
    return () => { document.removeEventListener("mousedown", down); document.removeEventListener("keydown", key); };
  }, [open]);
  return (
    <div className="menu-wrap" ref={wrap}>
      <button ref={trigger} className={`ibtn round ${open ? "on" : ""}`} title="More" aria-label="More" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Icon name="more" size={15} />
      </button>
      {open && (
        <div className="menu" role="menu">
          {items.map((it, i) => it === "rule"
            ? <hr key={i} />
            : (
              <button key={i} role="menuitem" className={it.danger ? "danger" : ""} onClick={() => { setOpen(false); it.run(); }}>
                <Icon name={it.icon} size={15} />{it.label}
              </button>
            ))}
        </div>
      )}
    </div>
  );
}

/** One finished meeting, full page. */
function MeetingDoc({ id, back, me }: { id: number; back: () => void; me: string }) {
  const [m, setM] = useState<Meeting | null>(null);
  const [tab, setTab] = useState<"notes" | "mine" | "transcript">("notes");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [title, setTitle] = useState("");
  const [copied, setCopied] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);
  const revert = useRef(false);
  const load = useCallback(() => { api.meetingGet(id).then(setM); }, [id]);
  useEffect(() => { load(); return on("meetings-changed", load); }, [load]);
  // Follow the stored title, but never under the user's cursor.
  useEffect(() => { if (m && document.activeElement !== titleRef.current) setTitle(m.title); }, [m?.title]);
  if (!m) return null;

  const segs: Segment[] = (() => { try { return JSON.parse(m.transcript || "[]"); } catch { return []; } })();
  const chat: ChatTurn[] = (() => { try { return JSON.parse(m.chat || "[]"); } catch { return []; } })();
  const writing = !m.title && !m.notes;
  const hasMine = ownLines(m.my_notes).length > 0;
  const copy = async () => {
    await navigator.clipboard.writeText(
      tab === "transcript" ? segs.map((s) => `[${clock(s.start)}] ${s.who}: ${s.text}`).join("\n") : tab === "mine" ? m.my_notes : `# ${m.title}\n\n${m.notes}`,
    );
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };
  const save = async () => { await api.meetingSaveNotes(id, title.trim() || m.title, draft); setEditing(false); load(); };
  const edit = () => { setTab("notes"); setDraft(m.notes); setEditing(true); };
  const commitTitle = () => {
    const t = title.trim();
    if (revert.current || !t || t === m.title) { revert.current = false; setTitle(m.title); return; }
    setM({ ...m, title: t });
    api.meetingSaveNotes(id, t, m.notes);
  };
  const remove = async () => { if (confirm("Delete this meeting, its notes and recording?")) { await api.meetingDelete(id); back(); } };
  const pick = (t: typeof tab) => { setTab(t); if (t !== "notes") setEditing(false); };

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
              {!writing && <IconButton icon="record" label="Resume" onClick={async () => { try { await api.meetingResume(id); } catch (e) { alert(String(e)); } }} />}
              <MoreMenu items={writing
                ? [{ icon: "trash", label: "Delete meeting", danger: true, run: remove }]
                : [
                    { icon: "edit", label: "Edit notes", run: edit },
                    { icon: "rewrite", label: "Rewrite from transcript", run: () => api.meetingRenotes(id) },
                    "rule",
                    { icon: "trash", label: "Delete meeting", danger: true, run: remove },
                  ]} />
            </>
          )}
        </div>
      </div>
      <div className="kicker mdoc-meta">{dayOf(m.ts)} · {hhmm(m.ts)} · {mins(m.secs)}</div>
      <input
        ref={titleRef}
        className="title-edit"
        dir="auto"
        aria-label="Meeting title"
        value={writing ? meetingName(m) : title}
        readOnly={writing}
        placeholder={m.cal?.trim() || "Untitled meeting"}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={commitTitle}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") { revert.current = true; e.currentTarget.blur(); }
        }}
      />
      {peopleLabel(m.people) && <div className="mdoc-people" dir="auto">{peopleLabel(m.people)}</div>}

      <div className="mtabs" role="tablist">
        <button role="tab" aria-selected={tab === "notes"} className={tab === "notes" ? "on" : ""} onClick={() => pick("notes")}>Notes</button>
        <button role="tab" aria-selected={tab === "mine"} className={tab === "mine" ? "on" : ""} onClick={() => pick("mine")}>My notes</button>
        <button role="tab" aria-selected={tab === "transcript"} className={tab === "transcript" ? "on" : ""} onClick={() => pick("transcript")}>Transcript</button>
        {hasMine && tab === "notes" && !writing && !editing && (
          <span className="notes-legend"><span className="dot mine" />Your notes<span className="dot ai" />Seda's additions</span>
        )}
      </div>

      <div className="mdoc-body">
        {tab === "notes" && (editing
          ? <textarea className="notes-edit" dir="auto" aria-label="Notes" value={draft} onChange={(e) => setDraft(e.target.value)} />
          : writing ? <div className="writing"><span className="sweep-bar" />Reading the transcript — about a minute per half hour.</div>
          : <Notes md={m.notes} mine={m.my_notes} />)}
        {tab === "mine" && <MyNotes id={id} initial={m.my_notes} onSaved={(v) => setM((p) => p && { ...p, my_notes: v })} />}
        {tab === "transcript" && <Transcript segs={segs} me={me} />}
      </div>

      {!writing && !editing && <AskBar key={id} id={id} initial={chat} />}
    </div>
  );
}

/** While recording: a notepad as the page, a small recording chip, the live transcript docked below. */
function LiveDoc({ st, stop, busy, me }: { st: MeetingStatus; stop: () => void; busy: boolean; me: string }) {
  const [mine, setMine] = useState(st.my_notes);
  const [showAll, setShowAll] = useState(false);
  // When this session started (a resumed meeting's clock includes the earlier part).
  const [startedAt] = useState(() => (st.resuming ? Date.now() : Date.now() - st.secs * 1000));
  const timer = useRef<number | undefined>(undefined);
  const scroller = useRef<HTMLDivElement>(null);
  const change = (v: string) => {
    setMine(v);
    clearTimeout(timer.current);
    timer.current = window.setTimeout(() => api.meetingSetMyNotes(v), 400);
  };
  // Keep the newest line in view inside the panel (never scroll the page itself).
  useEffect(() => {
    const el = scroller.current;
    if (showAll && el) el.scrollTop = el.scrollHeight;
  }, [st.segments.length, showAll]);
  // Save the last keystrokes before stopping, so the notes are written with them.
  const finish = async () => { clearTimeout(timer.current); try { await api.meetingSetMyNotes(mine); } catch { /* stop anyway */ } stop(); };
  const last = st.segments[st.segments.length - 1];
  const n = st.segments.length;

  return (
    <div className="mdoc live">
      <div className="mdoc-top">
        <span className="rec-chip"><span className="rec-dot" aria-hidden /><span className="rec-time">{clock(st.secs)}</span><span className="rec-word">Recording</span></span>
        <button className="btn primary" disabled={busy} onClick={finish}>{busy ? "Finishing…" : "Stop and write notes"}</button>
      </div>
      <div className="kicker mdoc-meta">{st.resuming ? "Continuing" : "Today"} · {hhmm(startedAt)} · You and the call</div>
      <h1 className="mdoc-title" dir="auto">{st.title || st.resuming || "New meeting"}</h1>
      {peopleLabel(st.people) && <div className="mdoc-people" dir="auto">{peopleLabel(st.people)}</div>}
      <div className={`live-help ${mine.trim() ? "gone" : ""}`} aria-hidden={!!mine.trim()}>
        # for a heading, - for a bullet. Seda keeps your words and fills in the rest.
      </div>
      <textarea className="live-pad" dir="auto" autoFocus value={mine} onChange={(e) => change(e.target.value)} placeholder="Write notes…" aria-label="Your notes" />

      <div className="live-dock">
        <div className={`live-panel ${showAll ? "open" : ""}`} aria-hidden={!showAll}>
          <div className="live-panel-in">
            <div className="live-scroll scroll" ref={scroller}><Transcript segs={st.segments} me={me} live /></div>
          </div>
        </div>
        <div className="live-bar">
          <span className={`wave ${st.running ? "on" : ""}`} aria-hidden><i /><i /><i /></span>
          <div className="live-last" dir="auto">
            {last
              ? <><b>{speaker(last.who, me)}</b> {last.text}</>
              : <span className="faint">Listening… lines appear after each pause.</span>}
          </div>
          <button className="link" aria-expanded={showAll} onClick={() => setShowAll(!showAll)}>{showAll ? "Hide transcript" : `Transcript · ${n}`}</button>
        </div>
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
      on<{ title: string; people: string; who: string }>("meeting-named", (n) => setSt((p) => ({
        ...p,
        title: n.title || p.title,
        people: n.people || p.people,
        segments: n.who ? p.segments.map((s) => (s.who === "Them" ? { ...s, who: n.who } : s)) : p.segments,
      }))),
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
    // Opening the mic takes a second or two. Show the live meeting on this click.
    setSt({ ...IDLE, running: true });
    try { await api.meetingStart(); } catch (e) { setSt(IDLE); setErr(String(e)); }
  };
  const stop = async () => {
    setBusy(true);
    const id = await api.meetingStop();
    setBusy(false);
    setSt(IDLE);
    loadList();
    if (id) setOpenId(id);
  };

  if (st.running) return <div className="meetings-page"><LiveDoc st={st} stop={stop} busy={busy} me={me} /></div>;
  if (openId !== null) return <div className="meetings-page"><MeetingDoc key={openId} id={openId} me={me} back={() => { setOpenId(null); loadList(); }} /></div>;

  const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const shown = words.length
    ? list.filter((m) => {
        const hay = (m.search || [m.title, m.people, m.cal].filter(Boolean).join("\n")).toLowerCase();
        return words.every((w) => hay.includes(w));
      })
    : list;
  const days: [string, Meeting[]][] = [];
  for (const m of shown) {
    const d = dayOf(m.ts);
    if (days.length && days[days.length - 1][0] === d) days[days.length - 1][1].push(m);
    else days.push([d, [m]]);
  }

  return (
    <div className="meetings-page">
      <Head n="Meetings" title="Meetings" right={
        <div className="head-actions">
          {list.length > 0 && <SelectBar sel={sel} all={shown.map((m) => m.id)} what="meeting" onDelete={async (ids) => { for (const id of ids) await api.meetingDelete(id); loadList(); }} />}
          {!sel.on && list.length > 0 && <button className="btn accent" onClick={start}>Start meeting</button>}
        </div>
      }>
        Seda records you and the call, transcribes as it goes, and writes the notes when it ends.
      </Head>
      {err && <div className="notice err">{err}</div>}
      {list.length > 0 && <input className="field m-search" dir="auto" aria-label="Search meetings" placeholder="Search meetings, people, or what was said…" value={q} onChange={(e) => setQ(e.target.value)} />}
      {shown.length === 0 && list.length > 0 && (
        <div className="empty">
          <div className="big">No matches</div>
          <p>Try a name, the calendar subject, or a word that was said.</p>
        </div>
      )}
      {days.map(([day, ms]) => (
        <div key={day} className="mday">
          <div className="kicker day-label">{day}</div>
          {ms.map((m) => {
            // In the list the notes are stripped; an empty title means Seda is still writing them.
            const writing = !m.title.trim();
            const people = peopleLabel(m.people);
            return (
              <button key={m.id} className={`mrow ${sel.on ? "selecting" : ""} ${sel.on && sel.has(m.id) ? "picked" : ""} ${writing ? "writing" : ""}`} onClick={() => (sel.on ? sel.toggle(m.id) : setOpenId(m.id))}>
                {sel.on && <Tick on={sel.has(m.id)} />}
                {writing
                  ? <span className="ttl"><span className="lamp busy" aria-hidden />Writing notes…{m.cal?.trim() && <span className="who-bit" dir="auto"> · {m.cal.trim()}</span>}</span>
                  : <span className="ttl" dir="auto">{m.title.trim()}{people && <span className="who-bit"> · {people}</span>}</span>}
                <span className="mmeta">{hhmm(m.ts)} · {mins(m.secs)}</span>
              </button>
            );
          })}
        </div>
      ))}
      {list.length === 0 && (
        <div className="empty">
          <div className="big">No meetings yet</div>
          <p>When a Teams, Zoom or Meet call starts, Seda asks to take notes.</p>
          <button className="btn" onClick={start}>Start meeting</button>
        </div>
      )}
    </div>
  );
}
