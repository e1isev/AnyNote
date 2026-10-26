# OneNote Reverse-Engineering & Build Plan

Oct 26, 2026 · @Elise

## Summary

OneNote is a binary **object database with revision history**, not a document file. Each section (`.one`) is a "revision store" of objects with property sets, cross-referenced by GUID, and the notebook (`.onetoc2`) is a small index of sections. Sync works by shipping only changed objects/revisions over an incremental binary protocol (FSSHTTP/FSSHTTPB) to OneDrive/SharePoint.

You don't need to blindly reverse-engineer it: Microsoft publishes the full format under its Open Specifications (\[MS-ONESTORE\], \[MS-ONE\], \[MS-FSSHTTPB\]), and an open-source Rust parser (onenote.rs) already reads it. The plan is therefore: **read the specs → use/extend the parser as an importer → build your own app on a modern local-first stack (CRDT data model, infinite canvas, open file format) → beat OneNote on openness, Markdown/code, search and offline sync.**

## How OneNote works under the hood

Three layers stack on each other: a **revision store** (a transactional object database in one file), a **content schema** (pages, outlines, paragraphs as property sets), and a **sync protocol** that ships changed objects to OneDrive/SharePoint. The diagram at the end of this section shows how they fit.

### 1. Storage hierarchy

- **Notebook** = a folder plus a `.onetoc2` table-of-contents file (section order, colours). Section groups are subfolders.
- **Section** = one `.one` file holding every page in that section, plus page version history.
- **Page hierarchy** is not a real tree: subpages are stored as a flat list with a `PageLevel` indent property ([MS-ONE properties](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-one/e9bf7da8-7aab-4668-be5e-e0c421175e3c)).

### 2. Revision store layer — \[MS-ONESTORE\]

