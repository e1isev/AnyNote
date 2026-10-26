import { useSyncExternalStore } from "react";

// Per-device preferences, kept in localStorage.

/** What one finger does on a drawing. The pen always draws, and two fingers always scroll. */
export type FingerMode = "auto" | "draw" | "scroll";

interface Prefs {
  /** "auto": the finger draws until a pen has been used on this device, then it scrolls. */
  fingerMode: FingerMode;
  /** A pen (stylus reported as `pointerType: "pen"`) has been used here. */
  penSeen: boolean;
  /** New pages are open canvases (write and type anywhere) or documents. */
  newPageLayout: "canvas" | "document";
}

const DEFAULTS: Prefs = { fingerMode: "auto", penSeen: false, newPageLayout: "canvas" };

function load(): Prefs {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem("prefs") ?? "{}") };
  } catch {
    return { ...DEFAULTS };
  }
}

let prefs = load();
const listeners = new Set<() => void>();

export function getPrefs(): Prefs {
  return prefs;
}

export function setPrefs(changes: Partial<Prefs>) {
  prefs = { ...prefs, ...changes };
  try {
    localStorage.setItem("prefs", JSON.stringify(prefs));
  } catch {
    /* storage unavailable: keep for this session */
  }
  listeners.forEach((l) => l());
}

export function usePrefs(): Prefs {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => prefs,
  );
}

/** Does a single finger draw (rather than scroll) on drawings right now? */
export function fingerDraws(p: Prefs = prefs): boolean {
  return p.fingerMode === "draw" || (p.fingerMode === "auto" && !p.penSeen);
}

/** Is this a touch screen? */
export const hasTouch = typeof navigator !== "undefined" && navigator.maxTouchPoints > 0;

/** Height for a new drawing area: most of the visible page. */
export function defaultDrawingHeight(): number {
  const h = typeof window === "undefined" ? 800 : window.innerHeight;
  return Math.round(Math.max(360, Math.min(h - 180, 1400)));
}
