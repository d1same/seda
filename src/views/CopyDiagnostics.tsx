import { useState } from "react";
import { api } from "../lib/api";

/** Copies hardware, model status, settings and recent log lines, for pasting into a support chat. */
export default function CopyDiagnostics({ small }: { small?: boolean }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    const text = (await api.diagnostics()) ?? "diagnostics unavailable in preview";
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2500);
  };
  return (
    <>
      <button className={`btn ${small ? "small" : ""}`} onClick={copy}>
        {copied ? "✓ Copied — paste it in your message" : "Copy diagnostics"}
      </button>
      <button className={`btn ghost ${small ? "small" : ""}`} onClick={() => api.openLog()}>Open log</button>
    </>
  );
}
