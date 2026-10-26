import type { AnyExtension } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import { Table, TableCell, TableHeader, TableRow } from "@tiptap/extension-table";
import Image from "@tiptap/extension-image";
import CodeBlockLowlight from "@tiptap/extension-code-block-lowlight";
import Highlight from "@tiptap/extension-highlight";
import { Color, TextStyle } from "@tiptap/extension-text-style";
import Subscript from "@tiptap/extension-subscript";
import Superscript from "@tiptap/extension-superscript";
import { Mathematics } from "@tiptap/extension-mathematics";
import UniqueID from "@tiptap/extension-unique-id";
import { Placeholder } from "@tiptap/extensions";
import Collaboration, { isChangeOrigin } from "@tiptap/extension-collaboration";
import { common, createLowlight } from "lowlight";
import type * as Y from "yjs";
import { Attachment } from "./Attachment";
import { assetSrc, canonicalAssetUrl } from "./assets";
import { InkBlock } from "./InkBlock";
import { SlashCommand } from "./SlashCommand";
import { WikiLink } from "./WikiLink";

export const lowlight = createLowlight(common);

/** Block types that get a stable `id` (kept in sync with BLOCK_ID_TYPES in anynote-core). */
export const BLOCK_ID_TYPES = [
  "paragraph",
  "heading",
  "bulletList",
  "orderedList",
  "listItem",
  "taskList",
  "taskItem",
  "codeBlock",
  "blockquote",
  "horizontalRule",
  "image",
  "table",
  "blockMath",
  "inkBlock",
  "attachment",
];

/** Images are stored as `anynote://asset/<hash>` and displayed through the asset protocol. */
const AssetImage = Image.extend({
  addAttributes() {
    return {
      ...this.parent?.(),
      src: {
        default: null,
        parseHTML: (el: HTMLElement) => canonicalAssetUrl(el.getAttribute("src")),
        renderHTML: (attrs: Record<string, unknown>) => ({ src: assetSrc(attrs.src as string) }),
      },
    };
  },
}).configure({ inline: false, allowBase64: false, resize: { enabled: true, alwaysPreserveAspectRatio: true } });

export interface EditorHooks {
  doc: Y.Doc;
  openPage: (pageId: string) => void;
  createPage: (title: string) => Promise<string | null>;
  editMath: (latex: string) => Promise<string | null>;
}

/** The single schema shared by every page. Changing node or attribute names
 * here requires the same change in anynote-core and docs/FORMAT.md. */
export function schemaExtensions(): AnyExtension[] {
  return [
    StarterKit.configure({
      codeBlock: false,
      undoRedo: false,
      link: { openOnClick: false, autolink: true, defaultProtocol: "https" },
      heading: { levels: [1, 2, 3, 4, 5, 6] },
    }),
    TaskList,
    TaskItem.configure({ nested: true }),
    Table.configure({ resizable: true }),
    TableRow,
    TableHeader,
    TableCell,
    AssetImage,
    CodeBlockLowlight.configure({ lowlight, defaultLanguage: null }),
    Highlight.configure({ multicolor: true }),
    TextStyle,
    Color,
    Subscript,
    Superscript,
    Attachment,
  ];
}

export function editorExtensions(hooks: EditorHooks): AnyExtension[] {
  const editMathAt = (kind: "inline" | "block") => async (node: { attrs: Record<string, unknown> }, pos: number) => {
    const latex = await hooks.editMath(String(node.attrs.latex ?? ""));
    if (latex === null) return;
    const editor = current;
    if (!editor) return;
    if (kind === "inline") editor.chain().setNodeSelection(pos).updateInlineMath({ latex }).focus().run();
    else editor.chain().setNodeSelection(pos).updateBlockMath({ latex }).focus().run();
  };
  // Mathematics' onClick receives no editor; capture it once the editor exists.
  let current: import("@tiptap/core").Editor | null = null;
  const capture = SlashCommand.extend({
    onCreate() {
      current = this.editor;
    },
  });

  return [
    ...schemaExtensions(),
    Mathematics.configure({
      katexOptions: { throwOnError: false },
      inlineOptions: { onClick: editMathAt("inline") },
      blockOptions: { onClick: editMathAt("block") },
    }),
    InkBlock.configure({ doc: hooks.doc }),
    WikiLink.configure({ onOpen: hooks.openPage, onCreate: hooks.createPage }),
    capture.configure({ editMath: hooks.editMath }),
    UniqueID.configure({
      types: BLOCK_ID_TYPES,
      filterTransaction: (tr) => !isChangeOrigin(tr),
    }),
    Placeholder.configure({
      placeholder: ({ node, pos }) =>
        node.type.name === "heading" ? "Heading" : pos === 0 ? "Start writing, or type / for blocks and [[ to link a page" : "",
    }),
    Collaboration.configure({ document: hooks.doc }),
  ];
}
