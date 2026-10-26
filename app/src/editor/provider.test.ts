import { describe, expect, it, vi } from "vitest";
import * as Y from "yjs";
import { PageSync } from "./provider";

function fakeBackend() {
  const stored = new Y.Doc();
  let listener: ((pageId: string, update: Uint8Array) => void) | null = null;
  let failNext = 0;
  const transport = {
    load: async () => Y.encodeStateAsUpdate(stored),
    send: vi.fn(async (_id: string, update: Uint8Array) => {
      if (failNext > 0) {
        failNext--;
        throw new Error("offline");
      }
      Y.applyUpdate(stored, update);
    }),
    subscribe: async (cb: (pageId: string, update: Uint8Array) => void) => {
      listener = cb;
      return () => (listener = null);
    },
  };
  return {
    stored,
    transport,
    failOnce: () => (failNext = 1),
    push: (pageId: string, update: Uint8Array) => listener?.(pageId, update),
  };
}

describe("PageSync", () => {
  it("loads stored state, batches local edits and sends them", async () => {
    const backend = fakeBackend();
    backend.stored.getText("t").insert(0, "stored ");
    const sync = new PageSync("p1", 10, backend.transport);
    await sync.load();
    expect(sync.doc.getText("t").toString()).toBe("stored ");

    sync.doc.getText("t").insert(7, "a");
    sync.doc.getText("t").insert(8, "b");
    await sync.flush();
    expect(backend.transport.send).toHaveBeenCalledTimes(1);
    expect(backend.stored.getText("t").toString()).toBe("stored ab");
    await sync.destroy();
  });

  it("applies backend changes without echoing them", async () => {
    const backend = fakeBackend();
    const sync = new PageSync("p1", 10, backend.transport);
    await sync.load();
    const other = new Y.Doc();
    Y.applyUpdate(other, Y.encodeStateAsUpdate(backend.stored));
    other.getText("t").insert(0, "from the task view");
    backend.push("p1", Y.encodeStateAsUpdate(other));
    backend.push("other-page", Y.encodeStateAsUpdate(other));
    expect(sync.doc.getText("t").toString()).toBe("from the task view");
    await sync.flush();
    expect(backend.transport.send).not.toHaveBeenCalled();
    await sync.destroy();
  });

  it("retries failed sends without losing edits", async () => {
    vi.useFakeTimers();
    const backend = fakeBackend();
    const sync = new PageSync("p1", 10, backend.transport);
    await sync.load();
    const statuses: string[] = [];
    sync.onStatus = (s) => statuses.push(s);
    backend.failOnce();
    sync.doc.getText("t").insert(0, "x");
    await sync.flush();
    expect(statuses).toContain("error");
    await vi.advanceTimersByTimeAsync(2100);
    expect(backend.stored.getText("t").toString()).toBe("x");
    expect(statuses.at(-1)).toBe("saved");
    vi.useRealTimers();
    await sync.destroy();
  });
});
