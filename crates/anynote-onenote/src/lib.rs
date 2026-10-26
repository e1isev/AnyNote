//! One-way importer from OneNote into AnyNote.
//!
//! Reads OneNote files with [`onenote_parser`] (desktop `.one` sections,
//! `.onetoc2` notebooks, `.onepkg` packages and OneDrive/FSSHTTP downloads)
//! and writes them into an [`anynote_core::Store`] as ordinary notebooks,
//! sections and pages. OneNote's schema never leaks into the store: every page
//! is converted to the editor's block tree (see `convert.rs` and
//! `docs/FORMAT.md`).
//!
//! | OneNote | AnyNote |
//! | --- | --- |
//! | notebook / section group / section | notebook / nested section / section |
//! | page level (subpages) | `parent_id` of the page |
//! | outlines on the canvas | blocks, in reading order (top→bottom, left→right) |
//! | heading styles h1–h6, code, quote | heading, codeBlock, blockquote |
//! | bullet / numbered list elements | bulletList / orderedList, nested by indent |
//! | checkbox note tags ("To Do" …) | taskList / taskItem (checked = completed) |
//! | other note tags | `#hashtag` at the start of the paragraph |
//! | run formatting, colours, links | marks (bold, italic, …, highlight, textStyle, link) |
//! | equations | inlineMath / blockMath (LaTeX) |
//! | images (+ alt / OCR text), files | image / attachment with content-addressed assets |
//! | ink (drawings and handwriting) | inkBlock + vector strokes |
//! | tables | tables with column widths |
//! | `onenote:` links between imported pages | wikiLink |

mod convert;
mod dump;
mod math;
mod source;
mod text;

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use anynote_core::node::{kinds, marks};
use anynote_core::store::Placement;
use anynote_core::{Node, Store};
use onenote_parser::notebook::Notebook;
use onenote_parser::page::Page;
use onenote_parser::section::{Section, SectionEntry};
use onenote_parser::FileIdentity;
use serde::Serialize;

use crate::convert::{AssetSink, ConvertedPage, Stats};
use crate::source::Parsed;

/// Keep reports readable when a damaged notebook produces many warnings.
const MAX_WARNINGS: usize = 500;

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub notebook_ids: Vec<String>,
    pub sections: usize,
    pub pages: usize,
    pub assets: usize,
    pub ink_strokes: usize,
    pub warnings: Vec<String>,
}

/// Import a `.one` section, a `.onetoc2` notebook, a `.onepkg` package, or a
/// folder containing them (e.g. a desktop notebook folder or an unzipped
/// OneNote export).
pub fn import(store: &mut Store, path: &Path) -> anyhow::Result<ImportReport> {
    let mut imp = Importer {
        store,
        stats: Stats::default(),
        report: ImportReport::default(),
        by_guid: HashMap::new(),
        relink: Vec::new(),
        seen: HashSet::new(),
    };
    if path.is_dir() {
        imp.folder(path)?;
        if imp.report.notebook_ids.is_empty() && imp.stats.warnings.is_empty() {
            bail!("no OneNote files (.one, .onetoc2, .onepkg) found in {}", path.display());
        }
    } else if source::ext(path) == "onetoc2" {
        // A notebook is its folder: the table of contents plus its files.
        let toc = match parse_guarded(path)? {
            Parsed::Notebook { title, notebook } => Some((title, notebook)),
            Parsed::Section(_) => None,
        };
        let dir = path.parent().unwrap_or(Path::new("."));
        imp.notebook_folder_root(dir, &sorted_entries(dir)?, toc)?;
    } else {
        imp.parsed(parse_guarded(path)?)?;
    }
    imp.resolve_links()?;
    if imp.report.pages == 0 && !imp.stats.warnings.is_empty() {
        bail!("nothing was imported: {}", imp.stats.warnings[0]);
    }
    let mut report = imp.report;
    report.assets = imp.stats.assets;
    report.ink_strokes = imp.stats.strokes;
    let extra = imp.stats.warnings.len().saturating_sub(MAX_WARNINGS);
    report.warnings = imp.stats.warnings.into_iter().take(MAX_WARNINGS).collect();
    if extra > 0 {
        report.warnings.push(format!("… and {extra} more warnings"));
    }
    Ok(report)
}

