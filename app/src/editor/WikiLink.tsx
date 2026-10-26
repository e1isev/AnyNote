import { Node, mergeAttributes } from "@tiptap/core";
import { NodeViewWrapper, ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react";
import Suggestion from "@tiptap/suggestion";
import { PluginKey } from "@tiptap/pm/state";
import { FilePlus2, FileText } from "lucide-react";
import { api } from "../api";
import { usePageTitle } from "../pageTitles";
import { renderSuggestions, type SuggestionItem } from "./SuggestionList";

export interface WikiLinkOptions {
  /** Open a linked page. */
  onOpen: (pageId: string) => void;
  /** Create a page for an unmatched `[[query`; returns its id. */
  onCreate: (title: string) => Promise<string | null>;
}

function WikiLinkView({ node, extension }: ReactNodeViewProps) {
  const pageId = node.attrs.pageId as string | null;
  const live = usePageTitle(pageId);
  const missing = live === null;
  const title = (live ?? (node.attrs.title as string)) || "Untitled";
  return (
    <NodeViewWrapper
      as="span"
      className={missing ? "wiki-link missing" : "wiki-link"}
      title={missing ? "This page no longer exists" : `Open “${title}”`}
      data-page-id={pageId ?? ""}
      onClick={() => pageId && !missing && (extension.options as WikiLinkOptions).onOpen(pageId)}
    >
      {title}
    </NodeViewWrapper>
  );
}

/** `[[Page]]` links, stored by page id so renames never break them. */
export const WikiLink = Node.create<WikiLinkOptions>({
  name: "wikiLink",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,

  addOptions() {
    return { onOpen: () => {}, onCreate: async () => null };
  },

  addAttributes() {
    return {
      pageId: {
        default: null,
        parseHTML: (el) => el.getAttribute("data-page-id"),
        renderHTML: (attrs) => ({ "data-page-id": attrs.pageId }),
      },
      title: {
        default: "",
        parseHTML: (el) => el.textContent ?? "",
        renderHTML: () => ({}),
      },
    };
  },

  parseHTML() {
    return [{ tag: "span[data-wiki-link]" }];
  },

  renderHTML({ node, HTMLAttributes }) {
    return ["span", mergeAttributes(HTMLAttributes, { "data-wiki-link": "", class: "wiki-link" }), node.attrs.title || "Untitled"];
  },

  renderText({ node }) {
    return `[[${node.attrs.title}]]`;
  },

  addNodeView() {
    return ReactNodeViewRenderer(WikiLinkView);
  },

  addProseMirrorPlugins() {
    const options = this.options;
    return [
      Suggestion<SuggestionItem, SuggestionItem>({
        editor: this.editor,
        pluginKey: new PluginKey("wikiLinkSuggestion"),
        char: "[[",
        allowSpaces: true,
        allowedPrefixes: null,
        items: async ({ query }) => {
          const q = query.replace(/\]+$/, "");
          const pages = await api.findPages(q, 8).catch(() => []);
          const items: SuggestionItem[] = pages.map((p) => ({
            key: p.id,
            label: p.title || "Untitled",
            detail: `${p.notebookTitle} › ${p.sectionTitle}`,
            icon: <FileText size={15} />,
          }));
          if (q.trim() && !pages.some((p) => p.title.toLowerCase() === q.trim().toLowerCase())) {
            items.push({ key: `new:${q.trim()}`, label: `Create “${q.trim()}”`, detail: "New page in this section", icon: <FilePlus2 size={15} /> });
          }
          return items;
        },
        command: async ({ editor, range, props }) => {
          let pageId = props.key;
          let title = props.label;
          if (pageId.startsWith("new:")) {
            title = pageId.slice(4);
            const created = await options.onCreate(title);
            if (!created) return;
            pageId = created;
          }
          editor
            .chain()
            .focus()
            .insertContentAt(range, [
              { type: "wikiLink", attrs: { pageId, title } },
              { type: "text", text: " " },
            ])
            .run();
        },
        render: renderSuggestions("No pages found"),
      }),
    ];
  },
});
