import { useCallback, useEffect, useMemo, useState } from "react";
import { ListTodo } from "lucide-react";
import { api, onPageUpdate } from "../api";
import type { TaskRow } from "../types";
import { reportError } from "../ui/overlay";

/** Every to-do from every notebook, grouped by page. */
export function TasksView({ onOpenPage }: { onOpenPage: (id: string) => void }) {
  const [showDone, setShowDone] = useState(false);
  const [tasks, setTasks] = useState<TaskRow[] | null>(null);

  const load = useCallback(() => api.listTasks(showDone).then(setTasks).catch(reportError), [showDone]);

  useEffect(() => {
    void load();
    const t = setInterval(load, 4000);
    const unlisten = onPageUpdate(() => void load());
    return () => {
      clearInterval(t);
      void unlisten.then((u) => u());
    };
  }, [load]);

  const groups = useMemo(() => {
    const map = new Map<string, TaskRow[]>();
    for (const t of tasks ?? []) {
      const list = map.get(t.page.id) ?? [];
      list.push(t);
      map.set(t.page.id, list);
    }
    return [...map.values()];
  }, [tasks]);

  const toggle = (t: TaskRow) => {
    setTasks((prev) => prev?.map((x) => (x === t ? { ...x, checked: !x.checked } : x)) ?? null);
    api.setTaskChecked(t.page.id, t.blockId, !t.checked).then(load).catch(reportError);
  };

  const open = tasks?.filter((t) => !t.checked).length ?? 0;

  return (
    <div className="list-view">
      <header className="list-view-header">
        <h1>
          <ListTodo size={22} /> Tasks
        </h1>
        <label className="toggle">
          <input type="checkbox" checked={showDone} onChange={(e) => setShowDone(e.target.checked)} /> Show completed
        </label>
      </header>
      <p className="muted">
        {tasks === null ? "Loading…" : `${open} open task${open === 1 ? "" : "s"} across all notebooks. Ticking one here updates its page.`}
      </p>
      {tasks?.length === 0 && <div className="empty-state">No tasks. Type “[ ] ” on any page to add a to-do.</div>}
      {groups.map((list) => (
        <section key={list[0].page.id} className="task-group">
          <button className="task-group-title" onClick={() => onOpenPage(list[0].page.id)}>
            {list[0].page.title || "Untitled page"}
            <span className="ref-path">
              {list[0].page.notebookTitle} › {list[0].page.sectionTitle}
            </span>
          </button>
          {list.map((t) => (
            <label key={t.blockId} className={t.checked ? "task-row done" : "task-row"}>
              <input type="checkbox" checked={t.checked} onChange={() => toggle(t)} />
              <span>{t.text || <em className="muted">Empty to-do</em>}</span>
            </label>
          ))}
        </section>
      ))}
    </div>
  );
}
