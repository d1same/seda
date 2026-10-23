import { useCallback, useEffect, useState } from "react";
import { api, bindingParts, type Binding, type Settings } from "../lib/api";
import type { Update } from "./sections/ui";

// DOM mouse buttons → Seda's codes (1 = back/X1, 2 = forward/X2, 3 = middle).
const MOUSE: Record<number, number> = { 3: 1, 4: 2, 1: 3 };
const MODIFIER_KEYS = ["Control", "Shift", "Alt", "Meta", "AltGraph"];

/** Keys that type something: binding one of them alone would stop you typing it. */
function typesSomething(e: KeyboardEvent) {
  const k = e.keyCode;
  return k === 32 || k === 13 || k === 9 || k === 8 || (k >= 0x30 && k <= 0x5a) || (k >= 0x60 && k <= 0x6f) || (k >= 0xba && k <= 0xe2);
}

/**
 * Records a new shortcut inside this window (it has focus while you choose), with Seda's global
 * hooks paused so nothing else is intercepted. Cancels on Esc, on leaving the window, or after 10 s.
 */
export default function ShortcutPicker({ which, settings, update, big }: { which: "dictate" | "fix"; settings: Settings; update: Update; big?: boolean }) {
  const [waiting, setWaiting] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  const key = which === "fix" ? "fix_binding" : "dictate_binding";
  const b = settings[key];

  const start = () => { setHint(null); setWaiting(true); api.setHotkeysPaused(true); };
  const stop = useCallback(() => { setWaiting(false); api.setHotkeysPaused(false); }, []);

  useEffect(() => {
    if (!waiting) return;
    const save = (nb: Binding) => { update(key, nb); stop(); };
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat || MODIFIER_KEYS.includes(e.key)) return;
      if (e.key === "Escape") return stop();
      if (!(e.ctrlKey || e.altKey || e.metaKey) && typesSomething(e)) {
        setHint("That key types text. Add Ctrl or Alt, or pick a mouse button or an F-key.");
        return;
      }
      save({ kind: "key", code: e.keyCode, ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, win: e.metaKey });
    };
    const onMouse = (e: MouseEvent) => {
      if (!(e.button in MOUSE)) return; // left/right clicks keep working (e.g. the Cancel button)
      e.preventDefault();
      e.stopPropagation();
      if (which === "fix") return setHint("Use a key combination for this one, e.g. Ctrl + Shift + Space.");
      save({ kind: "mouse", code: MOUSE[e.button], ctrl: false, shift: false, alt: false, win: false });
    };
    // Stop the back/forward buttons from doing anything else while choosing.
    const swallow = (e: MouseEvent) => { if (e.button in MOUSE) e.preventDefault(); };
    const opts = { capture: true };
    window.addEventListener("keydown", onKey, opts);
    window.addEventListener("mousedown", onMouse, opts);
    window.addEventListener("mouseup", swallow, opts);
    window.addEventListener("auxclick", swallow, opts);
    window.addEventListener("blur", stop);
    const timer = setTimeout(stop, 10_000);
    return () => {
      window.removeEventListener("keydown", onKey, opts);
      window.removeEventListener("mousedown", onMouse, opts);
      window.removeEventListener("mouseup", swallow, opts);
      window.removeEventListener("auxclick", swallow, opts);
      window.removeEventListener("blur", stop);
      clearTimeout(timer);
    };
  }, [waiting, which, key, update, stop]);

  // Never leave the hooks paused if this view closes mid-capture.
  useEffect(() => () => { api.setHotkeysPaused(false); }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div className={`binding ${big ? "big" : ""}`}>
        {waiting ? (
          <span className="keycap listening-cap">{which === "dictate" ? "Press a mouse button or key…" : "Press a key combination…"}</span>
        ) : (
          <span className="keys">{bindingParts(b).map((p) => <span key={p} className="keycap">{p}</span>)}</span>
        )}
        {waiting ? (
          <button className={`btn ${big ? "" : "small"}`} onClick={stop}>Cancel</button>
        ) : (
          <button className={`btn ${big ? "primary" : "small"}`} onClick={start}>Change</button>
        )}
      </div>
      {hint && waiting && <span className="error-text">{hint}</span>}
    </div>
  );
}