/// Instrumented dump of the parsed object tree (pages, outlines, elements,
/// text runs with formatting, images, files, ink, tables) as JSON.
pub fn dump_json(path: &Path) -> anyhow::Result<serde_json::Value> {
    match catch_unwind(AssertUnwindSafe(|| dump::dump(path))) {
        Ok(result) => result,
        Err(_) => bail!("the OneNote parser crashed on {}", path.display()),
    }
}

/// The parser is young; a malformed file must not take the app down.
fn parse_guarded(path: &Path) -> anyhow::Result<Parsed> {
    match catch_unwind(AssertUnwindSafe(|| source::parse(path))) {
        Ok(result) => result,
        Err(_) => bail!("the OneNote parser crashed on {}", path.display()),
    }
}

fn sorted_entries(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    Ok(entries)
}

fn file_name(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string()
}

/// Hidden folders and the recycle bin are skipped when walking folders.
fn skip_entry(path: &Path, name: &str) -> bool {
    path.is_dir() && (name.starts_with('.') || name == "OneNote_RecycleBin")
}

/// Whether a folder (recursively) holds any OneNote file.
fn contains_onenote(dir: &Path) -> bool {
    let Ok(entries) = sorted_entries(dir) else { return false };
    entries.iter().any(|p| {
        let name = file_name(p);
        if skip_entry(p, &name) {
            return false;
        }
        if p.is_dir() {
            contains_onenote(p)
        } else {
            matches!(source::ext(p).as_str(), "one" | "onetoc2" | "onepkg")
        }
    })
}

fn count_sections(entries: &[SectionEntry]) -> usize {
    entries
        .iter()
        .map(|e| match e {
            SectionEntry::Section(_) => 1,
            SectionEntry::SectionGroup(g) => count_sections(g.entries()),
        })
        .sum()
}

struct StoreSink<'a>(&'a mut Store);

impl AssetSink for StoreSink<'_> {
    fn put(&mut self, bytes: &[u8], name: Option<&str>, mime: Option<&str>) -> anyhow::Result<String> {
        Ok(self.0.put_asset(bytes, name, mime)?.url)
    }
}

struct Importer<'a> {
    store: &'a mut Store,
    stats: Stats,
    report: ImportReport,
    /// OneNote page GUID → (AnyNote page id, title), for `onenote:` links.
    by_guid: HashMap<String, (String, String)>,
    /// Pages whose content links to other OneNote pages.
    relink: Vec<(String, ConvertedPage)>,
    /// Sections already imported (a file can be reachable more than once).
    seen: HashSet<FileIdentity>,
}

