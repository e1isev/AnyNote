//! Tauri shell around `anynote-core`.
//!
//! The webview edits each page as a Yjs document and streams binary updates
//! to [`apply_page_update`]; Rust is the source of truth for storage, search
//! and every change made outside the editor (task view, version restore),
//! which it pushes back to open editors as `page-update` events.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use anynote_core::folder;
use anynote_core::paths::default_data_dir;
use anynote_core::store::{
    AssetInfo, NotebookInfo, PageInfo, PageRef, Placement, SearchHit, SectionInfo, TagCount, TaskRow, TrashItem,
    VersionInfo,
};
use anynote_core::{markdown, Node, Store};
use serde::Serialize;
use tauri::http::{header, Response as HttpResponse, StatusCode};
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    store: Mutex<Store>,
    data_dir: PathBuf,
}

impl AppState {
    fn store(&self) -> MutexGuard<'_, Store> {
        // A panic while holding the lock leaves SQLite consistent (every write
        // is a transaction), so recover rather than poison the app forever.
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageUpdateEvent {
    page_id: String,
    update: Vec<u8>,
}

fn broadcast(app: &AppHandle, page_id: &str, update: Vec<u8>) {
    let _ = app.emit("page-update", PageUpdateEvent { page_id: page_id.to_string(), update });
}

fn placement(before: Option<String>, after: Option<String>) -> Placement {
    match (before, after) {
        (Some(b), _) => Placement::Before(b),
        (None, Some(a)) => Placement::After(a),
        _ => Placement::End,
    }
}

// ---------------------------------------------------------------------------
// notebooks & sections

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NotebookTree {
    #[serde(flatten)]
    notebook: NotebookInfo,
    sections: Vec<SectionInfo>,
}

#[tauri::command]
async fn list_tree(state: State<'_, AppState>) -> CmdResult<Vec<NotebookTree>> {
    let store = state.store();
    let notebooks = store.list_notebooks().map_err(err)?;
    notebooks
        .into_iter()
        .map(|nb| Ok(NotebookTree { sections: store.list_sections(&nb.id).map_err(err)?, notebook: nb }))
        .collect()
}

#[tauri::command]
async fn create_notebook(state: State<'_, AppState>, title: String) -> CmdResult<NotebookInfo> {
    let mut store = state.store();
    let nb = store.create_notebook(&title, None).map_err(err)?;
    // A notebook is more useful with somewhere to write straight away.
    let sec = store.create_section(&nb.id, None, "Quick notes").map_err(err)?;
    store.create_page(&sec.id, None, "", Placement::End).map_err(err)?;
    Ok(nb)
}

#[tauri::command]
async fn update_notebook(
    state: State<'_, AppState>,
    id: String,
    title: Option<String>,
    color: Option<String>,
    clear_color: Option<bool>,
) -> CmdResult<NotebookInfo> {
    let color = if clear_color.unwrap_or(false) { Some(None) } else { color.as_deref().map(Some) };
    state.store().update_notebook(&id, title.as_deref(), color).map_err(err)
}

#[tauri::command]
async fn move_notebook(
    state: State<'_, AppState>,
    id: String,
    before: Option<String>,
    after: Option<String>,
) -> CmdResult<()> {
    state.store().move_notebook(&id, placement(before, after)).map_err(err)
}

#[tauri::command]
async fn delete_notebook(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.store().delete_notebook(&id).map_err(err)
}

#[tauri::command]
async fn create_section(
    state: State<'_, AppState>,
    notebook_id: String,
    parent_id: Option<String>,
    title: String,
) -> CmdResult<SectionInfo> {
    let mut store = state.store();
    let sec = store.create_section(&notebook_id, parent_id.as_deref(), &title).map_err(err)?;
    store.create_page(&sec.id, None, "", Placement::End).map_err(err)?;
    Ok(sec)
}

#[tauri::command]
async fn update_section(
    state: State<'_, AppState>,
    id: String,
    title: Option<String>,
    color: Option<String>,
    clear_color: Option<bool>,
) -> CmdResult<SectionInfo> {
    let color = if clear_color.unwrap_or(false) { Some(None) } else { color.as_deref().map(Some) };
    state.store().update_section(&id, title.as_deref(), color).map_err(err)
}

#[tauri::command]
async fn move_section(
    state: State<'_, AppState>,
    id: String,
    notebook_id: String,
    parent_id: Option<String>,
    before: Option<String>,
    after: Option<String>,
) -> CmdResult<()> {
    state.store().move_section(&id, &notebook_id, parent_id.as_deref(), placement(before, after)).map_err(err)
}

#[tauri::command]
async fn delete_section(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.store().delete_section(&id).map_err(err)
}

// ---------------------------------------------------------------------------
// pages

#[tauri::command]
async fn list_pages(state: State<'_, AppState>, section_id: String) -> CmdResult<Vec<PageInfo>> {
    state.store().list_pages(&section_id).map_err(err)
}

#[tauri::command]
async fn create_page(
    state: State<'_, AppState>,
    section_id: String,
    parent_id: Option<String>,
    title: String,
    after: Option<String>,
) -> CmdResult<PageInfo> {
    state.store().create_page(&section_id, parent_id.as_deref(), &title, placement(None, after)).map_err(err)
}

#[tauri::command]
async fn get_page(state: State<'_, AppState>, id: String) -> CmdResult<(PageInfo, PageRef)> {
    let store = state.store();
    Ok((store.page(&id).map_err(err)?, store.page_ref(&id).map_err(err)?))
}

#[tauri::command]
async fn rename_page(state: State<'_, AppState>, id: String, title: String) -> CmdResult<PageInfo> {
    state.store().rename_page(&id, &title).map_err(err)
}

#[tauri::command]
async fn move_page(
    state: State<'_, AppState>,
    id: String,
    section_id: String,
    parent_id: Option<String>,
    before: Option<String>,
    after: Option<String>,
) -> CmdResult<()> {
    state.store().move_page(&id, &section_id, parent_id.as_deref(), placement(before, after)).map_err(err)
}

#[tauri::command]
async fn delete_page(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.store().delete_page(&id).map_err(err)
}

/// The page's CRDT state as raw bytes (an `ArrayBuffer` in JS).
#[tauri::command]
async fn load_page_state(state: State<'_, AppState>, id: String) -> CmdResult<Response> {
    let bytes = state.store().page_state(&id).map_err(err)?;
    Ok(Response::new(bytes))
}

/// Apply a binary Yjs update from the editor. The page id travels in the
/// `x-page-id` header so the body can stay raw bytes.
#[tauri::command]
async fn apply_page_update(state: State<'_, AppState>, request: Request<'_>) -> CmdResult<()> {
    let InvokeBody::Raw(update) = request.body() else {
        return Err("expected a binary update".into());
    };
    let page_id =
        request.headers().get("x-page-id").and_then(|v| v.to_str().ok()).ok_or("missing x-page-id header")?.to_string();
    state.store().apply_update(&page_id, update).map_err(err)
}

#[tauri::command]
async fn page_markdown(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    let (doc, _) = state.store().page_content(&id).map_err(err)?;
    Ok(markdown::to_markdown(&doc, &mut markdown::PlainExport))
}

// ---------------------------------------------------------------------------
// queries

#[tauri::command]
async fn search(state: State<'_, AppState>, query: String, limit: Option<usize>) -> CmdResult<Vec<SearchHit>> {
    state.store().search(&query, limit.unwrap_or(50)).map_err(err)
}

#[tauri::command]
async fn find_pages(state: State<'_, AppState>, query: String, limit: Option<usize>) -> CmdResult<Vec<PageRef>> {
    let store = state.store();
    if query.trim().is_empty() {
        store.recent_pages(limit.unwrap_or(10)).map_err(err)
    } else {
        store.find_pages(query.trim(), limit.unwrap_or(10)).map_err(err)
    }
}

#[tauri::command]
async fn recent_pages(state: State<'_, AppState>, limit: Option<usize>) -> CmdResult<Vec<PageRef>> {
    state.store().recent_pages(limit.unwrap_or(20)).map_err(err)
}

#[tauri::command]
async fn backlinks(state: State<'_, AppState>, page_id: String) -> CmdResult<Vec<PageRef>> {
    state.store().backlinks(&page_id).map_err(err)
}

#[tauri::command]
async fn list_tasks(state: State<'_, AppState>, include_done: bool) -> CmdResult<Vec<TaskRow>> {
    state.store().tasks(include_done).map_err(err)
}

#[tauri::command]
async fn set_task_checked(
    app: AppHandle,
    state: State<'_, AppState>,
    page_id: String,
    block_id: String,
    checked: bool,
) -> CmdResult<()> {
    let update = state.store().set_task_checked(&page_id, &block_id, checked).map_err(err)?;
    broadcast(&app, &page_id, update);
    Ok(())
}

#[tauri::command]
async fn list_tags(state: State<'_, AppState>) -> CmdResult<Vec<TagCount>> {
    state.store().tags().map_err(err)
}

#[tauri::command]
async fn pages_with_tag(state: State<'_, AppState>, tag: String) -> CmdResult<Vec<PageRef>> {
    state.store().pages_with_tag(&tag).map_err(err)
}

// ---------------------------------------------------------------------------
// history & trash

#[tauri::command]
async fn list_versions(state: State<'_, AppState>, page_id: String) -> CmdResult<Vec<VersionInfo>> {
    state.store().list_versions(&page_id).map_err(err)
}

#[tauri::command]
async fn snapshot_version(
    state: State<'_, AppState>,
    page_id: String,
    label: Option<String>,
) -> CmdResult<VersionInfo> {
    state.store().snapshot_version(&page_id, label.as_deref()).map_err(err)
}

/// A version's body as ProseMirror JSON, for a read-only preview.
#[tauri::command]
async fn version_content(state: State<'_, AppState>, version_id: i64) -> CmdResult<Node> {
    state.store().version_content(version_id).map(|(_, doc, _)| doc).map_err(err)
}

#[tauri::command]
async fn restore_version(app: AppHandle, state: State<'_, AppState>, version_id: i64) -> CmdResult<()> {
    let (page_id, update) = state.store().restore_version(version_id).map_err(err)?;
    broadcast(&app, &page_id, update);
    Ok(())
}

#[tauri::command]
async fn list_trash(state: State<'_, AppState>) -> CmdResult<Vec<TrashItem>> {
    state.store().list_trash().map_err(err)
}

#[tauri::command]
async fn restore_item(state: State<'_, AppState>, kind: String, id: String) -> CmdResult<()> {
    state.store().restore(&kind, &id).map_err(err)
}

#[tauri::command]
async fn empty_trash(state: State<'_, AppState>) -> CmdResult<()> {
    state.store().empty_trash().map_err(err)
}

// ---------------------------------------------------------------------------
// assets, import & export

/// Store a file (raw body). `x-name` (URI-encoded) and `x-mime` headers are optional.
#[tauri::command]
async fn store_asset(state: State<'_, AppState>, request: Request<'_>) -> CmdResult<AssetInfo> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected file bytes".into());
    };
    let header = |k: &str| request.headers().get(k).and_then(|v| v.to_str().ok()).map(percent_decode);
    let name = header("x-name");
    let mime = header("x-mime");
    state.store().put_asset(bytes, name.as_deref(), mime.as_deref()).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportResult {
    notebook_ids: Vec<String>,
    sections: usize,
    pages: usize,
    assets: usize,
    warnings: Vec<String>,
}

