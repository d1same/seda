import { useEffect, useState } from "react";
import { api, type CallApp, type Settings as S, type Status } from "../../lib/api";
import ShortcutPicker from "../ShortcutPicker";
import CopyDiagnostics from "../CopyDiagnostics";
import MicPicker from "../MicPicker";
import { LANGUAGES } from "../../lib/languages";
import type { SectionId } from "../Main";
import Engine from "./Engine";
import Training from "./Training";
import { Head, Row, Segmented, ToggleRow, type Update } from "./ui";

const MODE_HELP: Record<S["dictate_mode"], string> = {
  both: "Hold while you talk, or tap once to talk hands-free and tap again to finish.",
  hold: "Talk while holding. Let go to paste.",
  toggle: "Tap to start. Tap again to finish and paste.",
};

/** Which apps count as calls: the ones found on this PC first. */
function CallApps({ settings, update }: { settings: S; update: Update }) {
  const [apps, setApps] = useState<CallApp[]>([]);
  useEffect(() => { api.callApps().then((a) => setApps([...a].sort((x, y) => Number(y.installed) - Number(x.installed)))); }, []);
  const on = new Set(settings.meeting_apps);
  const toggle = (id: string) => update("meeting_apps", on.has(id) ? settings.meeting_apps.filter((a) => a !== id) : [...settings.meeting_apps, id]);
  return (
    <div className="app-toggles">
      {apps.map((a) => (
        <button key={a.id} className={`app-toggle ${on.has(a.id) ? "on" : ""} ${a.installed ? "" : "missing"}`} onClick={() => toggle(a.id)} role="switch" aria-checked={on.has(a.id)}>
          <span className="tick" />
          {a.name}
          {!a.installed && <span className="faint"> · not found</span>}
        </button>
      ))}
    </div>
  );
}

/** Current version, and a manual "Check for updates" (Seda also checks by itself every few hours). */
function VersionRow() {
  const [version, setVersion] = useState("");
  const [state, setState] = useState("");
  useEffect(() => { api.appVersion().then(setVersion).catch(() => {}); }, []);
  const check = async () => {
    setState("Checking…");
    try {
      const u = await api.updateCheck();
      setState(u ? `Version ${u.version} is available — use the Update button in the sidebar.` : "You have the latest version.");
    } catch (e) { setState(`Couldn't check: ${e}`); }
  };
  return (
    <Row title="Version" help={state || "Seda checks for updates by itself every few hours. Updates are signed and verified before installing."}>
      <span className="mono">{version}</span>
      <button className="btn small" onClick={check}>Check for updates</button>
    </Row>
  );
}

/** Saved when you leave the field (not on every key). */
export function NameField({ settings, update, autoFocus }: { settings: S; update: Update; autoFocus?: boolean }) {
  const [v, setV] = useState(settings.user_name);
  const save = () => { if (v.trim() !== settings.user_name) update("user_name", v.trim()); };
  return <input className="field" style={{ width: 240 }} dir="auto" placeholder="e.g. Alex" value={v} autoFocus={autoFocus} onChange={(e) => setV(e.target.value)} onBlur={save} onKeyDown={(e) => e.key === "Enter" && save()} />;
}

const DETECT_HELP: Record<S["meeting_detect"], string> = {
  ask: "When a call starts, a small card asks whether to take notes.",
  auto: "Every call is recorded and summarised automatically. Stops when the call ends.",
  off: "Start meetings yourself from the Meetings page.",
};

