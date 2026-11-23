import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type MutableRefObject } from "react";
import { api, type ChatTurn, type CopyWhat, type Meeting, type MeetingStatus, type SearchHit, type Segment } from "../../lib/api";
import { on } from "../../lib/events";
import { usePopover } from "../../lib/hooks";
import Icon, { IconButton } from "../Icons";
import type { SectionId } from "../Main";
import { Head } from "./ui";
import { dayOf } from "./HistoryRow";
import { SelectBar, Tick, useSelection } from "./Select";
import AskBar from "./AskBar";
import { TemplateList, templateName, useTemplates } from "./TemplateMenu";
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
const SUGGESTIONS = ["What did I agree to do?", "Write a follow-up email", "Add more detail", "Make it shorter"];
const SUGGESTIONS_ALL = ["What did I agree to do this week?", "What was decided this week?", "What is still open?"];
type Go = (s: SectionId) => void;

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
 *  With `mine` (the user's own notes), their lines are marked so they read darker than the AI's.
 *  When `onToggleTask` is given, clicking a task checkbox toggles `[ ]` ↔ `[x]` in place. */
function Notes({ md, mine = "", onToggleTask }: { md: string; mine?: string; onToggleTask?: (lineIdx: number) => void }) {
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
        if (task) {
          const checked = task[1] !== " ";
          return (
            <div key={i} className={`task${checked ? " done" : ""}${who(l)}`} style={style} dir="auto">
              <button
                type="button"
                className={`box ${checked ? "on" : ""}`}
                role="checkbox"
                aria-checked={checked}
                aria-label={checked ? "Mark incomplete" : "Mark complete"}
                onClick={() => onToggleTask?.(i)}
              />
              <span className="task-text">{inline(task[2])}</span>
            </div>
          );
        }
        if (/^[-*]\s/.test(l)) return <div key={i} className={`bullet ${depth ? "sub" : ""}${who(l)}`} style={style} dir="auto">{inline(l.slice(2))}</div>;
        return <p key={i} className={who(l).trim()} style={style} dir="auto">{inline(l)}</p>;
      })}
    </div>
  );
}

