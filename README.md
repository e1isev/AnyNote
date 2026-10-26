# AnyNote

A local-first notebook app for Linux, in the spirit of OneNote but with an open format. It has notebooks, sections and pages, rich text, handwriting, tasks, math and code. Everything stays on your machine in files you can read without the app, and it imports your OneNote notebooks.

![AnyNote showing a page with headings, a wiki link, to-dos and a drawing](docs/screenshots/page.png)

## Why another notebook?

AnyNote follows the build plan in [docs/PLAN.md](docs/PLAN.md). It keeps OneNote's good idea, an object graph with history, and replaces the parts that hurt:

| OneNote | AnyNote |
| --- | --- |
| Proprietary binary `.one` files; exports lose layout and ink | Open folder format: Markdown for people and git, plus a lossless JSON sidecar ([FORMAT.md](docs/FORMAT.md)) |
| Sync delays and "conflict pages" | Every page is a CRDT document (Yjs), so concurrent edits merge instead of conflicting. Works fully offline. |
| No Markdown, weak code support | Markdown shortcuts, syntax-highlighted code blocks, LaTeX math (KaTeX) |
| Weak linking between pages | `[[wiki links]]` stored by page id, so renames never break them; backlinks panel |
| Tags are flat; to-dos are hard to find | `#tags` anywhere in the text, a tag browser, and one Tasks view across every notebook (tick tasks there and the page updates) |
| Free canvas leaves stray text boxes | Doc-first pages, with ink drawing blocks where you want them |
| API can't read ink | Ink is plain vector data in the page document, and is exported as SVG |

### Features

- Notebooks with colour coding, nested sections (section groups), and pages with subpages
- Rich text with headings, lists, to-dos, quotes, tables, links, highlight and text colour, sub/superscript
- Code blocks with syntax highlighting, and inline and display math
- Ink blocks with pen (pressure-sensitive), highlighter and stroke eraser, saved as vector strokes
- Images and file attachments (paste or drag & drop). Audio and video play inline.
- `/` slash menu for any block, and `[[` to link or create a page
- Full-text search (Ctrl+K) with highlighted snippets
- Tasks view, Tags view, backlinks, and version history with preview and restore
- Trash with restore
- **Import:**
  - OneNote `.one` sections, `.onetoc2` notebooks and `.onepkg` packages, including handwriting, equations (converted to LaTeX), tables, images, attachments and to-do tags
  - Markdown folders such as Obsidian vaults and AnyNote exports
- **Export:** Markdown + JSON sidecars + assets, per notebook or everything

![A OneNote page imported into AnyNote: its equations rendered as LaTeX](docs/screenshots/onenote-import.png)

The table at the top of [`crates/anynote-onenote/src/lib.rs`](crates/anynote-onenote/src/lib.rs) shows how each OneNote structure maps to AnyNote. [ROADMAP.md](docs/ROADMAP.md) lists what doesn't come across yet.

## Install

Release builds (`.deb`, `.rpm`, `.AppImage`) are produced by the [release workflow](.github/workflows/release.yml) when a version tag is pushed. To build them yourself, follow [Build from source](#build-from-source) and run `npx tauri build` in `app/`. The packages land in `target/release/bundle/`.

## Build from source

You need Rust 1.85+ and Node 22, plus the WebKitGTK development packages:

```sh
# Debian / Ubuntu
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
# Fedora
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file \
  libxdo-devel libappindicator-gtk3-devel librsvg2-devel
```

Then:

```sh
cd app
npm install
npx tauri dev      # run with hot reload
npx tauri build    # build release packages
```

Your notes live in `~/.local/share/anynote/`. Set `ANYNOTE_DATA_DIR` to use another folder, for example a separate test library.

## Command line

The `anynote` CLI works on the same data folder as the app:

```sh
cargo run -p anynote-cli -- import-onenote ~/OneNote/MyNotebook/   # or a .one / .onetoc2 / .onepkg file
cargo run -p anynote-cli -- list
cargo run -p anynote-cli -- search "centrifuge"
cargo run -p anynote-cli -- tasks
cargo run -p anynote-cli -- export ~/notes-export
cargo run -p anynote-cli -- onenote-dump Section.one --out dump.json   # inspect a OneNote file's object tree
```

`onenote-dump` is the instrumented parser output that the plan's reverse-engineering workflow uses: change one thing in OneNote, re-export, diff the dumps.

## Project layout

```text
crates/anynote-core      block model, CRDT page documents, SQLite store, search, folder format
crates/anynote-onenote   OneNote importer (built on onenote_parser)
crates/anynote-cli       `anynote` command-line tool
app/                     desktop app: React + TipTap UI
app/src-tauri            Tauri shell: commands, asset protocol, events
fixtures/editor-compat   documents shared by the Rust ↔ editor contract tests
docs/                    plan, architecture, formats, roadmap
```

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the pieces fit together, and [docs/ROADMAP.md](docs/ROADMAP.md) for what's next.

## Development

```sh
cargo test --workspace                 # Rust: core, importer, CLI, contract tests
cargo clippy --workspace --all-targets
cd app && npm test && npm run typecheck
```

The OneNote fixture tests need sample files, which are not committed:

```sh
git clone --depth 1 https://github.com/msiemens/onenote.rs /tmp/onenote.rs
ANYNOTE_ONENOTE_FIXTURES=/tmp/onenote.rs/crates/parser/tests/samples \
  cargo test -p anynote-onenote --test fixtures -- --nocapture
```

If you change the editor schema, regenerate the contract fixtures in this order: run `ANYNOTE_UPDATE_FIXTURES=1 cargo test -p anynote-core --test editor_compat`, then `ANYNOTE_UPDATE_FIXTURES=1 npm test` in `app/`, then commit both.

## Legal

AnyNote reads OneNote files using Microsoft's published Open Specifications ([MS-ONESTORE], [MS-ONE], [MS-FSSHTTPB]) through the open-source [`onenote_parser`](https://github.com/msiemens/onenote.rs) crate (MPL-2.0). It contains no Microsoft code or assets. "OneNote" is a trademark of Microsoft and is used here only to describe import compatibility.

No licence has been chosen for AnyNote itself yet.
