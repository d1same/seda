import { useState } from "react";
import { api, type Settings, type Status } from "../../lib/api";
import { Head, Row, type Update } from "./ui";

function PathField({ value, onSave }: { value: string; onSave: (v: string) => void }) {
  const [v, setV] = useState(value);
  return (
    <div style={{ display: "flex", gap: 8, width: "100%" }}>
      <input className="field mono" style={{ fontSize: 12 }} value={v} onChange={(e) => setV(e.target.value)} spellCheck={false} />
      <button className="btn small" disabled={v === value} onClick={() => onSave(v)}>Apply</button>
    </div>
  );
}

const lamp = (s: string) => (s === "ready" ? "ok" : s === "loading" ? "busy" : s === "error" ? "err" : "");

export default function Engine({ settings, update, status, embedded }: { settings: Settings; update: Update; status: Status; embedded?: boolean }) {
  return (
    <>
      {!embedded && <Head n="Engine" title="Models on this PC" />}
      <Row title="Models" help="Both run on this PC's graphics card. Restart them after changing a file below.">
        <button className="btn" onClick={() => api.reloadModels()}>Restart models</button>
      </Row>
      <Row top title="Speech model" help={<><span className={`lamp ${lamp(status.asr)}`} style={{ display: "inline-block", marginRight: 6 }} />Persian Whisper turbo (vhdm), q5_0</>}>
        <PathField value={settings.whisper_model} onSave={(v) => update("whisper_model", v)} />
        {status.asr_error && <div className="error-text">{status.asr_error}</div>}
      </Row>
      <Row top title="Writing AI" help={<><span className={`lamp ${lamp(status.llm)}`} style={{ display: "inline-block", marginRight: 6 }} />Gemma 4 E4B, Q4_K_M</>}>
        <PathField value={settings.llm_model} onSave={(v) => update("llm_model", v)} />
        {status.llm_error && <div className="error-text">{status.llm_error}</div>}
      </Row>
      <Row top title="AI server" help="llama.cpp server (Vulkan build), started and stopped by Seda.">
        <PathField value={settings.llm_server} onSave={(v) => update("llm_server", v)} />
      </Row>
      <TestRow />
      <Row title="Your data" help="History, recordings, learned fixes and settings.">
        <button className="btn" onClick={() => api.openDataFolder()}>Open folder</button>
      </Row>
    </>
  );
}

type Result = Awaited<ReturnType<typeof api.testFile>>;

function TestRow() {
  const [path, setPath] = useState("");
  const [style, setStyle] = useState("");
  const [busy, setBusy] = useState(false);
  const [res, setRes] = useState<Result | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const run = async () => {
    setBusy(true); setErr(null); setRes(null);
    try { setRes(await api.testFile(path, style)); } catch (e) { setErr(String(e)); }
    setBusy(false);
  };
  return (
    <Row top title="Test with a recording" help="Run any .wav file through the whole pipeline without pasting, to check accuracy and speed.">
      <div style={{ display: "flex", gap: 8, width: "100%" }}>
        <input className="field mono" style={{ fontSize: 12 }} placeholder="C:\path\to\recording.wav" value={path} onChange={(e) => setPath(e.target.value)} spellCheck={false} />
        <select className="field" style={{ width: 120 }} value={style} onChange={(e) => setStyle(e.target.value)}>
          <option value="">Plain</option>
          <option value="email">Email</option>
          <option value="ticket">Ticket</option>
          <option value="chat">Chat</option>
        </select>
        <button className="btn primary" disabled={!path || busy} onClick={run}>{busy ? "Running…" : "Run"}</button>
      </div>
      {err && <div className="error-text">{err}</div>}
      {res && (
        <div style={{ width: "100%", marginTop: 6 }}>
          <div className="label" style={{ margin: "8px 0 4px" }}>Heard · {(res.asr_ms / 1000).toFixed(2)}s for {res.secs.toFixed(1)}s of audio</div>
          <div dir="auto" className="muted" style={{ unicodeBidi: "plaintext" }}>{res.raw}</div>
          <div className="label" style={{ margin: "12px 0 4px" }}>Final · cleanup {(res.llm_ms / 1000).toFixed(2)}s</div>
          <div dir="auto" style={{ fontSize: 16, whiteSpace: "pre-wrap", unicodeBidi: "plaintext" }}>{res.text}</div>
        </div>
      )}
    </Row>
  );
}
