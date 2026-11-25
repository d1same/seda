import { useEffect, useRef, useState } from "react";
import type { ChatTurn, Source } from "../../lib/api";
import Icon from "../Icons";

export type AskSuggestion = string | { label: string; prompt: string };

type Props = {
  initial: ChatTurn[];
  /** Sends the question; resolves to the whole thread. */
  ask: (q: string) => Promise<ChatTurn[]>;
  clear: () => void;
  placeholder: string;
  /** The input's accessible name. */
  label: string;
  suggestions: AskSuggestion[];
  /** Shown in the thread head: "Ask Seda · 3 selected". */
  scope?: string;
  /** Opens a cited meeting. Without it citations are plain numbers. */
  openMeeting?: (id: number) => void;
  /** Meetings that still exist; a citation of any other id is disabled. */
  known?: Set<number>;
  /** Open the thread on mount (coming back from a cited meeting). */
  startOpen?: boolean;
};

function CopyBtn({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  const copy = () => {
    navigator.clipboard?.writeText(text).then(() => {
      setDone(true);
      window.setTimeout(() => setDone(false), 1400);
    }).catch(() => {});
  };
  return (
    <button type="button" className="bubble-copy" onClick={copy}>
      <Icon name={done ? "check" : "copy"} size={12} />
      {done ? "Copied" : "Copy"}
    </button>
  );
}

const shortDate = (ts: string) => new Date(ts).toLocaleDateString([], { month: "short", day: "numeric" });

/** An answer across meetings: `[n]` becomes a small button that opens meeting n, and the meetings
 *  read are listed under the text (cited first; the rest behind a link). */
function Answer({ t, openMeeting, known }: { t: ChatTurn; openMeeting?: (id: number) => void; known?: Set<number> }) {
  const [more, setMore] = useState(false);
  const sources = t.sources ?? [];
  const byN = new Map(sources.map((s) => [s.n, s]));
  const live = (s: Source) => !!openMeeting && (!known || known.has(s.id));
  const parts = t.text.split(/\[(\d+)\]/);
  const cited = sources.filter((s) => s.cited);
  const rest = sources.filter((s) => !s.cited);
  const row = (s: Source) => {
    const inner = <><span className="src-n">{s.n}</span><span className="src-title" dir="auto">{s.title || "Untitled meeting"}</span><span className="when">{shortDate(s.ts)}</span></>;
    return live(s)
      ? <button key={s.n} type="button" className="src" onClick={() => openMeeting!(s.id)}>{inner}</button>
      : <span key={s.n} className="src gone" title="This meeting was deleted">{inner}</span>;
  };
  const restLabel = cited.length
    ? `Also read ${rest.length} more`
    : `Read ${rest.length} meeting${rest.length === 1 ? "" : "s"}`;
  return (
    <div className="bubble ai">
      <div className="ans" dir="auto">
        {parts.map((p, i) => {
          if (i % 2 === 0) return p;
          const s = byN.get(Number(p));
          if (!s) return `[${p}]`;
          return (
            <button key={i} type="button" className="cite" title={`${s.title || "Untitled meeting"} · ${shortDate(s.ts)}`}
              aria-label={`Source ${s.n}: ${s.title}`} disabled={!live(s)} onClick={() => openMeeting?.(s.id)}>{s.n}</button>
          );
        })}
      </div>
      {sources.length > 0 && (
        <div className="srcs">
          {cited.map(row)}
          {rest.length > 0 && (more
            ? rest.map(row)
            : <button type="button" className="link src-more" onClick={() => setMore(true)}>{restLabel}</button>)}
        </div>
      )}
    </div>
  );
}

/** The bar docked at the bottom of a page: ask about one meeting (and change its notes), or about
 *  several meetings from the list. The conversation opens above the bar. */
export default function AskBar({ initial, ask, clear, placeholder, label, suggestions, scope, openMeeting, known, startOpen }: Props) {
  const [chat, setChat] = useState<ChatTurn[]>(initial);
  const [open, setOpen] = useState(!!startOpen && initial.length > 0);
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
    try { setChat(await ask(question)); }
    catch (e) { setErr(String(e)); setChat((c) => c.slice(0, -1)); setQ(question); }
    setBusy(false);
  };
  const answers = chat.filter((t) => t.role === "ai").length;

  return (
    <div className="askbar-dock">
      {open && (chat.length > 0 || busy) && (
        <div className="ask-thread">
          <div className="ask-thread-head">
            <span>Ask Seda{scope && <span className="ask-scope"> · {scope}</span>}</span>
            <span>
              <button className="link" onClick={() => { clear(); setChat([]); setOpen(false); }}>Clear</button>
              <span className="sep" aria-hidden>·</span>
              <button className="link" onClick={() => setOpen(false)}>Hide</button>
            </span>
          </div>
          <div className="ask-log">
            {/* The core serialises `sources: []` on every turn; only a turn that read meetings gets citations. */}
            {chat.map((t, i) => (t.role === "ai" && t.sources?.length
              ? <Answer key={i} t={t} openMeeting={openMeeting} known={known} />
              : (
                <div key={i} className={`bubble ${t.role}`} dir="auto">
                  {t.text}
                  {t.role === "ai" && (
                    <div className="bubble-foot">
                      {t.edited && <span className="edited"><Icon name="check" size={12} />Notes updated</span>}
                      <CopyBtn text={t.text} />
                    </div>
                  )}
                </div>
              )))}
            {busy && <div className="bubble ai thinking"><span /><span /><span /></div>}
            <div ref={end} />
          </div>
        </div>
      )}
      {!busy && (!open || chat.length === 0) && (
        <div className="sugg-row">
          {suggestions.map((s) => {
            const label = typeof s === "string" ? s : s.label;
            const prompt = typeof s === "string" ? s : s.prompt;
            return <button key={label} className="sugg" onClick={() => send(prompt)}>{label}</button>;
          })}
        </div>
      )}
      {err && <div className="error-text">{err}</div>}
      <form className="askbar" onSubmit={(e) => { e.preventDefault(); send(q); }}>
        <Icon name="spark" size={15} />
        <input dir="auto" value={q} onChange={(e) => setQ(e.target.value)} onFocus={() => chat.length && setOpen(true)} placeholder={placeholder} aria-label={label} disabled={busy} />
        {chat.length > 0 && !open && <button type="button" className="link" onClick={() => setOpen(true)}>{answers === 1 ? "1 answer" : `${answers} answers`}</button>}
        <button className="send" disabled={busy || !q.trim()} aria-label="Send"><Icon name="send" size={14} /></button>
      </form>
    </div>
  );
}
