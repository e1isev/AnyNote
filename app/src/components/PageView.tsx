import { useEffect, useRef, useState } from "react";
import { generateHTML } from "@tiptap/core";
import { Mathematics } from "@tiptap/extension-mathematics";
import { writeText } from "./clipboard";
import { Check, CloudOff, Copy, History, Link2, Loader2, MoreHorizontal, RotateCcw, Save, Trash2, X } from "lucide-react";
import { api } from "../api";
import { PageEditor } from "../editor/Editor";
import { schemaExtensions } from "../editor/extensions";
import { InkBlock } from "../editor/InkBlock";
import { WikiLink } from "../editor/WikiLink";
import { PageSync } from "../editor/provider";
import { setPageTitle } from "../pageTitles";
import { fullDate, relativeTime } from "../time";
import type { Page, PageRef, Version } from "../types";
import { confirmAction, openMenu, reportError, toast } from "../ui/overlay";

interface Props {
  pageId: string;
  onOpenPage: (pageId: string) => void;
  onCreatePage: (title: string) => Promise<string | null>;
  onPagesChanged: () => void;
  onDeleted: () => void;
}

type Panel = "backlinks" | "history" | null;

export function PageView({ pageId, onOpenPage, onCreatePage, onPagesChanged, onDeleted }: Props) {
  const [info, setInfo] = useState<{ page: Page; ref: PageRef } | null>(null);
  const [title, setTitle] = useState("");
  const [sync, setSync] = useState<PageSync | null>(null);
  const [status, setStatus] = useState<"saved" | "saving" | "error">("saved");
  const [panel, setPanel] = useState<Panel>(() => (localStorage.getItem("panel") as Panel) || null);
  const titleTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const titleRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    let cancelled = false;
    setInfo(null);
    api
      .getPage(pageId)
      .then(([page, ref]) => {
        if (cancelled) return;
        setInfo({ page, ref });
        setTitle(page.title);
        if (!page.title) setTimeout(() => titleRef.current?.focus(), 50);
      })
      .catch(reportError);
    const s = new PageSync(pageId);
    s.onStatus = setStatus;
    s.load()
      .then(() => !cancelled && setSync(s))
      .catch(reportError);
    return () => {
      cancelled = true;
      setSync(null);
      void s.destroy().then(onPagesChanged);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageId]);

  useEffect(() => {
    try {
      if (panel) localStorage.setItem("panel", panel);
      else localStorage.removeItem("panel");
    } catch {
      /* storage unavailable */
    }
  }, [panel]);

  // Grow the title box with its content.
  useEffect(() => {
    const el = titleRef.current;
    if (el) {
      el.style.height = "auto";
      el.style.height = `${el.scrollHeight}px`;
    }
  }, [title]);

  const saveTitle = (value: string) => {
    clearTimeout(titleTimer.current);
    titleTimer.current = setTimeout(() => {
      api
        .renamePage(pageId, value)
        .then((p) => {
          setPageTitle(pageId, p.title);
          onPagesChanged();
        })
        .catch(reportError);
    }, 400);
  };

  const moreMenu = (e: React.MouseEvent) =>
    openMenu(e, [
      {
        label: "Copy as Markdown",
        icon: <Copy size={15} />,
        onSelect: () =>
          api
            .pageMarkdown(pageId)
            .then((md) => writeText(`# ${title}\n\n${md}`))
            .then(() => toast("Copied as Markdown", "success"))
            .catch(reportError),
      },
      {
        label: "Save a version now",
        icon: <Save size={15} />,
        onSelect: async () => {
          await sync?.flush();
          api
            .snapshotVersion(pageId, "Saved manually")
            .then(() => toast("Version saved", "success"))
            .catch(reportError);
        },
      },
      "separator",
      {
        label: "Delete page",
        icon: <Trash2 size={15} />,
        danger: true,
        onSelect: async () => {
          if (await confirmAction({ title: "Delete page?", message: `“${title || "Untitled page"}” will move to the trash.`, confirmText: "Delete", danger: true })) {
            await sync?.flush();
            api.deletePage(pageId).then(onDeleted).catch(reportError);
          }
        },
      },
    ]);

  return (
    <div className="page-view">
      <div className="page-main">
        <header className="page-header">
          <div className="crumbs">
            {info && (
              <>
                <span>{info.ref.notebookTitle}</span>
                <span className="crumb-sep">›</span>
                <span>{info.ref.sectionTitle}</span>
              </>
            )}
            <span className="save-status" title={status === "error" ? "Saving failed; retrying" : status === "saving" ? "Saving…" : "All changes saved"}>
              {status === "saving" && <Loader2 size={13} className="spin" />}
              {status === "saved" && <Check size={13} />}
              {status === "error" && <CloudOff size={13} />}
            </span>
          </div>
          <div className="page-actions">
            <button className={panel === "backlinks" ? "icon-btn active" : "icon-btn"} title="Backlinks" onClick={() => setPanel(panel === "backlinks" ? null : "backlinks")}>
              <Link2 size={16} />
            </button>
            <button className={panel === "history" ? "icon-btn active" : "icon-btn"} title="Version history" onClick={() => setPanel(panel === "history" ? null : "history")}>
              <History size={16} />
            </button>
            <button className="icon-btn" title="More" onClick={moreMenu}>
              <MoreHorizontal size={16} />
            </button>
          </div>
        </header>
        <div className="page-scroll">
          <div className="page-paper">
            <textarea
              ref={titleRef}
              className="page-title"
              rows={1}
              value={title}
              placeholder="Untitled page"
              aria-label="Page title"
              onChange={(e) => {
                const v = e.target.value.replace(/\n/g, " ");
                setTitle(v);
                saveTitle(v);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === "ArrowDown") {
                  e.preventDefault();
                  (document.querySelector(".page-content") as HTMLElement | null)?.focus();
                }
              }}
            />
            {info && <div className="page-meta">Created {fullDate(info.page.createdAt)}</div>}
            {sync ? (
              <PageEditor key={pageId} sync={sync} onOpenPage={onOpenPage} onCreatePage={onCreatePage} />
            ) : (
              <div className="loading">Loading…</div>
            )}
          </div>
        </div>
      </div>
      {panel === "backlinks" && <BacklinksPanel pageId={pageId} onOpenPage={onOpenPage} onClose={() => setPanel(null)} />}
      {panel === "history" && <HistoryPanel pageId={pageId} sync={sync} onClose={() => setPanel(null)} />}
    </div>
  );
}

