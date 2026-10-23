import { listen } from "@tauri-apps/api/event";

/** True inside the Seda app; false when the UI is opened in a normal browser (design preview). */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Cb = (payload: any) => void;
const local = new Map<string, Set<Cb>>();

/** Subscribe to a backend event. Returns an unsubscribe function. */
export function on<T>(name: string, cb: (payload: T) => void): () => void {
  if (inTauri) {
    const p = listen<T>(name, (e) => cb(e.payload));
    return () => { p.then((f) => f()); };
  }
  const set = local.get(name) ?? new Set();
  set.add(cb);
  local.set(name, set);
  return () => set.delete(cb);
}

/** Preview mode only: fake a backend event. */
export function emitLocal(name: string, payload: unknown) {
  local.get(name)?.forEach((cb) => cb(payload));
}
