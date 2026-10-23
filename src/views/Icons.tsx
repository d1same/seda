/** Small line icons for the sidebar (16px, drawn on a 16 grid, stroke = currentColor). */
const P: Record<string, string> = {
  home: "M2.5 7.2 8 2.8l5.5 4.4V13a.5.5 0 0 1-.5.5H9.5V10h-3v3.5H3a.5.5 0 0 1-.5-.5z",
  meetings: "M2.5 4.5h11v8a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1zM2.5 7h11M5.5 2.5v3M10.5 2.5v3",
  history: "M8 2.5a5.5 5.5 0 1 1-5.2 3.7M2.5 2.8v3.4h3.4M8 5v3.2l2.2 1.4",
  words: "M3 3.5h4a1 1 0 0 1 1 1v9a1.5 1.5 0 0 0-1.5-1.5H3zM13 3.5H9a1 1 0 0 0-1 1v9a1.5 1.5 0 0 1 1.5-1.5H13z",
  settings: "M8 5.6a2.4 2.4 0 1 1 0 4.8 2.4 2.4 0 0 1 0-4.8zM8 1.8v1.6M8 12.6v1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M1.8 8h1.6M12.6 8h1.6M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1",
  send: "M2.5 8h9M8 4l4 4-4 4",
  edit: "M3 13l.6-2.6L10.5 3.5l2 2-6.9 6.9zM9.3 4.7l2 2",
  spark: "M8 2v3M8 11v3M2 8h3M11 8h3M4 4l1.8 1.8M10.2 10.2 12 12M4 12l1.8-1.8M10.2 5.8 12 4",
  back: "M9.5 3.5 5 8l4.5 4.5",
  copy: "M5.5 5.5h7v7h-7zM3.5 10.5v-7h7",
  trash: "M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5",
  check: "M3.5 8.5 6.5 11.5 12.5 4.5",
  rewrite: "M12.5 6.5A4.8 4.8 0 0 0 3.6 6M3.5 9.5a4.8 4.8 0 0 0 8.9.5M12.8 3.2v3.3H9.5M3.2 12.8V9.5h3.3",
  app: "M2.5 3.5h11v9h-11zM2.5 6h11",
  clock: "M8 2.5a5.5 5.5 0 1 1 0 11 5.5 5.5 0 0 1 0-11zM8 5v3.2l2 1.3",
  ear: "M5 6a3 3 0 1 1 6 0c0 2-2 2.5-2 4.5a1.8 1.8 0 0 1-3.4.8",
  mic: "M8 2.5a2 2 0 0 1 2 2v3.5a2 2 0 0 1-4 0V4.5a2 2 0 0 1 2-2zM4.5 7.5a3.5 3.5 0 0 0 7 0M8 11v2.5",
  wand: "M3 13 10 6M9 3.5v1.5M12.5 7H11M11.5 4.5l-1 1M6 3.5l.5 1",
  plus: "M8 3.5v9M3.5 8h9",
  record: "M8 3a5 5 0 1 1 0 10A5 5 0 0 1 8 3zM8 6a2 2 0 1 1 0 4 2 2 0 0 1 0-4z",
};

/** A button with an icon and a short label (label optional: then it shows as a tooltip). */
export function IconButton({ icon, label, onClick, primary, danger, iconOnly, disabled }: { icon: string; label: string; onClick: () => void; primary?: boolean; danger?: boolean; iconOnly?: boolean; disabled?: boolean }) {
  return (
    <button className={`ibtn ${primary ? "primary" : ""} ${danger ? "danger" : ""} ${iconOnly ? "round" : ""}`} onClick={onClick} title={label} aria-label={label} disabled={disabled}>
      <Icon name={icon} size={15} />
      {!iconOnly && <span>{label}</span>}
    </button>
  );
}

export default function Icon({ name, size = 16 }: { name: keyof typeof P | string; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d={P[name] ?? ""} />
    </svg>
  );
}
