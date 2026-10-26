import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import { isChangeOrigin } from "@tiptap/extension-collaboration";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as Y from "yjs";
import {
  Eraser,
  Hand,
  Highlighter,
  Maximize,
  Minus,
  PenLine,
  Plus,
  Pointer,
  Redo2,
  Trash2,
  Type,
  Undo2,
} from "lucide-react";
import type { Node as PMNode } from "@tiptap/pm/model";
import { fingerDraws, getPrefs, hasTouch, setPrefs, usePrefs } from "../prefs";
import { uuid } from "../uuid";
import { confirmAction, reportError, toast } from "../ui/overlay";
import { insertFiles } from "./assets";
import { editMath } from "./Editor";
import { editorExtensions } from "./extensions";
import {
  HIGHLIGHTER_COLORS,
  PEN_COLORS,
  SIZES,
  decodePoints,
  encodePoints,
  strokeOutline,
  strokesNear,
  type InkStroke,
  type Point,
} from "./ink";
import type { PageSync } from "./provider";
import { Toolbar } from "./Toolbar";

/** Key of the page-wide drawing in the page's ink map (CANVAS_INK in anynote-core). */
const CANVAS_INK = "canvas";
const MIN_ZOOM = 0.2;
const MAX_ZOOM = 5;

type CanvasTool = "pen" | "highlighter" | "eraser" | "text" | "hand";
interface View {
  x: number; // screen offset of page point (0, 0), in px
  y: number;
  zoom: number;
}

/** Last view of each page, so going back to a page keeps your place. */
const views = new Map<string, View>();

/** Is this page a canvas page (its text lives in positioned boxes)? */
export function isCanvasDoc(doc: Y.Doc): boolean {
  return doc
    .getXmlFragment("default")
    .toArray()
    .some((n) => n instanceof Y.XmlElement && n.nodeName === "canvasBox");
}

function isEmptyBox(node: PMNode): boolean {
  return node.childCount === 1 && node.firstChild?.type.name === "paragraph" && node.firstChild.content.size === 0;
}

const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));

/** The strokes, drawn once per change (not on every pan or zoom frame). */
const StrokeLayer = memo(function StrokeLayer({ strokes }: { strokes: { stroke: InkStroke; points: Point[] }[] }) {
  return (
    <>
      {strokes.map(({ stroke, points }) => (
        <path
          key={stroke.id}
          d={strokeOutline(points, stroke.tool, stroke.size, true)}
          fill={stroke.color}
          opacity={stroke.tool === "highlighter" ? 0.4 : 1}
        />
      ))}
    </>
  );
});

interface Props {
  sync: PageSync;
  onOpenPage: (pageId: string) => void;
  onCreatePage: (title: string) => Promise<string | null>;
}

/**
 * A canvas page: an open drawing surface you can pan and zoom, with text in
 * boxes placed anywhere on it. The pen draws everywhere (also over text);
 * the Text tool places and edits boxes.
 */
