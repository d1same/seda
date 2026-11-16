import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { api, type Template } from "../../lib/api";
import { on } from "../../lib/events";
import Icon from "../Icons";

/** Built-in and custom note templates, reloaded when the settings change (custom ones live there). */
export function useTemplates(): Template[] {
  const [list, setList] = useState<Template[]>([]);
  useEffect(() => {
    const load = () => { api.meetingTemplates().then(setList).catch(() => {}); };
    load();
    return on("settings-changed", load);
  }, []);
  return list;
}

/** The template a meeting uses. "" (never chosen) and a removed custom template both read as Auto. */
export function currentTemplate(templates: Template[], id: string | undefined): Template | undefined {
  return templates.find((t) => t.id === id) ?? templates.find((t) => t.id === "auto");
}
export const templateName = (templates: Template[], id: string | undefined) => currentTemplate(templates, id)?.name ?? "Auto";

/** The menu content: one item per template (built-ins, then the user's own), then "Edit templates…".
 *  ↑ ↓ move, Enter picks; the owner's popover closes it on Esc and on an outside click. */
export function TemplateList({ templates, value, onPick, onEdit, className = "", autoFocus = true }: {
  templates: Template[];
  value: string | undefined;
  onPick: (id: string) => void;
  onEdit?: () => void;
  /** Placement: "start" (left-aligned), "up" (rises above its anchor). */
  className?: string;
  autoFocus?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const cur = currentTemplate(templates, value)?.id ?? "auto";
  const items = () => [...(ref.current?.querySelectorAll<HTMLButtonElement>("[role^='menuitem']") ?? [])];
  useEffect(() => {
    if (!autoFocus) return;
    const all = items();
    (all.find((b) => b.getAttribute("aria-checked") === "true") ?? all[0])?.focus();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const key = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
    e.preventDefault();
    const all = items();
    if (!all.length) return;
    const i = all.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.key === "Home" ? 0 : e.key === "End" ? all.length - 1
      : e.key === "ArrowDown" ? (i < 0 ? 0 : (i + 1) % all.length)
      : (i < 0 ? all.length - 1 : (i - 1 + all.length) % all.length);
    all[next].focus();
  };
  const item = (t: Template) => (
    <button key={t.id} type="button" role="menuitemradio" aria-checked={t.id === cur}
      title={t.sections.map((s) => s.heading).join(" · ") || "Seda picks the sections from what was said"}
      onClick={() => onPick(t.id)}>
      <span className="tpl-name" dir="auto">{t.name}</span>
      {t.id === cur && <span className="tick"><Icon name="check" size={14} /></span>}
    </button>
  );
  const builtins = templates.filter((t) => t.builtin);
  const custom = templates.filter((t) => !t.builtin);
  return (
    <div ref={ref} className={`menu tpl-menu ${className}`} role="menu" aria-label="Template" onKeyDown={key}>
      <span className="kicker" aria-hidden>Template</span>
      {builtins.map(item)}
      {custom.length > 0 && <><hr />{custom.map(item)}</>}
      {onEdit && (
        <>
          <hr />
          <button type="button" role="menuitem" onClick={onEdit}><Icon name="edit" size={15} />Edit templates…</button>
        </>
      )}
    </div>
  );
}
