import { useCallback, useEffect, useMemo, useState } from "react";
import { api, bindingShort, type Settings } from "../lib/api";
import { emitLocal, inTauri, on } from "../lib/events";
import { changeCount, diffWords } from "../lib/diff";
import Logo from "./Logo";
import "./fix.css";

const MODES: { id: string; label: string }[] = [
  { id: "grammar", label: "Fix grammar" },
  { id: "formal", label: "More formal" },
  { id: "friendly", label: "Friendlier" },
  { id: "shorter", label: "Shorter" },
  { id: "to_en", label: "→ English" },
  { id: "to_fa", label: "→ فارسی" },
];

const DEMO_IN = "hi sam, i has went to the meeting yesterday and we discuss about the budget, its was good but we needs more datas before friday.";
const DEMO_OUT: Record<string, string> = {
  grammar: "Hi Sam, I went to the meeting yesterday and we discussed the budget. It was good, but we need more data before Friday.",
  formal: "Hi Sam, I attended yesterday's meeting, where we discussed the budget. It went well, though we will need additional data before Friday.",
  friendly: "Hi Sam! I went to the meeting yesterday and we talked through the budget. It went well, we just need a bit more data before Friday.",
  shorter: "Hi Sam, yesterday's budget meeting went well, but we need more data by Friday.",
  to_fa: "سلام سم، دیروز به جلسه رفتم و درباره‌ی بودجه صحبت کردیم. خوب بود، ولی تا جمعه به داده‌های بیشتری نیاز داریم.",
  to_en: "Hi Sam, I went to the meeting yesterday and we discussed the budget. It was good, but we need more data before Friday.",
};

export default function Fix() {
  const [original, setOriginal] = useState("");
  const [mode, setMode] = useState("grammar");
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showDiff, setShowDiff] = useState(true);
  const [settings, setSettings] = useState<Settings | null>(null);

  useEffect(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
    const a = on<{ original: string }>("fix-open", (p) => {
      setOriginal(p.original);
      setMode("grammar");
      setResult(null);
      setError(null);
      api.getSettings().then(setSettings).catch(() => {});
    });
    const b = on<{ mode: string; text?: string; error?: string }>("fix-result", (p) => {
      if (p.error) setError(p.error);
      else setResult(p.text ?? "");
    });
    if (!inTauri) {
      // Browser preview with a sample sentence.
      emitLocal("fix-open", { original: DEMO_IN });
      setTimeout(() => emitLocal("fix-result", { mode: "grammar", text: DEMO_OUT.grammar }), 1400);
    }
    return () => { a(); b(); };
  }, []);

  const run = (m: string) => {
    setMode(m);
    setResult(null);
    setError(null);
    if (inTauri) api.fixRewrite(m);
    else setTimeout(() => emitLocal("fix-result", { mode: m, text: DEMO_OUT[m] ?? DEMO_OUT.grammar }), 900);
  };

  const accept = useCallback(() => { if (result) api.fixAccept(result); }, [result]);
  const cancel = useCallback(() => api.fixCancel(), []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancel();
      if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); accept(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [accept, cancel]);

  const pieces = useMemo(() => (result != null ? diffWords(original, result) : []), [original, result]);
  const changes = changeCount(pieces);
  const isTranslate = mode.startsWith("to_");

  return (
    <div className="fix-stage">
      <div className="fix-card">
        <header className="fix-head" data-tauri-drag-region>
          <div data-tauri-drag-region>
            <div className="label" data-tauri-drag-region style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <Logo size={16} /> Seda · Fix my text
            </div>
            <div className="fix-title" data-tauri-drag-region>
              {result == null && !error ? "Reading your text…" : error ? "Couldn't rewrite" : changes === 0 && !isTranslate ? "Looks good already" : isTranslate ? "Translation" : `${changes} change${changes === 1 ? "" : "s"}`}
            </div>
          </div>
          {settings && <span className="keycap">{bindingShort(settings.fix_binding)}</span>}
        </header>

        <div className="fix-modes">
          {MODES.map((m) => (
            <button key={m.id} className={m.id === mode ? "on" : ""} onClick={() => run(m.id)}>
              {m.label}
            </button>
          ))}
        </div>

        <div className="fix-body scroll">
          {error ? (
            <p className="fix-error">{error}</p>
          ) : result == null ? (
            <div className="fix-loading">
              <p dir="auto" className="fix-ghost">{original}</p>
              <span className="scanline" />
            </div>
          ) : showDiff && !isTranslate ? (
            <p dir="auto" className="fix-text">
              {pieces.map((p, i) => (
                <span key={i} className={p.kind}>{p.text}</span>
              ))}
            </p>
          ) : (
            <p dir="auto" className="fix-text">{result}</p>
          )}
        </div>

        <footer className="fix-foot">
          {!isTranslate && result != null && (
            <label className="fix-toggle" onClick={() => setShowDiff(!showDiff)}>
              <span className={`switch ${showDiff ? "on" : ""}`} /> Show changes
            </label>
          )}
          <div className="spacer" />
          <button className="btn ghost" onClick={cancel}>Cancel <span className="keycap">Esc</span></button>
          <button className="btn" disabled={!result} onClick={() => result && navigator.clipboard.writeText(result)}>Copy</button>
          <button className="btn signal" disabled={!result} onClick={accept}>Replace <span className="keycap dark">↵</span></button>
        </footer>
      </div>
    </div>
  );
}
