import { getStroke } from "perfect-freehand";

// Strokes shared by drawing blocks (InkBlock) and canvas pages (CanvasEditor).
// A stroke is `{id, tool, color, size, pts}` with `pts` a Float32 (x, y,
// pressure) buffer; see crates/anynote-core/src/ink.rs for the Rust side.

export interface InkStroke {
  id: string;
  tool: "pen" | "highlighter";
  color: string;
  size: number;
  pts: Uint8Array;
}

export type Tool = "pen" | "highlighter" | "eraser";
export type Point = [number, number, number];

export const PEN_COLORS = ["#1f2328", "#2f6fdf", "#d93f3f", "#1f9d55", "#e07b00", "#8250df"];
export const HIGHLIGHTER_COLORS = ["#ffe14d", "#7ee787", "#ff9bd2", "#79c0ff"];
export const SIZES = [2, 4, 8];

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

export function svgPath(outline: number[][]): string {
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

export function strokeOutline(points: Point[], tool: "pen" | "highlighter", size: number, done: boolean): string {
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

export function distanceToSegment(px: number, py: number, a: Point, b: Point) {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len = dx * dx + dy * dy;
  const t = len === 0 ? 0 : Math.max(0, Math.min(1, ((px - a[0]) * dx + (py - a[1]) * dy) / len));
  return Math.hypot(px - (a[0] + t * dx), py - (a[1] + t * dy));
}

/** Indexes of the strokes within `radius` of (x, y). */
export function strokesNear(strokes: { stroke: InkStroke; points: Point[] }[], x: number, y: number, radius: number): number[] {
  const hits: number[] = [];
  strokes.forEach(({ stroke, points }, index) => {
    const reach = radius + (stroke.tool === "highlighter" ? stroke.size * 2 : stroke.size);
    for (let i = 0; i < points.length; i++) {
      if (distanceToSegment(x, y, points[i], points[i + 1] ?? points[i]) <= reach) {
        hits.push(index);
        break;
      }
    }
  });
  return hits;
}
