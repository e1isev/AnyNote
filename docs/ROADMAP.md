# Roadmap

The phases follow [PLAN.md](PLAN.md). Each phase has a gate that must be met before the next starts.

## Phase 1: Understand the format ✅

- [x] Read [MS-ONESTORE] / [MS-ONE] through `onenote_parser`, which mirrors the specs
- [x] Instrumented dump of the parsed object tree (`anynote onenote-dump file.one`) for change-by-change diffing
- [ ] Own test corpus: one notebook, one feature per page, exported three ways (desktop `.one`, OneNote Web ZIP, Graph HTML)
- [ ] Mapping table, "UI action → objects/properties touched", from diffs of that corpus

**Gate:** every feature in the corpus is identified in the dump.

## Phase 2: Importer ✅ (v1)

- [x] `.one`, `.onetoc2`, `.onepkg` and OneDrive/FSSHTTP section files; notebook folders with section groups
- [x] Subpages, rich-text runs → marks, headings, lists, tables, images (alt/OCR text), attachments, to-do tags, other tags → `#hashtags`
- [x] Ink → vector strokes; equations → LaTeX; `onenote:` links between imported pages → wiki links
- [x] Fixture test against the onenote.rs samples (52/54 files; the other two are git-lfs placeholders)
- [ ] Ink pressure (not exposed by the parser yet)
- [ ] Paragraph alignment, fonts and sizes, cell shading
- [ ] Indented children of plain paragraphs (currently flattened)
- [ ] Page version history from the revision store
- [ ] Password-protected sections
- [ ] Upstream fixes to onenote.rs for anything above that needs parser changes
- [ ] Optional: Microsoft Graph importer ("connect account"), text and images only, since Graph has no ink

**Gate:** your own notebooks import with no visible loss on spot checks.

## Phase 3: The app ✅ (v0.1)

- [x] Local store: SQLite catalogue, CRDT update log with compaction, FTS5 search, content-addressed assets
- [x] Editor: Markdown shortcuts, slash menu, code, math, tables, images, attachments, ink, wiki links
- [x] Backlinks, Tasks view across notebooks, Tags view, version history, trash
- [x] Open folder format: export and import, lossless round trip, Markdown folders
- [x] Linux packages (`.deb`, `.rpm`, `.AppImage`) via the release workflow
- [ ] Page and block embeds (`![[Page]]`, block references using block ids)
- [ ] Typed page properties and saved queries
- [ ] Find in page, outline/table-of-contents panel, word count
- [ ] Settings: theme override, editor width, data folder
- [ ] Keyboard-first navigation of the tree and page list

**Gate:** daily use for a month without reaching for OneNote.

## Phase 4: Sync

- [ ] Folder sync: write per-device update files next to the database so Syncthing or Dropbox can carry them, and merge on read
- [ ] Catalogue rows (titles, order, colours) as last-writer-wins records with `updated_at`
- [ ] Optional self-hostable relay: CRDT update exchange over WebSocket
- [ ] Live multi-window editing, with the Yjs awareness protocol for cursors

**Gate:** two machines edit the same page offline and merge without losing anything.

## Phase 5: Where to beat OneNote

- [ ] Canvas mode: free-positioned block groups (x, y, width), the same idea as OneNote outlines, opt-in per page
- [ ] OCR for images and handwriting (Tesseract), indexed into search
- [ ] Optional semantic search (local embeddings)
- [ ] Lab-notebook blocks: CSV/table blocks with live plots, timestamped entries
- [ ] Plugin API over the block model and ink
- [ ] Audio recordings linked to typed notes
- [ ] Mobile (Tauri mobile or Flutter with flutter_rust_bridge) sharing `anynote-core`
