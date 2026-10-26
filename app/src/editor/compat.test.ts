// Contract test with crates/anynote-core/tests/editor_compat.rs: the editor's
// Yjs binding must load documents written by Rust, and documents it writes
// must be readable by Rust (checked on the Rust side from js-state.bin).
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { getSchema } from "@tiptap/core";
import { Mathematics } from "@tiptap/extension-mathematics";
import UniqueID from "@tiptap/extension-unique-id";
import { prosemirrorJSONToYDoc, yXmlFragmentToProseMirrorRootNode } from "@tiptap/y-tiptap";
import { describe, expect, it } from "vitest";
import * as Y from "yjs";
import { BLOCK_ID_TYPES, schemaExtensions } from "./extensions";
import { decodePoints, encodePoints, InkBlock } from "./InkBlock";
import { WikiLink } from "./WikiLink";

const dir = resolve(__dirname, "../../../fixtures/editor-compat");
const sample = JSON.parse(readFileSync(resolve(dir, "sample.json"), "utf8"));
const schema = getSchema([...schemaExtensions(), Mathematics, InkBlock, WikiLink, UniqueID.configure({ types: BLOCK_ID_TYPES })]);

/** `expected` is contained in `actual` (the editor adds default attrs). */
function expectSubset(expected: unknown, actual: unknown, path = "doc") {
  if (Array.isArray(expected)) {
    expect(Array.isArray(actual), path).toBe(true);
    expect((actual as unknown[]).length, `${path}.length`).toBe(expected.length);
    expected.forEach((e, i) => expectSubset(e, (actual as unknown[])[i], `${path}[${i}]`));
  } else if (expected && typeof expected === "object") {
    expect(actual && typeof actual === "object", path).toBe(true);
    for (const [k, v] of Object.entries(expected)) expectSubset(v, (actual as Record<string, unknown>)[k], `${path}.${k}`);
  } else {
    expect(actual, path).toEqual(expected);
  }
}

const samplePoints: [number, number, number][] = [
  [10, 20, 0.5],
  [30.5, 40.25, 0.75],
  [60, 42, 0.5],
];

describe("editor ↔ Rust document compatibility", () => {
  it("loads documents written by Rust into a valid editor document", () => {
    const doc = new Y.Doc();
    Y.applyUpdate(doc, readFileSync(resolve(dir, "rust-state.bin")));
    const node = yXmlFragmentToProseMirrorRootNode(doc.getXmlFragment("default"), schema);
    node.check(); // throws if the content violates the schema
    expectSubset(sample, node.toJSON());

    const strokes = (doc.getMap("ink").get("ink-1") as Y.Array<{ pts: Uint8Array; color: string }>).toArray();
    expect(strokes).toHaveLength(1);
    expect(strokes[0].color).toBe("#2f6fdf");
    expect(decodePoints(strokes[0].pts)).toEqual(samplePoints);
  });

  it("writes documents in the shape Rust reads", () => {
    const ydoc = prosemirrorJSONToYDoc(schema, sample, "default");
    const ink = new Y.Array();
    ink.push([{ id: "stroke-1", tool: "pen", color: "#2f6fdf", size: 4, pts: encodePoints(samplePoints) }]);
    ydoc.getMap("ink").set("ink-1", ink);
    const bytes = Y.encodeStateAsUpdate(ydoc);
    if (process.env.ANYNOTE_UPDATE_FIXTURES) writeFileSync(resolve(dir, "js-state.bin"), bytes);

    const again = new Y.Doc();
    Y.applyUpdate(again, readFileSync(resolve(dir, "js-state.bin")));
    const node = yXmlFragmentToProseMirrorRootNode(again.getXmlFragment("default"), schema);
    node.check();
    expectSubset(sample, node.toJSON());
  });
});
