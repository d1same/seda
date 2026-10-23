import { useEffect, useState } from "react";
import { api, type Settings } from "../lib/api";
import type { Update } from "./sections/ui";

/** Choose the microphone; "" means whatever Windows has set as default. */
export default function MicPicker({ settings, update }: { settings: Settings; update: Update }) {
  const [mics, setMics] = useState<string[]>([]);
  useEffect(() => { api.listMics().then((m) => setMics(m ?? [])); }, []);
  const missing = settings.input_device && !mics.includes(settings.input_device);
  return (
    <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
      <select className="field" style={{ width: 300 }} value={settings.input_device} onChange={(e) => update("input_device", e.target.value)}>
        <option value="">Windows default microphone</option>
        {mics.map((m) => <option key={m} value={m}>{m}</option>)}
        {missing && <option value={settings.input_device}>{settings.input_device} (not connected)</option>}
      </select>
      <button className="btn ghost small" onClick={() => api.openMicPrivacy()}>No sound? Windows mic settings →</button>
    </div>
  );
}
