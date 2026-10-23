import { useCallback, useEffect, useState } from "react";
import { api, type Binding, type Settings, type Status } from "./api";
import { inTauri, on } from "./events";

export function useSettings() {
  const [settings, setSettings] = useState<Settings | null>(null);
  useEffect(() => {
    api.getSettings().then(setSettings);
    return on("settings-changed", () => api.getSettings().then(setSettings));
  }, []);
  // Send only the changed field; the backend merges it and returns the full, current settings.
  // (Sending a whole stale copy once wiped out model paths that setup had just filled in.)
  const update = useCallback(<K extends keyof Settings>(key: K, value: Settings[K]) => {
    setSettings((prev) => (prev ? { ...prev, [key]: value } : prev));
    api.updateSetting(key, value).then(setSettings).catch(() => api.getSettings().then(setSettings));
  }, []);
  return { settings, update };
}

export function useStatus() {
  const [status, setStatus] = useState<Status>({ asr: "loading", asr_error: null, llm: "loading", llm_error: null });
  useEffect(() => {
    api.getStatus().then(setStatus);
    return on<Status>("status", setStatus);
  }, []);
  return status;
}

/** Re-runs `load` whenever a new dictation is saved. */
export function useOnHistory(load: () => void) {
  useEffect(() => {
    load();
    return on("history-changed", () => load());
  }, [load]);
}

/** DOM mouse buttons → Seda's binding codes (1 = back/X1, 2 = forward/X2, 3 = middle). */
const MOUSE_CODE: Record<number, number> = { 3: 1, 4: 2, 1: 3 };

/** Windows doesn't call Seda's global keyboard/mouse hook while a Seda window has focus, so this
 *  window watches for the dictation shortcut itself and hands it to the same logic. */
export function useLocalShortcut(binding: Binding | undefined) {
  useEffect(() => {
    if (!binding || !inTauri) return;
    const b = binding;
    const keyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") { api.localHotkey("cancel"); return; }
      if (b.kind !== "key" || e.repeat || e.keyCode !== b.code) return;
      if (e.ctrlKey !== b.ctrl || e.shiftKey !== b.shift || e.altKey !== b.alt || e.metaKey !== b.win) return;
      e.preventDefault();
      api.localHotkey("down");
    };
    const keyUp = (e: KeyboardEvent) => {
      if (b.kind === "key" && e.keyCode === b.code) api.localHotkey("up");
    };
    const mouse = (down: boolean) => (e: MouseEvent) => {
      if (b.kind !== "mouse" || MOUSE_CODE[e.button] !== b.code) return;
      e.preventDefault();
      api.localHotkey(down ? "down" : "up");
    };
    const md = mouse(true), mu = mouse(false);
    window.addEventListener("keydown", keyDown);
    window.addEventListener("keyup", keyUp);
    window.addEventListener("mousedown", md);
    window.addEventListener("mouseup", mu);
    return () => {
      window.removeEventListener("keydown", keyDown);
      window.removeEventListener("keyup", keyUp);
      window.removeEventListener("mousedown", md);
      window.removeEventListener("mouseup", mu);
    };
  }, [binding?.kind, binding?.code, binding?.ctrl, binding?.shift, binding?.alt, binding?.win]); // eslint-disable-line react-hooks/exhaustive-deps
}