import type { ReactNode } from "react";
import type { Settings } from "../../lib/api";

export type Update = <K extends keyof Settings>(key: K, value: Settings[K]) => void;

export function Head({ n, title, children, right }: { n: string; title: string; children?: ReactNode; right?: ReactNode }) {
  return (
    <div className="section-head">
      <div>
        {n !== title && <div className="label">{n}</div>}
        <h1>{title}</h1>
        {children && <p>{children}</p>}
      </div>
      {right}
    </div>
  );
}

export function Row({ title, help, children, top }: { title: string; help?: ReactNode; children: ReactNode; top?: boolean }) {
  return (
    <div className={`row ${top ? "top" : ""}`}>
      <div>
        <h3>{title}</h3>
        {help && <p className="help">{help}</p>}
      </div>
      <div className="ctl">{children}</div>
    </div>
  );
}

export function ToggleRow({ title, help, on, onChange }: { title: string; help?: ReactNode; on: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="row toggle-row" onClick={() => onChange(!on)} role="switch" aria-checked={on}>
      <div>
        <h3>{title}</h3>
        {help && <p className="help">{help}</p>}
      </div>
      <div className="ctl">
        <span className={`switch ${on ? "on" : ""}`} />
        <span className="label">{on ? "On" : "Off"}</span>
      </div>
    </div>
  );
}

export function Segmented<T extends string>({ value, options, onChange }: { value: T; options: [T, string][]; onChange: (v: T) => void }) {
  return (
    <div className="segmented">
      {options.map(([v, label]) => (
        <button key={v} className={v === value ? "on" : ""} onClick={() => onChange(v)}>
          {label}
        </button>
      ))}
    </div>
  );
}

export function timeAgo(ts: string): string {
  const d = new Date(ts);
  const s = (Date.now() - d.getTime()) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400 && new Date().getDate() === d.getDate()) return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}
