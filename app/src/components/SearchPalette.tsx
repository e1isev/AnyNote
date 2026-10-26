import { useEffect, useRef, useState } from "react";
import { FileText, Search } from "lucide-react";
import { api } from "../api";
import type { PageRef, SearchHit } from "../types";
import { relativeTime } from "../time";
import { Snippet } from "./Snippet";

interface Props {
  onOpen: (pageId: string) => void;
  onClose: () => void;
}

interface Result {
  page: PageRef;
  snippet?: string;
}

/** Ctrl+K: full-text search across every notebook. Empty query lists recent pages. */
export function SearchPalette({ onOpen, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Result[]>([]);
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => inputRef.current?.focus(), []);

  useEffect(() => {
    let cancelled = false;
    const t = setTimeout(async () => {
      try {
        let found: Result[];
        if (!query.trim()) {
          found = (await api.recentPages(12)).map((page) => ({ page }));
        } else {
          const [hits, titles] = await Promise.all([api.search(query, 40), api.findPages(query, 5)]);
          const seen = new Set<string>();
          found = [];
          for (const page of titles) {
            seen.add(page.id);
            found.push({ page, snippet: hits.find((h: SearchHit) => h.page.id === page.id)?.snippet });
          }
          for (const h of hits) if (!seen.has(h.page.id)) found.push({ page: h.page, snippet: h.snippet });
        }
        if (!cancelled) {
          setResults(found);
          setSelected(0);
        }
      } catch (e) {
        console.error(e);
      }
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
  }, [query]);

  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${selected}"]`)?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const choose = (r: Result | undefined) => {
    if (!r) return;
    onOpen(r.page.id);
    onClose();
  };

  return (
    <div className="modal-backdrop palette-backdrop" onMouseDown={onClose}>
      <div className="palette" role="dialog" aria-label="Search" onMouseDown={(e) => e.stopPropagation()}>
        <div className="palette-input">
          <Search size={18} />
          <input
            ref={inputRef}
            value={query}
            placeholder="Search all notebooks…"
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setSelected((s) => Math.min(s + 1, results.length - 1));
              }
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setSelected((s) => Math.max(s - 1, 0));
              }
              if (e.key === "Enter") choose(results[selected]);
            }}
          />
        </div>
        <div className="palette-results" ref={listRef}>
          {!query.trim() && results.length > 0 && <div className="palette-section">Recent pages</div>}
          {query.trim() && results.length === 0 && <div className="palette-empty">No pages match “{query}”.</div>}
          {results.map((r, i) => (
            <button
              key={r.page.id}
              data-index={i}
              className={i === selected ? "palette-row selected" : "palette-row"}
              onMouseEnter={() => setSelected(i)}
              onClick={() => choose(r)}
            >
              <FileText size={16} className="palette-icon" />
              <span className="palette-text">
                <span className="palette-title">{r.page.title || "Untitled page"}</span>
                {r.snippet && <Snippet text={r.snippet} />}
                <span className="palette-path">
                  {r.page.notebookTitle} › {r.page.sectionTitle} · {relativeTime(r.page.updatedAt)}
                </span>
              </span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
