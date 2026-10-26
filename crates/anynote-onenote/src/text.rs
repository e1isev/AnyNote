//! Pure conversion helpers that don't touch parser state: splitting text
//! into runs, run formatting → marks, colours, note-tag mapping, page
//! hierarchy and reading order. Kept separate so they are easy to unit test.

use anynote_core::node::{kinds, marks};

use crate::math::{self, MathObj};
use anynote_core::{Mark, Node};
use onenote_parser::contents::ParagraphStyling;
use onenote_parser::property::common::ColorRef;

/// A timestamp in milliseconds since the Unix epoch.
pub fn datetime_ms(t: time::UtcDateTime) -> i64 {
    (t.unix_timestamp_nanos() / 1_000_000) as i64
}

/// Split `text` at UTF-16 run boundaries as OneNote stores them: style `i`
/// covers the text up to `indices[i]` and the last style runs to the end.
/// Returns each segment with the index of its style (`None` for text that no
/// style covers, which happens with malformed run tables).
pub fn split_utf16(text: &str, indices: &[u32], styles: usize) -> Vec<(String, Option<usize>)> {
    split_with_links(text, indices, styles, &[]).into_iter().map(|(t, s, _)| (t, s)).collect()
}

/// Like [`split_utf16`], also cutting at hyperlink edges (`(start, end,
/// target)` in UTF-16 units) and attaching each piece's link target.
pub fn split_with_links(
    text: &str,
    indices: &[u32],
    styles: usize,
    links: &[(u32, u32, String)],
) -> Vec<(String, Option<usize>, Option<String>)> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let total = utf16.len() as u32;
    let mut runs: Vec<(u32, u32, Option<usize>)> = Vec::new();
    let mut start = 0u32;
    for i in 0..styles {
        let end = indices.get(i).copied().unwrap_or(total).min(total).max(start);
        if end > start {
            runs.push((start, end, Some(i)));
        }
        start = end;
    }
    if start < total {
        runs.push((start, total, None));
    }
    let mut out = Vec::new();
    for (s, e, style) in runs {
        let mut cuts = vec![s, e];
        cuts.extend(links.iter().flat_map(|(ls, le, _)| [*ls, *le]).filter(|c| *c > s && *c < e));
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let link = links.iter().find(|(ls, le, _)| a >= *ls && a < *le).map(|l| l.2.clone());
            out.push((String::from_utf16_lossy(&utf16[a as usize..b as usize]), style, link));
        }
    }
    out
}

/// The formatting of one run, decoupled from the parser type.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RunStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub hidden: bool,
    pub math: bool,
    pub font: Option<String>,
    pub color: Option<[u8; 3]>,
    pub highlight: Option<[u8; 3]>,
}

impl RunStyle {
    pub fn from_styling(s: &ParagraphStyling) -> Self {
        let rgb = |c: Option<ColorRef>| match c {
            Some(ColorRef::Manual { r, g, b }) => Some([r, g, b]),
            _ => None,
        };
        RunStyle {
            bold: s.bold(),
            italic: s.italic(),
            underline: s.underline(),
            strike: s.strikethrough(),
            superscript: s.superscript(),
            subscript: s.subscript(),
            hidden: s.hidden(),
            math: s.math_formatting(),
            font: s.font().map(str::to_string),
            color: rgb(s.font_color()),
            highlight: rgb(s.highlight()),
        }
    }

    /// Combine paragraph-level formatting with a run's own formatting.
    pub fn inherit(&self, base: &RunStyle) -> RunStyle {
        RunStyle {
            bold: self.bold || base.bold,
            italic: self.italic || base.italic,
            underline: self.underline || base.underline,
            strike: self.strike || base.strike,
            superscript: self.superscript || base.superscript,
            subscript: self.subscript || base.subscript,
            hidden: self.hidden,
            math: self.math,
            font: self.font.clone().or_else(|| base.font.clone()),
            color: self.color.or(base.color),
            highlight: self.highlight.or(base.highlight),
        }
    }
}

pub fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Ink colours are Windows COLORREFs: `0x00BBGGRR`.
pub fn colorref_hex(value: u32) -> String {
    let [r, g, b, _] = value.to_le_bytes();
    hex([r, g, b])
}

pub fn is_monospace(font: &str) -> bool {
    let f = font.to_ascii_lowercase();
    [
        "consolas",
        "courier",
        "lucida console",
        "lucida sans typewriter",
        "menlo",
        "monaco",
        "cascadia",
        "source code",
        "fira code",
        "fira mono",
        "jetbrains mono",
        "dejavu sans mono",
        "liberation mono",
        "ubuntu mono",
        "sf mono",
        "roboto mono",
        "inconsolata",
        "monospace",
    ]
    .iter()
    .any(|m| f.contains(m))
}