export function CanvasEditor({ sync, onOpenPage, onCreatePage }: Props) {
  const pageId = sync.pageId;
  const viewRef = useRef<View>(views.get(pageId) ?? { x: 16, y: 16, zoom: 1 });
  const [view, setViewState] = useState<View>(viewRef.current);
  const setView = useCallback(
    (v: View) => {
      viewRef.current = v;
      views.set(pageId, v);
      setViewState(v);
    },
    [pageId],
  );

  const [tool, setTool] = useState<CanvasTool>("pen");
  const [color, setColor] = useState(PEN_COLORS[0]);
  const [size, setSize] = useState(SIZES[1]);
  const prefs = usePrefs();
  const fingerDrawsNow = fingerDraws(prefs);

  // --- the text editor (boxes) -------------------------------------------
  const extensions = useMemo(
    () =>
      editorExtensions({
        doc: sync.doc,
        layout: "canvas",
        getZoom: () => viewRef.current.zoom,
        openPage: onOpenPage,
        createPage: onCreatePage,
        editMath,
      }),
    // Rebuilt per page; the callbacks are stable for its lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sync],
  );
  const editorRef = useRef<Editor | null>(null);
  const editor = useEditor(
    {
      extensions,
      editorProps: {
        attributes: { class: "page-content canvas-content", spellcheck: "true" },
        handlePaste: (_view, event) => {
          const files = Array.from(event.clipboardData?.files ?? []);
          if (files.length === 0 || !editorRef.current) return false;
          event.preventDefault();
          insertFiles(editorRef.current, files).catch(reportError);
          return true;
        },
      },
    },
    [extensions],
  );
  editorRef.current = editor;

  // Remove boxes left empty (keeping one spare, which new text reuses).
  useEffect(() => {
    if (!editor) return;
    const tidy = () => {
      const { state } = editor;
      const sel = state.selection.from;
      const boxes: { pos: number; node: PMNode }[] = [];
      state.doc.forEach((node, pos) => {
        if (node.type.name === "canvasBox") boxes.push({ pos, node });
      });
      const doomed = boxes.filter(
        (b) => isEmptyBox(b.node) && !(editor.isFocused && sel > b.pos && sel < b.pos + b.node.nodeSize),
      );
      const remove = doomed.length === boxes.length ? doomed.slice(1) : doomed;
      if (remove.length === 0) return;
      const tr = state.tr;
      for (const b of [...remove].reverse()) tr.delete(b.pos, b.pos + b.node.nodeSize);
      tr.setMeta("addToHistory", false);
      editor.view.dispatch(tr);
    };
    // While focused, only once the caret has settled in a box (a box being
    // created is empty for a moment).
    const onSelection = () => editor.isFocused && tidy();
    editor.on("blur", tidy);
    editor.on("selectionUpdate", onSelection);
    return () => {
      editor.off("blur", tidy);
      editor.off("selectionUpdate", onSelection);
    };
  }, [editor]);

  // --- strokes ------------------------------------------------------------
  const inkMap = useMemo(() => sync.doc.getMap<Y.Array<InkStroke>>("ink"), [sync]);
  const [strokes, setStrokes] = useState<InkStroke[]>([]);
  const inkUndo = useRef<Y.UndoManager | null>(null);
  const [, forceRender] = useState(0);
  useEffect(() => {
    let arr: Y.Array<InkStroke> | undefined;
    const update = () => setStrokes(arr ? arr.toArray() : []);
    const attach = () => {
      arr?.unobserve(update);
      inkUndo.current?.destroy();
      arr = inkMap.get(CANVAS_INK);
      if (arr) {
        arr.observe(update);
        inkUndo.current = new Y.UndoManager(arr, { captureTimeout: 0 });
        inkUndo.current.on("stack-item-added", () => forceRender((n) => n + 1));
        inkUndo.current.on("stack-item-popped", () => forceRender((n) => n + 1));
      }
      update();
    };
    const onMap = (e: Y.YMapEvent<Y.Array<InkStroke>>) => e.keysChanged.has(CANVAS_INK) && attach();
    attach();
    inkMap.observe(onMap);
    return () => {
      inkMap.unobserve(onMap);
      arr?.unobserve(update);
      inkUndo.current?.destroy();
    };
  }, [inkMap]);
  const decoded = useMemo(() => strokes.map((s) => ({ stroke: s, points: decodePoints(s.pts) })), [strokes]);

  const strokeArray = () => {
    let arr = inkMap.get(CANVAS_INK);
    if (!arr) {
      arr = new Y.Array<InkStroke>();
      inkMap.set(CANVAS_INK, arr);
    }
    return arr;
  };

  // Undo follows what you did last: ink or text.
  const history = useRef<("ink" | "text")[]>([]);
  useEffect(() => {
    if (!editor) return;
    const onUpdate = ({ transaction }: { transaction: import("@tiptap/pm/state").Transaction }) => {
      if (!isChangeOrigin(transaction) && history.current.at(-1) !== "text") history.current.push("text");
    };
    editor.on("update", onUpdate);
    return () => {
      editor.off("update", onUpdate);
    };
  }, [editor]);
  const undo = () => {
    const last = history.current.pop();
    if (last === "ink" && inkUndo.current?.canUndo()) inkUndo.current.undo();
    else editor?.commands.undo();
  };
  const redo = () => {
    if (inkUndo.current?.canRedo()) inkUndo.current.redo();
    else editor?.commands.redo();
  };

  // --- coordinates and view -------------------------------------------------
  const viewportRef = useRef<HTMLDivElement>(null);
  const toWorld = (clientX: number, clientY: number): [number, number] => {
    const r = viewportRef.current!.getBoundingClientRect();
    const v = viewRef.current;
    return [(clientX - r.left - v.x) / v.zoom, (clientY - r.top - v.y) / v.zoom];
  };

  /** Zoom to `zoom`, keeping the page point under (sx, sy) (viewport px) in place. */
  const zoomAt = (zoom: number, sx: number, sy: number) => {
    const v = viewRef.current;
    const z = clampZoom(zoom);
    const wx = (sx - v.x) / v.zoom;
    const wy = (sy - v.y) / v.zoom;
    setView({ x: sx - wx * z, y: sy - wy * z, zoom: z });
  };
  const zoomBy = (factor: number) => {
    const r = viewportRef.current!.getBoundingClientRect();
    zoomAt(viewRef.current.zoom * factor, r.width / 2, r.height / 2);
  };

  /** Fit everything on the page into view. */
  const fit = () => {
    const vp = viewportRef.current!;
    let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const { points } of decoded)
      for (const [x, y] of points) [x0, y0, x1, y1] = [Math.min(x0, x), Math.min(y0, y), Math.max(x1, x), Math.max(y1, y)];
    vp.querySelectorAll<HTMLElement>(".canvas-box:not(.empty)").forEach((el) => {
      [x0, y0] = [Math.min(x0, el.offsetLeft), Math.min(y0, el.offsetTop)];
      [x1, y1] = [Math.max(x1, el.offsetLeft + el.offsetWidth), Math.max(y1, el.offsetTop + el.offsetHeight)];
    });
    if (!Number.isFinite(x0)) return setView({ x: 16, y: 16, zoom: 1 });
    const pad = 24;
    const z = clampZoom(Math.min((vp.clientWidth - 2 * pad) / (x1 - x0 || 1), (vp.clientHeight - 2 * pad) / (y1 - y0 || 1), 2));
    setView({ x: pad - x0 * z, y: pad - y0 * z, zoom: z });
  };

  // Wheel: scroll to pan, Ctrl/⌘ + wheel (or a trackpad pinch) to zoom.
  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = vp.getBoundingClientRect();
      const v = viewRef.current;
      if (e.ctrlKey || e.metaKey) zoomAt(v.zoom * Math.exp(-e.deltaY * 0.0025), e.clientX - r.left, e.clientY - r.top);
      else setView({ ...v, x: v.x - e.deltaX, y: v.y - e.deltaY });
    };
    vp.addEventListener("wheel", onWheel, { passive: false });
    return () => vp.removeEventListener("wheel", onWheel);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // --- text boxes -------------------------------------------------------------
  /** Start typing at page point (wx, wy): reuse the spare empty box or add one. */
  const typeAt = (wx: number, wy: number, text?: string) => {
    if (!editor) return;
    const { doc } = editor.state;
    let spare = -1;
    doc.forEach((node, pos) => {
      if (spare < 0 && node.type.name === "canvasBox" && isEmptyBox(node)) spare = pos;
    });
    const attrs = { x: Math.round(wx), y: Math.round(wy - 14), width: 420 };
    const chain = editor.chain();
    if (spare >= 0) {
      chain
        .command(({ tr }) => {
          tr.setNodeMarkup(spare, undefined, { ...doc.nodeAt(spare)!.attrs, ...attrs });
          return true;
        })
        .setTextSelection(spare + 2);
    } else {
      const end = doc.content.size;
      chain.insertContentAt(end, { type: "canvasBox", attrs, content: [{ type: "paragraph" }] }).setTextSelection(end + 2);
    }
    chain.focus().run();
    if (text) editor.commands.insertContent(text);
  };

  /** The text box element under a screen point, if any. */
  const boxAt = (clientX: number, clientY: number) =>
    Array.from(viewportRef.current?.querySelectorAll<HTMLElement>(".canvas-box:not(.empty)") ?? []).find((el) => {
      const r = el.getBoundingClientRect();
      return clientX >= r.left && clientX <= r.right && clientY >= r.top && clientY <= r.bottom;
    });

  const switchTool = (t: CanvasTool) => {
    setTool(t);
    if (t === "highlighter" && !HIGHLIGHTER_COLORS.includes(color)) setColor(HIGHLIGHTER_COLORS[0]);
    if (t === "pen" && !PEN_COLORS.includes(color)) setColor(PEN_COLORS[0]);
    if (t !== "text") editor?.commands.blur();
  };

  // --- pointer input ------------------------------------------------------------
  // Pen: always uses the current tool. Mouse: the tool, middle button pans.
  // Touch: two fingers pan and pinch-zoom; one finger draws or pans (prefs).
  const touchIds = useRef(new Set<number>());
  const penDown = useRef(false);
  const gesture = useRef<{ pointers: Map<number, { x: number; y: number }>; start: View; c0: { x: number; y: number }; d0: number } | null>(null);
  const drawing = useRef<{ id: number; type: string; points: Point[] } | null>(null);
  const tap = useRef<{ id: number; x: number; y: number; moved: boolean } | null>(null);
  const [draft, setDraft] = useState<Point[] | null>(null);
  const lastPoint = useRef<[number, number]>([60, 80]);

  const local = (x: number, y: number) => {
    const r = viewportRef.current!.getBoundingClientRect();
    return { x: x - r.left, y: y - r.top };
  };
  const measure = (pointers: Map<number, { x: number; y: number }>) => {
    const pts = [...pointers.values()];
    const c = { x: pts.reduce((s, p) => s + p.x, 0) / pts.length, y: pts.reduce((s, p) => s + p.y, 0) / pts.length };
    const d = pts.length >= 2 ? Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y) : 0;
    return { c, d };
  };
  const startGesture = (pointers: Map<number, { x: number; y: number }>) => {
    if (pointers.size === 0) {
      gesture.current = null;
      return;
    }
    const { c, d } = measure(pointers);
    gesture.current = { pointers, start: viewRef.current, c0: c, d0: d };
  };
  const updateGesture = () => {
    const g = gesture.current;
    if (!g) return;
    const { c, d } = measure(g.pointers);
    const zoom = clampZoom(g.d0 > 0 && d > 0 ? g.start.zoom * (d / g.d0) : g.start.zoom);
    const wx = (g.c0.x - g.start.x) / g.start.zoom;
    const wy = (g.c0.y - g.start.y) / g.start.zoom;
    setView({ x: c.x - wx * zoom, y: c.y - wy * zoom, zoom });
  };
  const cancelDraft = () => {
    drawing.current = null;
    setDraft(null);
  };

  const eraseAt = (wx: number, wy: number) => {
    const arr = inkMap.get(CANVAS_INK);
    if (!arr) return;
    const hits = strokesNear(decoded, wx, wy, 10 / viewRef.current.zoom);
    if (hits.length) {
      history.current.push("ink");
      sync.doc.transact(() => {
        for (const i of hits.reverse()) arr.delete(i, 1);
      });
    }
  };

  const onPointerDown = (e: React.PointerEvent) => {
    const target = e.target as HTMLElement;
    if (target.closest(".canvas-zoom")) return;
    const p = local(e.clientX, e.clientY);
    lastPoint.current = toWorld(e.clientX, e.clientY);
    const inBox = tool === "text" && !!target.closest(".canvas-box");

    if (e.pointerType === "touch") {
      touchIds.current.add(e.pointerId);
      if (penDown.current) return; // a palm resting while the pen writes
      if (touchIds.current.size >= 2) {
        // Second finger: pan and pinch-zoom with both.
        if (drawing.current?.type === "touch") cancelDraft();
        tap.current = null;
        const pointers = gesture.current?.pointers ?? new Map();
        pointers.set(e.pointerId, p);
        startGesture(pointers);
        viewportRef.current!.setPointerCapture(e.pointerId);
        e.preventDefault();
        return;
      }
    }
    if (inBox) return; // editing text: the editor handles it
    if (e.pointerType === "mouse" && e.button !== 0) {
      if (e.button === 1) {
        startGesture(new Map([[e.pointerId, p]]));
        e.preventDefault();
      }
      return;
    }
    viewportRef.current!.setPointerCapture(e.pointerId);
    e.preventDefault();

    const fingerPans = e.pointerType === "touch" && !fingerDraws() && tool !== "text";
    if (tool === "hand" || fingerPans) {
      startGesture(new Map([[e.pointerId, p]]));
      return;
    }
    if (tool === "text") {
      tap.current = { id: e.pointerId, x: e.clientX, y: e.clientY, moved: false };
      return;
    }
    if (e.pointerType === "pen") {
      penDown.current = true;
      if (drawing.current?.type === "touch") cancelDraft();
      if (!getPrefs().penSeen) {
        setPrefs({ penSeen: true });
        if (getPrefs().fingerMode === "auto") toast("Pen detected: the pen draws and your finger moves the page.");
      }
    }
    const [wx, wy] = toWorld(e.clientX, e.clientY);
    if (tool === "eraser") {
      drawing.current = { id: e.pointerId, type: e.pointerType, points: [] };
      eraseAt(wx, wy);
      return;
    }
    const pressure = e.pointerType === "pen" ? e.pressure || 0.5 : 0.5;
    drawing.current = { id: e.pointerId, type: e.pointerType, points: [[wx, wy, pressure]] };
    setDraft([[wx, wy, pressure]]);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const g = gesture.current;
    if (g?.pointers.has(e.pointerId)) {
      g.pointers.set(e.pointerId, local(e.clientX, e.clientY));
      updateGesture();
      return;
    }
    const t = tap.current;
    if (t && t.id === e.pointerId) {
      // A drag on empty canvas with the Text tool moves the page.
      if (Math.hypot(e.clientX - t.x, e.clientY - t.y) > 8) {
        tap.current = null;
        startGesture(new Map([[e.pointerId, local(t.x, t.y)]]));
        gesture.current!.pointers.set(e.pointerId, local(e.clientX, e.clientY));
        updateGesture();
      }
      return;
    }
    const d = drawing.current;
    if (!d || d.id !== e.pointerId) return;
    if (tool === "eraser") {
      eraseAt(...toWorld(e.clientX, e.clientY));
      return;
    }
    const events = (e.nativeEvent as PointerEvent).getCoalescedEvents?.() ?? [];
    for (const ev of events.length > 1 ? events : [e.nativeEvent as PointerEvent]) {
      const [wx, wy] = toWorld(ev.clientX, ev.clientY);
      d.points.push([wx, wy, ev.pointerType === "pen" ? ev.pressure || 0.5 : 0.5]);
    }
    setDraft([...d.points]);
  };

  const onPointerUp = (e: React.PointerEvent) => {
    if (e.pointerType === "pen") penDown.current = false;
    touchIds.current.delete(e.pointerId);
    const g = gesture.current;
    if (g?.pointers.has(e.pointerId)) {
      g.pointers.delete(e.pointerId);
      startGesture(g.pointers);
      return;
    }
    const t = tap.current;
    if (t && t.id === e.pointerId) {
      tap.current = null;
      if (e.type === "pointerup") {
        const [wx, wy] = toWorld(e.clientX, e.clientY);
        typeAt(wx, wy);
      }
      return;
    }
    const d = drawing.current;
    if (!d || d.id !== e.pointerId) return;
    drawing.current = null;
    setDraft(null);
    if (tool === "eraser" || d.points.length === 0) return;
    const pts = d.points.length === 1 ? [d.points[0], [d.points[0][0] + 0.3, d.points[0][1] + 0.3, d.points[0][2]] as Point] : d.points;
    history.current.push("ink");
    const id = uuid();
    const extent = Math.max(...pts.map(([x]) => x)) - Math.min(...pts.map(([x]) => x)) + Math.max(...pts.map(([, y]) => y)) - Math.min(...pts.map(([, y]) => y));
    if (extent < 3 / viewRef.current.zoom) dots.current = [...dots.current.filter((t) => Date.now() - t.at < 1000), { id, at: Date.now() }];
    strokeArray().push([
      {
        id,
        tool: tool === "highlighter" ? "highlighter" : "pen",
        color,
        size,
        pts: encodePoints(pts.map(([x, y, p]) => [Math.round(x * 10) / 10, Math.round(y * 10) / 10, p])),
      },
    ]);
  };

  // Double-click / double-tap on text with a drawing tool: edit that text
  // (and drop the two dots the clicks just drew).
  const dots = useRef<{ id: string; at: number }[]>([]);
  const onDoubleClick = (e: React.MouseEvent) => {
    if (tool === "text" || !editor) return;
    if (!boxAt(e.clientX, e.clientY)) return;
    const recent = new Set(dots.current.filter((t) => Date.now() - t.at < 800).map((t) => t.id));
    const arr = inkMap.get(CANVAS_INK);
    if (arr && recent.size) {
      const items = arr.toArray();
      sync.doc.transact(() => {
        for (let i = items.length - 1; i >= 0; i--) if (recent.has(items[i].id)) arr.delete(i, 1);
      });
    }
    dots.current = [];
    setTool("text");
    requestAnimationFrame(() => {
      const pos = editor.view.posAtCoords({ left: e.clientX, top: e.clientY });
      editor.chain().setTextSelection(pos?.pos ?? 1).focus().run();
    });
  };

  // Typing with a drawing tool selected starts a text box where you last pointed.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (tool === "text" || e.ctrlKey || e.metaKey || e.altKey || e.key.length !== 1) return;
    e.preventDefault();
    setTool("text");
    const [wx, wy] = lastPoint.current;
    requestAnimationFrame(() => typeAt(wx, wy, e.key));
  };

  const clearInk = async () => {
    if (!strokes.length) return;
    if (await confirmAction({ title: "Clear the drawing?", message: "Every stroke on this page will be removed. Undo brings them back.", confirmText: "Clear", danger: true })) {
      history.current.push("ink");
      const arr = inkMap.get(CANVAS_INK);
      if (arr?.length) arr.delete(0, arr.length);
    }
  };

  const palette = tool === "highlighter" ? HIGHLIGHTER_COLORS : PEN_COLORS;
  const inkTool = tool === "pen" || tool === "highlighter";
  const grid = 24 * view.zoom;

  return (
    <div className="canvas-page">
      <div className="canvas-tools" role="toolbar" aria-label="Canvas tools">
        <button className={tool === "pen" ? "active" : ""} title="Pen" aria-label="Pen" onClick={() => switchTool("pen")}>
          <PenLine size={18} />
        </button>
        <button className={tool === "highlighter" ? "active" : ""} title="Highlighter" aria-label="Highlighter" onClick={() => switchTool("highlighter")}>
          <Highlighter size={18} />
        </button>
        <button className={tool === "eraser" ? "active" : ""} title="Eraser (removes whole strokes)" aria-label="Eraser" onClick={() => switchTool("eraser")}>
          <Eraser size={18} />
        </button>
        <button className={tool === "text" ? "active" : ""} title="Text: tap anywhere to type" aria-label="Text" onClick={() => switchTool("text")}>
          <Type size={18} />
        </button>
        <button className={tool === "hand" ? "active" : ""} title="Move the page" aria-label="Move the page" onClick={() => switchTool("hand")}>
          <Hand size={18} />
        </button>
        {inkTool && <span className="ink-sep" />}
        {inkTool &&
          palette.map((c) => (
            <button key={c} className={c === color ? "swatch active" : "swatch"} style={{ background: c }} title={c} aria-label={`Colour ${c}`} onClick={() => setColor(c)} />
          ))}
        {inkTool && <span className="ink-sep" />}
        {inkTool &&
          SIZES.map((s) => (
            <button key={s} className={s === size ? "size active" : "size"} title={`Size ${s}`} aria-label={`Size ${s}`} onClick={() => setSize(s)}>
              <span style={{ width: s + 3, height: s + 3 }} />
            </button>
          ))}
        <span className="ink-spacer" />
        {(hasTouch || prefs.penSeen) && tool !== "text" && tool !== "hand" && (
          <button
            className="finger-mode"
            title="What one finger does. A pen always uses the tool; two fingers always move and zoom."
            onClick={() => setPrefs({ fingerMode: fingerDrawsNow ? "scroll" : "draw" })}
          >
            {fingerDrawsNow ? <Pointer size={16} /> : <Hand size={16} />}
            <span>{fingerDrawsNow ? "Finger draws" : "Finger moves"}</span>
          </button>
        )}
        <button title="Undo" aria-label="Undo" onClick={undo}>
          <Undo2 size={18} />
        </button>
        <button title="Redo" aria-label="Redo" onClick={redo}>
          <Redo2 size={18} />
        </button>
        <button title="Clear the drawing" aria-label="Clear the drawing" disabled={!strokes.length} onClick={() => void clearInk()}>
          <Trash2 size={18} />
        </button>
      </div>
      {tool === "text" && editor && (
        <div className="canvas-format">
          <Toolbar editor={editor} editMath={editMath} onDraw={() => switchTool("pen")} />
        </div>
      )}
      <div
        ref={viewportRef}
        className={`canvas-viewport tool-${tool}`}
        style={{ backgroundSize: `${grid}px ${grid}px`, backgroundPosition: `${view.x}px ${view.y}px` }}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={onDoubleClick}
        onKeyDown={onKeyDown}
        onDragOver={(e) => e.dataTransfer.types.includes("Files") && e.preventDefault()}
        onDrop={(e) => {
          const files = Array.from(e.dataTransfer.files);
          if (!files.length || !editor) return;
          e.preventDefault();
          const [wx, wy] = toWorld(e.clientX, e.clientY);
          typeAt(wx, wy);
          insertFiles(editor, files).catch(reportError);
        }}
      >
        <div className="canvas-world" style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.zoom})` }}>
          <EditorContent editor={editor} className="canvas-text" />
          <svg className="canvas-ink" width="1" height="1" aria-hidden="true">
            <StrokeLayer strokes={decoded} />
            {draft && tool !== "eraser" && (
              <path d={strokeOutline(draft, tool === "highlighter" ? "highlighter" : "pen", size, false)} fill={color} opacity={tool === "highlighter" ? 0.4 : 1} />
            )}
          </svg>
        </div>
        <div className="canvas-zoom">
          <button title="Zoom out" aria-label="Zoom out" onClick={() => zoomBy(1 / 1.25)}>
            <Minus size={16} />
          </button>
          <button className="zoom-level" title="Back to 100%" onClick={() => zoomBy(1 / viewRef.current.zoom)}>
            {Math.round(view.zoom * 100)}%
          </button>
          <button title="Zoom in" aria-label="Zoom in" onClick={() => zoomBy(1.25)}>
            <Plus size={16} />
          </button>
          <button title="Show everything" aria-label="Show everything" onClick={fit}>
            <Maximize size={16} />
          </button>
        </div>
      </div>
    </div>
  );
}
