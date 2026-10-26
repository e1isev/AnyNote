import { useEffect, useState } from "react";

/** Track a CSS media query. */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => typeof window !== "undefined" && window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const update = () => setMatches(mq.matches);
    update();
    mq.addEventListener?.("change", update);
    return () => mq.removeEventListener?.("change", update);
  }, [query]);
  return matches;
}

/** Places where a sideways finger movement means something else. */
const IGNORE =
  ".ink-surface, .ink-scroll, .ink-toolbar, .canvas-viewport, .canvas-tools, .tableWrapper, pre, input, textarea, select, .toolbar, .suggestions, .palette, .modal";

/**
 * On a canvas a finger draws or moves the page, so a drawer swipe there has
 * to start this close (px) to the left edge of the screen. The canvas leaves
 * touches that start here alone.
 */
export const SWIPE_EDGE = 24;

/**
 * A long, deliberate sideways swipe: at least 30% of the screen width
 * (minimum 120 px), mostly horizontal, within a second. Ordinary scrolling
 * and short drags never trigger it.
 */
export function isLongSwipe(dx: number, dy: number, ms: number, width: number): boolean {
  return Math.abs(dx) >= Math.max(120, width * 0.3) && Math.abs(dy) <= Math.abs(dx) * 0.5 && ms <= 1000;
}

/**
 * Call `onRight`/`onLeft` for long one-finger swipes anywhere in the window.
 * Touch events are used (passively) because they keep arriving while the
 * browser scrolls, unlike pointer events.
 */
export function useLongSwipe(onRight: () => void, onLeft: () => void) {
  useEffect(() => {
    let start: { x: number; y: number; t: number } | null = null;
    let last = { x: 0, y: 0 };
    const down = (e: TouchEvent) => {
      const target = e.target as Element | null;
      const t = e.touches[0];
      const fromEdge = t?.clientX <= SWIPE_EDGE && !!target?.closest?.(".canvas-viewport");
      if (e.touches.length !== 1 || (!fromEdge && target?.closest?.(IGNORE))) {
        start = null;
        return;
      }
      start = { x: t.clientX, y: t.clientY, t: Date.now() };
      last = { x: t.clientX, y: t.clientY };
    };
    const move = (e: TouchEvent) => {
      if (e.touches.length !== 1) start = null;
      else last = { x: e.touches[0].clientX, y: e.touches[0].clientY };
    };
    const up = () => {
      if (!start) return;
      const dx = last.x - start.x;
      const dy = last.y - start.y;
      if (isLongSwipe(dx, dy, Date.now() - start.t, window.innerWidth)) {
        if (dx > 0) onRight();
        else onLeft();
      }
      start = null;
    };
    window.addEventListener("touchstart", down, { passive: true });
    window.addEventListener("touchmove", move, { passive: true });
    window.addEventListener("touchend", up, { passive: true });
    window.addEventListener("touchcancel", up, { passive: true });
    return () => {
      window.removeEventListener("touchstart", down);
      window.removeEventListener("touchmove", move);
      window.removeEventListener("touchend", up);
      window.removeEventListener("touchcancel", up);
    };
  }, [onRight, onLeft]);
}