function BacklinksPanel({ pageId, onOpenPage, onClose }: { pageId: string; onOpenPage: (id: string) => void; onClose: () => void }) {
  const [links, setLinks] = useState<PageRef[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    const load = () =>
      api
        .backlinks(pageId)
        .then((l) => !cancelled && setLinks(l))
        .catch(reportError);
    void load();
    const t = setInterval(load, 5000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [pageId]);
  return (
    <aside className="side-panel" aria-label="Backlinks">
      <div className="side-panel-header">
        <span>Backlinks</span>
        <button className="icon-btn" title="Close" onClick={onClose}>
          <X size={15} />
        </button>
      </div>
      {links === null && <p className="muted">Loading…</p>}
      {links?.length === 0 && <p className="muted">No pages link here yet. Type [[ in another page to link to this one.</p>}
      {links?.map((l) => (
        <button key={l.id} className="ref-row" onClick={() => onOpenPage(l.id)}>
          <span className="ref-title">{l.title || "Untitled page"}</span>
          <span className="ref-path">
            {l.notebookTitle} › {l.sectionTitle}
          </span>
        </button>
      ))}
    </aside>
  );
}

const previewExtensions = [...schemaExtensions(), Mathematics, InkBlock, WikiLink];

function HistoryPanel({ pageId, sync, onClose }: { pageId: string; sync: PageSync | null; onClose: () => void }) {
  const [versions, setVersions] = useState<Version[] | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [preview, setPreview] = useState<string>("");

  const load = () =>
    api
      .listVersions(pageId)
      .then(setVersions)
      .catch(reportError);

  useEffect(() => {
    setSelected(null);
    setPreview("");
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageId]);

  useEffect(() => {
    if (selected === null) return;
    api
      .versionContent(selected)
      .then((doc) => {
        try {
          setPreview(generateHTML(doc as never, previewExtensions));
        } catch (e) {
          setPreview(`<p>Preview unavailable: ${String(e)}</p>`);
        }
      })
      .catch(reportError);
  }, [selected]);

  const restore = async (v: Version) => {
    if (!(await confirmAction({ title: "Restore this version?", message: `The page will go back to how it was ${relativeTime(v.createdAt)}. The current state is saved as a version first.`, confirmText: "Restore" })))
      return;
    await sync?.flush();
    api
      .restoreVersion(v.id)
      .then(() => {
        toast("Version restored", "success");
        setSelected(null);
        void load();
      })
      .catch(reportError);
  };

  return (
    <aside className="side-panel history" aria-label="Version history">
      <div className="side-panel-header">
        <span>Version history</span>
        <button className="icon-btn" title="Close" onClick={onClose}>
          <X size={15} />
        </button>
      </div>
      <p className="muted small">A version is kept every 10 minutes while you edit.</p>
      {versions?.length === 0 && <p className="muted">No versions yet.</p>}
      {versions?.map((v) => (
        <div key={v.id} className={selected === v.id ? "version-row selected" : "version-row"}>
          <button className="version-pick" onClick={() => setSelected(selected === v.id ? null : v.id)}>
            <span>{fullDate(v.createdAt)}</span>
            {v.label && <span className="version-label">{v.label}</span>}
          </button>
          {selected === v.id && (
            <>
              <div className="version-preview page-content" dangerouslySetInnerHTML={{ __html: preview }} />
              <button className="primary small" onClick={() => void restore(v)}>
                <RotateCcw size={14} /> Restore this version
              </button>
            </>
          )}
        </div>
      ))}
    </aside>
  );
}