impl Importer<'_> {
    fn parsed(&mut self, parsed: Parsed) -> anyhow::Result<()> {
        match parsed {
            Parsed::Notebook { title, notebook } => self.notebook(&title, &notebook).map(drop),
            Parsed::Section(section) => {
                let nb = self.store.create_notebook(section.display_name(), None)?;
                self.report.notebook_ids.push(nb.id.clone());
                self.section(&nb.id, None, &section)
            }
        }
    }

    /// Walk a folder that is not inside a notebook. A folder holding a
    /// `.onetoc2` is a notebook; a folder of loose `.one` files becomes a
    /// notebook named after the folder; other folders are walked.
    fn folder(&mut self, dir: &Path) -> anyhow::Result<()> {
        let entries = sorted_entries(dir)?;
        if entries.iter().any(|p| p.is_file() && source::ext(p) == "onetoc2") {
            let toc = self.best_toc(&entries);
            return self.notebook_folder_root(dir, &entries, toc);
        }
        let mut notebook_id: Option<String> = None;
        for path in entries {
            let name = file_name(&path);
            if skip_entry(&path, &name) {
                continue;
            }
            if path.is_dir() {
                self.folder(&path)?;
            } else if source::ext(&path) == "onepkg" {
                match parse_guarded(&path) {
                    Ok(parsed) => self.parsed(parsed)?,
                    Err(e) => self.stats.warnings.push(format!("{e:#}")),
                }
            } else if source::ext(&path) == "one" {
                let Some(section) = self.parse_section_file(&path) else { continue };
                let id = match &notebook_id {
                    Some(id) => id.clone(),
                    None => {
                        let title = file_name(dir);
                        let nb = self
                            .store
                            .create_notebook(if title.is_empty() { "OneNote import" } else { &title }, None)?;
                        self.report.notebook_ids.push(nb.id.clone());
                        notebook_id = Some(nb.id.clone());
                        nb.id
                    }
                };
                self.section(&id, None, &section)?;
            }
        }
        Ok(())
    }

    /// Import a notebook folder: the sections its table of contents lists,
    /// then any section files and section-group folders it doesn't (OneNote
    /// tables of contents are sometimes stale or unreadable).
    fn notebook_folder_root(
        &mut self,
        dir: &Path,
        entries: &[PathBuf],
        toc: Option<(String, Notebook)>,
    ) -> anyhow::Result<()> {
        let title = toc.as_ref().map(|(t, _)| t.clone()).unwrap_or_else(|| file_name(dir));
        let color = toc.as_ref().and_then(|(_, nb)| nb.color()).map(|c| text::hex([c.r(), c.g(), c.b()]));
        let nb = self.store.create_notebook(&title, color.as_deref())?;
        self.report.notebook_ids.push(nb.id.clone());
        if let Some((_, toc)) = &toc {
            for w in toc.report().warnings() {
                self.stats.warnings.push(format!("{title}: {}", w.message()));
            }
        }
        self.notebook_folder(entries, &nb.id, None, toc.as_ref().map(|(_, n)| n))
    }

    fn notebook_folder(
        &mut self,
        entries: &[PathBuf],
        notebook_id: &str,
        parent: Option<&str>,
        toc: Option<&Notebook>,
    ) -> anyhow::Result<()> {
        let mut covered_dirs = HashSet::new();
        if let Some(toc) = toc {
            for entry in toc.entries() {
                self.entry(notebook_id, parent, entry)?;
                if let SectionEntry::SectionGroup(g) = entry {
                    covered_dirs.insert(g.display_name().to_string());
                }
            }
        }
        for path in entries {
            let name = file_name(path);
            if skip_entry(path, &name) {
                continue;
            }
            if path.is_dir() {
                if covered_dirs.contains(&name) || !contains_onenote(path) {
                    continue;
                }
                let sub = sorted_entries(path)?;
                let sub_toc = self.best_toc(&sub);
                let group = self.store.create_section(notebook_id, parent, &name)?;
                self.report.sections += 1;
                self.notebook_folder(&sub, notebook_id, Some(&group.id), sub_toc.as_ref().map(|(_, n)| n))?;
            } else if source::ext(path) == "onepkg" {
                match parse_guarded(path) {
                    Ok(parsed) => self.parsed(parsed)?,
                    Err(e) => self.stats.warnings.push(format!("{e:#}")),
                }
            } else if source::ext(path) == "one" {
                if let Some(section) = self.parse_section_file(path) {
                    self.section(notebook_id, parent, &section)?;
                }
            }
        }
        Ok(())
    }

    /// Of the tables of contents in a folder, the one listing most sections.
    fn best_toc(&mut self, entries: &[PathBuf]) -> Option<(String, Notebook)> {
        let mut best: Option<(String, Notebook)> = None;
        for toc in entries.iter().filter(|p| p.is_file() && source::ext(p) == "onetoc2") {
            match parse_guarded(toc) {
                Ok(Parsed::Notebook { title, notebook }) => {
                    let better = match &best {
                        None => true,
                        Some((_, current)) => count_sections(notebook.entries()) > count_sections(current.entries()),
                    };
                    if better {
                        best = Some((title, notebook));
                    }
                }
                Ok(Parsed::Section(_)) => {}
                Err(e) => self.stats.warnings.push(format!("{e:#}")),
            }
        }
        best
    }

    /// Parse a section file, skipping ones already imported.
    fn parse_section_file(&mut self, path: &Path) -> Option<Section> {
        match parse_guarded(path) {
            Ok(Parsed::Section(section)) if !self.seen.contains(&section.file_identity()) => Some(section),
            Ok(_) => None,
            Err(e) => {
                self.stats.warnings.push(format!("{e:#}"));
                None
            }
        }
    }

    fn notebook(&mut self, title: &str, notebook: &Notebook) -> anyhow::Result<String> {
        let color = notebook.color().map(|c| text::hex([c.r(), c.g(), c.b()]));
        let nb = self.store.create_notebook(title, color.as_deref())?;
        self.report.notebook_ids.push(nb.id.clone());
        for w in notebook.report().warnings() {
            self.stats.warnings.push(format!("{title}: {}", w.message()));
        }
        for entry in notebook.entries() {
            self.entry(&nb.id, None, entry)?;
        }
        Ok(nb.id)
    }

    fn entry(&mut self, notebook_id: &str, parent: Option<&str>, entry: &SectionEntry) -> anyhow::Result<()> {
        match entry {
            SectionEntry::Section(section) => self.section(notebook_id, parent, section),
            SectionEntry::SectionGroup(group) => {
                if group.display_name() == "OneNote_RecycleBin" {
                    return Ok(());
                }
                let sec = self.store.create_section(notebook_id, parent, group.display_name())?;
                self.report.sections += 1;
                for child in group.entries() {
                    self.entry(notebook_id, Some(&sec.id), child)?;
                }
                Ok(())
            }
        }
    }

    fn section(&mut self, notebook_id: &str, parent: Option<&str>, section: &Section) -> anyhow::Result<()> {
        let name = section.display_name().trim_end_matches(".one");
        if !self.seen.insert(section.file_identity()) {
            return Ok(());
        }
        let sec = self.store.create_section(notebook_id, parent, name)?;
        self.report.sections += 1;
        if let Some(c) = section.color() {
            let color = text::hex([c.r(), c.g(), c.b()]);
            self.store.update_section(&sec.id, None, Some(Some(&color)))?;
        }
        for w in section.report().warnings() {
            let page = w.page().map(|(_, title)| format!(" / {title}")).unwrap_or_default();
            self.stats.warnings.push(format!("{name}{page}: {}", w.message()));
        }

        let pages: Vec<&Page> = section.page_series().iter().flat_map(|ps| ps.pages()).collect();
        let mut converted = Vec::with_capacity(pages.len());
        for page in pages {
            let mut sink = StoreSink(&mut *self.store);
            let stats = &mut self.stats;
            let result = catch_unwind(AssertUnwindSafe(|| convert::convert_page(page, name, &mut sink, stats)));
            match result {
                Ok(p) => converted.push(p),
                Err(_) => {
                    let title = page.title_text().unwrap_or("Untitled page").to_string();
                    self.stats.warnings.push(format!("{name} / {title}: conversion failed; imported the title only"));
                    let mut doc = Node::doc(vec![Node::paragraph(vec![])]);
                    doc.ensure_block_ids();
                    converted.push(ConvertedPage {
                        title,
                        level: page.level(),
                        created_at: None,
                        guid: text::normalize_guid(page.link_target_id()),
                        doc,
                        ink: Default::default(),
                        has_onenote_links: false,
                    });
                }
            }
        }

        let parents = text::resolve_parents(&converted.iter().map(|p| p.level).collect::<Vec<_>>());
        let mut ids: Vec<String> = Vec::with_capacity(converted.len());
        for (page, parent) in converted.into_iter().zip(parents) {
            let parent_id = parent.map(|i| ids[i].clone());
            let info = self.store.create_page_with_content(
                &sec.id,
                parent_id.as_deref(),
                &page.title,
                Placement::End,
                &page.doc,
                &page.ink,
                page.created_at,
            )?;
            self.report.pages += 1;
            if !page.guid.is_empty() {
                self.by_guid.insert(page.guid.clone(), (info.id.clone(), page.title.clone()));
            }
            ids.push(info.id.clone());
            if page.has_onenote_links {
                self.relink.push((info.id, page));
            }
        }
        Ok(())
    }

    /// Second pass: turn `onenote:` links to imported pages into wiki links.
    fn resolve_links(&mut self) -> anyhow::Result<()> {
        for (page_id, mut page) in std::mem::take(&mut self.relink) {
            if relink(&mut page.doc, &self.by_guid) {
                self.store.replace_page_content(&page_id, &page.doc, &page.ink)?;
            }
        }
        Ok(())
    }
}