/// Marks for a run. Black text and white highlights are dropped: they are
/// what OneNote writes for "default" colours in pasted content, and keeping
/// them would make text unreadable in a dark theme.
pub fn run_marks(style: &RunStyle, link: Option<&str>) -> Vec<Mark> {
    let mut out = Vec::new();
    if let Some(href) = link {
        out.push(Mark::new(marks::LINK).with_attr("href", href));
    }
    let flags = [
        (style.bold, marks::BOLD),
        (style.italic, marks::ITALIC),
        (style.underline && link.is_none(), marks::UNDERLINE),
        (style.strike, marks::STRIKE),
        (style.superscript, marks::SUPERSCRIPT),
        (style.subscript && !style.superscript, marks::SUBSCRIPT),
    ];
    for (on, kind) in flags {
        if on {
            out.push(Mark::new(kind));
        }
    }
    if style.font.as_deref().is_some_and(is_monospace) {
        out.push(Mark::new(marks::CODE));
    }
    if let Some(c) = style.highlight.filter(|c| *c != [255, 255, 255]) {
        out.push(Mark::new(marks::HIGHLIGHT).with_attr("color", hex(c)));
    }
    if let Some(c) = style.color.filter(|c| *c != [0, 0, 0]) {
        // Links carry their own colour; OneNote's default link blue is noise.
        if link.is_none() {
            out.push(Mark::new("textStyle").with_attr("color", hex(c)));
        }
    }
    out
}

/// A run of text with its formatting and (optional) hyperlink target.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub text: String,
    pub style: RunStyle,
    pub link: Option<String>,
    /// The equation structure attached to a math-formatted run.
    pub math: Option<MathObj>,
}

const HYPERLINK_MARKER: &str = "\u{fddf}HYPERLINK \"";

/// Turn styled segments into inline nodes: hidden text is dropped, line
/// breaks become `hardBreak`, math runs become `inlineMath`, bare URLs are
/// linked, and stray `HYPERLINK` field codes are removed.
pub fn inline_nodes(segments: &[Segment]) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    let mut math: Vec<(String, Option<MathObj>)> = Vec::new();
    for seg in segments {
        if seg.style.hidden {
            continue;
        }
        if seg.style.math {
            math.push((seg.text.clone(), seg.math));
            continue;
        }
        flush_math(&mut math, &mut out);
        let text = strip_field_codes(&seg.text);
        let mut first = true;
        for line in text.split(['\u{b}', '\n', '\r', '\u{2028}']) {
            if !first {
                out.push(Node::new(kinds::HARD_BREAK));
            }
            first = false;
            let clean: String = line.chars().filter(|c| !c.is_control() || *c == '\t').collect();
            if clean.is_empty() {
                continue;
            }
            match &seg.link {
                Some(href) => out.push(Node::styled_text(clean, run_marks(&seg.style, Some(href)))),
                None => {
                    for (piece, url) in linkify(&clean) {
                        out.push(Node::styled_text(piece, run_marks(&seg.style, url.as_deref())));
                    }
                }
            }
        }
    }
    flush_math(&mut math, &mut out);
    // "\r\n" produces an empty line between two breaks; collapse those.
    out.dedup_by(|b, a| a.is(kinds::HARD_BREAK) && b.is(kinds::HARD_BREAK));
    while out.last().is_some_and(|n| n.is(kinds::HARD_BREAK)) {
        out.pop();
    }
    while out.first().is_some_and(|n| n.is(kinds::HARD_BREAK)) {
        out.remove(0);
    }
    let mut p = Node::paragraph(out);
    p.normalize_text();
    p.content
}

fn flush_math(math: &mut Vec<(String, Option<MathObj>)>, out: &mut Vec<Node>) {
    if math.is_empty() {
        return;
    }
    let latex = math::to_latex(math);
    math.clear();
    if !latex.trim().is_empty() {
        out.push(Node::new(kinds::INLINE_MATH).with_attr("latex", latex));
    }
}

/// Remove `\u{fddf}HYPERLINK "target"` field codes left in visible text.
pub fn strip_field_codes(text: &str) -> String {
    let mut s = text.to_string();
    while let Some(start) = s.find(HYPERLINK_MARKER) {
        let after = start + HYPERLINK_MARKER.len();
        let end = s[after..].find('"').map_or(s.len(), |i| after + i + 1);
        s.replace_range(start..end, "");
    }
    s.replace('\u{fddf}', "")
}

