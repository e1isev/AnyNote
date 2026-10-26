import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import { useMemo, useRef } from "react";
import { PenTool } from "lucide-react";
import type * as Y from "yjs";
import { openExternal, uuid } from "../platform";
import { defaultDrawingHeight } from "../prefs";
import { editorExtensions } from "./extensions";
import { insertFiles } from "./assets";
import { Toolbar } from "./Toolbar";
import type { PageSync } from "./provider";
import { promptText, reportError } from "../ui/overlay";

interface Props {
  sync: PageSync;
  onOpenPage: (pageId: string) => void;
  onCreatePage: (title: string) => Promise<string | null>;
}

export function editMath(latex: string) {
  return promptText({
    title: "Math",
    label: "LaTeX, e.g. \\frac{a}{b} or \\int_0^1 x\\,dx",
    initial: latex,
    multiline: true,
    monospace: true,
    confirmText: "Done (Ctrl+Enter)",
  });
}

/** Scroll a drawing area into view once the editor has rendered it. */
function revealDrawing(editor: Editor, inkId: string) {
  requestAnimationFrame(() => {
    const el = editor.view.dom.querySelector(`[data-ink-id="${CSS.escape(inkId)}"]`);
    el?.scrollIntoView({ behavior: "smooth", block: "start" });
  });
}

/**
 * Start drawing: at the cursor while typing, otherwise at the end of the page,
 * reusing an empty drawing area that is already there.
 */
export function addDrawing(editor: Editor, doc: Y.Doc) {
  if (editor.isFocused) {
    const inkId = uuid();
    editor.chain().focus().insertInk({ inkId }).run();
    revealDrawing(editor, inkId);
    return;
  }
  const blocks = editor.state.doc.content;
  let last = blocks.lastChild;
  if (last?.type.name === "paragraph" && last.content.size === 0 && blocks.childCount > 1) {
    last = blocks.child(blocks.childCount - 2);
  }
  const existing = last?.type.name === "inkBlock" ? String(last.attrs.inkId) : null;
  const strokes = existing ? ((doc.getMap("ink").get(existing) as Y.Array<unknown> | undefined)?.length ?? 0) : 1;
  if (existing && strokes === 0) {
    revealDrawing(editor, existing);
    return;
  }
  const inkId = uuid();
  editor
    .chain()
    .insertContentAt(editor.state.doc.content.size, [
      { type: "inkBlock", attrs: { inkId, height: defaultDrawingHeight() } },
      { type: "paragraph" },
    ])
    .run();
  revealDrawing(editor, inkId);
}

/** The rich-text editor for one page, bound to the page's Y.Doc. */
export function PageEditor({ sync, onOpenPage, onCreatePage }: Props) {
  const extensions = useMemo(
    () => editorExtensions({ doc: sync.doc, openPage: onOpenPage, createPage: onCreatePage, editMath }),
    // The editor is rebuilt per page; the callbacks are stable for its lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sync],
  );

  const editorRef = useRef<Editor | null>(null);
  const editor = useEditor(
    {
      extensions,
      autofocus: false,
      editorProps: {
        attributes: { class: "page-content", spellcheck: "true" },
        handlePaste: (_view, event) => {
          const files = Array.from(event.clipboardData?.files ?? []);
          if (files.length === 0) return false;
          event.preventDefault();
          const ed = editorRef.current;
          if (ed) insertFiles(ed, files).catch(reportError);
          return true;
        },
        handleDrop: (view, event) => {
          const files = Array.from(event.dataTransfer?.files ?? []);
          if (files.length === 0) return false;
          event.preventDefault();
          const pos = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos;
          const ed = editorRef.current;
          if (ed) insertFiles(ed, files, pos).catch(reportError);
          return true;
        },
        handleClick: (view, pos, event) => {
          // Ctrl/Cmd-click follows links.
          if (!(event.ctrlKey || event.metaKey)) return false;
          const link = view.state.doc
            .resolve(pos)
            .marks()
            .find((m) => m.type.name === "link");
          const href = link?.attrs.href as string | undefined;
          if (!href) return false;
          openExternal(href).catch(reportError);
          return true;
        },
      },
    },
    [extensions],
  );
  editorRef.current = editor;

  if (!editor) return null;
  return (
    <div className="editor">
      <Toolbar editor={editor} editMath={editMath} onDraw={() => addDrawing(editor, sync.doc)} />
      <EditorContent editor={editor} className="editor-body" />
      <button
        className="fab-draw"
        title="Draw: add a drawing area for handwriting"
        aria-label="Draw"
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => addDrawing(editor, sync.doc)}
      >
        <PenTool size={24} />
      </button>
    </div>
  );
}
