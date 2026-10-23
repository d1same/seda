import { useCallback, useState } from "react";
import { api, type HistoryItem } from "../../lib/api";
import { useOnHistory } from "../../lib/hooks";
import HistoryRow, { dayOf } from "./HistoryRow";
import { Head } from "./ui";

export default function History({ open }: { open: (item: HistoryItem) => void }) {
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [q, setQ] = useState("");
  const load = useCallback(() => { api.history(500).then(setItems); }, []);
  useOnHistory(load);
  const needle = q.trim().toLowerCase();
  const shown = needle ? items.filter((i) => `${i.corrected ?? i.cleaned} ${i.raw} ${i.app}`.toLowerCase().includes(needle)) : items;

  // Group by day, keeping the newest-first order.
  const days: [string, HistoryItem[]][] = [];
  for (const i of shown) {
    const d = dayOf(i.ts);
    if (days.length && days[days.length - 1][0] === d) days[days.length - 1][1].push(i);
    else days.push([d, [i]]);
  }

  return (
    <>
      <Head n="History" title="History" right={<input className="field" style={{ width: 220 }} dir="auto" placeholder="Search…" value={q} onChange={(e) => setQ(e.target.value)} />}>
        Everything you dictated. Open one to read all of it, copy it, or correct it so Seda learns.
      </Head>
      {days.map(([day, list]) => (
        <div key={day}>
          <div className="label day-label">{day}</div>
          {list.map((i) => <HistoryRow key={i.id} item={i} open={open} />)}
        </div>
      ))}
      {shown.length === 0 && <div className="empty"><div className="big">{q ? "No matches." : "No dictations yet."}</div></div>}
    </>
  );
}
