import { useState } from "react";
import {
  BookOpen,
  ChevronDown,
  ChevronRight,
  Download,
  FilePlus2,
  FolderInput,
  FolderPlus,
  Hash,
  ListTodo,
  NotebookPen,
  Pencil,
  Plus,
  Search,
  Trash2,
  Upload,
  ArrowUp,
  ArrowDown,
} from "lucide-react";
import { pickExportFolder, pickImportPath } from "../platform";
import { api } from "../api";
import type { NotebookTree, Section, View } from "../types";
import { confirmAction, openMenu, promptText, reportError, toast, type MenuEntry } from "../ui/overlay";

export const COLORS = ["#7c5cff", "#2f6fdf", "#0ea5a4", "#1f9d55", "#b08800", "#e07b00", "#d93f3f", "#d6409f", "#6e7781"];

interface Props {
  tree: NotebookTree[];
  view: View;
  sectionId: string | null;
  onSelectSection: (s: Section) => void;
  onView: (v: View) => void;
  onSearch: () => void;
  onChanged: () => void;
  onImported: (notebookIds: string[]) => void;
  onDropPage: (pageId: string, section: Section) => void;
}

export function Sidebar(props: Props) {
  const { tree, view, sectionId } = props;
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>(() => {
    try {
      return JSON.parse(localStorage.getItem("collapsed") ?? "{}");
    } catch {
      return {};
    }
  });
  const [busy, setBusy] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const toggle = (id: string) => {
    const next = { ...collapsed, [id]: !collapsed[id] };
    setCollapsed(next);
    try {
      localStorage.setItem("collapsed", JSON.stringify(next));
    } catch {
      /* storage unavailable */
    }
  };

  const run = (p: Promise<unknown>) => p.then(props.onChanged).catch(reportError);

  const newNotebook = async () => {
    const title = await promptText({ title: "New notebook", label: "Name", initial: "", placeholder: "e.g. Work", confirmText: "Create" });
    if (title?.trim()) run(api.createNotebook(title.trim()));
  };

  const newSection = async (notebookId: string, parentId?: string) => {
    const title = await promptText({
      title: parentId ? "New subsection" : "New section",
      label: "Name",
      placeholder: "e.g. Meeting notes",
      confirmText: "Create",
    });
    if (!title?.trim()) return;
    try {
      const sec = await api.createSection(notebookId, title.trim(), parentId);
      props.onChanged();
      props.onSelectSection(sec);
    } catch (e) {
      reportError(e);
    }
  };

  const doImport = async (kind: "onenote-file" | "onenote-folder" | "markdown") => {
    const path = await pickImportPath(kind);
    if (!path) return;
    setBusy("Importing…");
    try {
      const r = kind === "markdown" ? await api.importFolder(path) : await api.importOneNote(path);
      toast(`Imported ${r.pages} page${r.pages === 1 ? "" : "s"} in ${r.sections} section${r.sections === 1 ? "" : "s"}`, "success");
      if (r.warnings.length) {
        console.warn("import warnings", r.warnings);
        toast(`${r.warnings.length} item(s) could not be fully imported. First: ${r.warnings[0]}`, "info");
      }
      props.onChanged();
      props.onImported(r.notebookIds);
    } catch (e) {
      reportError(e);
    } finally {
      setBusy(null);
    }
  };

  const doExport = async () => {
    const path = await pickExportFolder("Export notebooks to…");
    if (!path) return;
    setBusy("Exporting…");
    try {
      const s = await api.exportFolder(path);
      toast(`Exported ${s.pages} pages from ${s.notebooks} notebook(s) as Markdown`, "success");
    } catch (e) {
      reportError(e);
    } finally {
      setBusy(null);
    }
  };

  const notebookMenu = (e: React.MouseEvent, nb: NotebookTree, index: number) => {
    const items: MenuEntry[] = [
      { label: "New section", icon: <FolderPlus size={15} />, onSelect: () => void newSection(nb.id) },
      {
        label: "Rename",
        icon: <Pencil size={15} />,
        onSelect: async () => {
          const title = await promptText({ title: "Rename notebook", initial: nb.title, confirmText: "Rename" });
          if (title?.trim()) run(api.updateNotebook(nb.id, { title: title.trim() }));
        },
      },
      "separator",
      { colors: COLORS, current: nb.color, onPick: (c) => run(api.updateNotebook(nb.id, { color: c })) },
      "separator",
      {
        label: "Move up",
        icon: <ArrowUp size={15} />,
        disabled: index === 0,
        onSelect: () => run(api.moveNotebook(nb.id, { before: tree[index - 1]?.id })),
      },
      {
        label: "Move down",
        icon: <ArrowDown size={15} />,
        disabled: index === tree.length - 1,
        onSelect: () => run(api.moveNotebook(nb.id, { after: tree[index + 1]?.id })),
      },
      {
        label: "Export…",
        icon: <Download size={15} />,
        onSelect: async () => {
          const path = await pickExportFolder(`Export “${nb.title}” to…`);
          if (path)
            api
              .exportFolder(path, [nb.id])
              .then((s) => toast(`Exported ${s.pages} pages`, "success"))
              .catch(reportError);
        },
      },
      "separator",
      {
        label: "Delete notebook",
        icon: <Trash2 size={15} />,
        danger: true,
        onSelect: async () => {
          if (await confirmAction({ title: "Delete notebook?", message: `“${nb.title}” and everything in it will move to the trash.`, confirmText: "Delete", danger: true }))
            run(api.deleteNotebook(nb.id));
        },
      },
    ];
    openMenu(e, items);
  };

  const sectionMenu = (e: React.MouseEvent, nb: NotebookTree, sec: Section, siblings: Section[]) => {
    const i = siblings.findIndex((s) => s.id === sec.id);
    openMenu(e, [
      {
        label: "New page",
        icon: <FilePlus2 size={15} />,
        onSelect: () => {
          props.onSelectSection(sec);
          api.createPage(sec.id).then(props.onChanged).catch(reportError);
        },
      },
      { label: "New subsection", icon: <FolderPlus size={15} />, onSelect: () => void newSection(nb.id, sec.id) },
      {
        label: "Rename",
        icon: <Pencil size={15} />,
        onSelect: async () => {
          const title = await promptText({ title: "Rename section", initial: sec.title, confirmText: "Rename" });
          if (title?.trim()) run(api.updateSection(sec.id, { title: title.trim() }));
        },
      },
      "separator",
      { colors: COLORS, current: sec.color, onPick: (c) => run(api.updateSection(sec.id, { color: c })) },
      "separator",
      {
        label: "Move up",
        icon: <ArrowUp size={15} />,
        disabled: i <= 0,
        onSelect: () => run(api.moveSection(sec.id, nb.id, sec.parentId, { before: siblings[i - 1]?.id })),
      },
      {
        label: "Move down",
        icon: <ArrowDown size={15} />,
        disabled: i === siblings.length - 1,
        onSelect: () => run(api.moveSection(sec.id, nb.id, sec.parentId, { after: siblings[i + 1]?.id })),
      },
      ...tree
        .filter((other) => other.id !== nb.id)
        .slice(0, 6)
        .map((other) => ({
          label: `Move to “${other.title}”`,
          icon: <NotebookPen size={15} />,
          onSelect: () => run(api.moveSection(sec.id, other.id, null)),
        })),
      "separator",
      {
        label: "Delete section",
        icon: <Trash2 size={15} />,
        danger: true,
        onSelect: async () => {
          if (await confirmAction({ title: "Delete section?", message: `“${sec.title}” and its pages will move to the trash.`, confirmText: "Delete", danger: true }))
            run(api.deleteSection(sec.id));
        },
      },
    ]);
  };

  const renderSections = (nb: NotebookTree, parentId: string | null, depth: number) => {
    const siblings = nb.sections.filter((s) => s.parentId === parentId);
    return siblings.map((sec) => {
      const hasChildren = nb.sections.some((s) => s.parentId === sec.id);
      const open = !collapsed[sec.id];
      return (
        <div key={sec.id}>
          <div
            className={[
              "tree-row section-row",
              sectionId === sec.id && view.kind === "page" ? "selected" : "",
              sectionId === sec.id && view.kind !== "page" ? "current" : "",
              dropTarget === sec.id ? "drop-target" : "",
            ].join(" ")}
            style={{ paddingLeft: 26 + depth * 14 }}
            onClick={() => props.onSelectSection(sec)}
            onContextMenu={(e) => sectionMenu(e, nb, sec, siblings)}
            onDragOver={(e) => {
              if (e.dataTransfer.types.includes("application/x-anynote-page")) {
                e.preventDefault();
                setDropTarget(sec.id);
              }
            }}
            onDragLeave={() => setDropTarget(null)}
            onDrop={(e) => {
              setDropTarget(null);
              const pageId = e.dataTransfer.getData("application/x-anynote-page");
              if (pageId) props.onDropPage(pageId, sec);
            }}
            title={sec.title}
          >
            {hasChildren ? (
              <button className="twisty" onClick={(e) => (e.stopPropagation(), toggle(sec.id))} aria-label={open ? "Collapse" : "Expand"}>
                {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
              </button>
            ) : (
              <span className="twisty" />
            )}
            <span className="section-tab" style={{ background: sec.color ?? nb.color ?? "var(--muted)" }} />
            <span className="tree-label">{sec.title}</span>
          </div>
          {hasChildren && open && renderSections(nb, sec.id, depth + 1)}
        </div>
      );
    });
  };

  return (
    <nav className="sidebar" aria-label="Notebooks">
      <div className="sidebar-header">
        <span className="brand">
          <BookOpen size={18} /> AnyNote
        </span>
      </div>
      <button className="search-button" onClick={props.onSearch}>
        <Search size={15} /> <span>Search</span>
        <kbd>Ctrl K</kbd>
      </button>
      <div className="nav-links">
        <button className={view.kind === "tasks" ? "nav-link active" : "nav-link"} onClick={() => props.onView({ kind: "tasks" })}>
          <ListTodo size={16} /> Tasks
        </button>
        <button className={view.kind === "tags" ? "nav-link active" : "nav-link"} onClick={() => props.onView({ kind: "tags" })}>
          <Hash size={16} /> Tags
        </button>
        <button className={view.kind === "trash" ? "nav-link active" : "nav-link"} onClick={() => props.onView({ kind: "trash" })}>
          <Trash2 size={16} /> Trash
        </button>
      </div>
      <div className="tree-heading">
        <span>Notebooks</span>
        <button className="icon-btn" title="New notebook" onClick={() => void newNotebook()}>
          <Plus size={15} />
        </button>
      </div>
      <div className="tree">
        {tree.map((nb, index) => {
          const open = !collapsed[nb.id];
          return (
            <div key={nb.id} className="notebook">
              <div className="tree-row notebook-row" onClick={() => toggle(nb.id)} onContextMenu={(e) => notebookMenu(e, nb, index)} title={nb.title}>
                <span className="twisty">{open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</span>
                <span className="notebook-icon" style={{ color: nb.color ?? "var(--accent)" }}>
                  <NotebookPen size={16} />
                </span>
                <span className="tree-label">{nb.title}</span>
                <button
                  className="icon-btn row-action"
                  title="New section"
                  onClick={(e) => {
                    e.stopPropagation();
                    void newSection(nb.id);
                  }}
                >
                  <Plus size={14} />
                </button>
              </div>
              {open && renderSections(nb, null, 0)}
              {open && nb.sections.length === 0 && (
                <button className="tree-empty" onClick={() => void newSection(nb.id)}>
                  Add a section
                </button>
              )}
            </div>
          );
        })}
        {tree.length === 0 && (
          <button className="tree-empty" onClick={() => void newNotebook()}>
            Create your first notebook
          </button>
        )}
      </div>
      <div className="sidebar-footer">
        {busy ? (
          <span className="busy">{busy}</span>
        ) : (
          <>
            <button
              className="footer-btn"
              onClick={(e) =>
                openMenu({ clientX: e.clientX, clientY: e.clientY - 120, preventDefault: () => e.preventDefault() }, [
                  { label: "OneNote section or notebook (.one, .onetoc2)…", icon: <Upload size={15} />, onSelect: () => void doImport("onenote-file") },
                  { label: "OneNote notebook folder…", icon: <FolderInput size={15} />, onSelect: () => void doImport("onenote-folder") },
                  { label: "Markdown or AnyNote folder…", icon: <FolderInput size={15} />, onSelect: () => void doImport("markdown") },
                ])
              }
            >
              <Upload size={15} /> Import
            </button>
            <button className="footer-btn" onClick={() => void doExport()}>
              <Download size={15} /> Export
            </button>
            <button className="footer-btn" title="New notebook" onClick={() => void newNotebook()}>
              <Plus size={15} /> Notebook
            </button>
          </>
        )}
      </div>
    </nav>
  );
}
