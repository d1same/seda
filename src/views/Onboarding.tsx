import { useEffect, useMemo, useState } from "react";
import { api, bindingShort, type Pack, type SetupInfo, type Settings, type Status } from "../lib/api";
import { emitLocal, inTauri, on } from "../lib/events";
import ShortcutPicker from "./ShortcutPicker";
import MicPicker from "./MicPicker";
import Logo from "./Logo";
import { LANGUAGES } from "../lib/languages";
import type { Update } from "./sections/ui";
import "./onboarding.css";

const STEPS = ["Welcome", "Language", "Your computer", "Download", "Microphone", "Try it"];

const PACKS: { id: Pack; title: string; blurb: string }[] = [
  { id: "full", title: "Full", blurb: "Best writing: punctuation, “no wait” corrections, email and ticket styles. Needs 16 GB of memory." },
  { id: "light", title: "Light", blurb: "Smaller writing AI. Good punctuation, weaker at catching “no wait”. For 8–16 GB of memory." },
  { id: "speech", title: "Speech only", blurb: "Just speech to text, no writing AI. Fastest and smallest." },
];

const gb = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${Math.round(mb)} MB`);

export default function Onboarding({ settings, update, status, onDone }: { settings: Settings; update: Update; status: Status; onDone: () => void }) {
  const [step, setStep] = useState(0);
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [pack, setPack] = useState<Pack>("full");

  // The download list depends on the language (Persian has its own speech model).
  useEffect(() => {
    api.setupInfo(settings.language).then((i) => {
      setInfo(i);
      setPack((p) => (info ? p : i.hardware.recommended));
    });
  }, [settings.language]); // eslint-disable-line react-hooks/exhaustive-deps

  const finish = async () => { await api.finishOnboarding(); onDone(); };

  return (
    <div className="onb">
      <nav className="onb-steps">
        {STEPS.map((s, i) => (
          <div key={s} className={`onb-step ${i === step ? "on" : ""} ${i < step ? "done" : ""}`}>
            <span className="num mono">{i < step ? "✓" : String(i + 1).padStart(2, "0")}</span>
            {s}
          </div>
        ))}
        <div className="onb-foot faint">Everything runs on this PC.<br />Nothing you say is uploaded.</div>
      </nav>
      <main className="onb-body scroll">
        {step === 0 && <Welcome settings={settings} update={update} next={() => setStep(1)} />}
        {step === 1 && <Language settings={settings} update={update} next={() => setStep(2)} />}
        {step === 2 && <Computer info={info} pack={pack} setPack={setPack} next={() => setStep(3)} />}
        {step === 3 && info && <Download info={info} pack={pack} status={status} next={() => setStep(4)} back={() => setStep(2)} />}
        {step === 4 && <Microphone settings={settings} update={update} next={() => setStep(5)} />}
        {step === 5 && <TryIt settings={settings} finish={finish} />}
      </main>
    </div>
  );
}

function Welcome({ settings, update, next }: { settings: Settings; update: Update; next: () => void }) {
  const [name, setName] = useState(settings.user_name);
  const go = () => { if (name.trim() !== settings.user_name) update("user_name", name.trim()); next(); };
  return (
    <section className="onb-panel">
      <div style={{ marginBottom: 22 }}><Logo size={72} /></div>
      <div className="label">Setup · 1 of 6</div>
      <h1>Let's set up Seda.</h1>
      <p className="lead">Talk naturally — in your language, English, or both — and Seda types it properly wherever your cursor is.</p>
      <ul className="onb-facts">
        <li><b>Private.</b> The AI runs on this computer. Nothing you say leaves it.</li>
        <li><b>One download.</b> Two AI models, about 5 GB in total. Takes 5–10 minutes.</li>
        <li><b>Two minutes after that</b> you're dictating.</li>
      </ul>
      <label className="onb-name">
        <span>What should Seda call you?</span>
        <input className="field" dir="auto" placeholder="e.g. Alex" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && go()} />
        <small className="faint">Used in your greeting and meeting notes (“Alex, can you…” becomes your task). Optional.</small>
      </label>
      <button className="btn primary big-btn" onClick={go}>Start setup →</button>
    </section>
  );
}

function Language({ settings, update, next }: { settings: Settings; update: Update; next: () => void }) {
  return (
    <section className="onb-panel">
      <div className="label">Setup · 2 of 6</div>
      <h1>English, plus…?</h1>
      <p className="lead">English is always understood. Add one more language you speak (mixing both in a sentence works too), or pick None.</p>
      <div className="langs">
        {LANGUAGES.map((l) => (
          <button key={l.code} className={`lang ${settings.language === l.code ? "on" : ""}`} onClick={() => update("language", l.code)}>
            <span className="lang-native" dir={l.rtl ? "rtl" : "ltr"}>{l.native}</span>
            <span className="lang-name">{l.name}{l.code === "fa" ? " · tuned" : ""}</span>
          </button>
        ))}
      </div>
      <button className="btn primary big-btn" onClick={next}>Continue →</button>
    </section>
  );
}

function Computer({ info, pack, setPack, next }: { info: SetupInfo | null; pack: Pack; setPack: (p: Pack) => void; next: () => void }) {
  if (!info) return <section className="onb-panel"><div className="label">Checking your computer…</div></section>;
  const hw = info.hardware;
  return (
    <section className="onb-panel">
      <div className="label">Setup · 3 of 6</div>
      <h1>Your computer</h1>
      <div className="hw">
        <div><div className="label">Graphics</div><div className="hw-v">{hw.gpu}</div><div className="faint">{hw.integrated ? "Built-in graphics — works well" : `${hw.vram_gb.toFixed(0)} GB dedicated`}</div></div>
        <div><div className="label">Memory (RAM)</div><div className="hw-v">{Math.round(hw.ram_gb)} GB installed</div><div className="faint">{hw.ram_gb >= 15 ? "Plenty" : hw.ram_gb >= 7.5 ? "Enough for Light" : "Speech only recommended"}</div></div>
      </div>
      <div className="label" style={{ margin: "26px 0 10px" }}>Choose a package</div>
      <div className="packs">
        {PACKS.map((p) => {
          const missing = info.packs[p.id].filter((i) => i.state === "missing").reduce((n, i) => n + i.size_mb, 0);
          return (
            <button key={p.id} className={`pack ${pack === p.id ? "on" : ""}`} onClick={() => setPack(p.id)}>
              <div className="pack-top">
                <span className="pack-title">{p.title}</span>
                {hw.recommended === p.id && <span className="tag rec">Recommended</span>}
              </div>
              <p>{p.blurb}</p>
              <div className="mono pack-size">{missing === 0 ? "Already on this PC" : `${gb(missing)} to download`}</div>
            </button>
          );
        })}
      </div>
      <button className="btn primary big-btn" onClick={next}>Continue →</button>
    </section>
  );
}

type Progress = Record<string, { done: number; total: number }>;

function Download({ info, pack, status, next, back }: { info: SetupInfo; pack: Pack; status: Status; next: () => void; back: () => void }) {
  const items = info.packs[pack];
  const [progress, setProgress] = useState<Progress>({});
  const [phase, setPhase] = useState<"running" | "done" | "error">("running");
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    setPhase("running");
    setError(null);
    const offP = on<{ id: string; done: number; total: number }>("setup-progress", (p) => setProgress((x) => ({ ...x, [p.id]: { done: p.done, total: p.total } })));
    const offD = on<{ ok: boolean; error?: string }>("setup-done", (r) => { setPhase(r.ok ? "done" : "error"); setError(r.error ?? null); });
    if (inTauri) api.setupDownload(pack);
    else fakeDownload(items);
    return () => { offP(); offD(); };
  }, [pack, attempt]); // eslint-disable-line react-hooks/exhaustive-deps

  const modelsReady = status.asr === "ready" && (pack === "speech" || status.llm === "ready");
  const allFound = items.every((i) => i.state === "ready");

  return (
    <section className="onb-panel">
      <div className="label">Setup · 4 of 6</div>
      <h1>{allFound ? "Found everything" : phase === "done" ? "Downloaded" : "Downloading"}</h1>
      <p className="lead">{allFound ? "The models are already on this computer — nothing to download." : "You can keep using your computer meanwhile. If it stops, it resumes where it left off."}</p>
      <div className="dl-list">
        {items.map((it) => {
          const p = progress[it.id];
          const pct = it.state === "ready" ? 100 : p && p.total ? (p.done / p.total) * 100 : phase === "done" ? 100 : 0;
          return (
            <div key={it.id} className="dl">
              <div className="dl-row">
                <span>{it.name}</span>
                <span className="mono faint">
                  {it.state === "ready" ? "On this PC ✓" : pct >= 100 ? "Done ✓" : p ? `${gb(p.done / 1e6)} / ${gb(p.total / 1e6)}` : "Waiting"}
                </span>
              </div>
              <div className="dl-bar"><div style={{ width: `${pct}%` }} /></div>
            </div>
          );
        })}
      </div>
      {phase === "error" && (
        <div className="onb-error">
          <b>Download stopped.</b> {error}
          <div style={{ marginTop: 10, display: "flex", gap: 8 }}>
            <button className="btn primary" onClick={() => setAttempt((a) => a + 1)}>Try again</button>
            <button className="btn" onClick={back}>Choose another package</button>
          </div>
        </div>
      )}
      <div className="onb-actions">
        {phase === "running" && !allFound && <button className="btn ghost" onClick={() => api.setupCancel()}>Cancel</button>}
        <button className="btn primary big-btn" disabled={phase !== "done" || !modelsReady} onClick={next}>
          {phase !== "done" ? "Downloading…" : modelsReady ? "Continue →" : "Starting the models…"}
        </button>
      </div>
    </section>
  );
}

function Microphone({ settings, update, next }: { settings: Settings; update: Update; next: () => void }) {
  const [level, setLevel] = useState(0);
  const [peak, setPeak] = useState(0);
  const [err, setErr] = useState<string | null>(null);

  const [silentFor, setSilentFor] = useState(0);

  // Restart the meter whenever a different microphone is picked.
  useEffect(() => {
    setErr(null);
    setPeak(0);
    setSilentFor(0);
    const off = on<number>("mic-level", (l) => { setLevel(l); setPeak((p) => Math.max(p * 0.97, l)); });
    const tick = setInterval(() => setSilentFor((s) => s + 1), 1000);
    if (inTauri) api.micTestStart().catch((e) => setErr(String(e)));
    else {
      let t = 0;
      const id = setInterval(() => emitLocal("mic-level", Math.abs(Math.sin((t += 0.3))) * 0.6 * Math.random()), 50);
      return () => { clearInterval(id); clearInterval(tick); off(); };
    }
    return () => { clearInterval(tick); off(); api.micTestStop(); };
  }, [settings.input_device]);

  const cells = 28;
  const lit = Math.round(Math.min(1, level * 1.6) * cells);
  const heard = peak > 0.12;
  const stuck = !heard && silentFor >= 5;

  return (
    <section className="onb-panel">
      <div className="label">Setup · 5 of 6</div>
      <h1>Microphone</h1>
      <p className="lead">Say a few words. The meter should jump.</p>
      <div style={{ marginBottom: 16 }}><MicPicker settings={settings} update={update} /></div>
      {err ? (
        <div className="onb-error"><b>Can't open the microphone.</b> {err}<br />Check Windows Settings → Privacy → Microphone.</div>
      ) : (
        <>
          <div className="led">
            {Array.from({ length: cells }, (_, i) => (
              <i key={i} className={i < lit ? (i > cells * 0.85 ? "hot" : "on") : ""} />
            ))}
          </div>
          <div className="mono faint" style={{ marginTop: 8 }}>{heard ? "✓ Heard you clearly" : "Listening…"}</div>
          {stuck && (
            <div className="onb-error">
              <b>No sound yet.</b> Try another microphone above, or check that Windows lets desktop apps use the
              microphone (the “Windows mic settings” link).
            </div>
          )}
        </>
      )}
      <div className="label" style={{ margin: "34px 0 10px" }}>Your dictation button</div>
      <ShortcutPicker which="dictate" settings={settings} update={update} big />
      <p className="faint" style={{ marginTop: 10, fontSize: 13 }}>Hold it while you talk — or tap it once to talk hands-free, then tap again.</p>
      <button className="btn primary big-btn" onClick={next}>Continue →</button>
    </section>
  );
}

function TryIt({ settings, finish }: { settings: Settings; finish: () => void }) {
  const [result, setResult] = useState<string | null>(null);
  useEffect(() => on("history-changed", () => api.history(1).then((h) => setResult(h[0]?.cleaned ?? null))), []);
  const key = bindingShort(settings.dictate_binding);
  const sample = useMemo(
    () => (settings.language === "fa" ? "سلام! فردا ساعت ده جلسه داریم، نه ببخشید، ساعت یازده." : "Let's meet tomorrow at ten — no wait, at eleven."),
    [settings.language],
  );

  return (
    <section className="onb-panel">
      <div className="label">Setup · 6 of 6</div>
      <h1>Try it</h1>
      <p className="lead">
        Click in the box, then hold <span className="keycap">{key}</span> and say:
      </p>
      <blockquote dir={settings.language === "fa" ? "rtl" : "ltr"} className={settings.language === "fa" ? "fa" : ""}>{sample}</blockquote>
      <textarea className="field onb-try" dir="auto" autoFocus placeholder="Your words appear here…" />
      {result && <div className="onb-ok">✓ It works. Notice it dropped the part you took back.</div>}
      <div className="onb-actions">
        <button className="btn primary big-btn" onClick={finish}>{result ? "Finish →" : "Skip and finish"}</button>
      </div>
    </section>
  );
}

/** Browser preview only: pretend to download the missing items. */
function fakeDownload(items: SetupInfo["packs"][Pack]) {
  const missing = items.filter((i) => i.state === "missing");
  let k = 0;
  const tick = setInterval(() => {
    k++;
    missing.forEach((it, idx) => {
      const total = it.size_mb * 1e6;
      const done = Math.min(total, total * Math.max(0, k / 40 - idx * 0.4));
      emitLocal("setup-progress", { id: it.id, done, total });
    });
    if (k > 40 + missing.length * 16) { clearInterval(tick); emitLocal("setup-done", { ok: true }); }
  }, 120);
}