/// Replace text carrying an `onenote:` link to a known page with a
/// `wikiLink` node. Returns whether anything changed.
fn relink(node: &mut Node, by_guid: &HashMap<String, (String, String)>) -> bool {
    let mut changed = false;
    for child in &mut node.content {
        changed |= relink(child, by_guid);
    }
    let target = |n: &Node| -> Option<(String, String, String)> {
        let href = n.mark(marks::LINK)?.attr_str("href")?;
        let guid = text::onenote_link_page_id(href)?;
        let (id, title) = by_guid.get(&guid)?;
        Some((href.to_string(), id.clone(), title.clone()))
    };
    if !node.content.iter().any(|n| n.is_text() && target(n).is_some()) {
        return changed;
    }
    let mut out: Vec<Node> = Vec::with_capacity(node.content.len());
    let mut i = 0;
    while i < node.content.len() {
        let n = &node.content[i];
        match n.is_text().then(|| target(n)).flatten() {
            Some((href, page_id, page_title)) => {
                let mut label = String::new();
                while i < node.content.len()
                    && node.content[i].is_text()
                    && node.content[i].mark(marks::LINK).and_then(|m| m.attr_str("href")) == Some(href.as_str())
                {
                    label.push_str(node.content[i].text.as_deref().unwrap_or(""));
                    i += 1;
                }
                let label = label.trim();
                let title = if label.is_empty() { page_title } else { label.to_string() };
                out.push(Node::new(kinds::WIKI_LINK).with_attr("pageId", page_id).with_attr("title", title));
            }
            None => {
                out.push(n.clone());
                i += 1;
            }
        }
    }
    node.content = out;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use anynote_core::Mark;

    #[test]
    fn onenote_links_become_wiki_links() {
        let link = Mark::new(marks::LINK).with_attr("href", "onenote:#Target&section-id={S}&page-id={ABC-1}&end");
        let mut doc = Node::doc(vec![Node::paragraph(vec![
            Node::text("see "),
            Node::styled_text("the ", vec![link.clone()]),
            Node::styled_text("target", vec![link, Mark::new(marks::BOLD)]),
            Node::styled_text(
                " elsewhere",
                vec![Mark::new(marks::LINK).with_attr("href", "onenote:#X&page-id={nope}")],
            ),
        ])]);
        let mut by_guid = HashMap::new();
        by_guid.insert("abc-1".to_string(), ("page-1".to_string(), "Target".to_string()));
        assert!(relink(&mut doc, &by_guid));
        let p = &doc.content[0];
        assert_eq!(p.content.len(), 3);
        assert_eq!(p.content[1].kind, kinds::WIKI_LINK);
        assert_eq!(p.content[1].attr_str("pageId"), Some("page-1"));
        assert_eq!(p.content[1].attr_str("title"), Some("the target"));
        assert!(p.content[2].has_mark(marks::LINK), "unknown targets stay links");
    }
}
