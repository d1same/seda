import { useState } from "react";
import { api, type HistoryItem } from "../../lib/api";
import Icon, { IconButton } from "../Icons";
import { dayOf } from "./HistoryRow";

const STYLE_TAG: Record<string, string> = { email: "Email", ticket: "Ticket", chat: "Chat" };

/** A whole dictation on its own page: read it, copy it, or correct it so Seda learns. */
export default function HistoryDetail({ item, back }: { item: HistoryItem; back: () => void }) {
  const text = item.corrected ?? item.cleaned;
  const [shown, setShown] = useState(text);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(text);
  const [learned, setLearned] = useState<[string, string][] | null>(null);
  const [copied, setCopied] = useState(false);

  const save = async () => {
    const t = draft.trim();
    setLearned(await api.correctHistory(item.id, t));
    setShown(t);
    setEditing(false);
  };
  const copy = async () => {
    await navigator.clipboard.writeText(shown);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };
  const remove = async () => {
    await api.deleteHistory(item.id);
    back();
  };

  const time = new Date(item.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const words = shown.split(/\s+/).filter(Boolean).length;

  return (
    <div className="detail-page">
      <div className="detail-bar">
        <IconButton icon="back" label="Back" iconOnly onClick={back} />
        <div className="detail-bar-actions">
          {editing ? (
            <>
              <IconButton icon="check" label="Save & teach Seda" primary onClick={save} />
              <IconButton icon="back" label="Cancel" onClick={() => { setEditing(false); setDraft(shown); }} />
            </>
          ) : (
            <>
              <IconButton icon={copied ? "check" : "copy"} label={copied ? "Copied" : "Copy"} primary onClick={copy} />
              <IconButton icon="edit" label="Correct" onClick={() => { setDraft(shown); setEditing(true); }} />
              <IconButton icon="trash" label="Delete" iconOnly danger onClick={remove} />
            </>
          )}
        </div>
      </div>

      <div className="chip-row detail-chips">
        <span className="meta-chip"><Icon name="clock" size={13} />{dayOf(item.ts)}, {time}</span>
        <span className="meta-chip"><Icon name="app" size={13} />{item.app || "Unknown app"}{STYLE_TAG[item.style] ? ` · ${STYLE_TAG[item.style]}` : ""}</span>
        <span className="meta-chip">{words} words · {item.secs.toFixed(0)} s spoken · {((item.asr_ms + item.llm_ms) / 1000).toFixed(1)} s wait</span>
        {item.corrected && <span className="meta-chip ok"><Icon name="check" size={13} />Corrected · used for training</span>}
      </div>

      {editing
        ? <textarea className="detail-edit" dir="auto" value={draft} onChange={(e) => setDraft(e.target.value)} autoFocus />
        : <div className="detail-text" dir="auto">{shown}</div>}
      {editing && <p className="detail-hint">Fix any word Seda got wrong. It remembers the correction for next time.</p>}
      {learned && learned.length > 0 && <div className="learned" dir="auto"><Icon name="check" size={13} /> Learned: {learned.map(([w, r]) => `${w} → ${r}`).join(" · ")}</div>}

      {item.raw !== shown && !editing && (
        <div className="detail-raw">
          <div className="detail-raw-head"><Icon name="ear" size={14} />What Seda heard, before cleanup</div>
          <div dir="auto">{item.raw}</div>
        </div>
      )}
    </div>
  );
}
