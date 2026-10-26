import { useSyncExternalStore } from "react";
import { api } from "./api";

// Live page titles for wiki links, so a link shows the page's current name
// even after it is renamed. `null` means the page no longer exists.
const titles = new Map<string, string | null>();
const listeners = new Set<() => void>();
const inflight = new Set<string>();

function emit() {
  listeners.forEach((l) => l());
}

export function setPageTitle(id: string, title: string | null) {
  if (titles.get(id) === title) return;
  titles.set(id, title);
  emit();
}

export function forgetPageTitle(id: string) {
  titles.delete(id);
  emit();
}

function fetchTitle(id: string) {
  if (inflight.has(id)) return;
  inflight.add(id);
  api
    .getPage(id)
    .then(([page]) => setPageTitle(id, page.title))
    .catch(() => setPageTitle(id, null))
    .finally(() => inflight.delete(id));
}

/** Current title of a page; `undefined` while loading, `null` if missing. */
export function usePageTitle(id: string | null): string | null | undefined {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => {
      if (!id) return null;
      if (!titles.has(id)) queueMicrotask(() => fetchTitle(id));
      return titles.get(id);
    },
  );
}
