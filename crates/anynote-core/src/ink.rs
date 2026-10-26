//! Vector ink.
//!
//! Strokes live in the page's CRDT document (a `Y.Map` named [`INK_MAP`] whose
//! keys are the `inkId` of each `inkBlock` node, and whose values are `Y.Array`s
//! of strokes), so handwriting syncs and merges exactly like text does.
//!
//! On the wire a stroke is a map `{id, tool, color, size, pts}` where `pts` is a
//! byte buffer of little-endian `f32` triples `(x, y, pressure)` in CSS pixels
//! relative to the ink block's top-left corner. A flat buffer keeps ink-heavy
//! pages small (12 bytes per point).

use serde::{Deserialize, Serialize};

pub const INK_MAP: &str = "ink";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: String,
    /// `"pen"` or `"highlighter"`.
    pub tool: String,
    /// CSS colour, normally `#rrggbb`.
    pub color: String,
    /// Base stroke width in CSS pixels.
    pub size: f64,
    /// `[x, y, pressure]` samples; pressure is 0..1.
    pub points: Vec<[f32; 3]>,
}

impl Stroke {
    pub fn encode_points(points: &[[f32; 3]]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(points.len() * 12);
        for p in points {
            for v in p {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
        buf
    }

    pub fn decode_points(buf: &[u8]) -> Vec<[f32; 3]> {
        buf.chunks_exact(12)
            .map(|c| {
                let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
                [f(0), f(4), f(8)]
            })
            .collect()
    }

    /// Bounding box `(min_x, min_y, max_x, max_y)` including the stroke width.
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let half = (self.size / 2.0) as f32;
        self.points.iter().fold(None, |acc, [x, y, _]| {
            let (a, b, c, d) = acc.unwrap_or((f32::MAX, f32::MAX, f32::MIN, f32::MIN));
            Some((a.min(x - half), b.min(y - half), c.max(x + half), d.max(y + half)))
        })
    }

    /// Render as an SVG path (a polyline through the samples). Used by
    /// exporters; the app itself renders with perfect-freehand.
    pub fn to_svg_path(&self) -> String {
        let mut d = String::new();
        for (i, [x, y, _]) in self.points.iter().enumerate() {
            d.push_str(if i == 0 { "M" } else { " L" });
            d.push_str(&format!("{x:.1} {y:.1}"));
        }
        let opacity = if self.tool == "highlighter" { 0.35 } else { 1.0 };
        format!(
            r#"<path d="{d}" fill="none" stroke="{}" stroke-width="{:.1}" stroke-linecap="round" stroke-linejoin="round" opacity="{opacity}"/>"#,
            self.color, self.size
        )
    }
}

/// Render strokes as a standalone SVG document.
pub fn strokes_to_svg(strokes: &[Stroke]) -> String {
    let (mut w, mut h) = (1.0f32, 1.0f32);
    for s in strokes {
        if let Some((_, _, x, y)) = s.bounds() {
            w = w.max(x);
            h = h.max(y);
        }
    }
    let paths: String = strokes.iter().map(Stroke::to_svg_path).collect::<Vec<_>>().join("\n  ");
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.0} {h:.0}\">\n  {paths}\n</svg>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_round_trip() {
        let pts = vec![[1.5, 2.25, 0.5], [10.0, -3.0, 1.0]];
        assert_eq!(Stroke::decode_points(&Stroke::encode_points(&pts)), pts);
    }
}
