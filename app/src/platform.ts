// The UI runs in two hosts:
// * the Tauri desktop app (commands over Tauri IPC, native dialogs);
// * a browser talking to `anynote-server` (commands over HTTP, for machines
//   whose system WebKit is too old for Tauri, such as Raspbian 11).
// Everything host-specific lives here.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { promptText } from "./ui/overlay";
import { demoAssetUrl, demoCall, demoSubscribe } from "./demo/backend";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
/** The online demo: everything runs in this browser (built with VITE_DEMO=1). */
export const isDemo = import.meta.env.VITE_DEMO === "1";

/** Call a backend command. A `Uint8Array` body is sent as raw bytes. */
export async function call<T>(
  cmd: string,
  args?: Record<string, unknown> | Uint8Array,
  options?: { headers?: Record<string, string> },
): Promise<T> {
  if (isDemo) return demoCall<T>(cmd, args, options);
  if (isTauri) return invoke<T>(cmd, args as never, options?.headers ? { headers: options.headers } : undefined);
  const binary = args instanceof Uint8Array;
  const res = await fetch(`/api/${cmd}`, {
    method: "POST",
    headers: {
      "x-anynote": "1",
      "content-type": binary ? "application/octet-stream" : "application/json",
      ...options?.headers,
    },
    body: binary ? (args as BodyInit) : JSON.stringify(args ?? {}),
  });
  if (!res.ok) throw new Error((await res.text()) || `${cmd} failed (${res.status})`);
  const type = res.headers.get("content-type") ?? "";
  if (type.startsWith("application/octet-stream")) return (await res.arrayBuffer()) as T;
  if (type.startsWith("application/json")) return (await res.json()) as T;
  return undefined as T;
}

export interface PageUpdateEvent {
  pageId: string;
  update: number[];
}

type Listener = (e: PageUpdateEvent) => void;
const webListeners = new Set<Listener>();
let polling = false;

/** Long-poll the server for page updates made elsewhere. */
async function pollLoop() {
  let since = -1;
  let delay = 500;
  while (webListeners.size > 0) {
    try {
      const res = await fetch(`/api/events?since=${since}`, { headers: { "x-anynote": "1" } });
      if (!res.ok) throw new Error(String(res.status));
      const body = (await res.json()) as { next: number; events: PageUpdateEvent[] };
      if (since >= 0) for (const e of body.events) webListeners.forEach((l) => l(e));
      since = body.next;
      delay = 500;
    } catch {
      await new Promise((r) => setTimeout(r, delay));
      delay = Math.min(delay * 2, 10_000);
    }
  }
  polling = false;
}

/** Subscribe to `page-update` events; resolves to an unsubscribe function. */
export async function onPageUpdate(cb: Listener): Promise<() => void> {
  if (isDemo) return demoSubscribe(cb);
  if (isTauri) return listen<PageUpdateEvent>("page-update", (event) => cb(event.payload));
  webListeners.add(cb);
  if (!polling) {
    polling = true;
    void pollLoop();
  }
  return () => webListeners.delete(cb);
}

/** URL the webview can load for an `anynote://asset/<hash>` reference. */
export function assetUrl(hash: string): string {
  if (isDemo) return demoAssetUrl(hash);
  if (!isTauri) return `/asset/${hash}`;
  const windows = /windows/i.test(navigator.userAgent);
  return windows ? `http://anynote.localhost/asset/${hash}` : `anynote://localhost/asset/${hash}`;
}

function promptPath(title: string, label: string, placeholder: string) {
  return promptText({ title, label, placeholder, confirmText: "Continue" }).then((p) => p?.trim() || null);
}

/** Choose something to import: a OneNote file or folder, or a Markdown folder. */
export async function pickImportPath(kind: "onenote-file" | "onenote-folder" | "markdown"): Promise<string | null> {
  if (isTauri) {
    const path =
      kind === "onenote-file"
        ? await open({ multiple: false, filters: [{ name: "OneNote", extensions: ["one", "onetoc2"] }] })
        : await open({ multiple: false, directory: true });
    return typeof path === "string" ? path : null;
  }
  return kind === "markdown"
    ? promptPath("Import a folder", "Full path of a Markdown or AnyNote export folder on this computer", "/home/pi/notes")
    : promptPath(
        "Import from OneNote",
        "Full path of a .one file, a .onetoc2 file or a notebook folder on this computer",
        "/media/pi/USB/My Notebook",
      );
}

/** Choose a folder to export into. */
export async function pickExportFolder(title: string): Promise<string | null> {
  if (isTauri) {
    const path = await open({ multiple: false, directory: true, title });
    return typeof path === "string" ? path : null;
  }
  return promptPath(title, "Full path of a folder on this computer (it will be created if needed)", "/home/pi/AnyNote export");
}

/** Open a web link outside the app. */
export function openExternal(url: string): Promise<void> {
  if (isTauri) return openUrl(url);
  window.open(url, "_blank", "noopener");
  return Promise.resolve();
}

/** Open an attachment with the system's default app (or a browser tab). */
export async function openAttachment(hash: string, name: string): Promise<void> {
  if (isTauri) return invoke("open_asset", { hash, name });
  window.open(assetUrl(hash), "_blank", "noopener");
}

/** Save a copy of an attachment. */
export async function saveAttachment(hash: string, name: string): Promise<void> {
  if (isTauri) {
    const path = await save({ defaultPath: name });
    if (path) await invoke("save_asset", { hash, path });
    return;
  }
  const a = document.createElement("a");
  a.href = assetUrl(hash);
  a.download = name || "attachment";
  document.body.appendChild(a);
  a.click();
  a.remove();
}

export { uuid } from "./uuid";
