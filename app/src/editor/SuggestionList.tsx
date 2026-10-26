import { forwardRef, useEffect, useImperativeHandle, useRef, useState, type ReactNode } from "react";
import { ReactRenderer } from "@tiptap/react";
import type { SuggestionKeyDownProps, SuggestionProps } from "@tiptap/suggestion";

export interface SuggestionItem {
  key: string;
  label: string;
  detail?: string;
  icon?: ReactNode;
}

interface ListProps {
  items: SuggestionItem[];
  command: (item: SuggestionItem) => void;
  emptyText: string;
}

export interface SuggestionListHandle {
  onKeyDown: (event: KeyboardEvent) => boolean;
}

export const SuggestionList = forwardRef<SuggestionListHandle, ListProps>(function SuggestionList(
  { items, command, emptyText },
  ref,
) {
  const [selected, setSelected] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => setSelected(0), [items]);
  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${selected}"]`)?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  useImperativeHandle(ref, () => ({
    onKeyDown: (event) => {
      if (items.length === 0) return false;
      if (event.key === "ArrowDown") {
        setSelected((s) => (s + 1) % items.length);
        return true;
      }
      if (event.key === "ArrowUp") {
        setSelected((s) => (s - 1 + items.length) % items.length);
        return true;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        command(items[selected]);
        return true;
      }
      return false;
    },
  }));

  return (
    <div className="suggestions" ref={listRef} role="listbox">
      {items.length === 0 && <div className="suggestions-empty">{emptyText}</div>}
      {items.map((item, i) => (
        <button
          key={item.key}
          data-index={i}
          role="option"
          aria-selected={i === selected}
          className={i === selected ? "suggestion selected" : "suggestion"}
          onMouseEnter={() => setSelected(i)}
          onMouseDown={(e) => {
            e.preventDefault();
            command(item);
          }}
        >
          {item.icon && <span className="suggestion-icon">{item.icon}</span>}
          <span className="suggestion-label">{item.label}</span>
          {item.detail && <span className="suggestion-detail">{item.detail}</span>}
        </button>
      ))}
    </div>
  );
});

/** A `render` implementation for @tiptap/suggestion that shows {@link SuggestionList}. */
export function renderSuggestions(emptyText: string) {
  return () => {
    let renderer: ReactRenderer<SuggestionListHandle, ListProps> | null = null;
    let popup: HTMLDivElement | null = null;

    const place = (props: SuggestionProps<SuggestionItem>) => {
      const rect = props.clientRect?.();
      if (!popup || !rect) return;
      const height = popup.offsetHeight || 280;
      const below = rect.bottom + 6 + height < window.innerHeight;
      popup.style.left = `${Math.min(rect.left, window.innerWidth - 340)}px`;
      popup.style.top = below ? `${rect.bottom + 6}px` : `${Math.max(8, rect.top - height - 6)}px`;
    };

    return {
      onStart: (props: SuggestionProps<SuggestionItem>) => {
        renderer = new ReactRenderer(SuggestionList, {
          props: { items: props.items, command: props.command, emptyText },
          editor: props.editor,
        });
        popup = document.createElement("div");
        popup.className = "suggestion-popup";
        popup.appendChild(renderer.element);
        document.body.appendChild(popup);
        place(props);
      },
      onUpdate: (props: SuggestionProps<SuggestionItem>) => {
        renderer?.updateProps({ items: props.items, command: props.command, emptyText });
        requestAnimationFrame(() => place(props));
      },
      onKeyDown: ({ event }: SuggestionKeyDownProps) => {
        if (event.key === "Escape") {
          popup?.remove();
          popup = null;
          return true;
        }
        return renderer?.ref?.onKeyDown(event) ?? false;
      },
      onExit: () => {
        popup?.remove();
        popup = null;
        renderer?.destroy();
        renderer = null;
      },
    };
  };
}
