import { Extension, type Editor, type Range } from "@tiptap/core";
import Suggestion from "@tiptap/suggestion";
import { PluginKey } from "@tiptap/pm/state";
import {
  Code2,
  FileText,
  Heading1,
  Heading2,
  Heading3,
  Image as ImageIcon,
  Link2,
  List,
  ListChecks,
  ListOrdered,
  Minus,
  Paperclip,
  PenTool,
  Pilcrow,
  Quote,
  Sigma,
  SquareFunction,
  Table as TableIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { pickFiles } from "./assets";
import { renderSuggestions, type SuggestionItem } from "./SuggestionList";

interface Command {
  title: string;
  keywords: string;
  icon: ReactNode;
  run: (editor: Editor, range: Range) => void;
}

export interface SlashOptions {
  editMath: (latex: string) => Promise<string | null>;
}

function commands(options: SlashOptions): Command[] {
  const at = (editor: Editor, range: Range) => editor.chain().focus().deleteRange(range);
  return [
    { title: "Text", keywords: "paragraph plain", icon: <Pilcrow size={16} />, run: (e, r) => at(e, r).setParagraph().run() },
    { title: "Heading 1", keywords: "h1 title", icon: <Heading1 size={16} />, run: (e, r) => at(e, r).setHeading({ level: 1 }).run() },
    { title: "Heading 2", keywords: "h2 subtitle", icon: <Heading2 size={16} />, run: (e, r) => at(e, r).setHeading({ level: 2 }).run() },
    { title: "Heading 3", keywords: "h3", icon: <Heading3 size={16} />, run: (e, r) => at(e, r).setHeading({ level: 3 }).run() },
    { title: "Bulleted list", keywords: "ul unordered", icon: <List size={16} />, run: (e, r) => at(e, r).toggleBulletList().run() },
    { title: "Numbered list", keywords: "ol ordered", icon: <ListOrdered size={16} />, run: (e, r) => at(e, r).toggleOrderedList().run() },
    { title: "To-do list", keywords: "task checkbox todo", icon: <ListChecks size={16} />, run: (e, r) => at(e, r).toggleTaskList().run() },
    { title: "Quote", keywords: "blockquote", icon: <Quote size={16} />, run: (e, r) => at(e, r).toggleBlockquote().run() },
    { title: "Code block", keywords: "code snippet pre", icon: <Code2 size={16} />, run: (e, r) => at(e, r).toggleCodeBlock().run() },
    {
      title: "Table",
      keywords: "grid spreadsheet",
      icon: <TableIcon size={16} />,
      run: (e, r) => at(e, r).insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run(),
    },
    {
      title: "Math block",
      keywords: "latex equation formula katex",
      icon: <SquareFunction size={16} />,
      run: async (e, r) => {
        at(e, r).run();
        const latex = await options.editMath("");
        if (latex) e.chain().focus().insertBlockMath({ latex }).run();
      },
    },
    {
      title: "Inline math",
      keywords: "latex equation formula",
      icon: <Sigma size={16} />,
      run: async (e, r) => {
        at(e, r).run();
        const latex = await options.editMath("");
        if (latex) e.chain().focus().insertInlineMath({ latex }).run();
      },
    },
    { title: "Ink drawing", keywords: "draw sketch pen handwriting canvas", icon: <PenTool size={16} />, run: (e, r) => at(e, r).insertInk().run() },
    {
      title: "Image",
      keywords: "picture photo",
      icon: <ImageIcon size={16} />,
      run: (e, r) => {
        at(e, r).run();
        pickFiles(e, "image/*");
      },
    },
    {
      title: "File attachment",
      keywords: "attach upload pdf",
      icon: <Paperclip size={16} />,
      run: (e, r) => {
        at(e, r).run();
        pickFiles(e);
      },
    },
    { title: "Link to page", keywords: "wiki backlink reference", icon: <Link2 size={16} />, run: (e, r) => at(e, r).insertContent("[[").run() },
    { title: "Divider", keywords: "hr rule line separator", icon: <Minus size={16} />, run: (e, r) => at(e, r).setHorizontalRule().run() },
  ];
}

/** Type "/" to insert any block. */
export const SlashCommand = Extension.create<SlashOptions>({
  name: "slashCommand",

  addOptions() {
    return { editMath: async () => null };
  },

  addProseMirrorPlugins() {
    const all = commands(this.options);
    return [
      Suggestion<SuggestionItem, SuggestionItem>({
        editor: this.editor,
        pluginKey: new PluginKey("slashCommand"),
        char: "/",
        allow: ({ state, range }) => {
          const $from = state.doc.resolve(range.from);
          return !$from.parent.type.spec.code;
        },
        items: ({ query }) => {
          const q = query.toLowerCase();
          return all
            .filter((c) => !q || c.title.toLowerCase().includes(q) || c.keywords.includes(q))
            .map((c) => ({ key: c.title, label: c.title, icon: c.icon ?? <FileText size={16} /> }));
        },
        command: ({ editor, range, props }) => {
          all.find((c) => c.title === props.key)?.run(editor, range);
        },
        render: renderSuggestions("No matching blocks"),
      }),
    ];
  },
});
