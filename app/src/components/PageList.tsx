import { useMemo, useState } from "react";
import { FilePlus2, Indent, Outdent, Pencil, Plus, Trash2, CornerDownRight } from "lucide-react";
import { api } from "../api";
import type { Page, Section } from "../types";
import { relativeTime } from "../time";
import { confirmAction, openMenu, reportError } from "../ui/overlay";

interface Props {
  section: Section | null;
  pages: Page[];
  currentPageId: string | null;
  onOpen: (pageId: string) => void;
  onChanged: () => void;
  onNewPage: (parentId?: string | null, after?: string) => void;
}

/** Depth of each page in the subpage tree, in display order. */
function withDepth(pages: Page[]): { page: Page; depth: number }[] {
  const byId = new Map(pages.map((p) => [p.id, p]));
  return pages.map((page) => {
    let depth = 0;
    let parent = page.parentId ? byId.get(page.parentId) : undefined;
    while (parent && depth < 8) {
      depth++;
      parent = parent.parentId ? byId.get(parent.parentId) : undefined;
    }
    return { page, depth };
  });
}

export function PageList({ section, pages, currentPageId, onOpen, onChanged, onNewPage }: Props) {
  const rows = useMemo(() => withDepth(pages), [pages]);
  const [dragOver, setDragOver] = useState<string | null>(null);

  if (!section) {
    return (
      <aside className="page-list empty">
        <p>Select a section to see its pages.</p>
      </aside>
    );
  }

  const run = (p: Promise<unknown>) => p.then(onChanged).catch(reportError);

  /** The last row of a page's subtree, so a new sibling lands after its subpages. */
  const lastDescendant = (index: number) => {
    let end = index;
    while (end + 1 < rows.length && rows[end + 1].depth > rows[index].depth) end++;
    return rows[end].page.id;
  };

  const pageMenu = (e: React.MouseEvent, page: Page, index: number) => {
    const prev = index > 0 ? rows[index - 1] : null;
    const depth = rows[index].depth;
    // Indent under the closest previous page at the same depth.
    let newParent: string | null = null;
    for (let i = index - 1; i >= 0; i--) {
      if (rows[i].depth === depth) {
        newParent = rows[i].page.id;
        break;
      }
      if (rows[i].depth < depth) break;
    }
    const parent = page.parentId ? pages.find((p) => p.id === page.parentId) : undefined;
    openMenu(e, [
      { label: "New page below", icon: <FilePlus2 size={15} />, onSelect: () => onNewPage(page.parentId, lastDescendant(index)) },
      { label: "New subpage", icon: <CornerDownRight size={15} />, onSelect: () => onNewPage(page.id) },
      {
        label: "Rename",
        icon: <Pencil size={15} />,
        onSelect: () => {
          onOpen(page.id);
          setTimeout(() => (document.querySelector(".page-title") as HTMLInputElement | null)?.select(), 150);
        },
      },
      "separator",
      {
        label: "Make subpage",
        icon: <Indent size={15} />,
        disabled: !newParent || !prev,
        onSelect: () => newParent && run(api.movePage(page.id, section.id, newParent, { after: prev?.page.id })),
      },
      {
        label: "Promote",
        icon: <Outdent size={15} />,
        disabled: !parent,
        onSelect: () => parent && run(api.movePage(page.id, section.id, parent.parentId, { after: page.id })),
      },
      "separator",
      {
        label: "Delete page",
        icon: <Trash2 size={15} />,
        danger: true,
        onSelect: async () => {
          const hasChildren = pages.some((p) => p.parentId === page.id);
          if (
            await confirmAction({
              title: "Delete page?",
              message: `“${page.title || "Untitled page"}”${hasChildren ? " and its subpages" : ""} will move to the trash.`,
              confirmText: "Delete",
              danger: true,
            })
          )
            run(api.deletePage(page.id));
        },
      },
    ]);
  };

  return (
    <aside className="page-list" aria-label="Pages">
      <div className="page-list-header">
        <span className="section-name" title={section.title}>
          {section.title}
        </span>
        <button className="icon-btn" title="New page (Ctrl+N)" onClick={() => onNewPage(null)}>
          <Plus size={16} />
        </button>
      </div>
      <div className="pages">
        {rows.map(({ page, depth }, index) => (
          <div
            key={page.id}
            role="button"
            tabIndex={0}
            draggable
            className={[
              "page-row",
              page.id === currentPageId ? "selected" : "",
              dragOver === page.id ? "drag-over" : "",
            ].join(" ")}
            style={{ paddingLeft: 14 + depth * 16 }}
            onClick={() => onOpen(page.id)}
            onKeyDown={(e) => e.key === "Enter" && onOpen(page.id)}
            onContextMenu={(e) => pageMenu(e, page, index)}
            onDragStart={(e) => {
              e.dataTransfer.setData("application/x-anynote-page", page.id);
              e.dataTransfer.effectAllowed = "move";
            }}
            onDragOver={(e) => {
              if (e.dataTransfer.types.includes("application/x-anynote-page")) {
                e.preventDefault();
                setDragOver(page.id);
              }
            }}
            onDragLeave={() => setDragOver(null)}
            onDrop={(e) => {
              setDragOver(null);
              const id = e.dataTransfer.getData("application/x-anynote-page");
              if (id && id !== page.id) run(api.movePage(id, section.id, page.parentId, { after: page.id }));
            }}
          >
            <span className={page.title ? "page-title-text" : "page-title-text untitled"}>{page.title || "Untitled page"}</span>
            <span className="page-date">{relativeTime(page.updatedAt)}</span>
          </div>
        ))}
        <button className="add-page" onClick={() => onNewPage(null)}>
          <Plus size={15} /> Add page
        </button>
      </div>
    </aside>
  );
}
