import * as Y from "yjs";
import { api, onPageUpdate } from "../api";

/** Transaction origin for changes that came from the Rust store. */
export const BACKEND_ORIGIN = "anynote-backend";

/**
 * Keeps one page's Y.Doc in sync with the Rust store.
 *
 * Local edits are batched for `delay` ms, merged into one update and sent as
 * raw bytes. Changes made elsewhere (the task view, version restore) arrive as
 * `page-update` events and are applied with {@link BACKEND_ORIGIN} so they are
 * not echoed back. Failed sends are retried: Yjs updates are idempotent and
 * order-independent, so resending is always safe.
 */
export class PageSync {
  readonly doc = new Y.Doc();
  onStatus?: (status: "saved" | "saving" | "error") => void;

  private pending: Uint8Array[] = [];
  private timer: ReturnType<typeof setTimeout> | undefined;
  private chain: Promise<void> = Promise.resolve();
  private unlisten?: () => void;
  private destroyed = false;

  constructor(
    readonly pageId: string,
    private delay = 300,
    private transport: {
      load: (id: string) => Promise<Uint8Array>;
      send: (id: string, update: Uint8Array) => Promise<void>;
      subscribe: (cb: (pageId: string, update: Uint8Array) => void) => Promise<() => void>;
    } = defaultTransport,
  ) {
    this.doc.on("update", this.handleUpdate);
  }

  /** Load the stored state. Call before binding an editor to {@link doc}. */
  async load(): Promise<void> {
    const unlisten = await this.transport.subscribe((pageId, update) => {
      if (pageId === this.pageId && !this.destroyed) Y.applyUpdate(this.doc, update, BACKEND_ORIGIN);
    });
    if (this.destroyed) {
      unlisten();
      return;
    }
    this.unlisten = unlisten;
    const state = await this.transport.load(this.pageId);
    Y.applyUpdate(this.doc, state, BACKEND_ORIGIN);
  }

  private handleUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin === BACKEND_ORIGIN) return;
    this.pending.push(update);
    this.onStatus?.("saving");
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.flush(), this.delay);
  };

  /** Send everything queued so far; resolves once the store has it. */
  flush(): Promise<void> {
    clearTimeout(this.timer);
    if (this.pending.length === 0) return this.chain;
    const update = this.pending.length === 1 ? this.pending[0] : Y.mergeUpdates(this.pending);
    this.pending = [];
    this.chain = this.chain.then(async () => {
      try {
        await this.transport.send(this.pageId, update);
        if (this.pending.length === 0) this.onStatus?.("saved");
      } catch (e) {
        console.error("saving page failed, will retry", e);
        this.onStatus?.("error");
        this.pending.unshift(update);
        if (!this.destroyed) this.timer = setTimeout(() => void this.flush(), 2000);
      }
    });
    return this.chain;
  }

  async destroy(): Promise<void> {
    if (this.destroyed) return;
    await this.flush();
    this.destroyed = true;
    this.doc.off("update", this.handleUpdate);
    this.unlisten?.();
    this.doc.destroy();
  }
}

const defaultTransport = {
  load: (id: string) => api.loadPageState(id),
  send: (id: string, update: Uint8Array) => api.applyPageUpdate(id, update),
  subscribe: (cb: (pageId: string, update: Uint8Array) => void) =>
    onPageUpdate((e) => cb(e.pageId, Uint8Array.from(e.update))),
};