/// Split text into pieces, marking `http(s)://` URLs with their target.
pub fn linkify(text: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut rest = text;
    while let Some(pos) = find_url_start(rest) {
        let tail = &rest[pos..];
        let len = tail.find(char::is_whitespace).unwrap_or(tail.len());
        let mut url = &tail[..len];
        url = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
        if url.ends_with(')') && !url.contains('(') {
            url = &url[..url.len() - 1];
        }
        if url.len() <= "https://".len() {
            out.push((rest[..pos + len].to_string(), None));
            rest = &rest[pos + len..];
            continue;
        }
        if pos > 0 {
            out.push((rest[..pos].to_string(), None));
        }
        out.push((url.to_string(), Some(url.to_string())));
        rest = &rest[pos + url.len()..];
    }
    if !rest.is_empty() {
        out.push((rest.to_string(), None));
    }
    out
}

fn find_url_start(s: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(i) = s[search..].find("http") {
        let at = search + i;
        let ok_prefix = at == 0 || s[..at].ends_with(|c: char| c.is_whitespace() || c == '(' || c == '<');
        if ok_prefix && (s[at..].starts_with("http://") || s[at..].starts_with("https://")) {
            return Some(at);
        }
        search = at + 4;
    }
    None
}

pub fn heading_level(style_id: &str) -> Option<u8> {
    match style_id {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// How a OneNote note tag is represented in AnyNote:
///
/// * checkable tags ("To Do", "Discuss with …", and every other checkbox
///   shape) become task items, checked when OneNote marks them completed;
/// * every other tag ("Important", "Question", "Idea", custom tags) becomes a
///   `#hashtag` at the start of the paragraph (label lower-cased, spaces → `-`)
///   so it shows up in AnyNote's tag view and search.
#[derive(Debug, Clone, PartialEq)]
pub enum TagMapping {
    Task { checked: bool },
    Hashtag(String),
    Ignore,
}

pub fn map_note_tag(label: &str, checkable: bool, completed: bool) -> TagMapping {
    if checkable {
        return TagMapping::Task { checked: completed };
    }
    let tag: String = label
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/'))
        .collect();
    let tag = tag.trim_matches('-').to_string();
    if tag.is_empty() || tag.chars().all(|c| c.is_ascii_digit()) {
        TagMapping::Ignore
    } else {
        TagMapping::Hashtag(tag)
    }
}

/// Resolve OneNote page levels (1 = top level, 2 = subpage, …) to the index
/// of each page's parent: the nearest previous page with a lower level.
pub fn resolve_parents(levels: &[i32]) -> Vec<Option<usize>> {
    let mut stack: Vec<(i32, usize)> = Vec::new();
    levels
        .iter()
        .enumerate()
        .map(|(i, &level)| {
            while stack.last().is_some_and(|&(l, _)| l >= level) {
                stack.pop();
            }
            let parent = stack.last().map(|&(_, idx)| idx);
            stack.push((level, i));
            parent
        })
        .collect()
}

/// Indices of free-floating page items in reading order: top to bottom, then
/// left to right. Items without a position keep their stored order at the top.
pub fn reading_order(positions: &[(Option<f32>, Option<f32>)]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..positions.len()).collect();
    let key = |i: usize| {
        let (x, y) = positions[i];
        let f = |v: Option<f32>| v.filter(|v| v.is_finite()).unwrap_or(0.0);
        (f(y), f(x))
    };
    idx.sort_by(|&a, &b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
    idx
}

/// Half-inch increments (OneNote layout unit) to CSS pixels.
pub fn half_inch_to_px(v: f32) -> f64 {
    (f64::from(v) * 48.0).round()
}

/// Normalise a OneNote GUID (`{ABCD-…}`) for comparisons.
pub fn normalize_guid(s: &str) -> String {
    s.trim().trim_start_matches('{').trim_end_matches('}').to_ascii_lowercase()
}

/// The target page GUID of an `onenote:` link (`…&page-id={GUID}&…`).
pub fn onenote_link_page_id(href: &str) -> Option<String> {
    if !href.to_ascii_lowercase().starts_with("onenote:") {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    let at = lower.find("page-id=")?;
    let rest = &href[at + "page-id=".len()..];
    let end = rest.find('&').unwrap_or(rest.len());
    let id =
        normalize_guid(&rest[..end].replace("%7B", "{").replace("%7D", "}").replace("%7b", "{").replace("%7d", "}"));
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_runs_by_utf16_index() {
        // "😀" is two UTF-16 code units.
        let runs = split_utf16("ab😀cd", &[2, 4], 3);
        assert_eq!(runs, vec![("ab".into(), Some(0)), ("😀".into(), Some(1)), ("cd".into(), Some(2))]);
        assert_eq!(split_utf16("abc", &[], 0), vec![("abc".into(), None)]);
        // A run table that stops early leaves the rest unstyled.
        assert_eq!(
            split_utf16("abcdef", &[2, 3], 2),
            vec![("ab".into(), Some(0)), ("c".into(), Some(1)), ("def".into(), None)]
        );
        // Out-of-range indices are clamped.
        assert_eq!(split_utf16("ab", &[9], 2), vec![("ab".into(), Some(0))]);
        // Links cut runs.
        let pieces = split_with_links("see docs now", &[], 1, &[(4, 8, "https://d".into())]);
        assert_eq!(
            pieces,
            vec![
                ("see ".into(), Some(0), None),
                ("docs".into(), Some(0), Some("https://d".into())),
                (" now".into(), Some(0), None)
            ]
        );
    }

    #[test]
    fn formatting_becomes_marks() {
        let style = RunStyle {
            bold: true,
            underline: true,
            font: Some("Consolas".into()),
            color: Some([255, 0, 0]),
            highlight: Some([255, 255, 0]),
            ..Default::default()
        };
        let kinds: Vec<_> = run_marks(&style, None).into_iter().map(|m| m.kind).collect();
        assert_eq!(kinds, ["bold", "underline", "code", "highlight", "textStyle"]);
        let marks = run_marks(&style, Some("https://x.org"));
        assert_eq!(marks[0].attr_str("href"), Some("https://x.org"));
        assert!(!marks.iter().any(|m| m.kind == "underline" || m.kind == "textStyle"));
        let black = RunStyle { color: Some([0, 0, 0]), highlight: Some([255, 255, 255]), ..Default::default() };
        assert!(run_marks(&black, None).is_empty());
    }

    #[test]
    fn inline_nodes_handle_breaks_hidden_math_and_urls() {
        let seg = |t: &str, style: RunStyle| Segment { text: t.into(), style, link: None, math: None };
        let nodes = inline_nodes(&[
            seg("\u{fddf}HYPERLINK \"x\"", RunStyle { hidden: true, ..Default::default() }),
            seg("line one\u{b}see https://example.com/a.", RunStyle::default()),
            seg("x²", RunStyle { math: true, ..Default::default() }),
            seg("\r\n", RunStyle::default()),
        ]);
        let kinds: Vec<_> = nodes.iter().map(|n| n.kind.as_str()).collect();
        assert_eq!(kinds, ["text", "hardBreak", "text", "text", "text", "inlineMath"]);
        assert_eq!(nodes[3].text.as_deref(), Some("https://example.com/a"));
        assert_eq!(nodes[3].marks[0].attr_str("href"), Some("https://example.com/a"));
        assert_eq!(nodes[4].text.as_deref(), Some("."));
        assert_eq!(nodes[5].attr_str("latex"), Some("x²"));
    }

    #[test]
    fn strips_visible_field_codes() {
        assert_eq!(strip_field_codes("a\u{fddf}HYPERLINK \"http://x\"b"), "ab");
    }

    #[test]
    fn note_tags_map_to_tasks_or_hashtags() {
        assert_eq!(map_note_tag("To Do", true, true), TagMapping::Task { checked: true });
        assert_eq!(map_note_tag("Remember for later", false, false), TagMapping::Hashtag("remember-for-later".into()));
        assert_eq!(map_note_tag("Idea!", false, false), TagMapping::Hashtag("idea".into()));
        assert_eq!(map_note_tag("  ", false, false), TagMapping::Ignore);
    }

    #[test]
    fn page_levels_resolve_to_parents() {
        assert_eq!(
            resolve_parents(&[1, 2, 3, 2, 1, 3, 2]),
            vec![None, Some(0), Some(1), Some(0), None, Some(4), Some(4)]
        );
        // OneNote sometimes uses level 0 for top-level pages.
        assert_eq!(resolve_parents(&[0, 1, 0]), vec![None, Some(0), None]);
    }

    #[test]
    fn outlines_sort_top_to_bottom_then_left_to_right() {
        let order =
            reading_order(&[(Some(5.0), Some(2.0)), (Some(1.0), Some(2.0)), (None, None), (Some(0.0), Some(10.0))]);
        assert_eq!(order, vec![2, 1, 0, 3]);
    }

    #[test]
    fn colours_and_links() {
        assert_eq!(colorref_hex(0x0000_00FF), "#ff0000");
        assert_eq!(onenote_link_page_id("onenote:#Page&section-id={AB-12}&page-id={DE-34}&end"), Some("de-34".into()));
        assert_eq!(onenote_link_page_id("onenote:x#Page&page-id=%7BDE-34%7D&end"), Some("de-34".into()));
        assert_eq!(onenote_link_page_id("https://x"), None);
        assert_eq!(normalize_guid("{AB-CD}"), "ab-cd");
    }
}
