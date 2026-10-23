import type { HistoryItem } from "../../lib/api";

const clockTime = (ts: string) => new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/** "Today", "Yesterday", or a date: the group heading for a history entry. */
export function dayOf(ts: string): string {
  const d = new Date(ts);
  const today = new Date();
  const yesterday = new Date(Date.now() - 86_400_000);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", month: "short", day: "numeric" });
}

/** One dictation as a single line; clicking it opens the full entry. */
export default function HistoryRow({ item, open }: { item: HistoryItem; open: (item: HistoryItem) => void }) {
  const text = item.corrected ?? item.cleaned;
  return (
    <div className={`hrow ${item.corrected ? "corrected" : ""}`}>
      <button className="hrow-line" onClick={() => open(item)}>
        <span className="htime">{clockTime(item.ts)}</span>
        <span className="happ">{item.app || "—"}</span>
        <span className="htext" dir="auto">{text.replace(/\s+/g, " ")}</span>
        <span className="hdur">{item.secs.toFixed(0)}s</span>
      </button>
    </div>
  );
}
