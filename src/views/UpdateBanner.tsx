import { useEffect, useState } from "react";
import { api, type UpdateInfo } from "../lib/api";
import { on } from "../lib/events";

/** One quiet line in the sidebar footer when a newer Seda is out: one click downloads, installs
 *  and restarts. */
export default function UpdateBanner() {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [pct, setPct] = useState<number | null>(null);
  const [err, setErr] = useState("");

  useEffect(() => {
    api.updateStatus().then(setInfo).catch(() => {});
    const un = [
      on<UpdateInfo>("update-available", setInfo),
      on<{ done: number; total: number | null }>("update-progress", (p) => setPct(p.total ? Math.round((p.done / p.total) * 100) : null)),
    ];
    return () => un.forEach((f) => f());
  }, []);

  if (!info) return null;
  const busy = pct !== null || err === "…";
  const install = async () => {
    setErr("…");
    setPct(0);
    try { await api.updateInstall(); } catch (e) { setErr(String(e)); setPct(null); }
  };

  return (
    <div className="update-line">
      {busy
        ? <span className="faint">{pct ? `Installing… ${pct}%` : "Installing…"}</span>
        : <button className="link accent" onClick={install} title="Downloads, installs and restarts Seda (about a minute)">↻ Update to {info.version}</button>}
      {err && err !== "…" && <span className="error-text">{err}</span>}
    </div>
  );
}
