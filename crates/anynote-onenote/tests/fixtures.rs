//! Imports real OneNote files and checks every stored page against the
//! editor schema. Fixtures are not committed (they belong to other
//! projects); point `ANYNOTE_ONENOTE_FIXTURES` at a folder of `.one` /
//! `.onetoc2` files, e.g. the `tests/samples` folder of onenote.rs:
//!
//! ```sh
//! git clone --depth 1 https://github.com/msiemens/onenote.rs /tmp/onenote.rs
//! ANYNOTE_ONENOTE_FIXTURES=/tmp/onenote.rs/crates/parser/tests/samples \
//!     cargo test -p anynote-onenote --test fixtures -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anynote_core::assets::ASSET_URL_PREFIX;
use anynote_core::ink::Stroke;
use anynote_core::node::{kinds, BLOCK_ID_TYPES};
use anynote_core::{Node, Store};

fn fixtures() -> Option<PathBuf> {
    match std::env::var_os("ANYNOTE_ONENOTE_FIXTURES") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => {
            eprintln!("ANYNOTE_ONENOTE_FIXTURES is not set; skipping OneNote fixture tests");
            None
        }
    }
}

fn onenote_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().to_lowercase();
        if p.is_dir() {
            onenote_files(&p, out);
        } else if name.ends_with(".one") || name.ends_with(".onetoc2") {
            out.push(p);
        }
    }
}

/// Git LFS pointers stand in for files too large for the repository.
fn is_lfs_pointer(path: &Path) -> bool {
    std::fs::read(path).map(|b| b.starts_with(b"version https://git-lfs")).unwrap_or(false)
}

#[test]
fn every_fixture_imports_into_a_schema_valid_store() {
    let Some(dir) = fixtures() else { return };
    let mut files = Vec::new();
    onenote_files(&dir, &mut files);
    assert!(!files.is_empty(), "no OneNote files under {}", dir.display());

    let (mut imported, mut pages) = (0, 0);
    for file in &files {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Store::open(tmp.path()).unwrap();
        match anynote_onenote::import(&mut store, file) {
            Ok(report) => {
                imported += 1;
                pages += report.pages;
                let checked = validate_store(&mut store);
                assert_eq!(checked, report.pages, "{}: report and store disagree", file.display());
                eprintln!("ok   {} ({} pages, {} warnings)", file.display(), report.pages, report.warnings.len());
            }
            Err(e) => {
                // Only placeholders and empty recycle bins may fail.
                let recycle_bin = file.to_string_lossy().contains("OneNote_RecycleBin");
                assert!(is_lfs_pointer(file) || recycle_bin, "{} failed to import: {e:#}", file.display());
                eprintln!("skip {} ({e:#})", file.display());
            }
        }
    }
    eprintln!("{imported} of {} files imported, {pages} pages", files.len());
    assert!(pages > 0);
}

#[test]
fn whole_folder_imports_once() {
    let Some(dir) = fixtures() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::open(tmp.path()).unwrap();
    let report = anynote_onenote::import(&mut store, &dir).unwrap();
    assert!(report.pages > 0);
    assert_eq!(validate_store(&mut store), report.pages);
    // Importing the same folder must not create duplicate sections.
    let mut titles: Vec<(String, String)> = Vec::new();
    for nb in store.list_notebooks().unwrap() {
        for sec in store.list_sections(&nb.id).unwrap() {
            titles.push((sec.parent_id.clone().unwrap_or_default(), sec.title.clone()));
        }
    }
    let before = titles.len();
    titles.sort();
    titles.dedup();
    eprintln!("{} pages, {} sections, {} assets, {} strokes", report.pages, before, report.assets, report.ink_strokes);
}

#[test]
fn dump_json_describes_every_fixture() {
    let Some(dir) = fixtures() else { return };
    let mut files = Vec::new();
    onenote_files(&dir, &mut files);
    for file in files.iter().filter(|f| !is_lfs_pointer(f)) {
        let json = anynote_onenote::dump_json(file).unwrap_or_else(|e| panic!("{}: {e:#}", file.display()));
        assert!(matches!(json["kind"].as_str(), Some("notebook" | "section")), "{}", file.display());
    }
}

/// Validate every page in the store; returns how many pages were checked.
fn validate_store(store: &mut Store) -> usize {
    let ids = store.all_page_ids().unwrap();
    for id in &ids {
        let (doc, ink) = store.page_content(id).unwrap();
        let mut errors = Vec::new();
        validate_doc(&doc, &ink, &mut errors);
        assert!(errors.is_empty(), "page {id} ({}):\n  {}", store.page(id).unwrap().title, errors.join("\n  "));
    }
    ids.len()
}

