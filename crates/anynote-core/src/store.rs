//! The local store: one SQLite database plus a content-addressed asset folder.
//!
//! ```text
//! <root>/anynote.db      catalogue, CRDT update log, versions, search index
//! <root>/assets/ab/<hash> binary assets
//! ```
//!
//! Page bodies are CRDT documents. Every edit arrives as a binary Yjs update
//! that is appended to `page_updates`; the log is compacted into a single
//! snapshot once it grows, and periodic snapshots go to `page_versions` for
//! version history. After each change the derived index (full-text search,
//! links, tasks, tags) is refreshed from the merged document.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use yrs::Doc;

use crate::assets::{self, AssetStore};
use crate::derive::derive;
use crate::ids::{new_id, now_ms};
use crate::ink::Stroke;
use crate::node::Node;
use crate::{ydoc, Error, Result};

/// Compact a page's update log into one snapshot after this many entries.
const COMPACT_AFTER: i64 = 200;
/// Take an automatic version snapshot at most this often while editing.
const VERSION_INTERVAL_MS: i64 = 10 * 60 * 1000;
/// Keep at most this many parsed documents in memory.
const DOC_CACHE_SIZE: usize = 32;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS notebooks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    color TEXT,
    position REAL NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE TABLE IF NOT EXISTS sections (
    id TEXT PRIMARY KEY,
    notebook_id TEXT NOT NULL REFERENCES notebooks(id) ON DELETE CASCADE,
    parent_id TEXT REFERENCES sections(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    color TEXT,
    position REAL NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE INDEX IF NOT EXISTS sections_notebook ON sections(notebook_id);

CREATE TABLE IF NOT EXISTS pages (
    id TEXT PRIMARY KEY,
    section_id TEXT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
    parent_id TEXT REFERENCES pages(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    position REAL NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE INDEX IF NOT EXISTS pages_section ON pages(section_id);

CREATE TABLE IF NOT EXISTS page_updates (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    data BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS page_updates_page ON page_updates(page_id, seq);

CREATE TABLE IF NOT EXISTS page_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    label TEXT,
    state BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS page_versions_page ON page_versions(page_id, created_at);

CREATE TABLE IF NOT EXISTS links (
    src TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    dst TEXT NOT NULL,
    PRIMARY KEY (src, dst)
);
CREATE INDEX IF NOT EXISTS links_dst ON links(dst);

CREATE TABLE IF NOT EXISTS tasks (
    page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    block_id TEXT NOT NULL,
    ord INTEGER NOT NULL,
    text TEXT NOT NULL,
    checked INTEGER NOT NULL,
    PRIMARY KEY (page_id, block_id)
);

CREATE TABLE IF NOT EXISTS tags (
    page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (page_id, tag)
);
CREATE INDEX IF NOT EXISTS tags_tag ON tags(tag);

CREATE TABLE IF NOT EXISTS assets (
    hash TEXT PRIMARY KEY,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    name TEXT,
    created_at INTEGER NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS page_fts USING fts5(
    page_id UNINDEXED, title, body, tokenize = 'unicode61 remove_diacritics 2'
);
"#;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotebookInfo {
    pub id: String,
    pub title: String,
    pub color: Option<String>,
    pub position: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SectionInfo {
    pub id: String,
    pub notebook_id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub color: Option<String>,
    pub position: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub id: String,
    pub section_id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub position: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A page with its location, for lists that span notebooks.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageRef {
    pub id: String,
    pub title: String,
    pub section_id: String,
    pub section_title: String,
    pub notebook_id: String,
    pub notebook_title: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub page: PageRef,
    /// Matching excerpt; matches are wrapped in `\u{1}` … `\u{2}`.
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pub page: PageRef,
    pub block_id: String,
    pub text: String,
    pub checked: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub id: i64,
    pub page_id: String,
    pub created_at: i64,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    pub hash: String,
    pub url: String,
    pub mime: String,
    pub size: i64,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub deleted_at: i64,
}

/// Where to put a new or moved item among its siblings.
#[derive(Debug, Clone, Default)]
pub enum Placement {
    #[default]
    End,
    Before(String),
    After(String),
}

pub struct Store {
    conn: Connection,
    root: PathBuf,
    assets: AssetStore,
    docs: HashMap<String, Doc>,
    doc_lru: Vec<String>,
}

impl Store {
    /// Open (or create) a store in `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Store> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        let conn = Connection::open(root.join("anynote.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute("INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1')", [])?;
        let assets = AssetStore::new(root.join("assets"))?;
        Ok(Store { conn, root, assets, docs: HashMap::new(), doc_lru: Vec::new() })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn assets(&self) -> &AssetStore {
        &self.assets
    }

    // -----------------------------------------------------------------------
    // notebooks

    pub fn create_notebook(&mut self, title: &str, color: Option<&str>) -> Result<NotebookInfo> {
        let id = new_id();
        let now = now_ms();
        let position = next_position(&self.conn, "SELECT MAX(position) FROM notebooks WHERE deleted_at IS NULL", [])?;
        self.conn.execute(
            "INSERT INTO notebooks (id, title, color, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, clean_title(title, "Untitled notebook"), color, position, now],
        )?;
        self.notebook(&id)
    }

    pub fn notebook(&self, id: &str) -> Result<NotebookInfo> {
        self.conn
            .query_row(
                "SELECT id, title, color, position, created_at, updated_at FROM notebooks WHERE id = ?1",
                [id],
                notebook_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("notebook {id}")))
    }

    pub fn list_notebooks(&self) -> Result<Vec<NotebookInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, color, position, created_at, updated_at FROM notebooks
             WHERE deleted_at IS NULL ORDER BY position",
        )?;
        let rows = stmt.query_map([], notebook_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn update_notebook(
        &mut self,
        id: &str,
        title: Option<&str>,
        color: Option<Option<&str>>,
    ) -> Result<NotebookInfo> {
        let now = now_ms();
        if let Some(title) = title {
            self.conn.execute(
                "UPDATE notebooks SET title = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, clean_title(title, "Untitled notebook"), now],
            )?;
        }
        if let Some(color) = color {
            self.conn
                .execute("UPDATE notebooks SET color = ?2, updated_at = ?3 WHERE id = ?1", params![id, color, now])?;
        }
        self.notebook(id)
    }

    pub fn move_notebook(&mut self, id: &str, placement: Placement) -> Result<()> {
        let position = place(&self.conn, "notebooks", "deleted_at IS NULL", &[], id, &placement)?;
        self.conn.execute("UPDATE notebooks SET position = ?2 WHERE id = ?1", params![id, position])?;
        Ok(())
    }

    pub fn delete_notebook(&mut self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE notebooks SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, now_ms()],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // sections

    pub fn create_section(&mut self, notebook_id: &str, parent_id: Option<&str>, title: &str) -> Result<SectionInfo> {
        self.notebook(notebook_id)?;
        let id = new_id();
        let now = now_ms();
        let position = next_position(
            &self.conn,
            "SELECT MAX(position) FROM sections WHERE notebook_id = ?1 AND parent_id IS ?2 AND deleted_at IS NULL",
            params![notebook_id, parent_id],
        )?;
        self.conn.execute(
            "INSERT INTO sections (id, notebook_id, parent_id, title, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, notebook_id, parent_id, clean_title(title, "Untitled section"), position, now],
        )?;
        self.section(&id)
    }

    pub fn section(&self, id: &str) -> Result<SectionInfo> {
        self.conn
            .query_row(
                "SELECT id, notebook_id, parent_id, title, color, position, created_at, updated_at FROM sections WHERE id = ?1",
                [id],
                section_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("section {id}")))
    }

    /// All live sections of a notebook (nested sections included), in order.
    pub fn list_sections(&self, notebook_id: &str) -> Result<Vec<SectionInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, notebook_id, parent_id, title, color, position, created_at, updated_at FROM sections
             WHERE notebook_id = ?1 AND deleted_at IS NULL ORDER BY position",
        )?;
        let rows = stmt.query_map([notebook_id], section_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn update_section(
        &mut self,
        id: &str,
        title: Option<&str>,
        color: Option<Option<&str>>,
    ) -> Result<SectionInfo> {
        let now = now_ms();
        if let Some(title) = title {
            self.conn.execute(
                "UPDATE sections SET title = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, clean_title(title, "Untitled section"), now],
            )?;
        }
        if let Some(color) = color {
            self.conn
                .execute("UPDATE sections SET color = ?2, updated_at = ?3 WHERE id = ?1", params![id, color, now])?;
        }
        self.section(id)
    }

    pub fn move_section(
        &mut self,
        id: &str,
        notebook_id: &str,
        parent_id: Option<&str>,
        placement: Placement,
    ) -> Result<()> {
        if let Some(parent) = parent_id {
            // Refuse to move a section inside itself.
            let mut cursor = Some(parent.to_string());
            while let Some(p) = cursor {
                if p == id {
                    return Err(Error::Invalid("cannot move a section into itself".into()));
                }
                cursor = self.section(&p)?.parent_id;
            }
        }
        let position = place(
            &self.conn,
            "sections",
            "notebook_id = ?1 AND parent_id IS ?2 AND deleted_at IS NULL",
            &[&notebook_id, &parent_id],
            id,
            &placement,
        )?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE sections SET notebook_id = ?2, parent_id = ?3, position = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, notebook_id, parent_id, position, now_ms()],
        )?;
        // Nested sections follow their parent into the new notebook.
        tx.execute(
            "WITH RECURSIVE sub(id) AS (SELECT id FROM sections WHERE parent_id = ?1
                 UNION ALL SELECT s.id FROM sections s JOIN sub ON s.parent_id = sub.id)
             UPDATE sections SET notebook_id = ?2 WHERE id IN (SELECT id FROM sub)",
            params![id, notebook_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_section(&mut self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sections SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, now_ms()],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // pages

    /// Create an empty page.
    pub fn create_page(
        &mut self,
        section_id: &str,
        parent_id: Option<&str>,
        title: &str,
        placement: Placement,
    ) -> Result<PageInfo> {
        self.create_page_with_content(
            section_id,
            parent_id,
            title,
            placement,
            &Node::empty_doc(),
            &BTreeMap::new(),
            None,
        )
    }

    /// Create a page from existing content. Importers use this; `created_at`
    /// lets them keep the original timestamps.
    #[allow(clippy::too_many_arguments)]
    pub fn create_page_with_content(
        &mut self,
        section_id: &str,
        parent_id: Option<&str>,
        title: &str,
        placement: Placement,
        content: &Node,
        ink: &BTreeMap<String, Vec<Stroke>>,
        created_at: Option<i64>,
    ) -> Result<PageInfo> {
        self.section(section_id)?;
        let id = new_id();
        let now = now_ms();
        let created = created_at.unwrap_or(now);
        let position = match placement {
            Placement::End => next_position(
                &self.conn,
                "SELECT MAX(position) FROM pages WHERE section_id = ?1 AND deleted_at IS NULL",
                [section_id],
            )?,
            ref p => place(&self.conn, "pages", "section_id = ?1 AND deleted_at IS NULL", &[&section_id], &id, p)?,
        };
        let mut content = content.clone();
        content.ensure_block_ids();
        if content.content.is_empty() {
            content = Node::empty_doc();
            content.ensure_block_ids();
        }
        let doc = ydoc::doc_from_content(&content, ink);
        let state = ydoc::encode_state(&doc);
        let title = clean_title(title, "");

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO pages (id, section_id, parent_id, title, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, section_id, parent_id, title, position, created, created],
        )?;
        tx.execute(
            "INSERT INTO page_updates (page_id, data, created_at) VALUES (?1, ?2, ?3)",
            params![id, state, now],
        )?;
        tx.execute("INSERT INTO page_fts (page_id, title, body) VALUES (?1, ?2, '')", params![id, title])?;
        reindex(&tx, &id, &content)?;
        tx.commit()?;
        self.cache_doc(&id, doc);
        self.page(&id)
    }

    pub fn page(&self, id: &str) -> Result<PageInfo> {
        self.conn
            .query_row(
                "SELECT id, section_id, parent_id, title, position, created_at, updated_at FROM pages WHERE id = ?1",
                [id],
                page_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("page {id}")))
    }

    /// Live pages of a section in display order. Subpages carry `parent_id`.
    pub fn list_pages(&self, section_id: &str) -> Result<Vec<PageInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, section_id, parent_id, title, position, created_at, updated_at FROM pages
             WHERE section_id = ?1 AND deleted_at IS NULL ORDER BY position",
        )?;
        let rows = stmt.query_map([section_id], page_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn page_ref(&self, id: &str) -> Result<PageRef> {
        self.conn
            .query_row(&format!("{PAGE_REF_SELECT} WHERE p.id = ?1"), [id], page_ref_from_row)
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("page {id}")))
    }

    pub fn rename_page(&mut self, id: &str, title: &str) -> Result<PageInfo> {
        let title = clean_title(title, "");
        let tx = self.conn.transaction()?;
        let n =
            tx.execute("UPDATE pages SET title = ?2, updated_at = ?3 WHERE id = ?1", params![id, title, now_ms()])?;
        if n == 0 {
            return Err(Error::NotFound(format!("page {id}")));
        }
        tx.execute("UPDATE page_fts SET title = ?2 WHERE page_id = ?1", params![id, title])?;
        tx.commit()?;
        self.page(id)
    }

    /// Move a page (and its subpages) to `section_id` under `parent_id`.
    pub fn move_page(
        &mut self,
        id: &str,
        section_id: &str,
        parent_id: Option<&str>,
        placement: Placement,
    ) -> Result<()> {
        self.section(section_id)?;
        if let Some(parent) = parent_id {
            let mut cursor = Some(parent.to_string());
            while let Some(p) = cursor {
                if p == id {
                    return Err(Error::Invalid("cannot make a page its own subpage".into()));
                }
                cursor = self.page(&p)?.parent_id;
            }
        }
        let descendants = self.descendant_pages(id)?;
        let position =
            place(&self.conn, "pages", "section_id = ?1 AND deleted_at IS NULL", &[&section_id], id, &placement)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE pages SET section_id = ?2, parent_id = ?3, position = ?4 WHERE id = ?1",
            params![id, section_id, parent_id, position],
        )?;
        // Subpages travel with their parent and stay directly after it.
        for (i, child) in descendants.iter().enumerate() {
            tx.execute(
                "UPDATE pages SET section_id = ?2, position = ?3 WHERE id = ?1",
                params![child, section_id, position + (i as f64 + 1.0) * 1e-6],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn descendant_pages(&self, id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE sub(id, position) AS (
                 SELECT id, position FROM pages WHERE parent_id = ?1 AND deleted_at IS NULL
                 UNION ALL SELECT p.id, p.position FROM pages p JOIN sub ON p.parent_id = sub.id WHERE p.deleted_at IS NULL)
             SELECT id FROM sub ORDER BY position",
        )?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Move a page and its subpages to the trash.
    pub fn delete_page(&mut self, id: &str) -> Result<()> {
        let mut ids = self.descendant_pages(id)?;
        ids.push(id.to_string());
        let now = now_ms();
        let tx = self.conn.transaction()?;
        for pid in &ids {
            tx.execute("UPDATE pages SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![pid, now])?;
        }
        tx.commit()?;
        for pid in ids {
            self.evict_doc(&pid);
        }
        Ok(())
    }

    pub fn list_trash(&self) -> Result<Vec<TrashItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT 'notebook', id, title, deleted_at FROM notebooks WHERE deleted_at IS NOT NULL
             UNION ALL SELECT 'section', id, title, deleted_at FROM sections WHERE deleted_at IS NOT NULL
             UNION ALL SELECT 'page', id, title, deleted_at FROM pages WHERE deleted_at IS NOT NULL
             ORDER BY 4 DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(TrashItem { kind: r.get(0)?, id: r.get(1)?, title: r.get(2)?, deleted_at: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Restore a trashed item together with the containers it needs.
    pub fn restore(&mut self, kind: &str, id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        match kind {
            "notebook" => {
                tx.execute("UPDATE notebooks SET deleted_at = NULL WHERE id = ?1", [id])?;
            }
            "section" => {
                tx.execute(
                    "WITH RECURSIVE up(id, parent_id) AS (SELECT id, parent_id FROM sections WHERE id = ?1
                         UNION ALL SELECT s.id, s.parent_id FROM sections s JOIN up ON s.id = up.parent_id)
                     UPDATE sections SET deleted_at = NULL WHERE id IN (SELECT id FROM up)",
                    [id],
                )?;
                tx.execute(
                    "UPDATE notebooks SET deleted_at = NULL WHERE id = (SELECT notebook_id FROM sections WHERE id = ?1)",
                    [id],
                )?;
            }
            "page" => {
                let deleted_at: Option<i64> = tx
                    .query_row("SELECT deleted_at FROM pages WHERE id = ?1", [id], |r| r.get(0))
                    .optional()?
                    .flatten();
                // Bring back the subpages that were trashed together with this page.
                tx.execute(
                    "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL SELECT p.id FROM pages p JOIN sub ON p.parent_id = sub.id)
                     UPDATE pages SET deleted_at = NULL WHERE id IN (SELECT id FROM sub) AND deleted_at IS ?2",
                    params![id, deleted_at],
                )?;
                // A trashed parent page would hide this one; detach instead.
                tx.execute(
                    "UPDATE pages SET parent_id = NULL WHERE id = ?1
                     AND parent_id IN (SELECT id FROM pages WHERE deleted_at IS NOT NULL)",
                    [id],
                )?;
                let section: String = tx.query_row("SELECT section_id FROM pages WHERE id = ?1", [id], |r| r.get(0))?;
                tx.execute(
                    "WITH RECURSIVE up(id, parent_id) AS (SELECT id, parent_id FROM sections WHERE id = ?1
                         UNION ALL SELECT s.id, s.parent_id FROM sections s JOIN up ON s.id = up.parent_id)
                     UPDATE sections SET deleted_at = NULL WHERE id IN (SELECT id FROM up)",
                    [&section],
                )?;
                tx.execute(
                    "UPDATE notebooks SET deleted_at = NULL WHERE id = (SELECT notebook_id FROM sections WHERE id = ?1)",
                    [&section],
                )?;
            }
            other => return Err(Error::Invalid(format!("unknown kind {other}"))),
        }
        tx.commit()?;
        Ok(())
    }

    /// Permanently delete everything in the trash.
    pub fn empty_trash(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        let doomed_pages =
            "SELECT p.id FROM pages p JOIN sections s ON s.id = p.section_id JOIN notebooks n ON n.id = s.notebook_id
             WHERE p.deleted_at IS NOT NULL OR s.deleted_at IS NOT NULL OR n.deleted_at IS NOT NULL";
        tx.execute(&format!("DELETE FROM page_fts WHERE page_id IN ({doomed_pages})"), [])?;
        tx.execute(&format!("DELETE FROM links WHERE dst IN ({doomed_pages})"), [])?;
        tx.execute(&format!("DELETE FROM pages WHERE id IN ({doomed_pages})"), [])?;
        tx.execute("DELETE FROM sections WHERE deleted_at IS NOT NULL", [])?;
        tx.execute("DELETE FROM notebooks WHERE deleted_at IS NOT NULL", [])?;
        tx.commit()?;
        self.docs.clear();
        self.doc_lru.clear();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // page content (CRDT)

    /// The page's full CRDT state as one Yjs update.
    pub fn page_state(&mut self, id: &str) -> Result<Vec<u8>> {
        let doc = self.load_doc(id)?;
        Ok(ydoc::encode_state(&doc))
    }

    pub fn page_content(&mut self, id: &str) -> Result<(Node, BTreeMap<String, Vec<Stroke>>)> {
        let doc = self.load_doc(id)?;
        Ok((ydoc::read_content(&doc), ydoc::read_ink(&doc)))
    }

    /// Apply an update coming from an editor (or another replica).
    pub fn apply_update(&mut self, id: &str, update: &[u8]) -> Result<()> {
        let doc = self.load_doc(id)?;
        ydoc::apply_update(&doc, update)?;
        self.persist_update(id, &doc, update)
    }

    /// Mark a task done/undone from outside the editor (e.g. the task view).
    /// Returns the update to forward to open editors.
    pub fn set_task_checked(&mut self, page_id: &str, block_id: &str, checked: bool) -> Result<Vec<u8>> {
        let doc = self.load_doc(page_id)?;
        let update = ydoc::set_block_attr(&doc, block_id, "checked", serde_json::Value::Bool(checked))
            .ok_or_else(|| Error::NotFound(format!("task {block_id}")))?;
        self.persist_update(page_id, &doc, &update)?;
        Ok(update)
    }

    /// Replace a page's content (body and ink). Returns the update to forward
    /// to open editors.
    pub fn replace_page_content(
        &mut self,
        page_id: &str,
        content: &Node,
        ink: &BTreeMap<String, Vec<Stroke>>,
    ) -> Result<Vec<u8>> {
        let doc = self.load_doc(page_id)?;
        let update = ydoc::replace_all(&doc, content, ink);
        self.persist_update(page_id, &doc, &update)?;
        Ok(update)
    }

    fn persist_update(&mut self, id: &str, doc: &Doc, update: &[u8]) -> Result<()> {
        let content = ydoc::read_content(doc);
        let now = now_ms();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO page_updates (page_id, data, created_at) VALUES (?1, ?2, ?3)",
            params![id, update, now],
        )?;
        tx.execute("UPDATE pages SET updated_at = ?2 WHERE id = ?1", params![id, now])?;
        reindex(&tx, id, &content)?;

        let count: i64 = tx.query_row("SELECT COUNT(*) FROM page_updates WHERE page_id = ?1", [id], |r| r.get(0))?;
        let last_version: Option<i64> =
            tx.query_row("SELECT MAX(created_at) FROM page_versions WHERE page_id = ?1", [id], |r| r.get(0))?;
        let needs_version = last_version.is_none_or(|t| now - t >= VERSION_INTERVAL_MS);
        if count > COMPACT_AFTER || needs_version {
            let state = ydoc::encode_state(doc);
            if count > COMPACT_AFTER {
                tx.execute("DELETE FROM page_updates WHERE page_id = ?1", [id])?;
                tx.execute(
                    "INSERT INTO page_updates (page_id, data, created_at) VALUES (?1, ?2, ?3)",
                    params![id, state, now],
                )?;
            }
            if needs_version {
                tx.execute(
                    "INSERT INTO page_versions (page_id, created_at, state) VALUES (?1, ?2, ?3)",
                    params![id, now, state],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Record a named snapshot of the page right now.
    pub fn snapshot_version(&mut self, page_id: &str, label: Option<&str>) -> Result<VersionInfo> {
        let doc = self.load_doc(page_id)?;
        let state = ydoc::encode_state(&doc);
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO page_versions (page_id, created_at, label, state) VALUES (?1, ?2, ?3, ?4)",
            params![page_id, now, label, state],
        )?;
        Ok(VersionInfo {
            id: self.conn.last_insert_rowid(),
            page_id: page_id.into(),
            created_at: now,
            label: label.map(Into::into),
        })
    }

    pub fn list_versions(&self, page_id: &str) -> Result<Vec<VersionInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, page_id, created_at, label FROM page_versions WHERE page_id = ?1 ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([page_id], |r| {
            Ok(VersionInfo { id: r.get(0)?, page_id: r.get(1)?, created_at: r.get(2)?, label: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn version_content(&self, version_id: i64) -> Result<(String, Node, BTreeMap<String, Vec<Stroke>>)> {
        let (page_id, state): (String, Vec<u8>) = self
            .conn
            .query_row("SELECT page_id, state FROM page_versions WHERE id = ?1", [version_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("version {version_id}")))?;
        let doc = ydoc::doc_from_state(&state)?;
        Ok((page_id, ydoc::read_content(&doc), ydoc::read_ink(&doc)))
    }

    /// Restore a page to an earlier version. The current state is snapshotted
    /// first, so a restore can itself be undone.
    pub fn restore_version(&mut self, version_id: i64) -> Result<(String, Vec<u8>)> {
        let (page_id, content, ink) = self.version_content(version_id)?;
        self.snapshot_version(&page_id, Some("Before restore"))?;
        let update = self.replace_page_content(&page_id, &content, &ink)?;
        Ok((page_id, update))
    }

    fn load_doc(&mut self, id: &str) -> Result<Doc> {
        if let Some(doc) = self.docs.get(id) {
            let doc = doc.clone();
            self.touch_doc(id);
            return Ok(doc);
        }
        let mut stmt = self.conn.prepare_cached("SELECT data FROM page_updates WHERE page_id = ?1 ORDER BY seq")?;
        let updates: Vec<Vec<u8>> = stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        if updates.is_empty() {
            self.page(id)?; // NotFound for unknown pages
        }
        let doc = ydoc::new_doc();
        for update in &updates {
            ydoc::apply_update(&doc, update)?;
        }
        self.cache_doc(id, doc.clone());
        Ok(doc)
    }

    fn cache_doc(&mut self, id: &str, doc: Doc) {
        self.docs.insert(id.to_string(), doc);
        self.touch_doc(id);
        while self.doc_lru.len() > DOC_CACHE_SIZE {
            let old = self.doc_lru.remove(0);
            self.docs.remove(&old);
        }
    }

    fn touch_doc(&mut self, id: &str) {
        self.doc_lru.retain(|x| x != id);
        self.doc_lru.push(id.to_string());
    }

    fn evict_doc(&mut self, id: &str) {
        self.docs.remove(id);
        self.doc_lru.retain(|x| x != id);
    }

    // -----------------------------------------------------------------------
    // queries

    /// Full-text search over titles and bodies. Every word must match; the
    /// last word matches as a prefix so results appear while typing.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(fts) = fts_query(query) else { return Ok(Vec::new()) };
        let sql = format!(
            "SELECT p.id, p.title, s.id, s.title, n.id, n.title, p.updated_at,
                    snippet(page_fts, 2, char(1), char(2), '…', 16)
             FROM page_fts
             JOIN pages p ON p.id = page_fts.page_id
             JOIN sections s ON s.id = p.section_id
             JOIN notebooks n ON n.id = s.notebook_id
             WHERE page_fts MATCH ?1 AND {LIVE}
             ORDER BY bm25(page_fts, 0.0, 8.0, 1.0)
             LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![fts, limit as i64], |r| {
            Ok(SearchHit { page: page_ref_from_row(r)?, snippet: r.get(7)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Pages whose title contains `query` (for `[[` link completion).
    pub fn find_pages(&self, query: &str, limit: usize) -> Result<Vec<PageRef>> {
        let pattern = format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let sql = format!(
            "{PAGE_REF_SELECT} WHERE p.title LIKE ?1 ESCAPE '\\' AND {LIVE}
             ORDER BY (p.title LIKE ?2 ESCAPE '\\') DESC, p.updated_at DESC LIMIT ?3"
        );
        let prefix = format!("{}%", &pattern[1..]);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, prefix, limit as i64], page_ref_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn recent_pages(&self, limit: usize) -> Result<Vec<PageRef>> {
        let sql = format!("{PAGE_REF_SELECT} WHERE {LIVE} ORDER BY p.updated_at DESC LIMIT ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([limit as i64], page_ref_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Pages that link to `page_id`.
    pub fn backlinks(&self, page_id: &str) -> Result<Vec<PageRef>> {
        let sql = format!(
            "{PAGE_REF_SELECT} JOIN links l ON l.src = p.id WHERE l.dst = ?1 AND {LIVE} ORDER BY p.updated_at DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([page_id], page_ref_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Tasks across all notebooks; open ones first, then by page.
    pub fn tasks(&self, include_done: bool) -> Result<Vec<TaskRow>> {
        let sql = format!(
            "SELECT p.id, p.title, s.id, s.title, n.id, n.title, p.updated_at, t.block_id, t.text, t.checked
             FROM tasks t
             JOIN pages p ON p.id = t.page_id
             JOIN sections s ON s.id = p.section_id
             JOIN notebooks n ON n.id = s.notebook_id
             WHERE {LIVE} AND (?1 OR t.checked = 0)
             ORDER BY t.checked, p.updated_at DESC, t.ord"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([include_done], |r| {
            Ok(TaskRow { page: page_ref_from_row(r)?, block_id: r.get(7)?, text: r.get(8)?, checked: r.get(9)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn tags(&self) -> Result<Vec<TagCount>> {
        let sql = format!(
            "SELECT t.tag, COUNT(*) FROM tags t
             JOIN pages p ON p.id = t.page_id
             JOIN sections s ON s.id = p.section_id
             JOIN notebooks n ON n.id = s.notebook_id
             WHERE {LIVE} GROUP BY t.tag ORDER BY t.tag"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| Ok(TagCount { tag: r.get(0)?, count: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn pages_with_tag(&self, tag: &str) -> Result<Vec<PageRef>> {
        let sql = format!(
            "{PAGE_REF_SELECT} JOIN tags t ON t.page_id = p.id WHERE t.tag = ?1 AND {LIVE} ORDER BY p.updated_at DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([tag.trim_start_matches('#').to_lowercase()], page_ref_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every live page id, in notebook/section/page order.
    pub fn all_page_ids(&self) -> Result<Vec<String>> {
        let sql = format!(
            "SELECT p.id FROM pages p JOIN sections s ON s.id = p.section_id JOIN notebooks n ON n.id = s.notebook_id
             WHERE {LIVE} ORDER BY n.position, s.position, p.position"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Rebuild the derived index for every page (after upgrades or repairs).
    pub fn reindex_all(&mut self) -> Result<usize> {
        let ids: Vec<String> = {
            let mut stmt = self.conn.prepare("SELECT id FROM pages")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for id in &ids {
            let (content, _) = self.page_content(id)?;
            let tx = self.conn.transaction()?;
            let title: String = tx.query_row("SELECT title FROM pages WHERE id = ?1", [id], |r| r.get(0))?;
            tx.execute("DELETE FROM page_fts WHERE page_id = ?1", [id])?;
            tx.execute("INSERT INTO page_fts (page_id, title, body) VALUES (?1, ?2, '')", params![id, title])?;
            reindex(&tx, id, &content)?;
            tx.commit()?;
        }
        Ok(ids.len())
    }

    // -----------------------------------------------------------------------
    // assets

    pub fn put_asset(&mut self, bytes: &[u8], name: Option<&str>, mime: Option<&str>) -> Result<AssetInfo> {
        let hash = self.assets.put(bytes)?;
        let mime = mime
            .filter(|m| !m.is_empty() && *m != "application/octet-stream")
            .or_else(|| assets::sniff_mime(bytes))
            .or_else(|| name.map(assets::mime_from_name))
            .unwrap_or("application/octet-stream")
            .to_string();
        self.conn.execute(
            "INSERT OR IGNORE INTO assets (hash, mime, size, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![hash, mime, bytes.len() as i64, name, now_ms()],
        )?;
        self.asset_info(&hash)
    }

    pub fn asset_info(&self, hash: &str) -> Result<AssetInfo> {
        self.conn
            .query_row("SELECT hash, mime, size, name FROM assets WHERE hash = ?1", [hash], |r| {
                let hash: String = r.get(0)?;
                Ok(AssetInfo { url: assets::asset_url(&hash), hash, mime: r.get(1)?, size: r.get(2)?, name: r.get(3)? })
            })
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("asset {hash}")))
    }

    /// Bytes and MIME type of an asset.
    pub fn asset(&self, hash: &str) -> Result<(Vec<u8>, String)> {
        let bytes = self.assets.get(hash)?;
        let mime = self
            .conn
            .query_row("SELECT mime FROM assets WHERE hash = ?1", [hash], |r| r.get::<_, String>(0))
            .optional()?
            .or_else(|| assets::sniff_mime(&bytes).map(str::to_string))
            .unwrap_or_else(|| "application/octet-stream".into());
        Ok((bytes, mime))
    }
}

// ---------------------------------------------------------------------------
// helpers

/// SQL condition: the page and everything containing it are live.
const LIVE: &str = "p.deleted_at IS NULL AND s.deleted_at IS NULL AND n.deleted_at IS NULL";

const PAGE_REF_SELECT: &str = "SELECT p.id, p.title, s.id, s.title, n.id, n.title, p.updated_at FROM pages p
     JOIN sections s ON s.id = p.section_id JOIN notebooks n ON n.id = s.notebook_id";

fn page_ref_from_row(r: &rusqlite::Row) -> rusqlite::Result<PageRef> {
    Ok(PageRef {
        id: r.get(0)?,
        title: r.get(1)?,
        section_id: r.get(2)?,
        section_title: r.get(3)?,
        notebook_id: r.get(4)?,
        notebook_title: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

fn notebook_from_row(r: &rusqlite::Row) -> rusqlite::Result<NotebookInfo> {
    Ok(NotebookInfo {
        id: r.get(0)?,
        title: r.get(1)?,
        color: r.get(2)?,
        position: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

fn section_from_row(r: &rusqlite::Row) -> rusqlite::Result<SectionInfo> {
    Ok(SectionInfo {
        id: r.get(0)?,
        notebook_id: r.get(1)?,
        parent_id: r.get(2)?,
        title: r.get(3)?,
        color: r.get(4)?,
        position: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

fn page_from_row(r: &rusqlite::Row) -> rusqlite::Result<PageInfo> {
    Ok(PageInfo {
        id: r.get(0)?,
        section_id: r.get(1)?,
        parent_id: r.get(2)?,
        title: r.get(3)?,
        position: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

fn clean_title(title: &str, fallback: &str) -> String {
    let t: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        fallback.to_string()
    } else {
        t
    }
}

fn next_position<P: rusqlite::Params>(conn: &Connection, sql: &str, params: P) -> Result<f64> {
    let max: Option<f64> = conn.query_row(sql, params, |r| r.get(0))?;
    Ok(max.map_or(1.0, |m| m.floor() + 1.0))
}

/// Compute a fractional position for `id` among the rows of `table` matching
/// `scope` (which may use `?1`, `?2` bound to `scope_params`).
fn place(
    conn: &Connection,
    table: &str,
    scope: &str,
    scope_params: &[&dyn rusqlite::ToSql],
    id: &str,
    placement: &Placement,
) -> Result<f64> {
    let sql = format!("SELECT id, position FROM {table} WHERE {scope} ORDER BY position");
    let mut stmt = conn.prepare(&sql)?;
    let siblings: Vec<(String, f64)> = stmt
        .query_map(scope_params, |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(sid, _)| sid != id)
        .collect();
    let index_of = |target: &str| siblings.iter().position(|(sid, _)| sid == target);
    let between = |lo: Option<f64>, hi: Option<f64>| match (lo, hi) {
        (Some(a), Some(b)) => (a + b) / 2.0,
        (Some(a), None) => a.floor() + 1.0,
        (None, Some(b)) => b - 1.0,
        (None, None) => 1.0,
    };
    Ok(match placement {
        Placement::End => between(siblings.last().map(|s| s.1), None),
        Placement::Before(target) => match index_of(target) {
            Some(i) => between(i.checked_sub(1).map(|j| siblings[j].1), Some(siblings[i].1)),
            None => between(siblings.last().map(|s| s.1), None),
        },
        Placement::After(target) => match index_of(target) {
            Some(i) => between(Some(siblings[i].1), siblings.get(i + 1).map(|s| s.1)),
            None => between(siblings.last().map(|s| s.1), None),
        },
    })
}

/// Refresh the derived index rows of one page.
fn reindex(tx: &Transaction, page_id: &str, content: &Node) -> Result<()> {
    let d = derive(content);
    tx.execute("UPDATE page_fts SET body = ?2 WHERE page_id = ?1", params![page_id, d.text])?;
    tx.execute("DELETE FROM links WHERE src = ?1", [page_id])?;
    for dst in &d.links {
        tx.execute("INSERT OR IGNORE INTO links (src, dst) VALUES (?1, ?2)", params![page_id, dst])?;
    }
    tx.execute("DELETE FROM tasks WHERE page_id = ?1", [page_id])?;
    for (i, t) in d.tasks.iter().enumerate() {
        if t.block_id.is_empty() {
            continue;
        }
        tx.execute(
            "INSERT OR REPLACE INTO tasks (page_id, block_id, ord, text, checked) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![page_id, t.block_id, i as i64, t.text, t.checked],
        )?;
    }
    tx.execute("DELETE FROM tags WHERE page_id = ?1", [page_id])?;
    for tag in &d.tags {
        tx.execute("INSERT OR IGNORE INTO tags (page_id, tag) VALUES (?1, ?2)", params![page_id, tag])?;
    }
    Ok(())
}

/// Turn free text into a safe FTS5 query: quoted terms, last one as prefix.
fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-'))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if terms.is_empty() {
        return None;
    }
    let mut q = terms.join(" ");
    q.push('*');
    Some(q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::kinds;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, store)
    }

    fn paragraph_doc(text: &str) -> Node {
        Node::doc(vec![Node::paragraph(vec![Node::text(text)])])
    }

    #[test]
    fn hierarchy_and_ordering() {
        let (_d, mut s) = store();
        let nb = s.create_notebook("Work", None).unwrap();
        let sec = s.create_section(&nb.id, None, "Projects").unwrap();
        let a = s.create_page(&sec.id, None, "A", Placement::End).unwrap();
        let b = s.create_page(&sec.id, None, "B", Placement::End).unwrap();
        let c = s.create_page(&sec.id, None, "C", Placement::Before(b.id.clone())).unwrap();
        let titles: Vec<_> = s.list_pages(&sec.id).unwrap().into_iter().map(|p| p.title).collect();
        assert_eq!(titles, ["A", "C", "B"]);

        // Subpages move with their parent.
        s.move_page(&b.id, &sec.id, Some(&a.id), Placement::After(a.id.clone())).unwrap();
        let other = s.create_section(&nb.id, None, "Archive").unwrap();
        s.move_page(&a.id, &other.id, None, Placement::End).unwrap();
        let moved: Vec<_> = s.list_pages(&other.id).unwrap().into_iter().map(|p| p.title).collect();
        assert_eq!(moved, ["A", "B"]);
        assert_eq!(s.list_pages(&sec.id).unwrap().len(), 1);
        assert!(s.move_page(&a.id, &other.id, Some(&b.id), Placement::End).is_err());
        let _ = c;
    }

    #[test]
    fn content_search_links_tasks_tags() {
        let (_d, mut s) = store();
        let nb = s.create_notebook("Lab", None).unwrap();
        let sec = s.create_section(&nb.id, None, "Experiments").unwrap();
        let target = s.create_page(&sec.id, None, "Protocol", Placement::End).unwrap();
        let content = Node::doc(vec![
            Node::paragraph(vec![
                Node::text("Centrifuge the samples, see "),
                Node::new(kinds::WIKI_LINK).with_attr("pageId", target.id.as_str()).with_attr("title", "Protocol"),
                Node::text(" #biology"),
            ]),
            Node::element(
                kinds::TASK_LIST,
                vec![Node::element(kinds::TASK_ITEM, vec![Node::paragraph(vec![Node::text("Order pipettes")])])
                    .with_attr("checked", false)],
            ),
        ]);
        let page = s
            .create_page_with_content(&sec.id, None, "Day 1", Placement::End, &content, &BTreeMap::new(), None)
            .unwrap();

        let hits = s.search("centrif", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page.id, page.id);
        assert!(hits[0].snippet.contains('\u{1}'));
        assert_eq!(s.search("protocol", 10).unwrap()[0].page.id, target.id, "title outranks body");
        assert!(s.search("\"unbalanced AND (", 10).is_ok());

        assert_eq!(s.backlinks(&target.id).unwrap()[0].id, page.id);
        assert_eq!(s.pages_with_tag("#Biology").unwrap()[0].id, page.id);
        assert_eq!(s.tags().unwrap()[0].tag, "biology");

        let tasks = s.tasks(false).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text, "Order pipettes");
        let update = s.set_task_checked(&page.id, &tasks[0].block_id, true).unwrap();
        assert!(!update.is_empty());
        assert!(s.tasks(false).unwrap().is_empty());
        assert!(s.tasks(true).unwrap()[0].checked);

        assert_eq!(s.find_pages("proto", 5).unwrap()[0].id, target.id);
    }

    #[test]
    fn updates_persist_and_merge_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let page_id;
        {
            let mut s = Store::open(dir.path()).unwrap();
            let nb = s.create_notebook("N", None).unwrap();
            let sec = s.create_section(&nb.id, None, "S").unwrap();
            page_id = s.create_page(&sec.id, None, "P", Placement::End).unwrap().id;

            // Simulate an editor: take the state, edit a replica, send the diff.
            let editor = ydoc::doc_from_state(&s.page_state(&page_id).unwrap()).unwrap();
            let update = ydoc::replace_all(&editor, &paragraph_doc("hello from the editor"), &BTreeMap::new());
            s.apply_update(&page_id, &update).unwrap();
            for i in 0..(COMPACT_AFTER + 5) {
                let u = ydoc::replace_all(&editor, &paragraph_doc(&format!("edit {i}")), &BTreeMap::new());
                s.apply_update(&page_id, &u).unwrap();
            }
        }
        let mut s = Store::open(dir.path()).unwrap();
        let (content, _) = s.page_content(&page_id).unwrap();
        assert_eq!(content.plain_text(), format!("edit {}", COMPACT_AFTER + 4));
        let n: i64 =
            s.conn.query_row("SELECT COUNT(*) FROM page_updates WHERE page_id = ?1", [&page_id], |r| r.get(0)).unwrap();
        assert!(n < COMPACT_AFTER, "log was compacted ({n} rows)");
        assert!(s.search("edit", 5).unwrap().len() == 1);
    }

    #[test]
    fn versions_restore() {
        let (_d, mut s) = store();
        let nb = s.create_notebook("N", None).unwrap();
        let sec = s.create_section(&nb.id, None, "S").unwrap();
        let p = s
            .create_page_with_content(
                &sec.id,
                None,
                "P",
                Placement::End,
                &paragraph_doc("first draft"),
                &BTreeMap::new(),
                None,
            )
            .unwrap();
        let v1 = s.snapshot_version(&p.id, Some("v1")).unwrap();
        s.replace_page_content(&p.id, &paragraph_doc("second draft"), &BTreeMap::new()).unwrap();
        assert_eq!(s.page_content(&p.id).unwrap().0.plain_text(), "second draft");
        let (pid, update) = s.restore_version(v1.id).unwrap();
        assert_eq!(pid, p.id);
        assert!(!update.is_empty());
        assert_eq!(s.page_content(&p.id).unwrap().0.plain_text(), "first draft");
        assert!(s.list_versions(&p.id).unwrap().iter().any(|v| v.label.as_deref() == Some("Before restore")));
    }

    #[test]
    fn trash_and_restore() {
        let (_d, mut s) = store();
        let nb = s.create_notebook("N", None).unwrap();
        let sec = s.create_section(&nb.id, None, "S").unwrap();
        let parent = s.create_page(&sec.id, None, "Parent", Placement::End).unwrap();
        let child = s.create_page(&sec.id, Some(&parent.id), "Child", Placement::End).unwrap();
        s.delete_page(&parent.id).unwrap();
        assert!(s.list_pages(&sec.id).unwrap().is_empty());
        assert_eq!(s.list_trash().unwrap().len(), 2);
        s.restore("page", &parent.id).unwrap();
        assert_eq!(s.list_pages(&sec.id).unwrap().len(), 2);

        s.delete_section(&sec.id).unwrap();
        assert!(s.recent_pages(10).unwrap().is_empty());
        s.restore("page", &child.id).unwrap();
        assert_eq!(s.recent_pages(10).unwrap().len(), 2);

        s.delete_page(&child.id).unwrap();
        s.empty_trash().unwrap();
        assert!(s.page(&child.id).is_err());
        assert!(s.page(&parent.id).is_ok());
    }

    #[test]
    fn assets_are_deduplicated() {
        let (_d, mut s) = store();
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
        let a = s.put_asset(&png, Some("x.png"), None).unwrap();
        let b = s.put_asset(&png, Some("y.png"), None).unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.mime, "image/png");
        assert_eq!(s.asset(&a.hash).unwrap().0, png);
    }
}
