import { useCallback, useEffect, useState } from "react";
import { api, type Settings, type Status } from "./api";
import { on } from "./events";

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
