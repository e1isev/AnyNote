import { useCallback, useEffect, useState } from "react";
import { FileText, FolderClosed, NotebookPen, RotateCcw, Trash2 } from "lucide-react";
import { api } from "../api";
import type { TrashItem } from "../types";
import { relativeTime } from "../time";
import { confirmAction, reportError, toast } from "../ui/overlay";

const ICONS = { notebook: <NotebookPen size={16} />, section: <FolderClosed size={16} />, page: <FileText size={16} /> };

export function TrashView({ onChanged }: { onChanged: () => void }) {
  const [items, setItems] = useState<TrashItem[] | null>(null);
  const load = useCallback(() => api.listTrash().then(setItems).catch(reportError), []);
  useEffect(() => void load(), [load]);

  const restore = (item: TrashItem) =>
    api
      .restoreItem(item.kind, item.id)
      .then(() => {
        toast(`Restored “${item.title || "Untitled"}”`, "success");
        onChanged();
        return load();
      })
      .catch(reportError);

  const empty = async () => {
    if (!(await confirmAction({ title: "Empty trash?", message: "Everything in the trash will be deleted permanently.", confirmText: "Delete forever", danger: true }))) return;
    api
      .emptyTrash()
      .then(() => {
        onChanged();
        return load();
      })
      .catch(reportError);
  };

  return (
    <div className="list-view">
      <header className="list-view-header">
        <h1>
          <Trash2 size={22} /> Trash
        </h1>
        <button className="danger" disabled={!items?.length} onClick={() => void empty()}>
          Empty trash
        </button>
      </header>
      {items?.length === 0 && <div className="empty-state">The trash is empty.</div>}
      {items?.map((item) => (
        <div key={`${item.kind}:${item.id}`} className="trash-row">
          <span className="trash-icon">{ICONS[item.kind]}</span>
          <span className="trash-title">{item.title || "Untitled"}</span>
          <span className="muted small">
            {item.kind} · deleted {relativeTime(item.deletedAt)}
          </span>
          <button onClick={() => void restore(item)}>
            <RotateCcw size={14} /> Restore
          </button>
        </div>
      ))}
    </div>
  );
}
