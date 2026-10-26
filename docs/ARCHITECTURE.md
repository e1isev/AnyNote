# Architecture

AnyNote follows the build plan in [`docs/PLAN.md`](PLAN.md): keep OneNote's good idea (an object graph with history, synced incrementally) but replace the implementation with a **local-first CRDT model**, an **open file format**, and a **one-way importer** for OneNote files.

```text
┌──────────────────────────── app/ (Tauri) ─────────────────────────────┐
│  React UI ── TipTap editor ── Yjs Y.Doc per page ── PageSync          │
│                                   │ binary Yjs updates (raw IPC)      │
│                                   ▼                                    │
│  src-tauri: commands, `anynote://asset/<hash>` protocol, events        │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │
┌──────────────── crates/anynote-core ───────────────────────────────────┐
│  node     ProseMirror-compatible block tree (generic, lossless)       │
│  ydoc     yrs (Rust Yjs) ⇄ block tree, ink map, Rust-side edits       │
│  store    SQLite: catalogue, update log, versions, FTS5, links, tasks │
│  assets   content-addressed files (BLAKE3)                            │
│  markdown / folder   open folder format (Markdown + JSON sidecars)    │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │
┌──── crates/anynote-onenote ────┐  ┌──── crates/anynote-cli ────┐
│ onenote_parser → block tree    │  │ `anynote` command line      │
└────────────────────────────────┘  └─────────────────────────────┘
```

## Data model

| Concept | Representation |
| --- | --- |
| Notebook, section, page | Rows in SQLite with UUIDv7 ids and fractional `position`s. Sections nest (section groups); pages nest (subpages) through `parent_id`. |
| Page body | One Yjs document per page. The body is the `default` XML fragment in exactly the layout y-tiptap uses, so the editor binds to it directly. |
| Blocks | ProseMirror nodes. Every block carries a stable `id` attribute (TipTap UniqueID in the editor, `Node::ensure_block_ids` in Rust). |
| Ink | A `Y.Map` named `ink` in the same document: `inkId → Y.Array<stroke>`. Strokes merge like text. |
| Assets | Files stored once under `assets/<2 hex>/<blake3>`, referenced as `anynote://asset/<hash>`. |
| History | Automatic snapshots every 10 minutes of editing, plus "before restore" and manual snapshots. A restore is an ordinary CRDT edit, so it merges with concurrent changes. |

The block model in Rust is deliberately generic (`type` + `attrs` + `content` + `marks`), not a closed enum. Unknown node types and attributes survive every conversion. This is the forward-compatibility rule [MS-ONE] imposes on OneNote writers, applied to our own format.

## Editing flow

1. The UI asks Rust for the page's CRDT state (`load_page_state`, raw bytes) and applies it to a fresh `Y.Doc` **before** binding the editor, so no placeholder content is ever written.
2. Each local transaction produces a Yjs update. `PageSync` batches updates for 300 ms, merges them and sends one binary update (`apply_page_update`, page id in the `x-page-id` header).
3. Rust appends the update to `page_updates`, merges it into its cached `yrs` document, and re-derives the index: FTS5 text, wiki links, tasks and `#tags`.
4. Changes made outside the editor (ticking a task in the Tasks view, restoring a version) are applied to the Rust document. The resulting update is pushed to open editors as a `page-update` event.

Failed sends are retried, which is always safe: Yjs updates are idempotent and order-independent.

## Storage

`~/.local/share/anynote/` (override with `ANYNOTE_DATA_DIR`):

```text
anynote.db        SQLite (WAL): notebooks, sections, pages, page_updates,
                  page_versions, links, tasks, tags, assets, page_fts (FTS5)
assets/ab/abcd…   content-addressed files
```

The update log is compacted into a single snapshot after 200 entries. Deleting things moves them to the trash (`deleted_at`). "Empty trash" removes them for good.

## Search

SQLite FTS5 with `unicode61 remove_diacritics`. Titles are weighted 8× over bodies. User input becomes quoted terms, and the last term matches as a prefix, so results appear while typing and FTS syntax errors can't happen. Snippets mark matches with `\u0001…\u0002`. The UI turns those into `<mark>` elements, so there is no HTML injection.

## Why these choices

* **Tauri, not Electron**: the Rust core links `onenote_parser` and `yrs` directly, and the app is a small native binary using the system WebKitGTK.
* **Yjs (yrs) for the CRDT**: TipTap's collaboration binding speaks Yjs, and `yrs` reads and writes the same binary format. One document format therefore serves the editor, the importer, the exporter and the indexer. `tests/editor_compat.rs` and `app/src/editor/compat.test.ts` check the contract in both directions.
* **SQLite for the catalogue and index**: one file, transactional, FTS5 built in.
* **Markdown + JSON sidecar for export**: Markdown for people and git, the sidecar for a lossless round trip (ink, block ids, link targets).

## Sync (planned)

Every change is already a mergeable CRDT update stored in a log, so sync means exchanging updates. Two routes are planned (see [ROADMAP.md](ROADMAP.md)):

* folder sync (Syncthing or Dropbox), with per-device update files merged on read;
* a small self-hostable WebSocket relay.

Catalogue rows (titles, order) need last-writer-wins merging keyed by `updated_at`.
