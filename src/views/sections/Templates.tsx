import { useState } from "react";
import type { Settings, Template, TemplateSection } from "../../lib/api";
import Icon from "../Icons";
import type { Update } from "./ui";

const MAX_SECTIONS = 8;
const SEPARATORS = [" — ", " – ", " - ", ": "];
const SECTIONS_PLACEHOLDER = "Done — finished since last time, by person\nDoing — in progress now, by person\nBlocked — what is stuck and what it waits on\nNext steps";

/** "Heading — what belongs there", one per line. A leading #, -, * or "1." is dropped; the line is
 *  split at the first " — ", " – ", " - " or ": " into heading and hint. */
export function parseSections(text: string): TemplateSection[] {
  const out: TemplateSection[] = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim().replace(/^(#+|[-*]|\d+[.)])\s*/, "").trim();
    if (!line) continue;
    let cut = -1, len = 0;
    for (const sep of SEPARATORS) {
      const i = line.indexOf(sep);
      if (i > 0 && (cut < 0 || i < cut)) { cut = i; len = sep.length; }
    }
    const heading = (cut < 0 ? line : line.slice(0, cut)).trim();
    const hint = cut < 0 ? "" : line.slice(cut + len).trim();
    if (heading) out.push({ heading, hint });
  }
  return out;
}

const toLines = (t: Template) => t.sections.map((s) => (s.hint ? `${s.heading} — ${s.hint}` : s.heading)).join("\n");
const newId = () => "c_" + [...crypto.getRandomValues(new Uint8Array(4))].map((b) => b.toString(16).padStart(2, "0")).join("");

type Draft = { id: string | null; name: string; about: string; text: string };

/** The user's own note templates (Settings → Meetings). Built-ins are not listed: they can't change. */
export default function Templates({ settings, update }: { settings: Settings; update: Update }) {
  const list = settings.templates ?? [];
  const [draft, setDraft] = useState<Draft | null>(null);
  const start = (t?: Template) => setDraft(t ? { id: t.id, name: t.name, about: t.about, text: toLines(t) } : { id: null, name: "", about: "", text: "" });
  const sections = draft ? parseSections(draft.text) : [];
  const canSave = !!draft && !!draft.name.trim() && sections.length > 0;
  const save = () => {
    if (!draft || !canSave) return;
    const t: Template = { id: draft.id ?? newId(), name: draft.name.trim().slice(0, 40), about: draft.about.trim().slice(0, 300), sections: sections.slice(0, MAX_SECTIONS) };
    update("templates", draft.id ? list.map((x) => (x.id === draft.id ? t : x)) : [...list, t]);
    setDraft(null);
  };
  const remove = (id: string) => {
    update("templates", list.filter((x) => x.id !== id));
    if (draft?.id === id) setDraft(null);
  };
  const set = (k: keyof Draft) => (e: { target: { value: string } }) => setDraft((d) => d && { ...d, [k]: e.target.value });

  return (
    <div className="tpl-editor">
      {list.length === 0 && !draft && <div className="faint tpl-empty">No templates of your own yet.</div>}
      {list.map((t) => (
        <div key={t.id} className={`tpl-row ${draft?.id === t.id ? "on" : ""}`}>
          <div className="tpl-row-text">
            <span className="tpl-row-name" dir="auto">{t.name}</span>
            <span className="tpl-row-secs" dir="auto">{t.sections.map((s) => s.heading).join(" · ")}</span>
          </div>
          <button type="button" className="btn ghost small" onClick={() => start(t)}>Edit</button>
          <button type="button" className="btn ghost small danger" onClick={() => remove(t.id)}>Remove</button>
        </div>
      ))}
      {draft ? (
        <form className="tpl-form" onSubmit={(e) => { e.preventDefault(); save(); }}>
          <label className="tpl-field">
            <span className="label">Name</span>
            <input className="field" dir="auto" maxLength={40} autoFocus placeholder="e.g. Stand-up" value={draft.name} onChange={set("name")} />
          </label>
          <label className="tpl-field">
            <span className="label">About this kind of meeting <span className="faint">(optional)</span></span>
            <input className="field" dir="auto" maxLength={300} placeholder="A weekly status meeting with the team." value={draft.about} onChange={set("about")} />
          </label>
          <label className="tpl-field">
            <span className="label">Sections, one per line</span>
            <textarea className="field tpl-sections" dir="auto" rows={6} placeholder={SECTIONS_PLACEHOLDER} value={draft.text} onChange={set("text")} />
          </label>
          <p className="tpl-help">Up to 8 sections. After the dash, say what belongs there. Sections named Decisions, Next steps or Open questions are filled from the whole meeting automatically.</p>
          {sections.length > MAX_SECTIONS && <div className="error-text">Only the first {MAX_SECTIONS} sections are kept.</div>}
          <div className="tpl-actions">
            <button type="submit" className="btn primary small" disabled={!canSave}>Save</button>
            <button type="button" className="btn ghost small" onClick={() => setDraft(null)}>Cancel</button>
          </div>
        </form>
      ) : (
        <div><button type="button" className="btn small" onClick={() => start()}><Icon name="plus" size={14} />New template</button></div>
      )}
    </div>
  );
}
