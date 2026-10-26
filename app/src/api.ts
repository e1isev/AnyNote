// Typed wrappers around the backend commands (src-tauri/src/lib.rs in the
// desktop app, crates/anynote-server in the browser).
import { call as invoke } from "./platform";
import type {
  AssetInfo,
  ExportSummary,
  ImportResult,
  NotebookTree,
  Notebook,
  Page,
  PageRef,
  PMNode,
  SearchHit,
  Section,
  TagCount,
  TaskRow,
  TrashItem,
  Version,
} from "./types";

export const api = {
  listTree: () => invoke<NotebookTree[]>("list_tree"),
  createNotebook: (title: string) => invoke<Notebook>("create_notebook", { title }),
  updateNotebook: (id: string, changes: { title?: string; color?: string | null }) =>
    invoke<Notebook>("update_notebook", {
      id,
      title: changes.title,
      color: changes.color ?? undefined,
      clearColor: changes.color === null,
    }),
  moveNotebook: (id: string, place: { before?: string; after?: string }) =>
    invoke<void>("move_notebook", { id, ...place }),
  deleteNotebook: (id: string) => invoke<void>("delete_notebook", { id }),

  createSection: (notebookId: string, title: string, parentId?: string | null) =>
    invoke<Section>("create_section", { notebookId, parentId: parentId ?? null, title }),
  updateSection: (id: string, changes: { title?: string; color?: string | null }) =>
    invoke<Section>("update_section", {
      id,
      title: changes.title,
      color: changes.color ?? undefined,
      clearColor: changes.color === null,
    }),
  moveSection: (
    id: string,
    notebookId: string,
    parentId: string | null,
    place: { before?: string; after?: string } = {},
  ) => invoke<void>("move_section", { id, notebookId, parentId, ...place }),
  deleteSection: (id: string) => invoke<void>("delete_section", { id }),

  listPages: (sectionId: string) => invoke<Page[]>("list_pages", { sectionId }),
  createPage: (
    sectionId: string,
    opts: { parentId?: string | null; title?: string; after?: string; canvas?: boolean } = {},
  ) =>
    invoke<Page>("create_page", {
      sectionId,
      parentId: opts.parentId ?? null,
      title: opts.title ?? "",
      after: opts.after,
      canvas: opts.canvas ?? false,
    }),
  getPage: (id: string) => invoke<[Page, PageRef]>("get_page", { id }),
  renamePage: (id: string, title: string) => invoke<Page>("rename_page", { id, title }),
  movePage: (
    id: string,
    sectionId: string,
    parentId: string | null,
    place: { before?: string; after?: string } = {},
  ) => invoke<void>("move_page", { id, sectionId, parentId, ...place }),
  deletePage: (id: string) => invoke<void>("delete_page", { id }),
  loadPageState: async (id: string) => new Uint8Array(await invoke<ArrayBuffer>("load_page_state", { id })),
  applyPageUpdate: (pageId: string, update: Uint8Array) =>
    invoke<void>("apply_page_update", update, { headers: { "x-page-id": pageId } }),
  pageMarkdown: (id: string) => invoke<string>("page_markdown", { id }),

  search: (query: string, limit = 50) => invoke<SearchHit[]>("search", { query, limit }),
  findPages: (query: string, limit = 10) => invoke<PageRef[]>("find_pages", { query, limit }),
  recentPages: (limit = 20) => invoke<PageRef[]>("recent_pages", { limit }),
  backlinks: (pageId: string) => invoke<PageRef[]>("backlinks", { pageId }),
  listTasks: (includeDone: boolean) => invoke<TaskRow[]>("list_tasks", { includeDone }),
  setTaskChecked: (pageId: string, blockId: string, checked: boolean) =>
    invoke<void>("set_task_checked", { pageId, blockId, checked }),
  listTags: () => invoke<TagCount[]>("list_tags"),
  pagesWithTag: (tag: string) => invoke<PageRef[]>("pages_with_tag", { tag }),

  listVersions: (pageId: string) => invoke<Version[]>("list_versions", { pageId }),
  snapshotVersion: (pageId: string, label?: string) => invoke<Version>("snapshot_version", { pageId, label }),
  versionContent: (versionId: number) => invoke<PMNode>("version_content", { versionId }),
  restoreVersion: (versionId: number) => invoke<void>("restore_version", { versionId }),

  listTrash: () => invoke<TrashItem[]>("list_trash"),
  restoreItem: (kind: string, id: string) => invoke<void>("restore_item", { kind, id }),
  emptyTrash: () => invoke<void>("empty_trash"),

  storeAsset: (bytes: Uint8Array, name?: string, mime?: string) => {
    const headers: Record<string, string> = {};
    if (name) headers["x-name"] = encodeURIComponent(name);
    if (mime) headers["x-mime"] = encodeURIComponent(mime);
    return invoke<AssetInfo>("store_asset", bytes, { headers });
  },
  importOneNote: (path: string) => invoke<ImportResult>("import_onenote", { path }),
  importFolder: (path: string) => invoke<ImportResult>("import_folder", { path }),
  exportFolder: (path: string, notebookIds: string[] = []) =>
    invoke<ExportSummary>("export_folder", { path, notebookIds }),
  dataDir: () => invoke<string>("data_dir"),
};

export { onPageUpdate, type PageUpdateEvent } from "./platform";