/** "My notes" after the meeting: edit them, then have Seda rebuild the notes around them. */
function MyNotes({ id, initial, onSaved, flushRef }: { id: number; initial: string; onSaved: (text: string) => void; flushRef: MutableRefObject<(() => Promise<void>) | null> }) {
  const [text, setText] = useState(initial);
  const timer = useRef<number | undefined>(undefined);
  const pending = useRef<string | null>(null);
  // Copy reads the stored notepad: save what is typed but not yet saved first.
  flushRef.current = async () => {
    if (pending.current === null) return;
    clearTimeout(timer.current);
    const v = pending.current;
    pending.current = null;
    await api.meetingUpdateMyNotes(id, v);
    onSaved(v);
  };
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
    // The part notes don't depend on the notepad, so only the last step runs again. It refuses
    // while the writing AI is down (the notes stay as they are), so no fallback that wipes them.
    try { await api.meetingRebuild(id); } catch (e) { alert(String(e)); }
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

type MenuItem = { icon: string; label: string; run: () => void; danger?: boolean } | "rule";

/** The ⋯ button and its small menu. Closes on an outside click, on Esc, and after a pick. */
function MoreMenu({ items }: { items: MenuItem[] }) {
  const { open, setOpen, wrap, trigger } = usePopover();
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
function MeetingDoc({ id, back, me, go }: { id: number; back: () => void; me: string; go: Go }) {
  const [m, setM] = useState<Meeting | null>(null);
  const [tab, setTab] = useState<"notes" | "mine" | "transcript">("notes");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [title, setTitle] = useState("");
  const [copied, setCopied] = useState(false);
  // The template being applied (its name), shown in the writing state until the new notes arrive.
  const [rewriting, setRewriting] = useState("");
  const templates = useTemplates();
  const caret = usePopover();
  const titleRef = useRef<HTMLInputElement>(null);
  const revert = useRef(false);
  const flushMine = useRef<(() => Promise<void>) | null>(null);
  const load = useCallback(() => { api.meetingGet(id).then(setM); }, [id]);
  useEffect(() => { load(); return on("meetings-changed", load); }, [load]);
  // Follow the stored title, but never under the user's cursor.
  useEffect(() => { if (m && document.activeElement !== titleRef.current) setTitle(m.title); }, [m?.title]);
  useEffect(() => { if (m?.notes) setRewriting(""); }, [m?.notes]);
  if (!m) return null;

  const segs: Segment[] = (() => { try { return JSON.parse(m.transcript || "[]"); } catch { return []; } })();
  const chat: ChatTurn[] = (() => { try { return JSON.parse(m.chat || "[]"); } catch { return []; } })();
  const writing = !m.title && !m.notes;
  const hasMine = ownLines(m.my_notes).length > 0;
  const tplName = templateName(templates, m.template);
  const flipCopied = () => { setCopied(true); setTimeout(() => setCopied(false), 1200); };
  const copy = async (what: CopyWhat) => {
    try {
      if (what === "mine") await flushMine.current?.();
      await api.meetingCopy(id, what);
      flipCopied();
    } catch (e) { alert(String(e)); }
  };
  const exportMd = async () => { try { await api.meetingExport(id); } catch (e) { alert(String(e)); } };
  const save = async () => { await api.meetingSaveNotes(id, title.trim() || m.title, draft); setEditing(false); load(); };
  const toggleTask = (lineIdx: number) => {
    const lines = m.notes.split("\n");
    const line = lines[lineIdx];
    if (line === undefined) return;
    const next = line.replace(/^(\s*[-*]\s*\[)( |x)(\]\s*)/i, (_, a, box, b) => `${a}${box === " " ? "x" : " "}${b}`);
    if (next === line) return;
    lines[lineIdx] = next;
    const updated = lines.join("\n");
    setM({ ...m, notes: updated });
    api.meetingSaveNotes(id, m.title, updated);
  };
  const edit = () => { setTab("notes"); setDraft(m.notes); setEditing(true); };
  const commitTitle = () => {
    const t = title.trim();
    if (revert.current || !t || t === m.title) { revert.current = false; setTitle(m.title); return; }
    setM({ ...m, title: t });
    api.meetingSaveNotes(id, t, m.notes);
  };
  const remove = async () => { if (confirm("Delete this meeting, its notes and recording?")) { await api.meetingDelete(id); back(); } };
  const pick = (t: typeof tab) => { setTab(t); if (t !== "notes") setEditing(false); };
  const applyTemplate = async (tid: string) => {
    caret.setOpen(false);
    if ((m.template || "auto") === tid) return;
    setRewriting(templateName(templates, tid));
    try {
      await api.meetingSetTemplate(id, tid);
      // Nothing to rewrite (no transcript or notes yet): the notes are already back.
      const fresh = await api.meetingGet(id);
      if (fresh && (fresh.title || fresh.notes)) setRewriting("");
    } catch (e) { setRewriting(""); alert(String(e)); }
    load();
  };

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
              <IconButton icon={copied ? "check" : "copy"} label={copied ? "Copied" : "Copy"} onClick={() => copy(tab === "transcript" ? "transcript" : tab === "mine" ? "mine" : "notes")} />
              {!writing && <IconButton icon="record" label="Resume" onClick={async () => { try { await api.meetingResume(id); } catch (e) { alert(String(e)); } }} />}
              <MoreMenu items={writing
                ? [
                    { icon: "save", label: "Save as Markdown", run: exportMd },
                    "rule",
                    { icon: "trash", label: "Delete meeting", danger: true, run: remove },
                  ]
                : [
                    { icon: "edit", label: "Edit notes", run: edit },
                    { icon: "rewrite", label: "Rewrite from transcript", run: () => api.meetingRenotes(id) },
                    "rule",
                    { icon: "save", label: "Save as Markdown", run: exportMd },
                    { icon: "copy", label: "Copy with transcript", run: () => copy("all") },
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
        {tab === "notes" && !writing && !editing && (
          <div className="menu-wrap tab-caret-wrap" ref={caret.wrap}>
            <button ref={caret.trigger} type="button" className="tab-caret" aria-haspopup="menu" aria-expanded={caret.open}
              aria-label={`Template: ${tplName}`} title="Template" onClick={() => caret.setOpen(!caret.open)}>
              {tplName !== "Auto" && <span dir="auto">{tplName}</span>}
              <Icon name="chevron" size={12} />
            </button>
            {caret.open && <TemplateList className="start" templates={templates} value={m.template} onPick={applyTemplate} onEdit={() => go("settings")} />}
          </div>
        )}
        <button role="tab" aria-selected={tab === "mine"} className={tab === "mine" ? "on" : ""} onClick={() => pick("mine")}>My notes</button>
        <button role="tab" aria-selected={tab === "transcript"} className={tab === "transcript" ? "on" : ""} onClick={() => pick("transcript")}>Transcript</button>
        {hasMine && tab === "notes" && !writing && !editing && (
          <span className="notes-legend"><span className="dot mine" />Your notes<span className="dot ai" />Seda's additions</span>
        )}
      </div>

      <div className="mdoc-body">
        {tab === "notes" && (editing
          ? <textarea className="notes-edit" dir="auto" aria-label="Notes" value={draft} onChange={(e) => setDraft(e.target.value)} />
          : writing ? <div className="writing"><span className="sweep-bar" />{rewriting ? <>Rewriting the notes as <span dir="auto">{rewriting}</span>…</> : "Reading the transcript — about a minute per half hour."}</div>
          : <Notes md={m.notes} mine={m.my_notes} onToggleTask={toggleTask} />)}
        {tab === "mine" && <MyNotes id={id} initial={m.my_notes} flushRef={flushMine} onSaved={(v) => setM((p) => p && { ...p, my_notes: v })} />}
        {tab === "transcript" && <Transcript segs={segs} me={me} />}
      </div>

      {!writing && !editing && (
        <AskBar key={id} initial={chat} ask={(q) => api.meetingAsk(id, q)} clear={() => api.meetingClearChat(id)}
          placeholder="Ask anything about this meeting…" label="Ask about this meeting" suggestions={SUGGESTIONS} />
      )}
    </div>
  );
}

/** While recording: a notepad as the page, a small recording chip, the live transcript docked below. */
function LiveDoc({ st, stop, busy, me, setTemplate, go }: { st: MeetingStatus; stop: () => void; busy: boolean; me: string; setTemplate: (t: string) => void; go: Go }) {
  const [mine, setMine] = useState(st.my_notes);
  const [showAll, setShowAll] = useState(false);
  // When this session started (a resumed meeting's clock includes the earlier part).
  const [startedAt] = useState(() => (st.resuming ? Date.now() : Date.now() - st.secs * 1000));
  const timer = useRef<number | undefined>(undefined);
  const scroller = useRef<HTMLDivElement>(null);
  const pad = useRef<HTMLTextAreaElement>(null);
  const templates = useTemplates();
  const chip = usePopover();
  // "/" at the start of a line opens the template list from the dock; `at` is where it was typed.
  const at = useRef(0);
  // After the render that closes the menu (and, on Esc, puts the "/" back).
  const focusPad = (pos: number) => setTimeout(() => { pad.current?.focus(); pad.current?.setSelectionRange(pos, pos); }, 0);
  const slash = usePopover(() => {
    // Esc: the user meant a plain "/".
    const pos = at.current;
    change(mine.slice(0, pos) + "/" + mine.slice(pos));
    focusPad(pos + 1);
  });
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
  const liveId = st.id || 0;
  const pickTemplate = (tid: string) => {
    if (!liveId) return;
    const before = st.template ?? "";
    setTemplate(tid);
    api.meetingSetTemplate(liveId, tid).catch((e) => { setTemplate(before); alert(String(e)); });
  };
  const padKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "/" || e.ctrlKey || e.altKey || e.metaKey || e.nativeEvent.isComposing || !liveId) return;
    const el = e.currentTarget;
    const pos = el.selectionStart;
    if (pos !== el.selectionEnd || (pos > 0 && el.value[pos - 1] !== "\n")) return;
    e.preventDefault();
    at.current = pos;
    chip.setOpen(false);
    slash.setOpen(true);
  };

  return (
    <div className="mdoc live">
      <div className="mdoc-top">
        <span className="rec-chip"><span className="rec-dot" aria-hidden /><span className="rec-time">{clock(st.secs)}</span><span className="rec-word">Recording</span></span>
        <div className="mdoc-tools">
          <div className="menu-wrap" ref={chip.wrap}>
            <button ref={chip.trigger} type="button" className="btn ghost tpl-chip" disabled={!liveId} aria-haspopup="menu" aria-expanded={chip.open} onClick={() => chip.setOpen(!chip.open)}>
              <span className="tpl-chip-name">Template: <span dir="auto">{templateName(templates, st.template)}</span></span>
              <Icon name="chevron" size={14} />
            </button>
            {chip.open && <TemplateList templates={templates} value={st.template} onPick={(tid) => { chip.setOpen(false); pickTemplate(tid); }} onEdit={() => go("settings")} />}
          </div>
          <button className="btn primary" disabled={busy} onClick={finish}>{busy ? "Finishing…" : "Stop and write notes"}</button>
        </div>
      </div>
      <div className="kicker mdoc-meta">{st.resuming ? "Continuing" : "Today"} · {hhmm(startedAt)} · You and the call</div>
      <h1 className="mdoc-title" dir="auto">{st.title || st.resuming || "New meeting"}</h1>
      {peopleLabel(st.people) && <div className="mdoc-people" dir="auto">{peopleLabel(st.people)}</div>}
      <div className={`live-help ${mine.trim() ? "gone" : ""}`} aria-hidden={!!mine.trim()}>
        # for a heading, - for a bullet, / for a template. Seda keeps your words and fills in the rest.
      </div>
      <textarea ref={pad} className="live-pad" dir="auto" autoFocus value={mine} onChange={(e) => change(e.target.value)} onKeyDown={padKey} placeholder="Write notes…" aria-label="Your notes" />

      <div className="live-dock">
        <div className="menu-wrap slash-wrap" ref={slash.wrap}>
          {slash.open && (
            <TemplateList className="up start" templates={templates} value={st.template}
              onPick={(tid) => { slash.setOpen(false); pickTemplate(tid); focusPad(at.current); }}
              onEdit={() => go("settings")} />
          )}
        </div>
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

/** Every typed word is a substring of the title, the people or the calendar subject: instant, local. */
const localMatch = (m: Meeting, words: string[]) => {
  const hay = [m.title, m.people, m.cal].filter(Boolean).join("\n").toLowerCase();
  return words.every((w) => hay.includes(w));
};
/** A snippet is cut from the stored text: drop the bold marks so "**Send it**" reads as "Send it". */
const plain = (s: string) => s.replace(/\*\*/g, "");

export default function Meetings({ me, go }: { me: string; go: Go }) {
  const [list, setList] = useState<Meeting[]>([]);
  const [openId, setOpenId] = useState<number | null>(null);
  const [st, setSt] = useState<MeetingStatus>(IDLE);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [q, setQ] = useState("");
  // Full-text hits for `q` (what was said, the notes). Kept while the next query is pending.
  const [hits, setHits] = useState<Map<number, SearchHit> | null>(null);
  const seq = useRef(0);
  // The list page's Ask thread lives here, so opening a cited meeting and coming back keeps it.
  const [askLog, setAskLog] = useState<ChatTurn[] | null>(null);
  const [askReopen, setAskReopen] = useState(false);
  const sel = useSelection();

  const loadList = useCallback(() => { api.meetingsList().then(setList); }, []);
  useEffect(() => {
    loadList();
    api.meetingStatus().then(setSt);
    api.meetingsAskLog().then(setAskLog).catch(() => setAskLog([]));
    const un = [
      on("meetings-changed", loadList),
      on<Segment>("meeting-segment", (s) => setSt((p) => ({ ...p, segments: [...p.segments, s] }))),
      on<{ title: string; people: string; who: string; template?: string }>("meeting-named", (n) => setSt((p) => ({
        ...p,
        title: n.title || p.title,
        people: n.people || p.people,
        template: n.template || p.template,
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
  useEffect(() => {
    const query = q.trim();
    const n = ++seq.current;
    if (!query) { setHits(null); return; }
    const t = setTimeout(() => {
      api.meetingsSearch(query)
        .then((r) => { if (n === seq.current) setHits(new Map(r.map((h) => [h.id, h]))); })
        .catch(() => {});
    }, 120);
    return () => clearTimeout(t);
  }, [q, list]);

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
    if (id) { setAskReopen(false); setOpenId(id); }
  };
  const open = (id: number) => { setAskReopen(false); setOpenId(id); };
  const openCited = (id: number) => { setAskReopen(true); setOpenId(id); };

  if (st.running) return <div className="meetings-page"><LiveDoc st={st} stop={stop} busy={busy} me={me} go={go} setTemplate={(t) => setSt((p) => ({ ...p, template: t }))} /></div>;
  if (openId !== null) return <div className="meetings-page"><MeetingDoc key={openId} id={openId} me={me} go={go} back={() => { setOpenId(null); loadList(); }} /></div>;

  const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const shown = words.length ? list.filter((m) => localMatch(m, words) || hits?.has(m.id)) : list;
  const days: [string, Meeting[]][] = [];
  for (const m of shown) {
    const d = dayOf(m.ts);
    if (days.length && days[days.length - 1][0] === d) days[days.length - 1][1].push(m);
    else days.push([d, [m]]);
  }

  // Ask scope: the ticked rows, else what the search shows, else everything.
  const ticked = sel.on && sel.ids.size > 0 ? [...sel.ids] : null;
  const askIds = ticked ?? (words.length ? shown.map((m) => m.id) : undefined);
  const k = askIds?.length ?? 0;
  const scope = ticked ? `${k} selected` : askIds ? `${k} shown` : "all meetings";
  const placeholder = ticked
    ? (k === 1 ? "Ask about the selected meeting…" : `Ask about the ${k} selected meetings…`)
    : askIds ? (k === 1 ? "Ask about this meeting…" : `Ask about these ${k} meetings…`)
    : "Ask about all your meetings…";
  const ask = async (question: string) => { const t = await api.meetingsAsk(question, askIds); setAskLog(t); return t; };

  return (
    <div className="meetings-page">
      <div className="mlist">
        <div className="mlist-body">
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
                const hit = words.length ? hits?.get(m.id) : undefined;
                const snip = hit && (hit.field === "notes" || hit.field === "said") && hit.snippet[1] ? hit : null;
                return (
                  <button key={m.id} className={`mrow ${sel.on ? "selecting" : ""} ${sel.on && sel.has(m.id) ? "picked" : ""} ${writing ? "writing" : ""} ${snip ? "hit" : ""}`} onClick={() => (sel.on ? sel.toggle(m.id) : open(m.id))}>
                    {sel.on && <Tick on={sel.has(m.id)} />}
                    {writing
                      ? <span className="ttl"><span className="lamp busy" aria-hidden />Writing notes…{m.cal?.trim() && <span className="who-bit" dir="auto"> · {m.cal.trim()}</span>}</span>
                      : <span className="ttl" dir="auto">{m.title.trim()}{people && <span className="who-bit"> · {people}</span>}</span>}
                    <span className="mmeta">{hhmm(m.ts)} · {mins(m.secs)}</span>
                    {snip && (
                      <span className="snip">
                        <span className="where">{snip.field === "said" ? "Said" : "Notes"}</span>
                        <span dir="auto">{plain(snip.snippet[0])}<mark>{plain(snip.snippet[1])}</mark>{plain(snip.snippet[2])}</span>
                      </span>
                    )}
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
        {list.length >= 2 && askLog !== null && (!askIds || askIds.length > 0) && (
          <AskBar initial={askLog} ask={ask} clear={() => { api.meetingsAskClear(); setAskLog([]); }}
            placeholder={placeholder} label="Ask about your meetings" suggestions={SUGGESTIONS_ALL} scope={scope}
            openMeeting={openCited} known={new Set(list.map((m) => m.id))} startOpen={askReopen} />
        )}
      </div>
    </div>
  );
}
