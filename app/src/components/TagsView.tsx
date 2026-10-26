import { useEffect, useState } from "react";
import { Hash } from "lucide-react";
import { api } from "../api";
import type { PageRef, TagCount } from "../types";
import { relativeTime } from "../time";
import { reportError } from "../ui/overlay";

/** All #tags, and the pages using the selected one. */
export function TagsView({ tag, onSelectTag, onOpenPage }: { tag?: string; onSelectTag: (t: string) => void; onOpenPage: (id: string) => void }) {
  const [tags, setTags] = useState<TagCount[] | null>(null);
  const [pages, setPages] = useState<PageRef[]>([]);

  useEffect(() => {
    api.listTags().then(setTags).catch(reportError);
  }, []);

  useEffect(() => {
    if (tag) api.pagesWithTag(tag).then(setPages).catch(reportError);
    else setPages([]);
  }, [tag]);

  return (
    <div className="list-view">
      <header className="list-view-header">
        <h1>
          <Hash size={22} /> Tags
        </h1>
      </header>
      {tags?.length === 0 && <div className="empty-state">No tags yet. Write #something on any page to tag it.</div>}
      <div className="tag-cloud">
        {tags?.map((t) => (
          <button key={t.tag} className={t.tag === tag ? "tag-chip active" : "tag-chip"} onClick={() => onSelectTag(t.tag)}>
            #{t.tag} <span className="tag-count">{t.count}</span>
          </button>
        ))}
      </div>
      {tag && (
        <section className="tag-pages">
          <h2>#{tag}</h2>
          {pages.map((p) => (
            <button key={p.id} className="ref-row" onClick={() => onOpenPage(p.id)}>
              <span className="ref-title">{p.title || "Untitled page"}</span>
              <span className="ref-path">
                {p.notebookTitle} › {p.sectionTitle} · {relativeTime(p.updatedAt)}
              </span>
            </button>
          ))}
        </section>
      )}
    </div>
  );
}
