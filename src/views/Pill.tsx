import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { emitLocal, inTauri, on } from "../lib/events";
import Logo from "./Logo";
import "./pill.css";

/** Browser preview: loop through every pill state with a fake voice level. */
function runDemo(): () => void {
  const timers: number[] = [];
  let t = 0;
  const cycle = () => {
    emitLocal("pill", { state: "listening", app: "Outlook", message: "" });
    const start = performance.now();
    const meter = window.setInterval(() => {
      t += 1;
      const secs = (performance.now() - start) / 1000;
      const speech = Math.abs(Math.sin(t / 3)) * (0.4 + Math.random() * 0.6) * (Math.sin(t / 11) > -0.6 ? 1 : 0.1);
      emitLocal("level", { level: speech * 0.5, secs });
    }, 33);
    timers.push(
      window.setTimeout(() => { clearInterval(meter); emitLocal("pill", { state: "transcribing", app: "Outlook", message: "" }); }, 4200),
      window.setTimeout(() => emitLocal("pill", { state: "polishing", app: "Outlook", message: "" }), 5600),
      window.setTimeout(() => emitLocal("pill", { state: "done", app: "Outlook", message: "Hi Sam, I sent the report. Please take a look." }), 7200),
      window.setTimeout(() => emitLocal("pill", { state: "hidden", app: "", message: "" }), 9000),
      window.setTimeout(cycle, 9800),
    );
  };
  timers.push(window.setTimeout(cycle, 300));
  return () => timers.forEach(clearTimeout);
}

type PillState = { state: string; app: string; message: string };
const BARS = 16;

let audioCtx: AudioContext | null = null;
function blip(freqs: number[], gain = 0.05) {
  try {
    audioCtx ??= new AudioContext();
    const t0 = audioCtx.currentTime;
    freqs.forEach((f, i) => {
      const o = audioCtx!.createOscillator();
      const g = audioCtx!.createGain();
      o.type = "sine";
      o.frequency.value = f;
      const t = t0 + i * 0.055;
      g.gain.setValueAtTime(0, t);
      g.gain.linearRampToValueAtTime(gain, t + 0.008);
      g.gain.exponentialRampToValueAtTime(0.0001, t + 0.09);
      o.connect(g).connect(audioCtx!.destination);
      o.start(t);
      o.stop(t + 0.1);
    });
  } catch { /* sound is optional */ }
}

const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

const LABEL: Record<string, string> = {
  listening: "Listening",
  transcribing: "Transcribing",
  polishing: "Polishing",
  done: "Pasted",
  empty: "Nothing to paste",
  error: "Problem",
  cancelled: "Cancelled",
};

export default function Pill() {
  const [st, setSt] = useState<PillState>({ state: "hidden", app: "", message: "" });
  const [secs, setSecs] = useState(0);
  const levels = useRef<number[]>(new Array(BARS).fill(0));
  const [, force] = useState(0);
  const sounds = useRef(true);

  useEffect(() => {
    const un1 = on<PillState>("pill", async (s) => {
      if (s.state === "listening") {
        levels.current = new Array(BARS).fill(0);
        setSecs(0);
        sounds.current = (await api.getSettings().catch(() => null))?.sounds ?? true;
        if (sounds.current) blip([740, 1110]);
      } else if (s.state === "transcribing" && sounds.current) blip([880, 590], 0.04);
      else if (s.state === "done" && sounds.current) blip([1320], 0.025);
      setSt(s);
    });
    const un2 = on<{ level: number; secs: number }>("level", (p) => {
      const l = levels.current;
      l.shift();
      l.push(Math.min(1, Math.pow(p.level, 0.6)));
      setSecs(p.secs);
      force((x) => x + 1);
    });
    const stopDemo = inTauri ? () => {} : runDemo();
    return () => { un1(); un2(); stopDemo(); };
  }, []);

  const s = st.state;
  const working = s === "transcribing" || s === "polishing";
  const showText = s === "done" || s === "empty" || s === "error" || s === "cancelled";
  const lamp = s === "listening" ? "rec" : working ? "work" : s === "done" ? "ok" : s === "error" ? "err" : "idle";

  if (s === "idle") {
    return (
      <div className="pill-stage in">
        <div className="idle-logo" title="Seda is ready"><Logo size={14} outline={false} /></div>
      </div>
    );
  }

  const DOT: Record<string, string> = { rec: "#ff4b2b", work: "#f0a73a", ok: "#3ddc5c", err: "#ff4b2b", idle: "#8a8478" };

  return (
    <div className={`pill-stage ${s === "hidden" ? "out" : "in"}`}>
      <div className={`pill ${s}`}>
        <span className="pill-logo"><Logo size={18} dot={DOT[lamp]} pulse={lamp === "rec" || lamp === "work"} outline={false} /></span>
        {showText ? (
          <div className="pill-text" dir="auto" title={st.message}>
            {s === "done" ? "Pasted" : st.message || LABEL[s]}
          </div>
        ) : (
          <div className={`meter ${working ? "frozen" : ""}`}>
            {levels.current.map((v, i) => (
              <i key={i} style={{ transform: `scaleY(${0.12 + v * 0.88})` }} />
            ))}
            {working && <span className="sweep" />}
          </div>
        )}
        {s === "listening" && <span className="pill-time">{fmt(secs)}</span>}
      </div>
    </div>
  );
}