- Microsoft describes it as a revision store split into cross-referenced **object spaces**, holding objects with **property sets**, plus a **transaction log** so the file stays consistent across asynchronous writes ([spec feed](https://msopenspecs.microsoft.com/files/MS-ONESTORE/%5bMS-ONESTORE%5d.rss)).
- Core building blocks in the spec: file header, root file node list, **file node lists** (linked chunks of records), **file chunk references** (offset + size pointers), **revisions** and **revision manifest lists** ([spec ToC](https://msopenspecs.microsoft.com/files/MS-ONESTORE/%5bMS-ONESTORE%5d-250218.docx)).
- Mental model: a small log-structured database / git-like object store inside one file. Objects are addressed by ExtendedGUIDs (GUID + counter), each edit appends a new revision, and a page roughly corresponds to an object space.
- Two physical encodings exist: the desktop layout (\[MS-ONESTORE\] §2.3) and the FSSHTTP packaging used for OneDrive downloads (§2.8) ([onenote.rs](https://github.com/msiemens/onenote.rs)).

### 3. Content layer — \[MS-ONE\]

- Every structure is a property set; each property has a 32-bit PropertyID that also encodes its data type and size ([MS-ONE properties](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-one/e9bf7da8-7aab-4668-be5e-e0c421175e3c)).
- **Forward compatibility rule:** a reader must ignore unknown properties but write them back unchanged. Any writer you build must round-trip what it doesn't understand.
- Page element tree: **Page → Title + Outlines** (free-floating containers with `OffsetFromParentHoriz/Vert` and `LayoutMaxWidth`) **→ OutlineElements** (paragraphs, indented by `OutlineElementChildLevel`) **→ content**: rich text, images (`PictureContainer`, `ImageAltText`), tables (`RowCount`, `ColumnCount`, `TableColumnWidths`), embedded files, ink, math.
- **Text is not HTML.** It is a Unicode string (`RichEditTextUnicode`) plus a run-index array (`TextRunIndex`) and a parallel array of run formats (`TextRunFormatting`) — the classic RTF-style run model.
- **Tags/tasks** are properties on paragraphs: shape, label, created/completed times, due date (`NoteTag*`, `TaskTagDueDate`).
- **Conflicts are first-class:** properties such as `IsConflictPage` and `ConflictingUserName` exist, which is how unmergeable concurrent edits surface as "conflict pages".

### 4. Sync layer — \[MS-FSSHTTP\] / \[MS-FSSHTTPB\]

- Cloud notebooks sync through FSSHTTP, the same "cell storage" protocol Office uses for co-authoring: clients upload and download **only file changes** plus metadata ([Microsoft blog](https://learn.microsoft.com/en-us/archive/blogs/vedant/sharepoint-products-and-technologies-protocols)).
- The revision store is mapped onto FSSHTTPB data elements: storage index, storage manifest, cell manifests, revision manifests and object groups ([MS-ONESTORE §3.5](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-onestore/0ca8cbf7-85dd-4e34-b144-89d1ca16b5e2)). Sync granularity is therefore objects/revisions, not whole files.

### 5. API view — Microsoft Graph

- Graph exposes pages as HTML. What you send (input HTML) differs from what you get back (output HTML) ([Graph docs](https://learn.microsoft.com/en-MY/graph/onenote-input-output-html)).
- The canvas is modelled as absolutely positioned `div`s with `data-id`s on a body marked `data-absolute-enabled`; nested positioning is ignored ([Graph abs-pos](https://learn.microsoft.com/en-au/graph/onenote-abs-pos)). This is a handy reference for how Microsoft itself flattens the canvas.
- Ink is not exposed through the API, so API-based exporters lose handwriting ([Joplin forum](https://discourse.joplinapp.org/t/onenote-export-from-microsoft-365-for-joplin-import-experimental/13266)).

### 6. App-level features (not in the file spec)

The infinite-canvas renderer, ink engine, OCR of images, audio recordings linked to typed notes, and the search index live in the apps and services, not the format. You rebuild these, not reverse-engineer them.

&#91;embedded content: OneNote's three layers · content, revision store, encodings and sync\]

The revision store is the core: your importer reads it once, and nothing in your own app needs to copy it.

## Legal and clean-room approach

Work from the published specs and your own files, not from OneNote's binaries. That keeps this an interoperability project rather than a reverse-engineering-of-software one.

- **Specs are licensed for implementers.** Microsoft's Open Specifications notice lets you copy the docs to build implementations and says it claims no trade secrets in them ([MS-ONESTORE](https://msopenspecs.microsoft.com/files/MS-ONESTORE/%5bMS-ONESTORE%5d-160929.docx)). The FSSHTTPB notice adds that Microsoft may hold patents covering implementations ([MS-FSSHTTPB](https://msopenspecs.microsoft.com/files/MS-FSSHTTPB/%5bMS-FSSHTTPB%5d-240521.docx)) — relevant only if you clone the sync protocol, which this plan avoids.
- **Do:** parse files you own, use open-source parsers, compare against Graph API output, design your own format.
- **Don't:** decompile or debug OneNote's executables (licence terms), copy its UI assets, or use the OneNote name/logo in your product branding. "Imports from OneNote" is fine as a feature description.
- Not legal advice — worth a quick check before commercial release.

## Existing open-source work to build on

Start from onenote.rs: it already reads both desktop and OneDrive files, and Joplin's importer proves it works on real notebooks including handwriting.

| Project | Language | What it does | Use it for |
| --- | --- | --- | --- |
| [onenote.rs / onenote\_parser](https://docs.rs/onenote_parser/2.0.0/onenote_parser/) | Rust | Implements all three layers (FSSHTTPB, OneStore, MS-ONE); reads OneDrive packages, desktop 2016/2019/LTSC files and Mac backups. Read-only. | Your importer core. Code layout mirrors the specs, so it doubles as a study guide. |
| [Joplin OneNote importer](https://joplinapp.org/help/apps/import_export) | TS + Rust/WASM | Imports OneNote Web ZIP exports (Joplin ≥ 3.5.1) and desktop `.one` sections (≥ 3.5.5). Users rate it the best option for keeping handwriting, with some missing-section bugs ([forum](https://discourse.joplinapp.org/t/onenote-importer/37720/12)). | Reference for turning the object tree into rendered pages. |
| [pyOneNote](https://github.com/dissectmalware/pyonenote) | Python | Security-focused reader; dumps embedded files and properties. | Quick poking at raw properties in a notebook/REPL. |
| [LibMsON](https://www.github.com/msiemens/onenote.rs) | C++ | Work-in-progress revision store parser (listed in onenote.rs). | Second opinion on tricky structures. |
| [OneNoteMdExporter](https://discourse.joplinapp.org/t/onenote-export-to-joplin/14153?page=2) | .NET | Drives the installed OneNote desktop app to export Markdown / Joplin folders. | Ground-truth output to compare your converter against. |
| [onenote-export](https://discourse.joplinapp.org/t/onenote-export-from-microsoft-365-for-joplin-import-experimental/13266) | JS (browser) | Graph API exporter to HTML or ENEX; no ink. | Example of the Graph API import path. |

## Reverse-engineering plan

The method is **differential analysis**: make one known change in OneNote, export, and diff the parsed object graph. The specs tell you the grammar; diffs tell you how the app actually uses it.

1. **Build a test corpus.** One notebook, one feature per page: each text format, bullet/number lists, indentation, tables (merged widths, shading), images with alt text, attachments, ink (pen, highlighter, shapes), tags and to-dos, math, links, subpages, section groups, page history, a password-protected section, and a deliberate two-device conflict.
   - Export each three ways: desktop *Export → Section (.one)*, OneNote Web *Export notebook (ZIP)*, and Graph API page HTML.
   - Output: same content in three encodings for triangulation.
2. **Read the specs in this order.** \[MS-ONESTORE\] §1 overview → §2.3–2.5 (file structure, file node lists, node types) → \[MS-ONE\] page/outline/rich-text structures → \[MS-FSSHTTPB\] only when you handle OneDrive packages.
   - Output: your own one-page cheat sheet of the object tree.
3. **Instrument the parser.** Build onenote.rs locally and add a `--dump-json` mode that prints every object space, revision, object and raw property (ID, type, bytes).
   - Output: `dump.json` per test page.
4. **Diff change-by-change.** Script: baseline export → change one thing (bold one word, move an outline 50 px, tick a to-do) → re-export → JSON diff. Record each finding in a mapping table: UI action → objects/properties touched.
   - Tools: a JSON diff (`jq` + `difft`), a hex editor with templates (ImHex patterns or 010 Editor), and optionally a Kaitai Struct `.ksy` for the header and file-node lists — writing one is the fastest way to really learn the layout.
5. **Convert to your model.** Write `onenote → yourformat` on top of the parser. Golden tests: every corpus page converts, and a render of your output is visually compared with the Graph HTML / OneNoteMdExporter output.
6. **Close gaps.** Ink strokes, embedded files, tag definitions, version history. Upstream fixes to onenote.rs where you can — it keeps your importer maintained by more than you.
7. **Optional, high risk: writing `.one` files.** Only if you need round-trip back to OneNote. You must preserve unknown properties and the transaction log correctly; most projects never do this.
8. **Optional: live import via Graph API.** A "connect Microsoft account" importer for text and images; accept that ink is missing on this path.

Stop at step 6 for v1: a solid importer is what lets people switch, and everything after is your own app.

## Architecture for a better version

Keep OneNote's good idea (an object graph with history, synced incrementally) but swap the implementation for a **local-first CRDT model**: concurrent edits merge automatically, so conflict pages disappear.

Recommended stack: **Tauri** (Rust core + web UI). The Rust side can link onenote.rs directly for import, and the UI can be React, which you already use. If you'd rather go mobile-first, Flutter works but you'd call the Rust parser via FFI (flutter\_rust\_bridge).

| Layer | OneNote | Your version |
| --- | --- | --- |
| Data model | Revision store: objects + property sets, GUID IDs | One CRDT document per page (Yjs, Automerge or Loro). Blocks keyed by stable IDs; rich text as a CRDT text type |
| Text model | Unicode string + run-index/run-format arrays | Block tree (paragraph, heading, list, code, table, math) via ProseMirror/TipTap or Lexical, bound to the CRDT |
| Canvas | Free-floating outlines with x/y offsets | Doc-first page; optional canvas mode where each block group has x/y/width (same idea as OneNote outlines), drawn with tldraw or your own Canvas/WebGL layer |
| Ink | Proprietary ink objects | Vector strokes: points + pressure + tilt from Pointer Events, smoothed with perfect-freehand; stored in the same CRDT so ink syncs too |
| Storage | Binary `.one` per section | SQLite (pages, blocks, assets index, FTS) + CRDT update log; assets content-addressed by hash on disk |
| Open format | None (export is lossy) | Lossless export: folder per notebook, Markdown per page + JSON sidecar for layout/ink + assets folder (git-friendly) |
| Sync | FSSHTTP to OneDrive/SharePoint | CRDT update exchange over WebSocket to a small self-hostable server; or plain folder sync (Syncthing/Dropbox) since updates merge |
| Search | Server index + OCR in images | SQLite FTS5 locally; OCR via platform APIs (Apple Vision, Windows.Media.Ocr) or Tesseract; optional embeddings for semantic search |
| Extensibility | Limited Graph API, no ink | Plugin API over the block model + local scripting |

Key design rules:

- **IDs everywhere.** Every notebook, section, page, block and stroke gets a UUID/ULID at creation; links and backlinks point at IDs, never titles or positions.
- **History for free.** Keep CRDT snapshots per page for version history instead of OneNote-style page versions.
- **Import is a one-way adapter.** onenote.rs object tree → your block model; never let OneNote's schema leak into your core.

## Where to beat OneNote

Compete on openness, structure and engineering use; don't try to out-feature OneNote's canvas on day one.

| OneNote weakness | Your answer |
| --- | --- |
| Proprietary binary format; exports lose layout and ink | Open, documented folder format with lossless export and import |
| Free canvas creates stray text boxes and messy pages | Doc-first pages with an opt-in canvas mode |
| No Markdown, no real code blocks | Markdown shortcuts, syntax-highlighted code blocks, LaTeX math |
| Sync delays and conflict pages | Local-first CRDT sync, works fully offline, self-hostable |
| Weak linking between pages | `[[wiki links]]`, backlinks, page and block embeds |
| Tags are flat; tasks are hard to query | Typed page properties, saved queries, a task view across notebooks |
| API can't read ink | Everything, including ink, reachable from the plugin API |
| Generic notebook | Lab-notebook features: CSV/table blocks with live plots, timestamped entries, attachments versioned with the page |

Open question: is this for you only, or a product? A personal tool can skip accounts and the sync server (use folder sync) and ship much sooner.

## Roadmap and milestones

Build the importer first: it is the riskiest part, it teaches you the format, and it is the feature that makes people willing to switch.

&#91;embedded content: Build roadmap · 5 phases, 4 gates\]

Phases run in order; don't start the next until its gate is met. If this is personal-only, phase 4 can stop at folder sync.

## Sources

- [MS-ONESTORE: OneNote Revision Store File Format — spec feed](https://msopenspecs.microsoft.com/files/MS-ONESTORE/%5bMS-ONESTORE%5d.rss)
- [MS-ONESTORE spec document (2025)](https://msopenspecs.microsoft.com/files/MS-ONESTORE/%5bMS-ONESTORE%5d-250218.docx)
- [MS-ONESTORE §3.5 Reading an FSSHTTP-encoded file](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-onestore/0ca8cbf7-85dd-4e34-b144-89d1ca16b5e2)
- [MS-ONE §2.1.12 Properties](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-one/e9bf7da8-7aab-4668-be5e-e0c421175e3c)
- [MS-FSSHTTPB spec document](https://msopenspecs.microsoft.com/files/MS-FSSHTTPB/%5bMS-FSSHTTPB%5d-240521.docx)
- [SharePoint protocols / FSSHTTP overview (Microsoft blog)](https://learn.microsoft.com/en-us/archive/blogs/vedant/sharepoint-products-and-technologies-protocols)
- [Graph: input and output HTML on OneNote pages](https://learn.microsoft.com/en-MY/graph/onenote-input-output-html)
- [Graph: absolute positioned elements](https://learn.microsoft.com/en-au/graph/onenote-abs-pos)
- [onenote.rs (GitHub)](https://github.com/msiemens/onenote.rs) · [onenote\_parser 2.0 docs](https://docs.rs/onenote_parser/2.0.0/onenote_parser/)
- [pyOneNote (GitHub)](https://github.com/dissectmalware/pyonenote)
- [Joplin import/export docs](https://joplinapp.org/help/apps/import_export) · [Joplin OneNote importer thread](https://discourse.joplinapp.org/t/onenote-importer/37720/12)
- [OneNoteMdExporter thread](https://discourse.joplinapp.org/t/onenote-export-to-joplin/14153?page=2) · [onenote-export (Graph) thread](https://discourse.joplinapp.org/t/onenote-export-from-microsoft-365-for-joplin-import-experimental/13266)