#[tauri::command]
async fn import_onenote(app: AppHandle, path: String) -> CmdResult<ImportResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut store = state.store();
        let r = anynote_onenote::import(&mut store, std::path::Path::new(&path)).map_err(|e| format!("{e:#}"))?;
        Ok(ImportResult {
            notebook_ids: r.notebook_ids,
            sections: r.sections,
            pages: r.pages,
            assets: r.assets,
            warnings: r.warnings,
        })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn import_folder(app: AppHandle, path: String) -> CmdResult<ImportResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut store = state.store();
        let r = folder::import(&mut store, std::path::Path::new(&path)).map_err(err)?;
        Ok(ImportResult {
            notebook_ids: r.notebook_ids,
            sections: r.sections,
            pages: r.pages,
            assets: r.assets,
            warnings: r.warnings,
        })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn export_folder(app: AppHandle, path: String, notebook_ids: Vec<String>) -> CmdResult<folder::ExportSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut store = state.store();
        folder::export(&mut store, std::path::Path::new(&path), &notebook_ids).map_err(err)
    })
    .await
    .map_err(err)?
}

/// Open an asset with the system's default app (via a named temp copy).
#[tauri::command]
async fn open_asset(app: AppHandle, state: State<'_, AppState>, hash: String, name: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let (bytes, _) = state.store().asset(&hash).map_err(err)?;
    let dir = std::env::temp_dir().join("anynote-open").join(&hash[..hash.len().min(16)]);
    std::fs::create_dir_all(&dir).map_err(err)?;
    let file = dir.join(folder::file_name(if name.is_empty() { "attachment" } else { &name }));
    std::fs::write(&file, bytes).map_err(err)?;
    app.opener().open_path(file.to_string_lossy(), None::<&str>).map_err(err)
}

