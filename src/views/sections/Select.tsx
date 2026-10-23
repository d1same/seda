import { useState } from "react";
import { IconButton } from "../Icons";

/** Pick several items in a list (meetings, dictations) to delete them together. */
export function useSelection() {
  const [on, setOn] = useState(false);
  const [ids, setIds] = useState<Set<number>>(new Set());
  return {
    on,
    ids,
    has: (id: number) => ids.has(id),
    toggle: (id: number) => setIds((s) => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n; }),
    setAll: (all: number[]) => setIds((s) => (s.size === all.length ? new Set() : new Set(all))),
    start: () => { setIds(new Set()); setOn(true); },
    stop: () => { setIds(new Set()); setOn(false); },
  };
}
export type Selection = ReturnType<typeof useSelection>;

/** "Select" button; while selecting: count, Select all, Delete (with confirmation) and Cancel. */
export function SelectBar({ sel, all, what, onDelete }: { sel: Selection; all: number[]; what: string; onDelete: (ids: number[]) => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  if (!sel.on) return <IconButton icon="check" label="Select" onClick={sel.start} />;
  const n = sel.ids.size;
  const remove = async () => {
    if (!n || !confirm(`Delete ${n} ${what}${n === 1 ? "" : "s"}? This can't be undone.`)) return;
    setBusy(true);
    await onDelete([...sel.ids]);
    setBusy(false);
    sel.stop();
  };
  return (
    <div className="select-bar">
      <span className="faint">{n} selected</span>
      <button className="link" onClick={() => sel.setAll(all)}>{n === all.length && n > 0 ? "Select none" : "Select all"}</button>
      <IconButton icon="trash" label={busy ? "Deleting…" : `Delete${n ? ` ${n}` : ""}`} danger disabled={!n || busy} onClick={remove} />
      <IconButton icon="back" label="Cancel" onClick={sel.stop} />
    </div>
  );
}

/** The tick box shown on each row while selecting. */
export function Tick({ on }: { on: boolean }) {
  return <span className={`tick-box ${on ? "on" : ""}`} aria-hidden>{on && <svg width="10" height="10" viewBox="0 0 10 10"><path d="M1.5 5.2 4 7.5 8.5 2.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" /></svg>}</span>;
}
