// A small in-browser backend for the online demo: the same commands as the
// Rust store (crates/anynote-core), kept in localStorage. It exists so the
// app can be tried on a phone without a computer running AnyNote; it does
// no import or export.
import * as Y from "yjs";
import { yXmlFragmentToProsemirrorJSON } from "@tiptap/y-tiptap";
import type { PMNode } from "../types";
import { uuid } from "../uuid";
import welcome from "./welcome.json";

interface Row {
  id: string;
  title: string;
  color: string | null;
  position: number;
  createdAt: number;
  updatedAt: number;
  deletedAt: number | null;
}
interface SectionRow extends Row {
  notebookId: string;
  parentId: string | null;
}
interface PageRow extends Row {
  sectionId: string;
  parentId: string | null;
}
interface VersionRow {
  id: number;
  pageId: string;
  createdAt: number;
  label: string | null;
  state: string;
}
interface AssetRow {
  mime: string;
  size: number;
  name: string | null;
  data: string; // data: URL
}
interface DB {
  notebooks: Row[];
  sections: SectionRow[];
  pages: PageRow[];
  docs: Record<string, string>; // page id -> base64 Yjs state
  versions: VersionRow[];
  assets: Record<string, AssetRow>;
  nextVersion: number;
}

const STORAGE_KEY = "anynote-demo";
const VERSION_INTERVAL = 10 * 60 * 1000;

// ---------------------------------------------------------------- storage

function b64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

function unb64(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

const docs = new Map<string, Y.Doc>();
let saveTimer: ReturnType<typeof setTimeout> | undefined;
// Created on first use, so builds that never use the demo carry no state.
let db: DB;

function ensure() {
  db ??= load();
}

function load(): DB {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as DB;
  } catch {
    /* storage unavailable or damaged: start fresh */
  }
  return seed();
}

function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    for (const [id, doc] of docs) db.docs[id] = b64(Y.encodeStateAsUpdate(doc));
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(db));
    } catch (e) {
      console.warn("demo: could not save to this browser", e);
    }
  }, 300);
}

function doc(pageId: string): Y.Doc {
  let d = docs.get(pageId);
  if (!d) {
    d = new Y.Doc();
    const state = db.docs[pageId];
    if (state) Y.applyUpdate(d, unb64(state));
    docs.set(pageId, d);
  }
  return d;
}

// ------------------------------------------------------- ProseMirror <-> Y

const BLOCK_ID_TYPES = new Set([
  "paragraph", "heading", "bulletList", "orderedList", "listItem", "taskList", "taskItem", "codeBlock",
  "blockquote", "horizontalRule", "image", "table", "blockMath", "inkBlock", "attachment",
]);

/** Write ProseMirror JSON children into a Y.XmlFragment the way y-tiptap does. */
function writeChildren(parent: Y.XmlFragment | Y.XmlElement, nodes: PMNode[]) {
  type Delta = { insert: string; attributes: Record<string, unknown> }[];
  const items: (Y.XmlElement | Y.XmlText)[] = [];
  const fill: (() => void)[] = [];
  let i = 0;
  while (i < nodes.length) {
    if (nodes[i].type === "text") {
      // Consecutive text nodes become one Y.XmlText; marks are attributes.
      const text = new Y.XmlText();
      const delta: Delta = [];
      while (i < nodes.length && nodes[i].type === "text") {
        const n = nodes[i];
        const attributes: Record<string, unknown> = {};
        for (const m of n.marks ?? []) attributes[m.type] = m.attrs ?? {};
        if (n.text) delta.push({ insert: n.text, attributes });
        i++;
      }
      items.push(text);
      fill.push(() => text.applyDelta(delta));
    } else {
      const n = nodes[i];
      const el = new Y.XmlElement(n.type);
      const attrs = { ...(n.attrs ?? {}) };
      if (BLOCK_ID_TYPES.has(n.type) && !attrs.id) attrs.id = uuid();
      for (const [k, v] of Object.entries(attrs)) if (v !== null && v !== undefined) el.setAttribute(k, v as string);
      items.push(el);
      fill.push(() => writeChildren(el, n.content ?? []));
      i++;
    }
  }
  // Insert first, then fill: integrated types take edits directly.
  parent.insert(parent.length, items);
  fill.forEach((f) => f());
}

