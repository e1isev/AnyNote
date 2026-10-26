//! The same commands as the desktop app (`app/src-tauri/src/lib.rs`), over
//! HTTP: `POST /api/<command>` with JSON arguments (camelCase, like Tauri).

use std::path::Path;

use anynote_core::store::Placement;
use anynote_core::{folder, markdown, Store};
use serde::Serialize;
use serde_json::{json, Map, Value};

pub enum Reply {
    Json(Value),
    Bytes(Vec<u8>),
    Empty,
}

/// A change other open windows must apply.
pub struct Broadcast {
    pub page_id: String,
    pub update: Vec<u8>,
}

pub struct Request<'a> {
    pub command: &'a str,
    pub args: Map<String, Value>,
    pub body: Vec<u8>,
    pub header: &'a dyn Fn(&str) -> Option<String>,
}

type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn to_json<T: Serialize>(v: T) -> Result<Reply> {
    serde_json::to_value(v).map(Reply::Json).map_err(err)
}

struct Args<'a>(&'a Map<String, Value>);

impl Args<'_> {
    fn str(&self, key: &str) -> Result<&str> {
        self.opt(key).ok_or_else(|| format!("missing argument {key}"))
    }
    fn opt(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }
    fn bool(&self, key: &str) -> bool {
        self.0.get(key).and_then(Value::as_bool).unwrap_or(false)
    }
    fn usize(&self, key: &str, default: usize) -> usize {
        self.0.get(key).and_then(Value::as_u64).map_or(default, |v| v as usize)
    }
    fn i64(&self, key: &str) -> Result<i64> {
        self.0.get(key).and_then(Value::as_i64).ok_or_else(|| format!("missing argument {key}"))
    }
    fn placement(&self) -> Placement {
        match (self.opt("before"), self.opt("after")) {
            (Some(b), _) => Placement::Before(b.into()),
            (None, Some(a)) => Placement::After(a.into()),
            _ => Placement::End,
        }
    }
    /// `color` / `clearColor` as the store's `Option<Option<&str>>`.
    fn color(&self) -> Option<Option<&str>> {
        if self.bool("clearColor") {
            Some(None)
        } else {
            self.opt("color").map(Some)
        }
    }
}

