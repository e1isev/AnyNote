import { useEditorState, type Editor } from "@tiptap/react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Bold,
  Code,
  Code2,
  Columns3,
  Highlighter,
  Image as ImageIcon,
  Italic,
  Link as LinkIcon,
  List,
  ListChecks,
  ListOrdered,
  Minus,
  Paperclip,
  PenTool,
  Quote,
  Redo2,
  Rows3,
  SquareFunction,
  Strikethrough,
  Table as TableIcon,
  Trash2,
  Type,
  Underline as UnderlineIcon,
  Undo2,
} from "lucide-react";
import { pickFiles } from "./assets";
import { promptText } from "../ui/overlay";

const TEXT_COLORS = ["#d93f3f", "#e07b00", "#b08800", "#1f9d55", "#2f6fdf", "#8250df", "#6e7781"];
const HIGHLIGHTS = ["#fff3a3", "#c8f7c5", "#cfe8ff", "#ffd6e7", "#ffe1c2", "#e5dbff"];

function Btn(props: { title: string; active?: boolean; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      className={props.active ? "tb-btn active" : "tb-btn"}
      title={props.title}
      aria-label={props.title}
      aria-pressed={props.active}
      disabled={props.disabled}
      onMouseDown={(e) => e.preventDefault()}
      onClick={props.onClick}
    >
      {props.children}
    </button>
  );
}

