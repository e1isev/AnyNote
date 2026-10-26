import { useCallback, useEffect, useRef, useState } from "react";
import { FilePlus2, NotebookPen } from "lucide-react";
import { api } from "./api";
import { Sidebar } from "./components/Sidebar";
import { PageList } from "./components/PageList";
import { PageView } from "./components/PageView";
import { SearchPalette } from "./components/SearchPalette";
import { TasksView } from "./components/TasksView";
import { TagsView } from "./components/TagsView";
import { TrashView } from "./components/TrashView";
import { setPageTitle } from "./pageTitles";
import type { NotebookTree, Page, Section, View } from "./types";
import { OverlayHost, reportError } from "./ui/overlay";

function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v ? (JSON.parse(v) as T) : fallback;
  } catch {
    return fallback;
  }
}

function store(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage unavailable */
  }
}

export function App() {
  const [tree, setTree] = useState<NotebookTree[]>([]);
  const [section, setSection] = useState<Section | null>(null);
  const [pages, setPages] = useState<Page[]>([]);
  const [view, setView] = useState<View>({ kind: "empty" });
  const [searchOpen, setSearchOpen] = useState(false);
  const sectionRef = useRef<Section | null>(null);
  sectionRef.current = section;

  const refreshTree = useCallback(async () => {
    const t = await api.listTree();
    setTree(t);
    return t;
  }, []);

  const refreshPages = useCallback(async (sectionId?: string) => {
    const id = sectionId ?? sectionRef.current?.id;
    if (!id) return [];
    const list = await api.listPages(id);
    if (sectionRef.current?.id === id || sectionId) setPages(list);
    list.forEach((p) => setPageTitle(p.id, p.title));
    return list;
  }, []);

  const findSection = (t: NotebookTree[], id: string) => {
    for (const nb of t) {
      const s = nb.sections.find((x) => x.id === id);
      if (s) return s;
    }
    return null;
  };

  const openPage = useCallback(
    async (pageId: string, knownTree?: NotebookTree[]) => {
      try {
        const [page] = await api.getPage(pageId);
        if (sectionRef.current?.id !== page.sectionId) {
          const t = knownTree ?? (await refreshTree());
          const sec = findSection(t, page.sectionId);
          setSection(sec);
          sectionRef.current = sec;
        }
        await refreshPages(page.sectionId);
        setView({ kind: "page", pageId });
        store("lastPage", pageId);
        store(`lastPage:${page.sectionId}`, pageId);
      } catch (e) {
        reportError(e);
      }
    },
    [refreshPages, refreshTree],
  );

  const selectSection = useCallback(
    async (sec: Section) => {
      setSection(sec);
      sectionRef.current = sec;
      const list = await refreshPages(sec.id).catch((e) => (reportError(e), [] as Page[]));
      const remembered = load<string | null>(`lastPage:${sec.id}`, null);
      const target = list.find((p) => p.id === remembered) ?? list[0];
      if (target) void openPage(target.id);
      else setView({ kind: "empty" });
    },
    [openPage, refreshPages],
  );

  // First load: reopen the last page, else the first section.
  useEffect(() => {
    (async () => {
      const t = await refreshTree();
      const last = load<string | null>("lastPage", null);
      if (last) {
        try {
          await api.getPage(last);
          await openPage(last, t);
          return;
        } catch {
          /* page is gone */
        }
      }
      const first = t[0]?.sections[0];
      if (first) await selectSection(first);
    })().catch(reportError);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onTreeChanged = useCallback(async () => {
    const t = await refreshTree().catch((e) => (reportError(e), null));
    if (!t) return;
    const current = sectionRef.current;
    if (current) {
      const still = findSection(t, current.id);
      if (still) {
        setSection(still);
        sectionRef.current = still;
        const list = await refreshPages(still.id);
        if (view.kind === "page" && !list.some((p) => p.id === view.pageId)) {
          if (list[0]) void openPage(list[0].id);
          else setView({ kind: "empty" });
        }
      } else {
        setSection(null);
        sectionRef.current = null;
        setPages([]);
        const first = t[0]?.sections[0];
        if (first) void selectSection(first);
        else setView({ kind: "empty" });
      }
    }
  }, [openPage, refreshPages, refreshTree, selectSection, view]);

  const newPage = useCallback(
    async (parentId?: string | null) => {
      const sec = sectionRef.current;
      if (!sec) return;
      try {
        const page = await api.createPage(sec.id, { parentId: parentId ?? null });
        await openPage(page.id);
      } catch (e) {
        reportError(e);
      }
    },
    [openPage],
  );

  const createLinkedPage = useCallback(
    async (title: string) => {
      const sec = sectionRef.current;
      if (!sec) return null;
      try {
        const page = await api.createPage(sec.id, { title });
        setPageTitle(page.id, page.title);
        await refreshPages();
        return page.id;
      } catch (e) {
        reportError(e);
        return null;
      }
    },
    [refreshPages],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.key.toLowerCase() === "k" && !document.querySelector(".ProseMirror:focus")) {
        e.preventDefault();
        setSearchOpen(true);
      } else if (mod && e.shiftKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setSearchOpen(true);
      } else if (mod && e.key.toLowerCase() === "n") {
        e.preventDefault();
        void newPage(e.shiftKey && view.kind === "page" ? view.pageId : null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [newPage, view]);

  const currentPageId = view.kind === "page" ? view.pageId : null;

  let main;
  switch (view.kind) {
    case "page":
      main = (
        <PageView
          key={view.pageId}
          pageId={view.pageId}
          onOpenPage={(id) => void openPage(id)}
          onCreatePage={createLinkedPage}
          onPagesChanged={() => void refreshPages().catch(() => {})}
          onDeleted={() => void onTreeChanged()}
        />
      );
      break;
    case "tasks":
      main = <TasksView onOpenPage={(id) => void openPage(id)} />;
      break;
    case "tags":
      main = <TagsView tag={view.tag} onSelectTag={(tag) => setView({ kind: "tags", tag })} onOpenPage={(id) => void openPage(id)} />;
      break;
    case "trash":
      main = <TrashView onChanged={() => void onTreeChanged()} />;
      break;
    default:
      main = (
        <div className="empty-main">
          <NotebookPen size={40} />
          {section ? (
            <>
              <p>This section has no pages yet.</p>
              <button className="primary" onClick={() => void newPage(null)}>
                <FilePlus2 size={16} /> New page
              </button>
            </>
          ) : (
            <p>Pick a section on the left, or create a notebook to get started.</p>
          )}
        </div>
      );
  }

  return (
    <div className="app">
      <Sidebar
        tree={tree}
        view={view}
        sectionId={section?.id ?? null}
        onSelectSection={(s) => void selectSection(s)}
        onView={setView}
        onSearch={() => setSearchOpen(true)}
        onChanged={() => void onTreeChanged()}
        onImported={(ids) => {
          void refreshTree().then((t) => {
            const nb = t.find((n) => n.id === ids[0]);
            if (nb?.sections[0]) void selectSection(nb.sections[0]);
          });
        }}
        onDropPage={(pageId, sec) =>
          void api
            .movePage(pageId, sec.id, null)
            .then(() => refreshPages())
            .catch(reportError)
        }
      />
      <PageList
        section={section}
        pages={pages}
        currentPageId={currentPageId}
        onOpen={(id) => void openPage(id)}
        onChanged={() => void refreshPages().catch(reportError)}
        onNewPage={(parentId) => void newPage(parentId)}
      />
      <main className="main">{main}</main>
      {searchOpen && <SearchPalette onOpen={(id) => void openPage(id)} onClose={() => setSearchOpen(false)} />}
      <OverlayHost />
    </div>
  );
}