export default function Settings({ settings, update, status, go }: { settings: S; update: Update; status: Status; go: (s: SectionId) => void }) {
  const [advanced, setAdvanced] = useState(false);

  return (
    <>
      <Head n="Settings" title="Settings" />

      <div className="group-label label">Dictation</div>

      <Row title="Dictation shortcut" help="Any mouse button or key. Mouse 4 is the side “back” button.">
        <ShortcutPicker which="dictate" settings={settings} update={update} big />
      </Row>
      <Row title="How it works" help={MODE_HELP[settings.dictate_mode]}>
        <Segmented
          value={settings.dictate_mode}
          options={[["both", "Hold or tap"], ["hold", "Hold only"], ["toggle", "Tap to start / stop"]]}
          onChange={(v) => update("dictate_mode", v)}
        />
      </Row>
      <Row title="Microphone" help="Which mic Seda listens to.">
        <MicPicker settings={settings} update={update} />
      </Row>
      <Row title="Extra language" help="English is always understood. Add one more language you speak, or None. Switching may download a different speech model.">
        <select className="field" style={{ width: 240 }} value={settings.language} onChange={(e) => update("language", e.target.value)}>
          {LANGUAGES.map((l) => (
            <option key={l.code} value={l.code}>{l.native} — {l.name}</option>
          ))}
        </select>
      </Row>
      <Row
        title="Writing AI"
        help={
          !settings.cleanup ? "Off: text is pasted exactly as heard. Fastest."
          : settings.polish === "always" ? "Every dictation is polished. Best formatting, adds 1–7 s."
          : "Only when you say “no wait”, “um”, “I mean”… or write a long email. Everything else pastes instantly."
        }
      >
        <Segmented
          value={!settings.cleanup ? "off" : settings.polish === "always" ? "always" : "smart"}
          options={[["smart", "When needed"], ["always", "Always"], ["off", "Off"]]}
          onChange={(v) => {
            if (v === "off") update("cleanup", false);
            else { if (!settings.cleanup) update("cleanup", true); update("polish", v); }
          }}
        />
      </Row>
      <div className="group-label label">Meetings</div>
      <Row title="When a call starts" help={DETECT_HELP[settings.meeting_detect]}>
        <Segmented
          value={settings.meeting_detect}
          options={[["ask", "Ask me"], ["auto", "Record automatically"], ["off", "Off"]]}
          onChange={(v) => update("meeting_detect", v)}
        />
      </Row>
      {settings.meeting_detect !== "off" && (
        <Row title="Call apps" help="Seda notices when one of these starts using your microphone." top>
          <CallApps settings={settings} update={update} />
        </Row>
      )}

      <ToggleRow
        title="Keep meeting recordings"
        help="Off: like Granola, the audio is transcribed as you talk and never saved; only the transcript and notes stay. On: also saves the audio on this PC."
        on={settings.keep_meeting_audio}
        onChange={(v) => update("keep_meeting_audio", v)}
      />

      <div className="group-label label">General</div>
      <Row title="Your name" help="Seda greets you, labels you in transcripts, and knows “Alex, can you…” in a meeting is a task for you.">
        <NameField settings={settings} update={update} />
      </Row>
      <Row title="When not recording" help="What you see on screen while Seda waits.">
        <Segmented value={settings.idle_dot ? "dot" : "none"} options={[["none", "Nothing"], ["dot", "Small dot"]]} onChange={(v) => update("idle_dot", v === "dot")} />
      </Row>
      <Row title="Fix selected text" help="Select text anywhere and press this to correct or rewrite it.">
        <ShortcutPicker which="fix" settings={settings} update={update} />
      </Row>
      <ToggleRow title="Sounds" on={settings.sounds} onChange={(v) => update("sounds", v)} />
      <VersionRow />
      <ToggleRow title="Start with Windows" help="Opens quietly in the tray so your shortcut always works." on={settings.autostart} onChange={(v) => update("autostart", v)} />

      <button className="advanced-toggle" onClick={() => setAdvanced(!advanced)} aria-expanded={advanced}>
        <span className="label">Advanced</span>
        <span className="faint">{advanced ? "Hide" : "Show"}</span>
      </button>

      {advanced && (
        <div className="advanced">
          <ToggleRow title="Match the app" help="Email layout in Outlook/Gmail, bullets in Jira, casual in Teams/Slack." on={settings.app_styles} onChange={(v) => update("app_styles", v)} />
          <ToggleRow title="English words in English letters" help="Keeps words like “deploy” in English letters instead of your language's script." on={settings.latin_tech_terms} onChange={(v) => update("latin_tech_terms", v)} />
          <div className="group-label label">Voice training</div>
          <Training settings={settings} update={update} go={go} embedded />
          <div className="group-label label">Engine</div>
          <Engine settings={settings} update={update} status={status} embedded />
          <Row title="Setup" help="Download models again, switch package, or re-test your microphone.">
            <button className="btn" onClick={() => update("onboarded", false)}>Run setup again</button>
          </Row>
          <Row title="Something not working?" help="Copies your hardware, model status and recent activity. Paste it to whoever is helping you.">
            <CopyDiagnostics />
          </Row>
        </div>
      )}
    </>
  );
}