function ColorPicker(props: {
  title: string;
  icon: ReactNode;
  colors: string[];
  current: string | null;
  onPick: (c: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);
  return (
    <div className="tb-popover-anchor" ref={ref}>
      <button
        type="button"
        className="tb-btn"
        title={props.title}
        aria-label={props.title}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => setOpen(!open)}
      >
        {props.icon}
        <span className="tb-color-bar" style={{ background: props.current ?? "transparent" }} />
      </button>
      {open && (
        <div className="tb-popover">
          <button className="swatch none" title="None" onMouseDown={(e) => e.preventDefault()} onClick={() => (props.onPick(null), setOpen(false))} />
          {props.colors.map((c) => (
            <button
              key={c}
              className={c === props.current ? "swatch active" : "swatch"}
              style={{ background: c }}
              title={c}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => (props.onPick(c), setOpen(false))}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export function Toolbar({ editor, editMath }: { editor: Editor; editMath: (latex: string) => Promise<string | null> }) {
  const s = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      canUndo: e.can().undo(),
      canRedo: e.can().redo(),
      block: e.isActive("heading", { level: 1 })
        ? "h1"
        : e.isActive("heading", { level: 2 })
          ? "h2"
          : e.isActive("heading", { level: 3 })
            ? "h3"
            : "p",
      bold: e.isActive("bold"),
      italic: e.isActive("italic"),
      underline: e.isActive("underline"),
      strike: e.isActive("strike"),
      code: e.isActive("code"),
      link: e.isActive("link"),
      bullet: e.isActive("bulletList"),
      ordered: e.isActive("orderedList"),
      task: e.isActive("taskList"),
      quote: e.isActive("blockquote"),
      codeBlock: e.isActive("codeBlock"),
      table: e.isActive("table"),
      color: (e.getAttributes("textStyle").color as string | undefined) ?? null,
      highlight: e.isActive("highlight") ? ((e.getAttributes("highlight").color as string | undefined) ?? "#fff3a3") : null,
    }),
  });

  const chain = () => editor.chain().focus();

  const setBlock = (v: string) => {
    if (v === "p") chain().setParagraph().run();
    else chain().toggleHeading({ level: Number(v[1]) as 1 | 2 | 3 }).run();
  };

  const editLink = async () => {
    const prev = (editor.getAttributes("link").href as string | undefined) ?? "";
    const href = await promptText({ title: prev ? "Edit link" : "Add link", label: "URL", initial: prev, placeholder: "https://…", confirmText: "Apply" });
    if (href === null) return;
    if (href.trim() === "") chain().extendMarkRange("link").unsetLink().run();
    else chain().extendMarkRange("link").setLink({ href: href.trim() }).run();
  };

  const insertMath = async () => {
    const latex = await editMath("");
    if (latex) chain().insertBlockMath({ latex }).run();
  };

  return (
    <div className="toolbar" role="toolbar" aria-label="Formatting">
      <div className="tb-group">
        <Btn title="Undo (Ctrl+Z)" disabled={!s.canUndo} onClick={() => chain().undo().run()}>
          <Undo2 size={16} />
        </Btn>
        <Btn title="Redo (Ctrl+Shift+Z)" disabled={!s.canRedo} onClick={() => chain().redo().run()}>
          <Redo2 size={16} />
        </Btn>
      </div>
      <div className="tb-group">
        <select className="tb-select" value={s.block} onChange={(e) => setBlock(e.target.value)} aria-label="Text style">
          <option value="p">Text</option>
          <option value="h1">Heading 1</option>
          <option value="h2">Heading 2</option>
          <option value="h3">Heading 3</option>
        </select>
      </div>
      <div className="tb-group">
        <Btn title="Bold (Ctrl+B)" active={s.bold} onClick={() => chain().toggleBold().run()}>
          <Bold size={16} />
        </Btn>
        <Btn title="Italic (Ctrl+I)" active={s.italic} onClick={() => chain().toggleItalic().run()}>
          <Italic size={16} />
        </Btn>
        <Btn title="Underline (Ctrl+U)" active={s.underline} onClick={() => chain().toggleUnderline().run()}>
          <UnderlineIcon size={16} />
        </Btn>
        <Btn title="Strikethrough (Ctrl+Shift+S)" active={s.strike} onClick={() => chain().toggleStrike().run()}>
          <Strikethrough size={16} />
        </Btn>
        <Btn title="Inline code (Ctrl+E)" active={s.code} onClick={() => chain().toggleCode().run()}>
          <Code size={16} />
        </Btn>
        <ColorPicker
          title="Highlight"
          icon={<Highlighter size={16} />}
          colors={HIGHLIGHTS}
          current={s.highlight}
          onPick={(c) => (c ? chain().setHighlight({ color: c }).run() : chain().unsetHighlight().run())}
        />
        <ColorPicker
          title="Text colour"
          icon={<Type size={16} />}
          colors={TEXT_COLORS}
          current={s.color}
          onPick={(c) => (c ? chain().setColor(c).run() : chain().unsetColor().run())}
        />
        <Btn title="Link" active={s.link} onClick={() => void editLink()}>
          <LinkIcon size={16} />
        </Btn>
      </div>
      <div className="tb-group">
        <Btn title="Bulleted list" active={s.bullet} onClick={() => chain().toggleBulletList().run()}>
          <List size={16} />
        </Btn>
        <Btn title="Numbered list" active={s.ordered} onClick={() => chain().toggleOrderedList().run()}>
          <ListOrdered size={16} />
        </Btn>
        <Btn title="To-do list (Ctrl+Shift+9)" active={s.task} onClick={() => chain().toggleTaskList().run()}>
          <ListChecks size={16} />
        </Btn>
        <Btn title="Quote" active={s.quote} onClick={() => chain().toggleBlockquote().run()}>
          <Quote size={16} />
        </Btn>
        <Btn title="Code block" active={s.codeBlock} onClick={() => chain().toggleCodeBlock().run()}>
          <Code2 size={16} />
        </Btn>
      </div>
      <div className="tb-group">
        <Btn title="Table" onClick={() => chain().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()}>
          <TableIcon size={16} />
        </Btn>
        <Btn title="Math (LaTeX)" onClick={() => void insertMath()}>
          <SquareFunction size={16} />
        </Btn>
        <Btn title="Image" onClick={() => pickFiles(editor, "image/*")}>
          <ImageIcon size={16} />
        </Btn>
        <Btn title="Attach a file" onClick={() => pickFiles(editor)}>
          <Paperclip size={16} />
        </Btn>
        <Btn title="Ink drawing" onClick={() => chain().insertInk().run()}>
          <PenTool size={16} />
        </Btn>
        <Btn title="Divider" onClick={() => chain().setHorizontalRule().run()}>
          <Minus size={16} />
        </Btn>
      </div>
      {s.table && (
        <div className="tb-group tb-table">
          <Btn title="Add row below" onClick={() => chain().addRowAfter().run()}>
            <Rows3 size={16} />
            <span className="tb-plus">+</span>
          </Btn>
          <Btn title="Add column right" onClick={() => chain().addColumnAfter().run()}>
            <Columns3 size={16} />
            <span className="tb-plus">+</span>
          </Btn>
          <Btn title="Delete row" onClick={() => chain().deleteRow().run()}>
            <Rows3 size={16} />
            <span className="tb-plus">−</span>
          </Btn>
          <Btn title="Delete column" onClick={() => chain().deleteColumn().run()}>
            <Columns3 size={16} />
            <span className="tb-plus">−</span>
          </Btn>
          <Btn title="Toggle header row" onClick={() => chain().toggleHeaderRow().run()}>
            <span className="tb-text">H</span>
          </Btn>
          <Btn title="Delete table" onClick={() => chain().deleteTable().run()}>
            <Trash2 size={16} />
          </Btn>
        </div>
      )}
    </div>
  );
}
