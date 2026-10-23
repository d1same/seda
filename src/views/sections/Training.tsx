import { useCallback, useState } from "react";
import { api, type Settings, type TrainingStats } from "../../lib/api";
import { useOnHistory } from "../../lib/hooks";
import type { SectionId } from "../Main";
import { Head, ToggleRow, type Update } from "./ui";

const GOAL_MIN = 60;

export default function Training({ settings, update, go, embedded }: { settings: Settings; update: Update; go: (s: SectionId) => void; embedded?: boolean }) {
  const [stats, setStats] = useState<TrainingStats | null>(null);
  const load = useCallback(() => { api.trainingStats().then(setStats); }, []);
  useOnHistory(load);
  const minutes = (stats?.seconds ?? 0) / 60;
  const pct = Math.min(100, (minutes / GOAL_MIN) * 100);

  return (
    <>
      {embedded ? (
        <p className="muted" style={{ margin: "8px 0 0", fontSize: 13 }}>
          Each dictation you correct in History teaches Seda your accent. At about an hour, it can train on your voice overnight.
        </p>
      ) : (
        <Head n="Voice training" title="Make it learn your accent">
          Every dictation you correct becomes a lesson: your voice plus the right text. With about an hour of these, Seda can
          fine-tune the speech model on your own voice, overnight on this PC.
        </Head>
      )}

      <div className="gauge">
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 10 }}>
          <div className="label">Collected</div>
          <div className="mono" style={{ fontSize: 22 }}>
            {minutes.toFixed(1)} <span className="faint" style={{ fontSize: 13 }}>/ {GOAL_MIN} min · {stats?.samples ?? 0} clips</span>
          </div>
        </div>
        <div className="gauge-track"><div className="gauge-fill" style={{ width: `${pct}%` }} /></div>
        <div className="gauge-scale"><span>0</span><span>15</span><span>30</span><span>45</span><span>60 min</span></div>
      </div>

      <ol className="steps">
        <li><span><b>Dictate as usual.</b> Seda keeps the audio of each dictation on this PC.</span></li>
        <li><span><b>Correct mistakes in History.</b> Only corrected clips are used, so the model learns from your fixes. <button className="btn ghost small" onClick={() => go("history")}>Open History →</button></span></li>
        <li><span><b>Train overnight.</b> When the bar is full, start training before bed. It runs locally and never uploads your voice. (Available once you reach the goal.)</span></li>
      </ol>

      <div className="group-label label">Recording storage</div>
      <ToggleRow
        title="Keep audio of my dictations"
        help="Needed for voice training. Stored only in your Seda data folder; deleting a history entry deletes its audio."
        on={settings.save_audio}
        onChange={(v) => update("save_audio", v)}
      />
    </>
  );
}
