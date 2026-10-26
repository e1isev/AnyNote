import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import { useMemo, useRef } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
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
          openUrl(href).catch(reportError);
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
      <Toolbar editor={editor} editMath={editMath} />
      <EditorContent editor={editor} className="editor-body" />
    </div>
  );
}
