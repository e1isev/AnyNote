//! The open folder format: export and import.
//!
//! ```text
//! <export>/
//!   <Notebook>/
//!     notebook.json                 id, title, colour, section order
//!     assets/<hash>.<ext>           images, attachments, ink-<id>.svg
//!     <Section>/
//!       section.json                id, title, colour, page order + hierarchy
//!       <Page>.md                   readable Markdown with YAML front matter
//!       <Page>.anynote.json         lossless sidecar: block tree + ink
//!       <Nested section>/...
//! ```
//!
//! Markdown is for people, git diffs and other tools; the sidecar keeps
//! everything Markdown cannot (ink, block IDs, page links). Import prefers the
//! sidecar and falls back to Markdown, so any folder of `.md` files (an
//! Obsidian vault, a docs repo) can be imported too.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assets::{self, extension_for_mime};
use crate::ink::{strokes_to_svg, Stroke};
use crate::markdown::{self, ExportContext};
use crate::node::{kinds, Node};
use crate::store::{Placement, SectionInfo, Store};
use crate::{Error, Result};

pub const SIDECAR_EXT: &str = "anynote.json";

#[derive(Debug, Serialize, Deserialize)]
struct NotebookManifest {
    id: String,
    title: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    sections: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SectionManifest {
    id: String,
    title: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    pages: Vec<PageEntry>,
    #[serde(default)]
    sections: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageEntry {
    id: String,
    file: String,
    title: String,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    created_at: Option<i64>,
    #[serde(default)]
    updated_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sidecar {
    format: String,
    version: u32,
    id: String,
    title: String,
    created_at: i64,
    updated_at: i64,
    doc: Node,
    #[serde(default)]
    ink: BTreeMap<String, Vec<Stroke>>,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub notebooks: usize,
    pub sections: usize,
    pub pages: usize,
    pub assets: usize,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub notebook_ids: Vec<String>,
    pub sections: usize,
    pub pages: usize,
    pub assets: usize,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// export

/// Export notebooks (all of them when `notebook_ids` is empty) into `out_dir`.
pub fn export(store: &mut Store, out_dir: &Path, notebook_ids: &[String]) -> Result<ExportSummary> {
    fs::create_dir_all(out_dir)?;
    let mut summary = ExportSummary::default();
    let mut used = HashSet::new();
    for nb in store.list_notebooks()? {
        if !notebook_ids.is_empty() && !notebook_ids.contains(&nb.id) {
            continue;
        }
        let dir = out_dir.join(unique_name(&mut used, &file_name(&nb.title)));
        fs::create_dir_all(&dir)?;
        let sections = store.list_sections(&nb.id)?;
        let mut ctx = NotebookExport { assets_dir: dir.join("assets"), written: HashSet::new() };
        let mut used_sections = HashSet::new();
        let mut order = Vec::new();
        for sec in sections.iter().filter(|s| s.parent_id.is_none()) {
            let name = unique_name(&mut used_sections, &file_name(&sec.title));
            export_section(store, sec, &sections, &dir.join(&name), 1, &mut ctx, &mut summary)?;
            order.push(name);
        }
        let manifest =
            NotebookManifest { id: nb.id.clone(), title: nb.title.clone(), color: nb.color.clone(), sections: order };
        write_json(&dir.join("notebook.json"), &manifest)?;
        summary.notebooks += 1;
        summary.assets += ctx.written.len();
    }
    Ok(summary)
}

struct NotebookExport {
    assets_dir: PathBuf,
    written: HashSet<String>,
}

fn export_section(
    store: &mut Store,
    section: &SectionInfo,
    all: &[SectionInfo],
    dir: &Path,
    depth: usize,
    ctx: &mut NotebookExport,
    summary: &mut ExportSummary,
) -> Result<()> {
    fs::create_dir_all(dir)?;
    let mut used = HashSet::from(["section.json".to_string(), "assets".to_string()]);
    let mut entries = Vec::new();
    for page in store.list_pages(&section.id)? {
        let (doc, ink) = store.page_content(&page.id)?;
        let base = unique_name(&mut used, &file_name(if page.title.is_empty() { "Untitled" } else { &page.title }));
        let rel_assets = format!("{}assets", "../".repeat(depth));

        // Copy referenced assets and ink renders into the notebook's assets folder.
        let mut page_ctx = PageExport { store, ctx, rel_assets: &rel_assets, ink: &ink, error: None };
        let mut body = format!("# {}\n\n", markdown::escape(&page.title));
        body.push_str(&markdown::to_markdown(&doc, &mut page_ctx));
        if let Some(e) = page_ctx.error.take() {
            return Err(e);
        }
        let front = format!(
            "---\nid: {}\ntitle: {}\ncreated: {}\nupdated: {}\n---\n\n",
            page.id,
            serde_json::to_string(&page.title)?,
            iso8601(page.created_at),
            iso8601(page.updated_at)
        );
        fs::write(dir.join(format!("{base}.md")), front + &body)?;
        let sidecar = Sidecar {
            format: "anynote-page".into(),
            version: 1,
            id: page.id.clone(),
            title: page.title.clone(),
            created_at: page.created_at,
            updated_at: page.updated_at,
            doc,
            ink,
        };
        write_json(&dir.join(format!("{base}.{SIDECAR_EXT}")), &sidecar)?;
        entries.push(PageEntry {
            id: page.id,
            file: format!("{base}.md"),
            title: page.title,
            parent_id: page.parent_id,
            created_at: Some(page.created_at),
            updated_at: Some(page.updated_at),
        });
        summary.pages += 1;
    }
    let mut children = Vec::new();
    for child in all.iter().filter(|s| s.parent_id.as_deref() == Some(section.id.as_str())) {
        let name = unique_name(&mut used, &file_name(&child.title));
        export_section(store, child, all, &dir.join(&name), depth + 1, ctx, summary)?;
        children.push(name);
    }
    let manifest = SectionManifest {
        id: section.id.clone(),
        title: section.title.clone(),
        color: section.color.clone(),
        pages: entries,
        sections: children,
    };
    write_json(&dir.join("section.json"), &manifest)?;
    summary.sections += 1;
    Ok(())
}

struct PageExport<'a> {
    store: &'a Store,
    ctx: &'a mut NotebookExport,
    rel_assets: &'a str,
    ink: &'a BTreeMap<String, Vec<Stroke>>,
    error: Option<Error>,
}

impl PageExport<'_> {
    fn write_asset(&mut self, hash: &str) -> Option<String> {
        let (bytes, mime) = match self.store.asset(hash) {
            Ok(a) => a,
            Err(Error::NotFound(_)) => return None,
            Err(e) => {
                self.error.get_or_insert(e);
                return None;
            }
        };
        let name = format!("{hash}.{}", extension_for_mime(&mime));
        if self.ctx.written.insert(name.clone()) {
            if let Err(e) = fs::create_dir_all(&self.ctx.assets_dir)
                .and_then(|_| fs::write(self.ctx.assets_dir.join(&name), &bytes))
            {
                self.error.get_or_insert(e.into());
            }
        }
        Some(format!("{}/{name}", self.rel_assets))
    }
}

impl ExportContext for PageExport<'_> {
    fn url(&mut self, url: &str) -> String {
        match assets::hash_from_url(url) {
            Some(hash) => self.write_asset(hash).unwrap_or_else(|| url.to_string()),
            None => url.to_string(),
        }
    }

    fn ink(&mut self, ink_id: &str) -> Option<String> {
        let strokes = self.ink.get(ink_id)?;
        let name = format!("ink-{}.svg", file_name(ink_id));
        if self.ctx.written.insert(name.clone()) {
            if let Err(e) = fs::create_dir_all(&self.ctx.assets_dir)
                .and_then(|_| fs::write(self.ctx.assets_dir.join(&name), strokes_to_svg(strokes)))
            {
                self.error.get_or_insert(e.into());
            }
        }
        Some(format!("![Ink drawing]({}/{name})", self.rel_assets))
    }
}

// ---------------------------------------------------------------------------
// import

/// Import a folder: an AnyNote export, a single exported notebook, or any
/// folder of Markdown files (subfolders become sections).
pub fn import(store: &mut Store, dir: &Path) -> Result<ImportSummary> {
    if !dir.is_dir() {
        return Err(Error::Invalid(format!("{} is not a folder", dir.display())));
    }
    let mut imp = Importer { store, summary: ImportSummary::default(), pages: Vec::new(), id_map: HashMap::new() };
    if dir.join("notebook.json").is_file() {
        imp.notebook(dir)?;
    } else {
        let subs = sorted_entries(dir)?;
        let notebooks: Vec<_> = subs.iter().filter(|p| p.join("notebook.json").is_file()).collect();
        if notebooks.is_empty() {
            imp.notebook(dir)?;
        } else {
            for nb in notebooks {
                imp.notebook(nb)?;
            }
        }
    }
    imp.resolve_links()?;
    Ok(imp.summary)
}

/// A page created during import, kept for the link-resolution pass.
struct ImportedPage {
    id: String,
    title: String,
    doc: Node,
    ink: BTreeMap<String, Vec<Stroke>>,
}

struct Importer<'a> {
    store: &'a mut Store,
    summary: ImportSummary,
    pages: Vec<ImportedPage>,
    /// Page ids in the source folder -> ids in the store.
    id_map: HashMap<String, String>,
}

