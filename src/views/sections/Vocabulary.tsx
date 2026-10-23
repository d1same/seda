import { useCallback, useEffect, useState } from "react";
import { api, type Fix } from "../../lib/api";
import { Head } from "./ui";

export default function Vocabulary() {
  const [terms, setTerms] = useState<string[]>([]);
  const [fixes, setFixes] = useState<Fix[]>([]);
  const [term, setTerm] = useState("");
  const [wrong, setWrong] = useState("");
  const [right, setRight] = useState("");

  const load = useCallback(() => {
    api.vocabList().then(setTerms);
    api.fixesList().then(setFixes);
  }, []);
  useEffect(load, [load]);

  const addTerm = async () => {
    for (const t of term.split(/[,،\n]/).map((s) => s.trim()).filter(Boolean)) await api.vocabAdd(t);
    setTerm("");
    load();
  };
  const addFix = async () => {
    if (!wrong.trim() || !right.trim()) return;
    await api.fixAdd(wrong, right);
    setWrong("");
    setRight("");
    load();
  };

  return (
    <>
      <Head n="Words" title="Teach it your words">
        Names and terms Seda should always spell your way.
      </Head>

      <div className="add-row">
        <input className="field" dir="auto" placeholder="e.g. Alex Morgan, Jira, Kubernetes, staging" value={term}
          onChange={(e) => setTerm(e.target.value)} onKeyDown={(e) => e.key === "Enter" && addTerm()} />
        <button className="btn primary" onClick={addTerm}>Add</button>
      </div>
      <div className="chips">
        {terms.map((t) => (
          <span key={t} className="chip" dir="auto">
            {t}
            <button aria-label={`Remove ${t}`} onClick={async () => { await api.vocabRemove(t); load(); }}>×</button>
          </span>
        ))}
        {terms.length === 0 && <span className="faint">No words yet. Start with the names of people you work with.</span>}
      </div>

      <div className="group-label label">Learned fixes</div>
      <p className="muted" style={{ margin: "0 0 4px", fontSize: 13 }}>
        Always replace one phrase with another. Seda adds these automatically when you correct a dictation in History.
      </p>
      <div className="add-row">
        <input className="field" dir="auto" placeholder="When it writes…" value={wrong} onChange={(e) => setWrong(e.target.value)} />
        <input className="field" dir="auto" placeholder="…write this instead" value={right} onChange={(e) => setRight(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && addFix()} />
        <button className="btn primary" onClick={addFix}>Add</button>
      </div>
      <table className="fix-table">
        <tbody>
          {fixes.map((f) => (
            <tr key={f.wrong}>
              <td className="w" dir="auto">{f.wrong}</td>
              <td className="arrow">→</td>
              <td dir="auto">{f.right}</td>
              <td className="mono faint" style={{ width: 90, fontSize: 12 }}>used {f.hits}×</td>
              <td style={{ width: 70, textAlign: "end" }}>
                <button className="btn ghost small" onClick={async () => { await api.fixRemove(f.wrong); load(); }}>Remove</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {fixes.length === 0 && <div className="empty" style={{ padding: 30 }}>No fixes yet.</div>}
    </>
  );
}
