import { useEffect, useState } from "react";
import { api, type UpdateInfo } from "../lib/api";
import { on } from "../lib/events";
import Icon from "./Icons";

/** Sidebar notice when a newer Seda is out: one click downloads, installs and restarts. */
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
    <div className="update-banner">
      <div className="update-text">
        <b>Seda {info.version} is available</b>
        {busy ? <span>{pct ? `Downloading… ${pct}%` : "Preparing…"} Seda restarts by itself.</span> : <span>Takes about a minute.</span>}
        {err && err !== "…" && <span className="error-text">{err}</span>}
      </div>
      <button className="ibtn primary" disabled={busy} onClick={install}>
        <Icon name="rewrite" size={14} />
        <span>{busy ? "Updating" : "Update"}</span>
      </button>
    </div>
  );
}
