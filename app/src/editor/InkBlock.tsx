import { Node, mergeAttributes } from "@tiptap/core";
import { NodeViewWrapper, ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react";
import { getStroke } from "perfect-freehand";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as Y from "yjs";
import { Eraser, Highlighter, PenLine, Redo2, Trash2, Undo2 } from "lucide-react";

/**
 * A drawing canvas block. Strokes live in the page's Y.Doc under
 * `ink[<inkId>]` (a Y.Array), so they sync and merge like text. Each stroke is
 * `{id, tool, color, size, pts}` with `pts` a Float32 (x, y, pressure) buffer;
 * see crates/anynote-core/src/ink.rs for the Rust side.
 */

export interface InkStroke {
  id: string;
  tool: "pen" | "highlighter";
  color: string;
  size: number;
  pts: Uint8Array;
}

type Tool = "pen" | "highlighter" | "eraser";
type Point = [number, number, number];

const PEN_COLORS = ["#1f2328", "#2f6fdf", "#d93f3f", "#1f9d55", "#e07b00", "#8250df"];
const HIGHLIGHTER_COLORS = ["#ffe14d", "#7ee787", "#ff9bd2", "#79c0ff"];
const SIZES = [2, 4, 8];

export function encodePoints(points: Point[]): Uint8Array {
  const f = new Float32Array(points.length * 3);
  points.forEach(([x, y, p], i) => f.set([x, y, p], i * 3));
  return new Uint8Array(f.buffer);
}

export function decodePoints(buf: Uint8Array | number[]): Point[] {
  if (Array.isArray(buf)) {
    const out: Point[] = [];
    for (let i = 0; i + 2 < buf.length; i += 3) out.push([buf[i], buf[i + 1], buf[i + 2]]);
    return out;
  }
  // Copy: a Float32Array view needs 4-byte alignment.
  const f = new Float32Array(buf.slice().buffer, 0, Math.floor(buf.byteLength / 4));
  const out: Point[] = [];
  for (let i = 0; i + 2 < f.length; i += 3) out.push([f[i], f[i + 1], f[i + 2]]);
  return out;
}

function svgPath(outline: number[][]): string {
  if (outline.length < 4) return "";
  const avg = (a: number, b: number) => (a + b) / 2;
  let a = outline[0];
  let b = outline[1];
  const c = outline[2];
  let d = `M${a[0].toFixed(2)},${a[1].toFixed(2)} Q${b[0].toFixed(2)},${b[1].toFixed(2)} ${avg(b[0], c[0]).toFixed(2)},${avg(b[1], c[1]).toFixed(2)} T`;
  for (let i = 2; i < outline.length - 1; i++) {
    a = outline[i];
    b = outline[i + 1];
    d += `${avg(a[0], b[0]).toFixed(2)},${avg(a[1], b[1]).toFixed(2)} `;
  }
  return d + "Z";
}

function strokeOutline(points: Point[], tool: "pen" | "highlighter", size: number, done: boolean): string {
  const pressureFromDevice = points.some(([, , p]) => p !== 0.5 && p > 0);
  return svgPath(
    getStroke(points, {
      size: tool === "highlighter" ? size * 4 : size * 1.6,
      thinning: tool === "highlighter" ? 0 : 0.55,
      smoothing: 0.5,
      streamline: 0.45,
      simulatePressure: tool === "pen" && !pressureFromDevice,
      last: done,
    }),
  );
}

function distanceToSegment(px: number, py: number, a: Point, b: Point) {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len = dx * dx + dy * dy;
  const t = len === 0 ? 0 : Math.max(0, Math.min(1, ((px - a[0]) * dx + (py - a[1]) * dy) / len));
  return Math.hypot(px - (a[0] + t * dx), py - (a[1] + t * dy));
}

function InkView({ node, updateAttributes, editor, extension }: ReactNodeViewProps) {
  const doc = (extension.options as InkOptions).doc;
  const inkId = node.attrs.inkId as string;
  const height = Number(node.attrs.height) || 320;
  const editable = editor.isEditable;

  const [strokes, setStrokes] = useState<InkStroke[]>([]);
  const [tool, setTool] = useState<Tool>("pen");
  const [color, setColor] = useState(PEN_COLORS[0]);
  const [size, setSize] = useState(SIZES[1]);
  const [draft, setDraft] = useState<Point[] | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const drawing = useRef<{ points: Point[]; pointerId: number } | null>(null);
  const undoRef = useRef<Y.UndoManager | null>(null);
  const [, forceRender] = useState(0);

  const inkMap = useMemo(() => doc?.getMap<Y.Array<InkStroke>>("ink") ?? null, [doc]);

  // Follow the stroke array (it may be created or replaced by a peer).
  useEffect(() => {
    if (!inkMap) return;
    let arr: Y.Array<InkStroke> | undefined;
    const sync = () => setStrokes(arr ? arr.toArray() : []);
    const attach = () => {
      arr?.unobserve(sync);
      undoRef.current?.destroy();
      arr = inkMap.get(inkId);
      if (arr) {
        arr.observe(sync);
        undoRef.current = new Y.UndoManager(arr, { captureTimeout: 0 });
        undoRef.current.on("stack-item-added", () => forceRender((n) => n + 1));
        undoRef.current.on("stack-item-popped", () => forceRender((n) => n + 1));
      }
      sync();
    };
    const onMap = (e: Y.YMapEvent<Y.Array<InkStroke>>) => {
      if (e.keysChanged.has(inkId)) attach();
    };
    attach();
    inkMap.observe(onMap);
    return () => {
      inkMap.unobserve(onMap);
      arr?.unobserve(sync);
      undoRef.current?.destroy();
      undoRef.current = null;
    };
  }, [inkMap, inkId]);

  const decoded = useMemo(
    () => strokes.map((s) => ({ stroke: s, points: decodePoints(s.pts) })),
    [strokes],
  );

  const contentWidth = useMemo(() => {
    let max = 0;
    for (const { points } of decoded) for (const [x] of points) max = Math.max(max, x);
    return Math.ceil(max + 24);
  }, [decoded]);

  const strokesArray = useCallback((): Y.Array<InkStroke> | null => {
    if (!inkMap || !doc) return null;
    let arr = inkMap.get(inkId);
    if (!arr) {
      arr = new Y.Array<InkStroke>();
      inkMap.set(inkId, arr);
    }
    return arr;
  }, [inkMap, doc, inkId]);

  const localPoint = (e: React.PointerEvent): Point => {
    const rect = svgRef.current!.getBoundingClientRect();
    const pressure = e.pointerType === "pen" ? e.pressure || 0.5 : 0.5;
    return [e.clientX - rect.left, e.clientY - rect.top, pressure];
  };

  const eraseAt = (x: number, y: number) => {
    const arr = inkMap?.get(inkId);
    if (!arr) return;
    const radius = 8;
    const hits: number[] = [];
    decoded.forEach(({ stroke, points }, index) => {
      const reach = radius + (stroke.tool === "highlighter" ? stroke.size * 2 : stroke.size);
      for (let i = 0; i < points.length; i++) {
        const a = points[i];
        const b = points[i + 1] ?? a;
        if (distanceToSegment(x, y, a, b) <= reach) {
          hits.push(index);
          break;
        }
      }
    });
    if (hits.length) {
      doc!.transact(() => {
        for (const i of hits.reverse()) arr.delete(i, 1);
      });
    }
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (!editable || e.button !== 0) return;
    e.preventDefault();
    (e.target as Element).setPointerCapture?.(e.pointerId);
    const p = localPoint(e);
    if (tool === "eraser") {
      drawing.current = { points: [], pointerId: e.pointerId };
      eraseAt(p[0], p[1]);
      return;
    }
    drawing.current = { points: [p], pointerId: e.pointerId };
    setDraft([p]);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = drawing.current;
    if (!d || d.pointerId !== e.pointerId) return;
    const p = localPoint(e);
    if (tool === "eraser") {
      eraseAt(p[0], p[1]);
      return;
    }
    // Coalesced events give smooth lines with fast pens.
    const events = (e.nativeEvent as PointerEvent).getCoalescedEvents?.() ?? [];
    if (events.length > 1) {
      const rect = svgRef.current!.getBoundingClientRect();
      for (const ce of events) {
        const pressure = ce.pointerType === "pen" ? ce.pressure || 0.5 : 0.5;
        d.points.push([ce.clientX - rect.left, ce.clientY - rect.top, pressure]);
      }
    } else {
      d.points.push(p);
    }
    setDraft([...d.points]);
  };

  const onPointerUp = (e: React.PointerEvent) => {
    const d = drawing.current;
    if (!d || d.pointerId !== e.pointerId) return;
    drawing.current = null;
    setDraft(null);
    if (tool === "eraser" || d.points.length === 0) return;
    const points = d.points.length === 1 ? [d.points[0], [d.points[0][0] + 0.5, d.points[0][1] + 0.5, d.points[0][2]] as Point] : d.points;
    const arr = strokesArray();
    arr?.push([
      {
        id: crypto.randomUUID(),
        tool: tool as "pen" | "highlighter",
        color,
        size,
        pts: encodePoints(points.map(([x, y, p]) => [Math.round(x * 10) / 10, Math.round(y * 10) / 10, p])),
      },
    ]);
    // Grow the canvas when drawing near the bottom edge.
    const maxY = Math.max(...points.map(([, y]) => y));
    if (maxY > height - 40) updateAttributes({ height: Math.round(maxY + 120) });
  };

  const startResize = (e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const startY = e.clientY;
    const startH = height;
    const move = (ev: PointerEvent) => updateAttributes({ height: Math.max(120, Math.round(startH + ev.clientY - startY)) });
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const pickTool = (t: Tool) => {
    setTool(t);
    if (t === "highlighter" && !HIGHLIGHTER_COLORS.includes(color)) setColor(HIGHLIGHTER_COLORS[0]);
    if (t === "pen" && !PEN_COLORS.includes(color)) setColor(PEN_COLORS[0]);
  };

  const undo = undoRef.current;
  const palette = tool === "highlighter" ? HIGHLIGHTER_COLORS : PEN_COLORS;

  return (
    <NodeViewWrapper className="ink-block" data-drag-handle="">
      {editable && (
        <div className="ink-toolbar" contentEditable={false}>
          <button className={tool === "pen" ? "active" : ""} title="Pen" onClick={() => pickTool("pen")}>
            <PenLine size={16} />
          </button>
          <button className={tool === "highlighter" ? "active" : ""} title="Highlighter" onClick={() => pickTool("highlighter")}>
            <Highlighter size={16} />
          </button>
          <button className={tool === "eraser" ? "active" : ""} title="Eraser (removes whole strokes)" onClick={() => pickTool("eraser")}>
            <Eraser size={16} />
          </button>
          <span className="ink-sep" />
          {tool !== "eraser" &&
            palette.map((c) => (
              <button
                key={c}
                className={c === color ? "swatch active" : "swatch"}
                style={{ background: c }}
                title={c}
                aria-label={`Colour ${c}`}
                onClick={() => setColor(c)}
              />
            ))}
          {tool !== "eraser" && <span className="ink-sep" />}
          {tool !== "eraser" &&
            SIZES.map((s) => (
              <button key={s} className={s === size ? "size active" : "size"} title={`Size ${s}`} onClick={() => setSize(s)}>
                <span style={{ width: s + 2, height: s + 2 }} />
              </button>
            ))}
          <span className="ink-spacer" />
          <button title="Undo stroke" disabled={!undo?.canUndo()} onClick={() => undo?.undo()}>
            <Undo2 size={16} />
          </button>
          <button title="Redo stroke" disabled={!undo?.canRedo()} onClick={() => undo?.redo()}>
            <Redo2 size={16} />
          </button>
          <button
            title="Clear drawing"
            disabled={strokes.length === 0}
            onClick={() => {
              const arr = inkMap?.get(inkId);
              if (arr && arr.length) arr.delete(0, arr.length);
            }}
          >
            <Trash2 size={16} />
          </button>
        </div>
      )}
      <div className="ink-scroll" contentEditable={false}>
        <svg
          ref={svgRef}
          className={`ink-surface tool-${tool}`}
          style={{ height, minWidth: contentWidth }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        >
          {decoded.map(({ stroke, points }) => (
            <path
              key={stroke.id}
              d={strokeOutline(points, stroke.tool, stroke.size, true)}
              fill={stroke.color}
              opacity={stroke.tool === "highlighter" ? 0.4 : 1}
            />
          ))}
          {draft && tool !== "eraser" && (
            <path d={strokeOutline(draft, tool, size, false)} fill={color} opacity={tool === "highlighter" ? 0.4 : 1} />
          )}
        </svg>
      </div>
      {editable && <div className="ink-resize" title="Drag to resize" contentEditable={false} onPointerDown={startResize} />}
    </NodeViewWrapper>
  );
}

export interface InkOptions {
  doc: Y.Doc | null;
}

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    inkBlock: {
      insertInk: () => ReturnType;
    };
  }
}

export const InkBlock = Node.create<InkOptions>({
  name: "inkBlock",
  group: "block",
  atom: true,
  draggable: true,
  selectable: true,

  addOptions() {
    return { doc: null };
  },

  addAttributes() {
    return {
      inkId: { default: null, parseHTML: (el) => el.getAttribute("data-ink-id") },
      height: { default: 320, parseHTML: (el) => Number(el.getAttribute("data-height")) || 320 },
      width: { default: null },
    };
  },

  parseHTML() {
    return [{ tag: "div[data-ink-id]" }];
  },

  renderHTML({ node, HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes, { "data-ink-id": node.attrs.inkId, "data-height": node.attrs.height, class: "ink-block" })];
  },

  addCommands() {
    return {
      insertInk:
        () =>
        ({ commands }) =>
          commands.insertContent([
            { type: this.name, attrs: { inkId: crypto.randomUUID(), height: 320 } },
            { type: "paragraph" },
          ]),
    };
  },

  addNodeView() {
    return ReactNodeViewRenderer(InkView, {
      stopEvent: ({ event }) => {
        const target = event.target as HTMLElement | null;
        return !!target?.closest?.(".ink-surface, .ink-toolbar, .ink-resize");
      },
      ignoreMutation: () => true,
    });
  },
});
