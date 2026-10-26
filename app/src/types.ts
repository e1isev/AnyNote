export interface Notebook {
  id: string;
  title: string;
  color: string | null;
  position: number;
  createdAt: number;
  updatedAt: number;
}

export interface Section {
  id: string;
  notebookId: string;
  parentId: string | null;
  title: string;
  color: string | null;
  position: number;
  createdAt: number;
  updatedAt: number;
}

export interface NotebookTree extends Notebook {
  sections: Section[];
}

export interface Page {
  id: string;
  sectionId: string;
  parentId: string | null;
  title: string;
  position: number;
  createdAt: number;
  updatedAt: number;
}

/** A page with its location, for lists spanning notebooks. */
export interface PageRef {
  id: string;
  title: string;
  sectionId: string;
  sectionTitle: string;
  notebookId: string;
  notebookTitle: string;
  updatedAt: number;
}

export interface SearchHit {
  page: PageRef;
  /** Matches are wrapped in \u0001 … \u0002. */
  snippet: string;
}

export interface TaskRow {
  page: PageRef;
  blockId: string;
  text: string;
  checked: boolean;
}

export interface TagCount {
  tag: string;
  count: number;
}

export interface Version {
  id: number;
  pageId: string;
  createdAt: number;
  label: string | null;
}

export interface TrashItem {
  kind: "notebook" | "section" | "page";
  id: string;
  title: string;
  deletedAt: number;
}

export interface AssetInfo {
  hash: string;
  url: string;
  mime: string;
  size: number;
  name: string | null;
}

export interface ImportResult {
  notebookIds: string[];
  sections: number;
  pages: number;
  assets: number;
  warnings: string[];
}

export interface ExportSummary {
  notebooks: number;
  sections: number;
  pages: number;
  assets: number;
}

/** ProseMirror JSON. */
export interface PMNode {
  type: string;
  attrs?: Record<string, unknown>;
  content?: PMNode[];
  text?: string;
  marks?: { type: string; attrs?: Record<string, unknown> }[];
}

/** What the main pane shows. */
export type View =
  | { kind: "page"; pageId: string }
  | { kind: "tasks" }
  | { kind: "tags"; tag?: string }
  | { kind: "trash" }
  | { kind: "empty" };
