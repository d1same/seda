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
      <Head n="Words" title="Words">
        Names and terms Seda should always spell your way.
      </Head>

      <div className="add-row">
        <input className="field" dir="auto" placeholder="e.g. Alex Morgan, Jira, Kubernetes, staging" value={term}
          onChange={(e) => setTerm(e.target.value)} onKeyDown={(e) => e.key === "Enter" && addTerm()} />
        <button className="btn primary" onClick={addTerm}>Add</button>
      </div>
      <div className="chips">
        {terms.map((t) => (
          <span key={t} className="chip">
            <span className="bidi-clip" dir="auto">{t}</span>
            <button aria-label={`Remove ${t}`} onClick={async () => { await api.vocabRemove(t); load(); }}>×</button>
          </span>
        ))}
        {terms.length === 0 && (
          <div className="empty compact">
            <div className="big">No words yet</div>
            Start with the people you work with.
          </div>
        )}
      </div>

      <div className="group-label">Learned fixes</div>
      <p className="muted" style={{ margin: "10px 0 0", fontSize: 13 }}>
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
              <td><span className="bidi-clip w" dir="auto">{f.wrong}</span></td>
              <td className="arrow" aria-label="becomes">→</td>
              <td><span className="bidi-clip r" dir="auto">{f.right}</span></td>
              <td className="uses">used {f.hits}×</td>
              <td className="act">
                <button className="btn ghost small" onClick={async () => { await api.fixRemove(f.wrong); load(); }}>Remove</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {fixes.length === 0 && (
        <div className="empty compact">
          <div className="big">No fixes yet</div>
          Correct a dictation in History and Seda remembers it.
        </div>
      )}
    </>
  );
}