/// Save a copy of an asset to `path`.
#[tauri::command]
async fn save_asset(state: State<'_, AppState>, hash: String, path: String) -> CmdResult<()> {
    let (bytes, _) = state.store().asset(&hash).map_err(err)?;
    std::fs::write(path, bytes).map_err(err)
}

#[tauri::command]
async fn data_dir(state: State<'_, AppState>) -> CmdResult<String> {
    Ok(state.data_dir.display().to_string())
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

/// Serve `anynote://asset/<hash>` (or `http://anynote.localhost/asset/<hash>`
/// on Windows) from the asset store.
fn serve_asset(app: &AppHandle, uri: &tauri::http::Uri) -> HttpResponse<Vec<u8>> {
    let hash = uri.path().rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    let state = app.state::<AppState>();
    let result = state.store().asset(hash);
    match result {
        Ok((bytes, mime)) => HttpResponse::builder()
            .header(header::CONTENT_TYPE, mime)
            .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
            .body(bytes)
            .unwrap_or_else(|_| HttpResponse::new(Vec::new())),
        Err(_) => HttpResponse::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Vec::new())
            .unwrap_or_else(|_| HttpResponse::new(Vec::new())),
    }
}

/// First run: a welcome notebook that shows off what pages can hold.
fn seed_welcome(store: &mut Store) -> anynote_core::Result<()> {
    if !store.list_notebooks()?.is_empty() || !store.list_trash()?.is_empty() {
        return Ok(());
    }
    let nb = store.create_notebook("My notebook", Some("#7c5cff"))?;
    let sec = store.create_section(&nb.id, None, "Getting started")?;
    let welcome_md = include_str!("../welcome.md");
    let doc = markdown::from_markdown(welcome_md, &mut |_| None);
    store.create_page_with_content(
        &sec.id,
        None,
        "Welcome to AnyNote",
        Placement::End,
        &doc,
        &Default::default(),
        None,
    )?;
    store.create_section(&nb.id, None, "Ideas")?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let dir = default_data_dir();
    let mut store = Store::open(&dir).unwrap_or_else(|e| panic!("cannot open data folder {}: {e}", dir.display()));
    if let Err(e) = seed_welcome(&mut store) {
        eprintln!("could not create the welcome notebook: {e}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState { store: Mutex::new(store), data_dir: dir })
        .register_uri_scheme_protocol("anynote", |ctx, request| serve_asset(ctx.app_handle(), request.uri()))
        .invoke_handler(tauri::generate_handler![
            list_tree,
            create_notebook,
            update_notebook,
            move_notebook,
            delete_notebook,
            create_section,
            update_section,
            move_section,
            delete_section,
            list_pages,
            create_page,
            get_page,
            rename_page,
            move_page,
            delete_page,
            load_page_state,
            apply_page_update,
            page_markdown,
            search,
            find_pages,
            recent_pages,
            backlinks,
            list_tasks,
            set_task_checked,
            list_tags,
            pages_with_tag,
            list_versions,
            snapshot_version,
            version_content,
            restore_version,
            list_trash,
            restore_item,
            empty_trash,
            store_asset,
            import_onenote,
            import_folder,
            export_folder,
            open_asset,
            save_asset,
            data_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AnyNote");
}