impl Importer<'_> {
    fn notebook(&mut self, dir: &Path) -> Result<()> {
        let manifest: Option<NotebookManifest> = read_json(&dir.join("notebook.json"));
        let title = manifest.as_ref().map(|m| m.title.clone()).unwrap_or_else(|| dir_title(dir));
        let nb = self.store.create_notebook(&title, manifest.as_ref().and_then(|m| m.color.as_deref()))?;
        self.summary.notebook_ids.push(nb.id.clone());
        let assets_dir = dir.join("assets");

        let section_dirs = ordered_subdirs(dir, manifest.as_ref().map(|m| m.sections.as_slice()))?;
        // Loose Markdown files at the notebook root go into their own section.
        if md_files(dir)?.next().is_some() {
            let sec = self.store.create_section(&nb.id, None, &title)?;
            self.summary.sections += 1;
            self.pages_in(dir, &sec.id, None, &assets_dir)?;
        }
        for sub in section_dirs {
            self.section(&sub, &nb.id, None, &assets_dir)?;
        }
        Ok(())
    }

    fn section(&mut self, dir: &Path, notebook_id: &str, parent: Option<&str>, assets_dir: &Path) -> Result<()> {
        let manifest: Option<SectionManifest> = read_json(&dir.join("section.json"));
        let title = manifest.as_ref().map(|m| m.title.clone()).unwrap_or_else(|| dir_title(dir));
        let sec = self.store.create_section(notebook_id, parent, &title)?;
        if let Some(color) = manifest.as_ref().and_then(|m| m.color.as_deref()) {
            self.store.update_section(&sec.id, None, Some(Some(color)))?;
        }
        self.summary.sections += 1;
        self.pages_in(dir, &sec.id, manifest.as_ref(), assets_dir)?;
        for sub in ordered_subdirs(dir, manifest.as_ref().map(|m| m.sections.as_slice()))? {
            self.section(&sub, notebook_id, Some(&sec.id), assets_dir)?;
        }
        Ok(())
    }

    fn pages_in(
        &mut self,
        dir: &Path,
        section_id: &str,
        manifest: Option<&SectionManifest>,
        assets_dir: &Path,
    ) -> Result<()> {
        // Use the manifest order and hierarchy when present, then any extra files.
        let mut files: Vec<(PathBuf, Option<&PageEntry>)> = Vec::new();
        let mut seen = HashSet::new();
        if let Some(m) = manifest {
            for entry in &m.pages {
                let path = dir.join(&entry.file);
                if path.is_file() && seen.insert(path.clone()) {
                    files.push((path, Some(entry)));
                }
            }
        }
        for path in md_files(dir)? {
            if seen.insert(path.clone()) {
                files.push((path, None));
            }
        }
        for (path, entry) in files {
            match self.page(&path, section_id, entry, assets_dir) {
                Ok(()) => {}
                Err(e) => self.summary.warnings.push(format!("{}: {e}", path.display())),
            }
        }
        Ok(())
    }

    fn page(&mut self, path: &Path, section_id: &str, entry: Option<&PageEntry>, assets_dir: &Path) -> Result<()> {
        let source = fs::read_to_string(path)?;
        let (front, body) = split_front_matter(&source);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled").to_string();
        let sidecar_path = path.with_extension(SIDECAR_EXT);
        let sidecar: Option<Sidecar> = read_json(&sidecar_path);

        let (title, mut doc, ink, created, old_id) = if let Some(sc) = sidecar {
            (sc.title, sc.doc, sc.ink, Some(sc.created_at), Some(sc.id))
        } else {
            let title = front.get("title").cloned().or_else(|| entry.map(|e| e.title.clone())).unwrap_or(stem);
            let base = path.parent().unwrap_or(Path::new("."));
            let store = &mut *self.store;
            let summary = &mut self.summary;
            let mut resolve = |url: &str| -> Option<String> {
                if url.contains("://") || url.starts_with('#') || url.starts_with("mailto:") {
                    return None;
                }
                let rel = percent_decode(url);
                let file = base.join(&rel);
                let bytes = fs::read(&file).ok()?;
                let name = file.file_name().and_then(|n| n.to_str());
                let info = store.put_asset(&bytes, name, None).ok()?;
                summary.assets += 1;
                Some(info.url)
            };
            let mut doc = markdown::from_markdown(body, &mut resolve);
            // Exports start with "# <title>"; don't duplicate it in the body.
            if doc
                .content
                .first()
                .is_some_and(|n| n.is(kinds::HEADING) && n.attr_i64("level") == Some(1) && n.plain_text() == title)
            {
                doc.content.remove(0);
            }
            let created = front.get("created").and_then(|s| parse_iso8601(s)).or(entry.and_then(|e| e.created_at));
            let old_id = front.get("id").cloned().or_else(|| entry.map(|e| e.id.clone()));
            (title, doc, BTreeMap::new(), created, old_id)
        };

        // Sidecar docs reference assets by hash; copy them in from the export.
        let mut missing = Vec::new();
        doc.walk_mut(&mut |n| {
            for key in ["src", "href"] {
                if let Some(hash) = n.attr_str(key).and_then(assets::hash_from_url).map(str::to_string) {
                    missing.push(hash);
                }
            }
            for m in &n.marks {
                if let Some(hash) = m.attr_str("href").and_then(assets::hash_from_url) {
                    missing.push(hash.to_string());
                }
            }
        });
        for hash in missing {
            if self.store.asset_info(&hash).is_ok() {
                continue;
            }
            if let Some(file) = find_asset_file(assets_dir, &hash) {
                let bytes = fs::read(&file)?;
                self.store.put_asset(&bytes, file.file_name().and_then(|n| n.to_str()), None)?;
                self.summary.assets += 1;
            } else {
                self.summary.warnings.push(format!("{}: missing asset {hash}", path.display()));
            }
        }

        let parent = entry.and_then(|e| e.parent_id.as_ref()).and_then(|p| self.id_map.get(p)).map(String::as_str);
        let page =
            self.store.create_page_with_content(section_id, parent, &title, Placement::End, &doc, &ink, created)?;
        if let Some(old) = old_id {
            self.id_map.insert(old, page.id.clone());
        }
        self.pages.push(ImportedPage { id: page.id, title, doc, ink });
        self.summary.pages += 1;
        Ok(())
    }

    /// Point wiki links at the imported pages (by original id, else by title).
    fn resolve_links(&mut self) -> Result<()> {
        let by_title: HashMap<String, String> =
            self.pages.iter().map(|p| (p.title.to_lowercase(), p.id.clone())).collect();
        for ImportedPage { id: page_id, mut doc, ink, .. } in std::mem::take(&mut self.pages) {
            let mut changed = false;
            doc.walk_mut(&mut |n| {
                if !n.is(kinds::WIKI_LINK) {
                    return;
                }
                let current = n.attr_str("pageId").map(str::to_string);
                let resolved = current
                    .as_ref()
                    .and_then(|old| self.id_map.get(old))
                    .or_else(|| {
                        let key = n.attr_str("target").or(n.attr_str("title")).unwrap_or("").to_lowercase();
                        by_title.get(&key)
                    })
                    .cloned();
                if let Some(new_id) = resolved {
                    if current.as_deref() != Some(new_id.as_str()) {
                        n.attrs.insert("pageId".into(), Value::String(new_id));
                        changed = true;
                    }
                }
                if n.attrs.remove("target").is_some() {
                    changed = true;
                }
            });
            if changed {
                // Block ids were assigned at creation; keep them stable.
                let (current, _) = self.store.page_content(&page_id)?;
                copy_block_ids(&current, &mut doc);
                self.store.replace_page_content(&page_id, &doc, &ink)?;
            }
        }
        Ok(())
    }
}

