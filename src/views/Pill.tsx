import { useEffect, useRef, useState, type PointerEvent } from "react";
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

type PillState = { state: string; app: string; message: string; shape?: string };
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
  const [st, setSt] = useState<PillState>(() =>
    !inTauri && new URLSearchParams(location.search).get("demo") !== "1"
      ? { state: "idle", app: "", message: "", shape: "dot" }
      : { state: "hidden", app: "", message: "", shape: "pill" },
  );
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
    const stopDemo = !inTauri && new URLSearchParams(location.search).get("demo") === "1" ? runDemo() : () => {};
    return () => { un1(); un2(); stopDemo(); };
  }, []);

  // Browser preview has no microphone, so the clock runs on its own after a click.
  useEffect(() => {
    if (inTauri || st.state !== "listening") return;
    const start = performance.now();
    const id = window.setInterval(() => setSecs((performance.now() - start) / 1000), 200);
    return () => clearInterval(id);
  }, [st.state]);

  const s = st.state;
  const working = s === "transcribing" || s === "polishing";
  const showText = s === "done" || s === "empty" || s === "error" || s === "cancelled";
  const lamp = s === "listening" ? "rec" : working ? "work" : s === "done" ? "ok" : s === "error" ? "err" : "idle";
  const shape = st.shape ?? (s === "idle" ? "dot" : "pill");

  function toggleDot() {
    if (inTauri) {
      api.dotClick().catch(() => {});
      return;
    }
    setSecs(0);
    setSt((cur) =>
      cur.state === "listening"
        ? { state: "idle", app: "", message: "", shape: "dot" }
        : { state: "listening", app: "", message: "", shape: "dot" },
    );
  }

  if (shape === "dot" && (s === "idle" || s === "listening")) {
    return (
      <div className="pill-stage in dot">
        <IdleCircle listening={s === "listening"} secs={secs} onToggle={toggleDot} />
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

/** The on-screen circle. A short press starts or stops dictation. A drag moves it. */
function IdleCircle({ listening, secs, onToggle }: { listening: boolean; secs: number; onToggle: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const drag = useRef<{ sx: number; sy: number; ox: number; oy: number; left: number; top: number; moved: boolean; chain: Promise<void> } | null>(null);
  // A drag still ends with a click. Swallow that click so it does not also start dictation.
  const swallowClick = useRef(false);

  function down(e: PointerEvent<HTMLButtonElement>) {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const rect = e.currentTarget.getBoundingClientRect();
    drag.current = { sx: e.screenX, sy: e.screenY, ox: e.screenX, oy: e.screenY, left: rect.left, top: rect.top, moved: false, chain: Promise.resolve() };
  }

  function move(e: PointerEvent<HTMLButtonElement>) {
    const d = drag.current;
    if (!d) return;
    const dx = e.screenX - d.sx;
    const dy = e.screenY - d.sy;
    d.sx = e.screenX;
    d.sy = e.screenY;
    if ((e.screenX - d.ox) ** 2 + (e.screenY - d.oy) ** 2 > 25) d.moved = true;
    if (!d.moved || (dx === 0 && dy === 0)) return;
    if (inTauri) {
      d.chain = d.chain.then(() => api.dotNudge(dx, dy)).catch(() => {});
      return;
    }
    d.left += dx;
    d.top += dy;
    const el = ref.current;
    if (!el) return;
    el.style.position = "fixed";
    el.style.left = `${d.left}px`;
    el.style.top = `${d.top}px`;
  }

  function up() {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    if (d.moved) {
      swallowClick.current = true;
      if (inTauri) d.chain.then(() => api.dotSave()).catch(() => {});
    }
  }

  return (
    <button
      ref={ref}
      type="button"
      className={`idle-dot ${listening ? "rec" : ""}`}
      aria-pressed={listening}
      aria-label={listening ? "Stop dictation" : "Start dictation. Drag to move."}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={up}
      onClick={() => {
        if (swallowClick.current) {
          swallowClick.current = false;
          return;
        }
        onToggle();
      }}
    >
      <Logo size={listening ? 28 : 52} dot={listening ? "#ff4b2b" : "#ff1f26"} pulse={listening} outline={false} plate={false} />
      {listening && <span className="idle-time">{fmt(secs)}</span>}
    </button>
  );
}
