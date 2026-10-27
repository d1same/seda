import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { emitLocal, inTauri, on } from "../lib/events";
import Logo from "./Logo";
import "./call.css";

type Card = { state: "detected" | "starting" | "recording" | "ended" | "hidden"; app: string };

/** How long each card stays before it goes away by itself (a detected call counts as "Not now"). */
const LINGER: Record<string, number> = { detected: 45_000, recording: 5_000, ended: 8_000 };

/** The card in the bottom-right corner when a call starts or ends. */
export default function Call() {
  const [card, setCard] = useState<Card>({ state: "hidden", app: "" });
  const [err, setErr] = useState("");

  useEffect(() => {
    const un = on<Card>("call", setCard);
    if (!inTauri) setTimeout(() => emitLocal("call", { state: "detected", app: "Microsoft Teams" }), 300);
    return un;
  }, []);
  useEffect(() => {
    const ms = LINGER[card.state];
    if (!ms) return;
    const t = setTimeout(() => (card.state === "detected" ? api.callDismiss() : api.callHide()), ms);
    return () => clearTimeout(t);
  }, [card]);

  if (card.state === "hidden") return <div className="call-card" />;
  const text = {
    detected: [`${card.app} call`, "Take notes with Seda?"],
    starting: [`Starting notes · ${card.app}`, "Opening the microphone…"],
    recording: [`Taking notes · ${card.app}`, "Stops by itself when the call ends"],
    ended: ["Call ended", "Writing your notes…"],
  }[card.state];

  const startNotes = () => {
    const app = card.app;
    // The mic takes a second or two. Change the card on this click so it doesn't feel missed.
    setErr("");
    setCard({ state: "starting", app });
    api.callStart().then(
      () => setCard((c) => (c.state === "starting" ? { state: "recording", app } : c)),
      (e) => {
        setErr(e instanceof Error ? e.message : String(e));
        setCard((c) => (c.state === "starting" ? { state: "detected", app } : c));
      },
    );
  };

  return (
    <div className={`call-card ${card.state}`}>
      <Logo size={34} pulse={card.state === "recording" || card.state === "starting"} outline={false} />
      <div className="call-text">
        <b>{text[0]}</b>
        <span>{card.state === "detected" && err ? err : text[1]}</span>
      </div>
      <div className="call-acts">
        {card.state === "detected" ? (
          <>
            <button className="primary" onClick={startNotes}>Start notes</button>
            <button onClick={() => api.callDismiss()}>Not now</button>
          </>
        ) : card.state === "starting" ? (
          <>
            <button className="primary" disabled aria-busy="true">Starting…</button>
            <button disabled aria-label="Close">✕</button>
          </>
        ) : (
          <>
            <button className="primary" onClick={() => api.callOpen()}>{card.state === "ended" ? "Open notes" : "Open"}</button>
            <button onClick={() => api.callHide()} aria-label="Close">✕</button>
          </>
        )}
      </div>
    </div>
  );
}
