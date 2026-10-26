import { Node, mergeAttributes } from "@tiptap/core";
import { NodeViewContent, NodeViewWrapper, ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react";
import { GripHorizontal } from "lucide-react";

/**
 * A text box on a canvas page: rich text placed at (`x`, `y`) on the page,
 * `width` px wide, the same idea as a OneNote outline. A canvas page's
 * document is just these boxes; its drawing lives in the page's ink map.
 */
export interface CanvasBoxOptions {
  /** Current canvas zoom, to turn pointer movement into page pixels. */
  getZoom: () => number;
}

const MIN_WIDTH = 120;

function CanvasBoxView({ node, updateAttributes, extension, selectionInside, editor }: ReactNodeViewProps) {
  const { x, y, width } = node.attrs as { x: number; y: number; width: number };
  const empty = node.childCount === 1 && node.firstChild?.type.name === "paragraph" && node.firstChild.content.size === 0;
  const zoom = () => (extension.options as CanvasBoxOptions).getZoom();

  /** Drag with the pointer, turning screen pixels into page pixels. */
  const drag = (e: React.PointerEvent, apply: (dx: number, dy: number) => void) => {
    e.preventDefault();
    e.stopPropagation();
    const start = { x: e.clientX, y: e.clientY };
    let frame = 0;
    const move = (ev: PointerEvent) => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => apply((ev.clientX - start.x) / zoom(), (ev.clientY - start.y) / zoom()));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const classes = ["canvas-box", empty ? "empty" : "", selectionInside ? "editing" : "", editor.isEditable ? "" : "readonly"];
  return (
    <NodeViewWrapper className={classes.join(" ")} style={{ left: x, top: y, width }} data-canvas-box="">
      <div
        className="canvas-box-handle"
        contentEditable={false}
        title="Drag to move"
        onPointerDown={(e) => drag(e, (dx, dy) => updateAttributes({ x: Math.round(x + dx), y: Math.round(y + dy) }))}
      >
        <GripHorizontal size={14} />
      </div>
      <NodeViewContent className="canvas-box-content" />
      <div
        className="canvas-box-resize"
        contentEditable={false}
        title="Drag to change the width"
        onPointerDown={(e) => drag(e, (dx) => updateAttributes({ width: Math.max(MIN_WIDTH, Math.round(width + dx)) }))}
      />
    </NodeViewWrapper>
  );
}

export const CanvasBox = Node.create<CanvasBoxOptions>({
  name: "canvasBox",
  group: "block",
  content: "block+",
  defining: true,
  isolating: true,

  addOptions() {
    return { getZoom: () => 1 };
  },

  addAttributes() {
    const num = (name: string, fallback: number) => ({
      default: fallback,
      parseHTML: (el: HTMLElement) => Number(el.getAttribute(`data-${name}`)) || fallback,
      renderHTML: (attrs: Record<string, unknown>) => ({ [`data-${name}`]: attrs[name] }),
    });
    return { x: num("x", 0), y: num("y", 0), width: num("width", 480) };
  },

  parseHTML() {
    return [{ tag: "div[data-canvas-box]" }];
  },

  renderHTML({ HTMLAttributes, node }) {
    return [
      "div",
      mergeAttributes(HTMLAttributes, {
        "data-canvas-box": "",
        class: "canvas-box",
        style: `left:${node.attrs.x}px;top:${node.attrs.y}px;width:${node.attrs.width}px`,
      }),
      0,
    ];
  },

  addNodeView() {
    return ReactNodeViewRenderer(CanvasBoxView, {
      stopEvent: ({ event }) => !!(event.target as HTMLElement | null)?.closest?.(".canvas-box-handle, .canvas-box-resize"),
    });
  },
});
