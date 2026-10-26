# AnyNote formats

This document is the contract shared by the editor (`app/src/editor/extensions.ts`), the Rust core (`crates/anynote-core/src/node.rs`) and the importers. Renaming a node, mark or attribute requires changing all three. The fixture in `fixtures/editor-compat/` and its tests catch drift.

## Block schema

Pages are ProseMirror documents. JSON looks like `{"type": "paragraph", "attrs": {...}, "content": [...]}`. Text nodes are `{"type": "text", "text": "...", "marks": [...]}`.

| Node | Attributes | Content |
| --- | --- | --- |
| `doc` | | block+ |
| `paragraph` | `id` | inline* |
| `heading` | `id`, `level` 1–6 | inline* |
| `bulletList` | `id` | listItem+ |
| `orderedList` | `id`, `start` | listItem+ |
| `listItem` | `id` | paragraph block* |
| `taskList` | `id` | taskItem+ |
| `taskItem` | `id`, `checked` | paragraph block* |
| `codeBlock` | `id`, `language` | text* (no marks) |
| `blockquote` | `id` | block+ |
| `horizontalRule` | `id` | |
| `image` | `id`, `src`, `alt`, `title`, `width`, `height` | |
| `table` | `id` | tableRow+ |
| `tableRow` | | (tableCell \| tableHeader)+ |
| `tableCell`, `tableHeader` | `colspan`, `rowspan`, `colwidth` (px array or null) | block+ |
| `blockMath` | `id`, `latex` | |
| `inkBlock` | `id`, `inkId`, `height`, `width` | |
| `attachment` | `id`, `src`, `name`, `size`, `mime` | |
| `canvasBox` | `id`, `x`, `y`, `width` (whole CSS px) | block+ |
| `hardBreak` (inline) | | |
| `inlineMath` (inline) | `latex` | |
| `wikiLink` (inline) | `pageId`, `title` | |

Marks: `bold`, `italic`, `underline`, `strike`, `code`, `subscript`, `superscript`, `link {href, target, rel}`, `highlight {color}`, `textStyle {color}`.

`image` is a block node and is never inside a paragraph. `wikiLink.title` is the title when the link was made. The app shows the target's current title.

### Canvas pages

A page whose `doc` has a top-level `canvasBox` is a canvas page: an open, zoomable surface where you write anywhere. Each `canvasBox` is a text box placed at (`x`, `y`) from the page's top-left, `width` px wide, like a OneNote outline. The page's drawing is the ink list under the key `canvas` in the ink map, in the same page coordinates. New canvas pages start with one empty box, which the editor reuses for the first text typed. Exporters write the boxes top to bottom, then left to right (rows 24 px tall), followed by the drawing.

## CRDT layout (per page)

Each page is a Yjs document (binary update format v1):

* `XmlFragment "default"`: the body. Each node is a `Y.XmlElement` named after its type, with its attrs as attributes (null attrs omitted). Consecutive text nodes form one `Y.XmlText`, and each run's marks are formatting attributes `{markName: markAttrs}`. This is y-prosemirror's layout.
* `Map "ink"`: `inkId → Y.Array` of strokes. A stroke is a map:

  | Key | Type | Meaning |
  | --- | --- | --- |
  | `id` | string | stroke id |
  | `tool` | string | `pen` or `highlighter` |
  | `color` | string | CSS colour |
  | `size` | number | base width in CSS px |
  | `pts` | bytes | little-endian `f32` triples `(x, y, pressure)`, CSS px from the ink block's top-left (from the page's top-left for `canvas`), pressure 0–1 |

  The key `canvas` holds a canvas page's drawing. Other keys are `inkBlock` ids.

## Asset URLs

`anynote://asset/<blake3 hex>` everywhere in documents. The app serves them through its custom protocol. Exporters rewrite them to relative file paths.

## Folder format (export and import)

```text
<Notebook>/
  notebook.json                 {"id", "title", "color", "sections": [folder names in order]}
  assets/<hash>.<ext>           every asset used in the notebook; ink-<inkId>.svg renders
                                (ink-<pageId>.svg for a canvas page's drawing)
  <Section>/
    section.json                {"id", "title", "color", "pages": [...], "sections": [...]}
    <Page>.md                   front matter + "# Title" + Markdown body
    <Page>.anynote.json         lossless sidecar
    <Nested section>/...
```

`section.json` `pages` entries are `{"id", "file", "title", "parentId", "createdAt", "updatedAt"}` in display order. `parentId` encodes subpages.

Page Markdown:

```markdown
---
id: 0190f0e0-…
title: "Experiment 1"
created: 2026-10-26T08:20:00Z
updated: 2026-10-26T08:31:12Z
---

# Experiment 1

Centrifuge at **4 °C**, see [[Protocol]] #biology

- [x] Order pipettes

$$
\int_0^1 x^2\,dx
$$

![Ink drawing](../assets/ink-3f2a….svg)
```

Extensions to CommonMark/GFM: `$inline$` and `$$display$$` math, `[[wiki links]]`, and `<u>`, `<mark>`, `<sub>`, `<sup>` for marks Markdown lacks.

Sidecar (`<Page>.anynote.json`):

```json
{
  "format": "anynote-page",
  "version": 1,
  "id": "…",
  "title": "…",
  "createdAt": 1793002781285,
  "updatedAt": 1793002841827,
  "doc": { "type": "doc", "content": [ … ] },
  "ink": { "<inkId>": [ { "id": "…", "tool": "pen", "color": "#1f2328", "size": 4, "points": [[x, y, p], …] } ] }
}
```

On import, the sidecar wins when present. Otherwise the Markdown is parsed, and relative image and link targets become assets. Wiki links are re-pointed by original page id, then by title. Imports always create new ids, so importing twice never collides.