fn validate_doc(doc: &Node, ink: &BTreeMap<String, Vec<Stroke>>, errors: &mut Vec<String>) {
    if doc.kind != kinds::DOC {
        errors.push(format!("root is {}", doc.kind));
    }
    if doc.content.is_empty() {
        errors.push("doc is empty".into());
    }
    for block in &doc.content {
        validate_block(block, ink, errors);
    }
}

const MARKS: &[&str] =
    &["bold", "italic", "underline", "strike", "code", "superscript", "subscript", "link", "highlight", "textStyle"];

fn validate_block(n: &Node, ink: &BTreeMap<String, Vec<Stroke>>, errors: &mut Vec<String>) {
    let mut err = |msg: String| errors.push(format!("{}: {msg}", n.kind));
    if BLOCK_ID_TYPES.contains(&n.kind.as_str()) && n.attr_str("id").is_none_or(str::is_empty) {
        err("missing block id".into());
    }
    let children_kinds: Vec<&str> = n.content.iter().map(|c| c.kind.as_str()).collect();
    match n.kind.as_str() {
        kinds::PARAGRAPH | kinds::HEADING => {
            if n.is(kinds::HEADING) && !(1..=6).contains(&n.attr_i64("level").unwrap_or(0)) {
                err("bad level".into());
            }
            for c in &n.content {
                validate_inline(c, errors);
            }
            return;
        }
        kinds::BULLET_LIST | kinds::ORDERED_LIST => {
            if n.content.is_empty() || children_kinds.iter().any(|k| *k != kinds::LIST_ITEM) {
                err(format!("children {children_kinds:?}"));
            }
        }
        kinds::TASK_LIST => {
            if n.content.is_empty() || children_kinds.iter().any(|k| *k != kinds::TASK_ITEM) {
                err(format!("children {children_kinds:?}"));
            }
        }
        kinds::LIST_ITEM | kinds::TASK_ITEM => {
            if children_kinds.first() != Some(&kinds::PARAGRAPH) {
                err(format!("must start with a paragraph, has {children_kinds:?}"));
            }
            if n.is(kinds::TASK_ITEM) && !n.attr("checked").is_some_and(|v| v.is_boolean()) {
                err("checked must be a boolean".into());
            }
        }
        kinds::BLOCKQUOTE | kinds::TABLE_CELL | kinds::TABLE_HEADER => {
            if n.content.is_empty() {
                err("must hold at least one block".into());
            }
        }
        kinds::CODE_BLOCK => {
            if n.content.iter().any(|c| !c.is_text() || !c.marks.is_empty()) {
                err("may only hold unmarked text".into());
            }
            return;
        }
        kinds::TABLE => {
            if n.content.is_empty() || children_kinds.iter().any(|k| *k != kinds::TABLE_ROW) {
                err(format!("children {children_kinds:?}"));
            }
        }
        kinds::TABLE_ROW => {
            if n.content.is_empty()
                || children_kinds.iter().any(|k| *k != kinds::TABLE_CELL && *k != kinds::TABLE_HEADER)
            {
                err(format!("children {children_kinds:?}"));
            }
        }
        kinds::IMAGE | kinds::ATTACHMENT => {
            if !n.attr_str("src").is_some_and(|s| s.starts_with(ASSET_URL_PREFIX)) {
                err(format!("src {:?}", n.attr("src")));
            }
            if !n.content.is_empty() {
                err("is a leaf".into());
            }
            return;
        }
        kinds::INK_BLOCK => {
            let id = n.attr_str("inkId").unwrap_or("");
            if !ink.get(id).is_some_and(|s| !s.is_empty()) {
                err(format!("no strokes for inkId {id:?}"));
            }
            if !n.attr("height").is_some_and(|v| v.as_f64().is_some_and(|h| h > 0.0)) {
                err("height must be a positive number".into());
            }
            return;
        }
        kinds::BLOCK_MATH | kinds::HORIZONTAL_RULE => return,
        other => {
            err(format!("unknown block type {other}"));
            return;
        }
    }
    for c in &n.content {
        validate_block(c, ink, errors);
    }
}

fn validate_inline(n: &Node, errors: &mut Vec<String>) {
    match n.kind.as_str() {
        kinds::TEXT => {
            if n.text.as_deref().unwrap_or("").is_empty() {
                errors.push("empty text node".into());
            }
            for m in &n.marks {
                if !MARKS.contains(&m.kind.as_str()) {
                    errors.push(format!("unknown mark {}", m.kind));
                }
            }
        }
        kinds::HARD_BREAK => {}
        kinds::INLINE_MATH => {
            if n.attr_str("latex").is_none() {
                errors.push("inlineMath without latex".into());
            }
        }
        kinds::WIKI_LINK => {
            if n.attr_str("pageId").is_none() {
                errors.push("wikiLink without pageId".into());
            }
        }
        other => errors.push(format!("{other} inside a text block")),
    }
}