fn copy_block_ids(from: &Node, to: &mut Node) {
    if from.kind == to.kind {
        if let Some(id) = from.attr("id") {
            to.attrs.entry("id").or_insert_with(|| id.clone());
        }
        for (a, b) in from.content.iter().zip(to.content.iter_mut()) {
            copy_block_ids(a, b);
        }
    }
}

fn find_asset_file(dir: &Path, hash: &str) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(hash))
}

// ---------------------------------------------------------------------------
// helpers

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut s = serde_json::to_string_pretty(value)?;
    s.push('\n');
    fs::write(path, s)?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)?.flatten().map(|e| e.path()).collect();
    v.sort();
    Ok(v)
}

fn md_files(dir: &Path) -> Result<impl Iterator<Item = PathBuf>> {
    Ok(sorted_entries(dir)?.into_iter().filter(|p| {
        p.is_file()
            && p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
    }))
}

/// Subfolders that are sections: manifest order first, then the rest by
/// name. Hidden folders and `assets` are skipped.
fn ordered_subdirs(dir: &Path, order: Option<&[String]>) -> Result<Vec<PathBuf>> {
    let all: Vec<PathBuf> = sorted_entries(dir)?
        .into_iter()
        .filter(|p| p.is_dir())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !name.starts_with('.') && name != "assets" && name != "node_modules"
        })
        .collect();
    let mut out = Vec::new();
    for name in order.unwrap_or(&[]) {
        let p = dir.join(name);
        if all.contains(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    for p in all {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

fn dir_title(dir: &Path) -> String {
    dir.file_name().and_then(|n| n.to_str()).unwrap_or("Imported").to_string()
}

/// A safe, readable file name.
pub fn file_name(title: &str) -> String {
    let cleaned: String =
        title
            .chars()
            .map(|c| {
                if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '-'
                } else {
                    c
                }
            })
            .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let mut name: String = trimmed.chars().take(80).collect();
    if name.is_empty() {
        name = "Untitled".into();
    }
    name
}

fn unique_name(used: &mut HashSet<String>, base: &str) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while !used.insert(name.to_lowercase()) {
        name = format!("{base} ({n})");
        n += 1;
    }
    name
}

/// Split YAML front matter (only simple `key: value` lines are read).
fn split_front_matter(source: &str) -> (HashMap<String, String>, &str) {
    let mut map = HashMap::new();
    let Some(rest) = source.strip_prefix("---\n").or_else(|| source.strip_prefix("---\r\n")) else {
        return (map, source);
    };
    let Some(end) = rest.find("\n---") else { return (map, source) };
    for line in rest[..end].lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            let v = serde_json::from_str::<String>(v).unwrap_or_else(|_| v.trim_matches('\'').to_string());
            map.insert(k.trim().to_string(), v);
        }
    }
    let after = &rest[end + 4..];
    let body = after.split_once('\n').map(|(_, b)| b).unwrap_or("");
    (map, body)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Milliseconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn iso8601(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn parse_iso8601(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = if s.len() >= 19 { (num(11, 13)?, num(14, 16)?, num(17, 19)?) } else { (0, 0, 0) };
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + hh * 3600 + mm * 60 + ss) * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink::Stroke;
    use crate::node::Mark;

    #[test]
    fn dates_round_trip() {
        for ms in [0, 1_700_000_000_000, 951_782_400_000 /* 2000-02-29 */] {
            assert_eq!(parse_iso8601(&iso8601(ms)), Some(ms));
        }
        assert_eq!(iso8601(951_782_400_000), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn export_then_import_is_lossless() {
        let src_dir = tempfile::tempdir().unwrap();
        let mut src = Store::open(src_dir.path()).unwrap();
        let nb = src.create_notebook("Research", Some("#3366ff")).unwrap();
        let sec = src.create_section(&nb.id, None, "Papers").unwrap();
        let sub = src.create_section(&nb.id, Some(&sec.id), "2026").unwrap();
        let target = src.create_page(&sec.id, None, "Reading list", Placement::End).unwrap();
        let png = src.put_asset(&[0x89, b'P', b'N', b'G', 0, 1, 2], Some("fig.png"), None).unwrap();
        let mut ink = BTreeMap::new();
        ink.insert(
            "ink-a".to_string(),
            vec![Stroke {
                id: "s".into(),
                tool: "pen".into(),
                color: "#000".into(),
                size: 2.0,
                points: vec![[1.0, 2.0, 0.5], [30.0, 40.0, 0.5]],
            }],
        );
        let doc = Node::doc(vec![
            Node::paragraph(vec![
                Node::styled_text("Key", vec![Mark::new("bold")]),
                Node::text(" result, see "),
                Node::new(kinds::WIKI_LINK).with_attr("pageId", target.id.as_str()).with_attr("title", "Reading list"),
            ]),
            Node::new(kinds::IMAGE).with_attr("src", png.url.as_str()).with_attr("alt", "Figure 1"),
            Node::new(kinds::INK_BLOCK).with_attr("inkId", "ink-a").with_attr("height", 200),
        ]);
        let page =
            src.create_page_with_content(&sub.id, None, "Notes: day/1", Placement::End, &doc, &ink, None).unwrap();
        src.create_page(&sec.id, Some(&target.id), "Child", Placement::End).unwrap();

        let out = tempfile::tempdir().unwrap();
        let summary = export(&mut src, out.path(), &[]).unwrap();
        assert_eq!((summary.notebooks, summary.sections, summary.pages), (1, 2, 3));
        let md_path = out.path().join("Research/Papers/2026/Notes- day-1.md");
        let md = fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("**Key** result, see [[Reading list]]"), "{md}");
        assert!(md.contains(&format!("![Figure 1](../../assets/{}.png)", png.hash)), "{md}");
        assert!(md.contains("![Ink drawing](../../assets/ink-ink-a.svg)"), "{md}");
        assert!(out.path().join("Research/assets/ink-ink-a.svg").is_file());

        let dst_dir = tempfile::tempdir().unwrap();
        let mut dst = Store::open(dst_dir.path()).unwrap();
        let summary = import(&mut dst, out.path()).unwrap();
        assert!(summary.warnings.is_empty(), "{:?}", summary.warnings);
        assert_eq!(summary.pages, 3);
        let nb2 = &dst.list_notebooks().unwrap()[0];
        assert_eq!(nb2.title, "Research");
        assert_eq!(nb2.color.as_deref(), Some("#3366ff"));
        let secs = dst.list_sections(&nb2.id).unwrap();
        assert_eq!(secs.len(), 2);
        let papers = secs.iter().find(|s| s.title == "Papers").unwrap();
        let pages = dst.list_pages(&papers.id).unwrap();
        let reading = pages.iter().find(|p| p.title == "Reading list").unwrap();
        let child = pages.iter().find(|p| p.title == "Child").unwrap();
        assert_eq!(child.parent_id.as_deref(), Some(reading.id.as_str()));

        let notes = dst.find_pages("Notes", 1).unwrap().remove(0);
        let (doc2, ink2) = dst.page_content(&notes.id).unwrap();
        assert_eq!(ink2, ink);
        let mut link = None;
        doc2.walk(&mut |n| {
            if n.is(kinds::WIKI_LINK) {
                link = n.attr_str("pageId").map(str::to_string);
            }
        });
        assert_eq!(link.as_deref(), Some(reading.id.as_str()), "link re-pointed to the imported page");
        assert_eq!(dst.backlinks(&reading.id).unwrap()[0].id, notes.id);
        assert_eq!(dst.asset(&png.hash).unwrap().0, [0x89, b'P', b'N', b'G', 0, 1, 2]);
        let _ = page;
    }

    #[test]
    fn imports_plain_markdown_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("Ideas")).unwrap();
        fs::write(dir.path().join("Ideas/First.md"), "# First\n\nSee [[Second]] ![pic](img%201.png)\n").unwrap();
        fs::write(dir.path().join("Ideas/img 1.png"), [0x89, b'P', b'N', b'G']).unwrap();
        fs::write(dir.path().join("Ideas/Second.md"), "---\ntitle: \"Second\"\n---\nBody with #tag\n").unwrap();
        fs::write(dir.path().join("Loose.md"), "loose note").unwrap();

        let store_dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(store_dir.path()).unwrap();
        let summary = import(&mut store, dir.path()).unwrap();
        assert_eq!(summary.pages, 3);
        assert_eq!(summary.sections, 2);
        let first = store.find_pages("First", 1).unwrap().remove(0);
        let second = store.find_pages("Second", 1).unwrap().remove(0);
        assert_eq!(store.backlinks(&second.id).unwrap()[0].id, first.id);
        let (doc, _) = store.page_content(&first.id).unwrap();
        assert!(!doc.content.iter().any(|n| n.is(kinds::HEADING)), "duplicate title heading removed");
        let image = doc.content.iter().find(|n| n.is(kinds::IMAGE)).unwrap();
        assert!(image.attr_str("src").unwrap().starts_with(assets::ASSET_URL_PREFIX));
        assert_eq!(store.pages_with_tag("tag").unwrap()[0].id, second.id);
    }
}