pub fn dispatch(store: &mut Store, data_dir: &Path, req: Request, broadcasts: &mut Vec<Broadcast>) -> Result<Reply> {
    let a = Args(&req.args);
    match req.command {
        "ping" => to_json("anynote"),
        "list_tree" => {
            let mut out = Vec::new();
            for nb in store.list_notebooks().map_err(err)? {
                let sections = store.list_sections(&nb.id).map_err(err)?;
                let mut v = serde_json::to_value(&nb).map_err(err)?;
                v["sections"] = serde_json::to_value(sections).map_err(err)?;
                out.push(v);
            }
            Ok(Reply::Json(Value::Array(out)))
        }
        "create_notebook" => {
            let nb = store.create_notebook(a.str("title")?, None).map_err(err)?;
            let sec = store.create_section(&nb.id, None, "Quick notes").map_err(err)?;
            store.create_page(&sec.id, None, "", Placement::End).map_err(err)?;
            to_json(nb)
        }
        "update_notebook" => to_json(store.update_notebook(a.str("id")?, a.opt("title"), a.color()).map_err(err)?),
        "move_notebook" => to_json(store.move_notebook(a.str("id")?, a.placement()).map_err(err)?),
        "delete_notebook" => to_json(store.delete_notebook(a.str("id")?).map_err(err)?),
        "create_section" => {
            let sec = store.create_section(a.str("notebookId")?, a.opt("parentId"), a.str("title")?).map_err(err)?;
            store.create_page(&sec.id, None, "", Placement::End).map_err(err)?;
            to_json(sec)
        }
        "update_section" => to_json(store.update_section(a.str("id")?, a.opt("title"), a.color()).map_err(err)?),
        "move_section" => to_json(
            store.move_section(a.str("id")?, a.str("notebookId")?, a.opt("parentId"), a.placement()).map_err(err)?,
        ),
        "delete_section" => to_json(store.delete_section(a.str("id")?).map_err(err)?),
        "list_pages" => to_json(store.list_pages(a.str("sectionId")?).map_err(err)?),
        "create_page" => {
            let placement = a.opt("after").map_or(Placement::End, |x| Placement::After(x.into()));
            let (section, parent, title) = (a.str("sectionId")?, a.opt("parentId"), a.opt("title").unwrap_or(""));
            if a.bool("ink") {
                let height = req.args.get("inkHeight").and_then(Value::as_u64).unwrap_or(640) as u32;
                let doc = anynote_core::Node::drawing_doc(height);
                to_json(
                    store
                        .create_page_with_content(section, parent, title, placement, &doc, &Default::default(), None)
                        .map_err(err)?,
                )
            } else {
                to_json(store.create_page(section, parent, title, placement).map_err(err)?)
            }
        }
        "get_page" => {
            let id = a.str("id")?;
            to_json((store.page(id).map_err(err)?, store.page_ref(id).map_err(err)?))
        }
        "rename_page" => to_json(store.rename_page(a.str("id")?, a.str("title")?).map_err(err)?),
        "move_page" => {
            to_json(store.move_page(a.str("id")?, a.str("sectionId")?, a.opt("parentId"), a.placement()).map_err(err)?)
        }
        "delete_page" => to_json(store.delete_page(a.str("id")?).map_err(err)?),
        "load_page_state" => Ok(Reply::Bytes(store.page_state(a.str("id")?).map_err(err)?)),
        "apply_page_update" => {
            let page_id = (req.header)("x-page-id").ok_or("missing x-page-id header")?;
            store.apply_update(&page_id, &req.body).map_err(err)?;
            // Other windows on the same page pick the edit up; the sender
            // ignores its own update (Yjs updates are idempotent).
            broadcasts.push(Broadcast { page_id, update: req.body });
            Ok(Reply::Empty)
        }
        "page_markdown" => {
            let (doc, _) = store.page_content(a.str("id")?).map_err(err)?;
            to_json(markdown::to_markdown(&doc, &mut markdown::PlainExport))
        }
        "search" => to_json(store.search(a.str("query")?, a.usize("limit", 50)).map_err(err)?),
        "find_pages" => {
            let q = a.opt("query").unwrap_or("").trim();
            let limit = a.usize("limit", 10);
            if q.is_empty() {
                to_json(store.recent_pages(limit).map_err(err)?)
            } else {
                to_json(store.find_pages(q, limit).map_err(err)?)
            }
        }
        "recent_pages" => to_json(store.recent_pages(a.usize("limit", 20)).map_err(err)?),
        "backlinks" => to_json(store.backlinks(a.str("pageId")?).map_err(err)?),
        "list_tasks" => to_json(store.tasks(a.bool("includeDone")).map_err(err)?),
        "set_task_checked" => {
            let page_id = a.str("pageId")?;
            let update = store.set_task_checked(page_id, a.str("blockId")?, a.bool("checked")).map_err(err)?;
            broadcasts.push(Broadcast { page_id: page_id.into(), update });
            Ok(Reply::Empty)
        }
        "list_tags" => to_json(store.tags().map_err(err)?),
        "pages_with_tag" => to_json(store.pages_with_tag(a.str("tag")?).map_err(err)?),
        "list_versions" => to_json(store.list_versions(a.str("pageId")?).map_err(err)?),
        "snapshot_version" => to_json(store.snapshot_version(a.str("pageId")?, a.opt("label")).map_err(err)?),
        "version_content" => to_json(store.version_content(a.i64("versionId")?).map_err(err)?.1),
        "restore_version" => {
            let (page_id, update) = store.restore_version(a.i64("versionId")?).map_err(err)?;
            broadcasts.push(Broadcast { page_id, update });
            Ok(Reply::Empty)
        }
        "list_trash" => to_json(store.list_trash().map_err(err)?),
        "restore_item" => to_json(store.restore(a.str("kind")?, a.str("id")?).map_err(err)?),
        "empty_trash" => to_json(store.empty_trash().map_err(err)?),
        "store_asset" => {
            let name = (req.header)("x-name").map(|v| percent_decode(&v));
            let mime = (req.header)("x-mime").map(|v| percent_decode(&v));
            to_json(store.put_asset(&req.body, name.as_deref(), mime.as_deref()).map_err(err)?)
        }
        "import_onenote" => {
            let r = anynote_onenote::import(store, Path::new(a.str("path")?)).map_err(|e| format!("{e:#}"))?;
            to_json(json!({
                "notebookIds": r.notebook_ids, "sections": r.sections, "pages": r.pages,
                "assets": r.assets, "warnings": r.warnings,
            }))
        }
        "import_folder" => to_json(folder::import(store, Path::new(a.str("path")?)).map_err(err)?),
        "export_folder" => {
            let ids: Vec<String> = req
                .args
                .get("notebookIds")
                .and_then(Value::as_array)
                .map(|v| v.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            to_json(folder::export(store, Path::new(a.str("path")?), &ids).map_err(err)?)
        }
        "save_asset" => {
            let (bytes, _) = store.asset(a.str("hash")?).map_err(err)?;
            std::fs::write(a.str("path")?, bytes).map_err(err)?;
            Ok(Reply::Empty)
        }
        "data_dir" => to_json(data_dir.display().to_string()),
        other => Err(format!("unknown command {other}")),
    }
}

pub fn percent_decode(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn call(store: &mut Store, command: &str, args: Value, out: &mut Vec<Broadcast>) -> Result<Reply> {
        let Value::Object(args) = args else { panic!("args must be an object") };
        let no_headers = |_: &str| None;
        dispatch(store, Path::new("/tmp"), Request { command, args, body: Vec::new(), header: &no_headers }, out)
    }

    fn json_of(r: Result<Reply>) -> Value {
        match r {
            Ok(Reply::Json(v)) => v,
            Ok(_) => panic!("expected JSON"),
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn commands_mirror_the_desktop_app() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let mut out = Vec::new();
        let nb = json_of(call(&mut store, "create_notebook", json!({"title": "Pi"}), &mut out));
        let tree = json_of(call(&mut store, "list_tree", json!({}), &mut out));
        assert_eq!(tree[0]["title"], "Pi");
        assert_eq!(tree[0]["sections"][0]["title"], "Quick notes");
        let section = tree[0]["sections"][0]["id"].as_str().unwrap().to_string();
        let pages = json_of(call(&mut store, "list_pages", json!({ "sectionId": section }), &mut out));
        assert_eq!(pages.as_array().unwrap().len(), 1);
        json_of(call(&mut store, "update_notebook", json!({"id": nb["id"], "clearColor": true}), &mut out));

        let page = pages[0]["id"].as_str().unwrap();
        assert!(
            matches!(call(&mut store, "load_page_state", json!({ "id": page }), &mut out), Ok(Reply::Bytes(b)) if !b.is_empty())
        );
        assert!(call(&mut store, "no_such_command", json!({}), &mut out).is_err());
        assert!(call(&mut store, "rename_page", json!({}), &mut out).is_err(), "missing arguments are errors");
        assert!(out.is_empty());
    }

    #[test]
    fn decodes_percent_encoded_headers() {
        assert_eq!(percent_decode("caf%C3%A9%20menu.pdf"), "café menu.pdf");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