function content(pageId: string): PMNode {
  return yXmlFragmentToProsemirrorJSON(doc(pageId).getXmlFragment("default")) as PMNode;
}

function newDoc(body: PMNode): Uint8Array {
  const d = new Y.Doc();
  d.transact(() => writeChildren(d.getXmlFragment("default"), body.content ?? []));
  return Y.encodeStateAsUpdate(d);
}

function walk(n: PMNode, f: (n: PMNode) => void) {
  f(n);
  for (const c of n.content ?? []) walk(c, f);
}

function plainText(n: PMNode): string {
  if (n.type === "text") return n.text ?? "";
  if (n.type === "hardBreak") return "\n";
  if (n.type === "wikiLink") return String(n.attrs?.title ?? "");
  if (n.type === "inlineMath" || n.type === "blockMath") return String(n.attrs?.latex ?? "");
  const inner = (n.content ?? []).map(plainText).join("");
  const inline = (n.content ?? []).some((c) => ["text", "hardBreak", "wikiLink", "inlineMath"].includes(c.type));
  return inline || !n.content?.length ? inner + "\n" : inner;
}

function tags(text: string): Set<string> {
  const out = new Set<string>();
  for (const m of text.matchAll(/(^|[\s(])#([\p{L}\p{N}_\-/]+)/gu)) {
    const t = m[2].replace(/[-/]+$/, "").toLowerCase();
    if (t && !/^\d+$/.test(t)) out.add(t);
  }
  return out;
}

// --------------------------------------------------------------- helpers

const now = () => Date.now();
const live = <T extends Row>(rows: T[]) => rows.filter((r) => r.deletedAt === null).sort((a, b) => a.position - b.position);
const byId = <T extends Row>(rows: T[], id: string) => {
  const r = rows.find((x) => x.id === id);
  if (!r) throw new Error("Not found");
  return r;
};

function sectionLive(s: SectionRow): boolean {
  return s.deletedAt === null && db.notebooks.some((n) => n.id === s.notebookId && n.deletedAt === null);
}

function pageLive(p: PageRow): boolean {
  const s = db.sections.find((x) => x.id === p.sectionId);
  return p.deletedAt === null && !!s && sectionLive(s);
}

function ref(p: PageRow) {
  const s = byId(db.sections, p.sectionId);
  const n = byId(db.notebooks, s.notebookId);
  return { id: p.id, title: p.title, sectionId: s.id, sectionTitle: s.title, notebookId: n.id, notebookTitle: n.title, updatedAt: p.updatedAt };
}

function place<T extends Row>(siblings: T[], id: string, before?: string, after?: string): number {
  const list = siblings.filter((x) => x.id !== id).sort((a, b) => a.position - b.position);
  const idx = (t?: string) => (t ? list.findIndex((x) => x.id === t) : -1);
  const mid = (lo?: number, hi?: number) =>
    lo !== undefined && hi !== undefined ? (lo + hi) / 2 : lo !== undefined ? Math.floor(lo) + 1 : hi !== undefined ? hi - 1 : 1;
  const b = idx(before);
  if (b >= 0) return mid(list[b - 1]?.position, list[b].position);
  const a = idx(after);
  if (a >= 0) return mid(list[a].position, list[a + 1]?.position);
  return mid(list[list.length - 1]?.position, undefined);
}

function row(title: string, position: number, color: string | null = null): Row {
  const t = now();
  return { id: uuid(), title: title.trim().replace(/\s+/g, " "), color, position, createdAt: t, updatedAt: t, deletedAt: null };
}

function addPage(sectionId: string, parentId: string | null, title: string, body: PMNode, after?: string, into: DB = db): PageRow {
  const siblings = into.pages.filter((p) => p.sectionId === sectionId && p.deletedAt === null);
  const page: PageRow = { ...row(title, 0), sectionId, parentId };
  page.position = place(siblings, page.id, undefined, after);
  into.pages.push(page);
  into.docs[page.id] = b64(newDoc(body));
  if (into === db) save();
  return page;
}

/** An empty canvas page (Node::canvas_doc in anynote-core). */
const canvasDoc = (): PMNode => ({
  type: "doc",
  content: [{ type: "canvasBox", attrs: { x: 48, y: 48, width: 480 }, content: [{ type: "paragraph" }] }],
});

const emptyDoc: PMNode = { type: "doc", content: [{ type: "paragraph" }] };

function seed(): DB {
  const fresh: DB = { notebooks: [], sections: [], pages: [], docs: {}, versions: [], assets: {}, nextVersion: 1 };
  const nb: Row = row("My notebook", 1, "#7c5cff");
  fresh.notebooks.push(nb);
  const start: SectionRow = { ...row("Getting started", 1), notebookId: nb.id, parentId: null };
  const ideas: SectionRow = { ...row("Ideas", 2), notebookId: nb.id, parentId: null };
  fresh.sections.push(start, ideas);
  addPage(start.id, null, "Handwriting", canvasDoc(), undefined, fresh);
  addPage(start.id, null, "Welcome to AnyNote", welcome as PMNode, undefined, fresh);
  return fresh;
}

// -------------------------------------------------------------- events

type Listener = (e: { pageId: string; update: number[] }) => void;
const listeners = new Set<Listener>();

export function demoSubscribe(cb: Listener): () => void {
  ensure();
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/** Apply a change made outside the editor and tell open editors. */
function change(pageId: string, f: (d: Y.Doc) => void) {
  const d = doc(pageId);
  let update: Uint8Array | null = null;
  const capture = (u: Uint8Array) => (update = u);
  d.on("update", capture);
  d.transact(() => f(d));
  d.off("update", capture);
  touch(pageId);
  if (update) {
    const u = Array.from(update as Uint8Array);
    listeners.forEach((l) => l({ pageId, update: u }));
  }
}

function touch(pageId: string) {
  const p = db.pages.find((x) => x.id === pageId);
  if (p) p.updatedAt = now();
  const last = db.versions.filter((v) => v.pageId === pageId).reduce((m, v) => Math.max(m, v.createdAt), 0);
  if (now() - last > VERSION_INTERVAL) snapshot(pageId, null);
  save();
}

function snapshot(pageId: string, label: string | null) {
  const v = { id: db.nextVersion++, pageId, createdAt: now(), label, state: b64(Y.encodeStateAsUpdate(doc(pageId))) };
  db.versions.push(v);
  save();
  return { id: v.id, pageId, createdAt: v.createdAt, label };
}

function findElement(parent: Y.XmlFragment | Y.XmlElement, id: string): Y.XmlElement | null {
  for (const child of parent.toArray()) {
    if (child instanceof Y.XmlElement) {
      if (child.getAttribute("id") === id) return child;
      const found = findElement(child, id);
      if (found) return found;
    }
  }
  return null;
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

function dataUrl(bytes: Uint8Array, mime: string): string {
  return `data:${mime};base64,${b64(bytes)}`;
}

export function demoAssetUrl(hash: string): string {
  ensure();
  return db.assets[hash]?.data ?? "";
}

const NOT_IN_DEMO =
  "Importing and exporting files needs AnyNote on your computer or Raspberry Pi. This online demo keeps notes only in this browser.";

// ------------------------------------------------------------ commands

type Args = Record<string, unknown>;

export async function demoCall<T>(cmd: string, raw?: Args | Uint8Array, options?: { headers?: Record<string, string> }): Promise<T> {
  ensure();
  const a = (raw instanceof Uint8Array ? {} : raw ?? {}) as Args;
  const s = (k: string) => a[k] as string;
  const opt = (k: string) => (a[k] ?? undefined) as string | undefined;
  const result = (v: unknown) => v as T;

  switch (cmd) {
    case "list_tree":
      return result(live(db.notebooks).map((n) => ({ ...n, sections: live(db.sections.filter((x) => x.notebookId === n.id)) })));
    case "create_notebook": {
      const nb = row(s("title") || "Untitled notebook", place(live(db.notebooks), ""));
      db.notebooks.push(nb);
      const sec: SectionRow = { ...row("Quick notes", 1), notebookId: nb.id, parentId: null };
      db.sections.push(sec);
      addPage(sec.id, null, "", emptyDoc);
      return result(nb);
    }
    case "update_notebook":
    case "update_section": {
      const r = byId<Row>(cmd === "update_notebook" ? db.notebooks : db.sections, s("id"));
      if (opt("title")) r.title = s("title");
      if (a.clearColor) r.color = null;
      else if (opt("color")) r.color = s("color");
      r.updatedAt = now();
      save();
      return result(r);
    }
    case "move_notebook": {
      byId(db.notebooks, s("id")).position = place(live(db.notebooks), s("id"), opt("before"), opt("after"));
      save();
      return result(undefined);
    }
    case "delete_notebook":
    case "delete_section": {
      byId<Row>(cmd === "delete_notebook" ? db.notebooks : db.sections, s("id")).deletedAt = now();
      save();
      return result(undefined);
    }
    case "create_section": {
      const parentId = opt("parentId") ?? null;
      const siblings = db.sections.filter((x) => x.notebookId === s("notebookId") && x.parentId === parentId && x.deletedAt === null);
      const sec: SectionRow = { ...row(s("title") || "Untitled section", place(siblings, "")), notebookId: s("notebookId"), parentId };
      db.sections.push(sec);
      addPage(sec.id, null, "", emptyDoc);
      return result(sec);
    }
    case "move_section": {
      const sec = byId(db.sections, s("id"));
      const parentId = opt("parentId") ?? null;
      const siblings = db.sections.filter((x) => x.notebookId === s("notebookId") && x.parentId === parentId && x.deletedAt === null);
      sec.position = place(siblings, sec.id, opt("before"), opt("after"));
      sec.parentId = parentId;
      const move = (id: string) => {
        for (const c of db.sections.filter((x) => x.parentId === id)) {
          c.notebookId = s("notebookId");
          move(c.id);
        }
      };
      sec.notebookId = s("notebookId");
      move(sec.id);
      save();
      return result(undefined);
    }
    case "list_pages":
      return result(live(db.pages.filter((p) => p.sectionId === s("sectionId"))));
    case "create_page": {
      const body = a.canvas ? canvasDoc() : emptyDoc;
      return result(addPage(s("sectionId"), opt("parentId") ?? null, s("title") ?? "", body, opt("after")));
    }
    case "get_page": {
      const p = byId(db.pages, s("id"));
      return result([p, ref(p)]);
    }
    case "rename_page": {
      const p = byId(db.pages, s("id"));
      p.title = s("title").trim().replace(/\s+/g, " ");
      p.updatedAt = now();
      save();
      return result(p);
    }
    case "move_page": {
      const p = byId(db.pages, s("id"));
      const siblings = db.pages.filter((x) => x.sectionId === s("sectionId") && x.deletedAt === null);
      p.position = place(siblings, p.id, opt("before"), opt("after"));
      p.sectionId = s("sectionId");
      p.parentId = opt("parentId") ?? null;
      const move = (id: string, i: number) => {
        for (const c of db.pages.filter((x) => x.parentId === id)) {
          c.sectionId = p.sectionId;
          c.position = p.position + ++i * 1e-6;
          i = move(c.id, i);
        }
        return i;
      };
      move(p.id, 0);
      save();
      return result(undefined);
    }
    case "delete_page": {
      const t = now();
      const del = (id: string) => {
        byId(db.pages, id).deletedAt = t;
        db.pages.filter((x) => x.parentId === id && x.deletedAt === null).forEach((c) => del(c.id));
      };
      del(s("id"));
      save();
      return result(undefined);
    }
    case "load_page_state":
      return result(Y.encodeStateAsUpdate(doc(s("id"))).slice().buffer);
    case "apply_page_update": {
      const pageId = options?.headers?.["x-page-id"] ?? "";
      byId(db.pages, pageId);
      Y.applyUpdate(doc(pageId), raw as Uint8Array);
      touch(pageId);
      return result(undefined);
    }
    case "page_markdown":
      return result(plainText(content(s("id"))));
    case "search": {
      const terms = s("query").toLowerCase().split(/\s+/).filter(Boolean);
      if (!terms.length) return result([]);
      const hits = [];
      for (const p of db.pages.filter(pageLive)) {
        const body = plainText(content(p.id)).replace(/\s+/g, " ").trim();
        const hay = `${p.title} ${body}`.toLowerCase();
        if (!terms.every((t) => hay.includes(t))) continue;
        const at = Math.max(0, body.toLowerCase().indexOf(terms[0]));
        let snip = body.slice(Math.max(0, at - 50), at + 110);
        for (const t of terms) snip = snip.replace(new RegExp(t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi"), (m) => `\u0001${m}\u0002`);
        hits.push({ page: ref(p), snippet: snip, score: p.title.toLowerCase().includes(terms[0]) ? 0 : 1 });
      }
      hits.sort((x, y) => x.score - y.score || y.page.updatedAt - x.page.updatedAt);
      return result(hits.slice(0, Number(a.limit) || 50).map(({ page, snippet }) => ({ page, snippet })));
    }
    case "find_pages":
    case "recent_pages": {
      const q = (opt("query") ?? "").trim().toLowerCase();
      const pages = db.pages.filter(pageLive).filter((p) => !q || p.title.toLowerCase().includes(q));
      pages.sort((x, y) => y.updatedAt - x.updatedAt);
      return result(pages.slice(0, Number(a.limit) || 10).map(ref));
    }
    case "backlinks": {
      const target = s("pageId");
      const out = db.pages.filter(pageLive).filter((p) => {
        let hit = false;
        walk(content(p.id), (n) => (hit ||= n.type === "wikiLink" && n.attrs?.pageId === target));
        return hit;
      });
      return result(out.map(ref));
    }
    case "list_tasks": {
      const rows: { page: ReturnType<typeof ref>; blockId: string; text: string; checked: boolean }[] = [];
      for (const p of db.pages.filter(pageLive)) {
        walk(content(p.id), (n) => {
          if (n.type !== "taskItem" || !n.attrs?.id) return;
          const checked = Boolean(n.attrs.checked);
          if (!checked || a.includeDone) {
            const first = n.content?.find((c) => c.type === "paragraph");
            rows.push({ page: ref(p), blockId: String(n.attrs.id), text: first ? plainText(first).trim() : "", checked });
          }
        });
      }
      rows.sort((x, y) => Number(x.checked) - Number(y.checked) || y.page.updatedAt - x.page.updatedAt);
      return result(rows);
    }
    case "set_task_checked": {
      change(s("pageId"), (d) => findElement(d.getXmlFragment("default"), s("blockId"))?.setAttribute("checked", a.checked as never));
      return result(undefined);
    }
    case "list_tags":
    case "pages_with_tag": {
      const counts = new Map<string, PageRow[]>();
      for (const p of db.pages.filter(pageLive)) {
        for (const t of tags(plainText(content(p.id)))) counts.set(t, [...(counts.get(t) ?? []), p]);
      }
      if (cmd === "pages_with_tag") return result((counts.get(s("tag").replace(/^#/, "").toLowerCase()) ?? []).map(ref));
      return result([...counts].sort(([x], [y]) => x.localeCompare(y)).map(([tag, pages]) => ({ tag, count: pages.length })));
    }
    case "list_versions":
      return result(
        db.versions
          .filter((v) => v.pageId === s("pageId"))
          .sort((x, y) => y.createdAt - x.createdAt)
          .map(({ id, pageId, createdAt, label }) => ({ id, pageId, createdAt, label })),
      );
    case "snapshot_version":
      return result(snapshot(s("pageId"), opt("label") ?? null));
    case "version_content": {
      const v = db.versions.find((x) => x.id === Number(a.versionId));
      if (!v) throw new Error("Version not found");
      const d = new Y.Doc();
      Y.applyUpdate(d, unb64(v.state));
      return result(yXmlFragmentToProsemirrorJSON(d.getXmlFragment("default")));
    }
    case "restore_version": {
      const v = db.versions.find((x) => x.id === Number(a.versionId));
      if (!v) throw new Error("Version not found");
      snapshot(v.pageId, "Before restore");
      const old = new Y.Doc();
      Y.applyUpdate(old, unb64(v.state));
      const body = yXmlFragmentToProsemirrorJSON(old.getXmlFragment("default")) as PMNode;
      change(v.pageId, (d) => {
        const frag = d.getXmlFragment("default");
        frag.delete(0, frag.length);
        writeChildren(frag, body.content ?? []);
        const ink = d.getMap("ink");
        old.getMap("ink").forEach((strokes, key) => {
          const arr = new Y.Array();
          arr.push((strokes as Y.Array<unknown>).toArray());
          ink.set(key, arr);
        });
      });
      return result(undefined);
    }
    case "list_trash": {
      const items = [
        ...db.notebooks.filter((x) => x.deletedAt !== null).map((x) => ({ kind: "notebook", id: x.id, title: x.title, deletedAt: x.deletedAt })),
        ...db.sections.filter((x) => x.deletedAt !== null).map((x) => ({ kind: "section", id: x.id, title: x.title, deletedAt: x.deletedAt })),
        ...db.pages.filter((x) => x.deletedAt !== null).map((x) => ({ kind: "page", id: x.id, title: x.title, deletedAt: x.deletedAt })),
      ];
      return result(items.sort((x, y) => (y.deletedAt ?? 0) - (x.deletedAt ?? 0)));
    }
    case "restore_item": {
      const kind = s("kind");
      if (kind === "notebook") byId(db.notebooks, s("id")).deletedAt = null;
      if (kind === "section" || kind === "page") {
        let sec: SectionRow | undefined;
        if (kind === "page") {
          const p = byId(db.pages, s("id"));
          const when = p.deletedAt;
          const undelete = (id: string) => {
            for (const c of db.pages.filter((x) => x.parentId === id && x.deletedAt === when)) {
              c.deletedAt = null;
              undelete(c.id);
            }
          };
          p.deletedAt = null;
          undelete(p.id);
          if (p.parentId && db.pages.find((x) => x.id === p.parentId)?.deletedAt !== null) p.parentId = null;
          sec = byId(db.sections, p.sectionId);
        } else {
          sec = byId(db.sections, s("id"));
        }
        for (let cur: SectionRow | undefined = sec; cur; cur = db.sections.find((x) => x.id === cur!.parentId)) cur.deletedAt = null;
        byId(db.notebooks, sec.notebookId).deletedAt = null;
      }
      save();
      return result(undefined);
    }
    case "empty_trash": {
      const doomed = new Set(db.pages.filter((p) => !pageLive(p)).map((p) => p.id));
      db.pages = db.pages.filter((p) => !doomed.has(p.id));
      for (const id of doomed) {
        delete db.docs[id];
        docs.delete(id);
      }
      db.versions = db.versions.filter((v) => !doomed.has(v.pageId));
      db.sections = db.sections.filter((x) => x.deletedAt === null);
      db.notebooks = db.notebooks.filter((x) => x.deletedAt === null);
      save();
      return result(undefined);
    }
    case "store_asset": {
      const bytes = raw as Uint8Array;
      const hash = await sha256(bytes);
      const name = options?.headers?.["x-name"] ? decodeURIComponent(options.headers["x-name"]) : null;
      const mime = (options?.headers?.["x-mime"] && decodeURIComponent(options.headers["x-mime"])) || "application/octet-stream";
      db.assets[hash] ??= { mime, size: bytes.length, name, data: dataUrl(bytes, mime) };
      save();
      return result({ hash, url: `anynote://asset/${hash}`, mime, size: bytes.length, name });
    }
    case "import_onenote":
    case "import_folder":
    case "export_folder":
      throw new Error(NOT_IN_DEMO);
    case "data_dir":
      return result("this browser (online demo)");
    default:
      throw new Error(`${cmd} is not available in the online demo`);
  }
}

/** Forget everything and start again with the sample notebook. */
export function demoReset() {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    /* nothing stored */
  }
  docs.clear();
  db = seed();
  save();
}
