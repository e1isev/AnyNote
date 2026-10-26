//! One OneNote page → one AnyNote block tree (+ ink).
//!
//! OneNote pages are free-form canvases of outlines, images, files and ink.
//! AnyNote pages are documents, so the converter flattens the canvas into
//! reading order and maps OneNote's paragraph model onto the editor schema
//! (see `docs/FORMAT.md` and the schema notes in `lib.rs`).

use std::collections::{BTreeMap, HashMap};
use std::io::Read;

use anynote_core::assets::mime_from_name;
use anynote_core::ids::new_id;
use anynote_core::ink::Stroke;
use anynote_core::node::{kinds, marks};
use anynote_core::Node;
use onenote_parser::contents::{
    Content, EmbeddedFile, EmbeddedObject, FileDataStatus, Image, Ink, InkStroke, OutlineElement, OutlineItem,
    RichText, Table,
};
use onenote_parser::page::{Page, PageContent};
use serde_json::Value;

use crate::math::MathObj;
use crate::text::{self, RunStyle, Segment, TagMapping};

/// Ink units are 0.01 mm: 2540 per inch, 96 CSS px per inch.
const INK_UNITS_PER_PX: f32 = 2540.0 / 96.0;

/// Where converted binaries go (the store in production, a map in tests).
pub trait AssetSink {
    /// Store `bytes` and return the URL documents should reference.
    fn put(&mut self, bytes: &[u8], name: Option<&str>, mime: Option<&str>) -> anyhow::Result<String>;
}

/// Counters and warnings shared by all pages of an import.
#[derive(Debug, Default)]
pub struct Stats {
    pub assets: usize,
    pub strokes: usize,
    pub warnings: Vec<String>,
}

pub struct ConvertedPage {
    pub title: String,
    pub level: i32,
    pub created_at: Option<i64>,
    /// Normalised page GUID, the target of `onenote:` links.
    pub guid: String,
    pub doc: Node,
    pub ink: BTreeMap<String, Vec<Stroke>>,
    pub has_onenote_links: bool,
}

pub fn convert_page(page: &Page, context: &str, sink: &mut dyn AssetSink, stats: &mut Stats) -> ConvertedPage {
    let title = page.title_text().map(clean_title).filter(|t| !t.is_empty()).unwrap_or_else(|| "Untitled page".into());
    let mut conv = Converter { sink, stats, ink: BTreeMap::new(), context: format!("{context} / {title}") };

    // Lay the canvas out: everything gets a top-left position in px. Ink is
    // often stored as many one-stroke objects; strokes that touch or nearly
    // touch are clustered into one drawing.
    let contents = page.contents();
    let mut items: Vec<(Option<f32>, Option<f32>)> = Vec::new();
    let mut kinds_: Vec<Item> = Vec::new();
    let mut inks: Vec<(Vec<PxStroke>, [f32; 4])> = Vec::new();
    for (i, c) in contents.iter().enumerate() {
        let at = |x: Option<f32>, y: Option<f32>| (x.map(|v| v * 48.0), y.map(|v| v * 48.0));
        match c {
            PageContent::Outline(o) => {
                items.push(at(o.offset_horizontal(), o.offset_vertical()));
                kinds_.push(Item::Content(i));
            }
            PageContent::Image(img) => {
                items.push(at(img.offset_horizontal(), img.offset_vertical()));
                kinds_.push(Item::Content(i));
            }
            PageContent::EmbeddedFile(f) => {
                items.push(at(f.offset_horizontal(), f.offset_vertical()));
                kinds_.push(Item::Content(i));
            }
            PageContent::Ink(ink) => {
                let origin =
                    (ink.offset_horizontal().unwrap_or(0.0) * 48.0, ink.offset_vertical().unwrap_or(0.0) * 48.0);
                let mut strokes = Vec::new();
                px_strokes(ink, origin, &mut strokes);
                if let Some(bounds) = bounds(&strokes) {
                    inks.push((strokes, bounds));
                }
            }
            PageContent::Unknown => conv.warn("skipped an unsupported page element"),
        }
    }
    for (strokes, b) in cluster_ink(inks) {
        items.push((Some(b[0]), Some(b[1])));
        kinds_.push(Item::Ink(strokes));
    }
    let order = text::reading_order(&items);
    let mut slots: Vec<Option<Item>> = kinds_.into_iter().map(Some).collect();
    let mut blocks = Vec::new();
    for i in order {
        match slots[i].take() {
            Some(Item::Content(ci)) => match &contents[ci] {
                PageContent::Outline(o) => blocks.extend(conv.items(o.items())),
                PageContent::Image(img) => blocks.extend(conv.image(img)),
                PageContent::EmbeddedFile(f) => blocks.extend(conv.file(f)),
                _ => {}
            },
            Some(Item::Ink(strokes)) => blocks.extend(conv.ink_block(&strokes)),
            None => {}
        }
    }
    merge_adjacent(&mut blocks);
    if blocks.is_empty() {
        blocks.push(Node::paragraph(vec![]));
    }
    let mut doc = Node::doc(blocks);
    doc.ensure_block_ids();

    let mut has_onenote_links = false;
    doc.walk(&mut |n| {
        if n.marks
            .iter()
            .any(|m| m.kind == marks::LINK && m.attr_str("href").is_some_and(|h| h.starts_with("onenote:")))
        {
            has_onenote_links = true;
        }
    });
    let created = text::datetime_ms(page.created_time());
    ConvertedPage {
        title,
        level: page.level(),
        created_at: (created > 0).then_some(created),
        guid: text::normalize_guid(page.link_target_id()),
        doc,
        ink: conv.ink,
        has_onenote_links,
    }
}

fn clean_title(t: &str) -> String {
    text::strip_field_codes(t)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ListKind {
    Bullet,
    Ordered(Option<i32>),
    Task(bool),
}

impl ListKind {
    fn same_list(self, other: ListKind) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

struct Converter<'a> {
    sink: &'a mut dyn AssetSink,
    stats: &'a mut Stats,
    ink: BTreeMap<String, Vec<Stroke>>,
    context: String,
}

impl Converter<'_> {
    fn warn(&mut self, msg: impl AsRef<str>) {
        self.stats.warnings.push(format!("{}: {}", self.context, msg.as_ref()));
    }

    // -- outlines -------------------------------------------------------------

    fn items(&mut self, items: &[OutlineItem]) -> Vec<Node> {
        let mut elements = Vec::new();
        flatten(items, &mut elements);
        self.elements(&elements)
    }

    /// Convert a run of outline elements, grouping list items into lists.
    fn elements(&mut self, elements: &[&OutlineElement]) -> Vec<Node> {
        let mut out = Vec::new();
        let mut list: Option<(ListKind, Vec<Node>)> = None;
        for el in elements {
            match self.list_kind(el) {
                Some(kind) => {
                    let item = self.list_item(el, kind);
                    match &mut list {
                        Some((k, items)) if k.same_list(kind) => items.push(item),
                        _ => {
                            if let Some(done) = list.take() {
                                out.push(list_node(done));
                            }
                            list = Some((kind, vec![item]));
                        }
                    }
                }
                None => {
                    if let Some(done) = list.take() {
                        out.push(list_node(done));
                    }
                    out.extend(self.element_blocks(el, false));
                    // Indented children of a plain paragraph: keep them, flattened.
                    out.extend(self.items(el.children()));
                }
            }
        }
        if let Some(done) = list.take() {
            out.push(list_node(done));
        }
        merge_adjacent(&mut out);
        out
    }

    fn list_kind(&self, el: &OutlineElement) -> Option<ListKind> {
        for content in el.contents() {
            if let Content::RichText(rt) = content {
                for tag in rt.note_tags() {
                    if let TagMapping::Task { checked } = map_tag(tag) {
                        return Some(ListKind::Task(checked));
                    }
                }
            }
        }
        let list = el.list_contents().first()?;
        if list.list_format().first() == Some(&'\u{fffd}') {
            Some(ListKind::Ordered(list.list_restart()))
        } else {
            Some(ListKind::Bullet)
        }
    }

    fn list_item(&mut self, el: &OutlineElement, kind: ListKind) -> Node {
        let mut blocks = self.element_blocks(el, true);
        blocks.extend(self.items(el.children()));
        match blocks.first_mut() {
            Some(first) if first.is(kinds::HEADING) => {
                // A list item must start with a paragraph; keep the emphasis.
                first.kind = kinds::PARAGRAPH.into();
                first.attrs.remove("level");
                for n in &mut first.content {
                    if n.is_text() && !n.has_mark(marks::BOLD) {
                        n.marks.push(anynote_core::Mark::new(marks::BOLD));
                    }
                }
            }
            Some(first) if first.is(kinds::PARAGRAPH) => {}
            _ => blocks.insert(0, Node::paragraph(vec![])),
        }
        match kind {
            ListKind::Task(checked) => Node::element(kinds::TASK_ITEM, blocks).with_attr("checked", checked),
            _ => Node::element(kinds::LIST_ITEM, blocks),
        }
    }

    fn element_blocks(&mut self, el: &OutlineElement, in_list: bool) -> Vec<Node> {
        let mut out = Vec::new();
        for content in el.contents() {
            match content {
                Content::RichText(rt) => out.extend(self.rich_text(rt, in_list)),
                Content::Table(t) => out.extend(self.table(t)),
                Content::Image(img) => out.extend(self.image(img)),
                Content::EmbeddedFile(f) => out.extend(self.file(f)),
                Content::Ink(ink) => {
                    let mut strokes = Vec::new();
                    px_strokes(ink, (0.0, 0.0), &mut strokes);
                    out.extend(self.ink_block(&strokes));
                }
                Content::Unknown => self.warn("skipped an unsupported paragraph element"),
            }
        }
        out
    }

    // -- text -----------------------------------------------------------------

    fn rich_text(&mut self, rt: &RichText, in_list: bool) -> Vec<Node> {
        // Handwriting written inline: OneNote stores it as ink objects in the
        // paragraph, positioned in page coordinates.
        if !rt.embedded_objects().is_empty() {
            let mut strokes = Vec::new();
            for obj in rt.embedded_objects() {
                if let EmbeddedObject::Ink(container) = obj {
                    px_strokes(container.ink(), (0.0, 0.0), &mut strokes);
                }
            }
            return self.ink_block(&strokes).into_iter().collect();
        }

        let style_id = rt.paragraph_style().style_id().unwrap_or("p").to_string();
        let heading = text::heading_level(&style_id);
        let mut base = RunStyle::from_styling(rt.paragraph_style());
        if heading.is_some() || matches!(style_id.as_str(), "PageTitle" | "cite") {
            // The structure already conveys this emphasis.
            base.bold = false;
            base.italic = false;
        }

        let segments = segments(rt, &base);
        let mut inline = text::inline_nodes(&segments);

        let hashtags: Vec<String> = rt
            .note_tags()
            .iter()
            .filter_map(|t| match map_tag(t) {
                TagMapping::Hashtag(tag) => Some(format!("#{tag}")),
                _ => None,
            })
            .collect();
        if !hashtags.is_empty() {
            inline.insert(0, Node::text(format!("{} ", hashtags.join(" "))));
        }

        if let Some(level) = heading {
            return vec![Node::heading(level, inline)];
        }
        if style_id == "code" || (!in_list && is_code_paragraph(&inline)) {
            return vec![code_block(&inline)];
        }
        if style_id == "blockquote" || style_id == "quote" {
            return vec![Node::element(kinds::BLOCKQUOTE, vec![Node::paragraph(inline)])];
        }
        if !in_list {
            let maths: Vec<&Node> = inline.iter().filter(|n| n.is(kinds::INLINE_MATH)).collect();
            let only_math = maths.len() == 1
                && inline.iter().all(|n| {
                    n.is(kinds::INLINE_MATH) || n.is_text() && n.text.as_deref().unwrap_or("").trim().is_empty()
                });
            if only_math {
                let latex = maths[0].attr_str("latex").unwrap_or("").to_string();
                return vec![Node::new(kinds::BLOCK_MATH).with_attr("latex", latex)];
            }
        }
        vec![Node::paragraph(inline)]
    }

    // -- tables ---------------------------------------------------------------

    fn table(&mut self, t: &Table) -> Option<Node> {
        let widths: Vec<f64> = t.col_widths().iter().map(|w| text::half_inch_to_px(*w)).collect();
        let mut rows = Vec::new();
        for row in t.contents() {
            let mut cells = Vec::new();
            for (ci, cell) in row.contents().iter().enumerate() {
                let elements: Vec<&OutlineElement> = cell.contents().iter().collect();
                let mut blocks = self.elements(&elements);
                if blocks.is_empty() {
                    blocks.push(Node::paragraph(vec![]));
                }
                cells.push(table_cell(blocks, widths.get(ci).copied()));
            }
            rows.push(cells);
        }
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return None;
        }
        let rows = rows
            .into_iter()
            .map(|mut cells| {
                while cells.len() < cols {
                    let w = widths.get(cells.len()).copied();
                    cells.push(table_cell(vec![Node::paragraph(vec![])], w));
                }
                Node::element(kinds::TABLE_ROW, cells)
            })
            .collect();
        Some(Node::element(kinds::TABLE, rows))
    }

    // -- images, files --------------------------------------------------------

    fn image(&mut self, img: &Image) -> Option<Node> {
        let ext = img.extension().map(|e| e.trim_start_matches('.').to_ascii_lowercase());
        // Browsers can't show EMF/WMF/TIFF; OneNote often keeps a web copy.
        let web = img.web_picture().filter(|_| matches!(ext.as_deref(), Some("emf" | "wmf" | "tif" | "tiff")));
        let (bytes, ext) = match web {
            Some(pic) => {
                (read_all(pic.read()), pic.extension().map(|e| e.trim_start_matches('.').to_ascii_lowercase()))
            }
            None => (img.read().map(read_all).unwrap_or_default(), ext),
        };
        if bytes.is_empty() {
            self.warn(format!("image data is missing ({:?})", img.data_status()));
            return None;
        }
        let name = img
            .image_filename()
            .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n).to_string())
            .unwrap_or_else(|| format!("image.{}", ext.as_deref().unwrap_or("png")));
        let mime = ext.as_deref().map(|e| mime_from_name(&format!("x.{e}")));
        let url = match self.sink.put(&bytes, Some(&name), mime) {
            Ok(url) => url,
            Err(e) => {
                self.warn(format!("could not store image: {e}"));
                return None;
            }
        };
        self.stats.assets += 1;
        let mut node = Node::new(kinds::IMAGE).with_attr("src", url);
        // OCR text OneNote extracted from the picture keeps it searchable.
        let alt = img.alt_text().filter(|a| !a.trim().is_empty()).or(img.text().filter(|t| !t.trim().is_empty()));
        if let Some(alt) = alt {
            let alt: String = alt.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(2000).collect();
            node = node.with_attr("alt", alt);
        }
        if let Some(w) = img.layout_max_width().or(img.picture_width()).filter(|w| *w > 0.0) {
            node = node.with_attr("width", text::half_inch_to_px(w));
        }
        Some(node)
    }

    fn file(&mut self, f: &EmbeddedFile) -> Option<Node> {
        let name = f.filename().rsplit(['/', '\\']).next().unwrap_or("attachment").to_string();
        let bytes = if f.data_status() == FileDataStatus::Available { read_all(f.read()) } else { Vec::new() };
        if bytes.is_empty() && f.size() > 0 {
            self.warn(format!("attachment {name:?} has no data"));
            return None;
        }
        let mime = mime_from_name(&name);
        let url = match self.sink.put(&bytes, Some(&name), Some(mime)) {
            Ok(url) => url,
            Err(e) => {
                self.warn(format!("could not store attachment {name:?}: {e}"));
                return None;
            }
        };
        self.stats.assets += 1;
        Some(
            Node::new(kinds::ATTACHMENT)
                .with_attr("src", url)
                .with_attr("name", name)
                .with_attr("size", bytes.len() as u64)
                .with_attr("mime", mime),
        )
    }

    // -- ink ------------------------------------------------------------------

    /// One `inkBlock` holding `strokes`, translated so the drawing starts at
    /// a small margin from the block's top-left corner.
    fn ink_block(&mut self, strokes: &[PxStroke]) -> Option<Node> {
        let [min_x, min_y, max_x, max_y] = bounds(strokes)?;
        let max_size = strokes.iter().map(|(s, _)| stroke_size(s)).fold(0f32, f32::max);
        let margin = max_size / 2.0 + 4.0;
        let converted: Vec<Stroke> = strokes
            .iter()
            .filter(|(_, pts)| !pts.is_empty())
            .map(|(s, pts)| {
                let transparency = s.transparency().unwrap_or(0);
                let highlighter = transparency >= 64 || s.pen_tip() == Some(1);
                Stroke {
                    id: new_id(),
                    tool: if highlighter { "highlighter" } else { "pen" }.into(),
                    color: s.color().map(text::colorref_hex).unwrap_or_else(|| "#000000".into()),
                    size: f64::from(stroke_size(s)),
                    points: pts
                        .iter()
                        .map(|p| [round1(p[0] - min_x + margin), round1(p[1] - min_y + margin), 0.5])
                        .collect(),
                }
            })
            .collect();
        let width = (max_x - min_x + 2.0 * margin).ceil();
        let height = (max_y - min_y + 2.0 * margin).ceil();
        self.stats.strokes += converted.len();
        let ink_id = new_id();
        self.ink.insert(ink_id.clone(), converted);
        Some(
            Node::new(kinds::INK_BLOCK)
                .with_attr("inkId", ink_id)
                .with_attr("width", f64::from(width))
                .with_attr("height", f64::from(height)),
        )
    }
}

/// A stroke with its points in page pixels.
type PxStroke<'a> = (&'a InkStroke, Vec<[f32; 2]>);

enum Item<'a> {
    Content(usize),
    Ink(Vec<PxStroke<'a>>),
}

/// Strokes of an ink object (and its nested groups) in pixels, offset by
/// the object's position on the page.
fn px_strokes<'a>(ink: &'a Ink, origin: (f32, f32), out: &mut Vec<PxStroke<'a>>) {
    for s in ink.ink_strokes() {
        let pts: Vec<[f32; 2]> = s
            .absolute_path()
            .iter()
            .map(|p| [origin.0 + p.x() / INK_UNITS_PER_PX, origin.1 + p.y() / INK_UNITS_PER_PX])
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .collect();
        if !pts.is_empty() {
            out.push((s, pts));
        }
    }
    for child in ink.child_groups() {
        let o = match (child.offset_horizontal(), child.offset_vertical()) {
            (None, None) => origin,
            (x, y) => (x.map_or(origin.0, |v| v * 48.0), y.map_or(origin.1, |v| v * 48.0)),
        };
        px_strokes(child, o, out);
    }
}

fn bounds(strokes: &[PxStroke]) -> Option<[f32; 4]> {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (_, pts) in strokes {
        for p in pts {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
    }
    (b[0] <= b[2]).then_some(b)
}

/// Merge ink objects whose bounds touch (within a small gap) into drawings:
/// single-linkage clustering with a union-find.
fn cluster_ink<T>(inks: Vec<(Vec<T>, [f32; 4])>) -> Vec<(Vec<T>, [f32; 4])> {
    const GAP: f32 = 24.0;
    let near = |a: &[f32; 4], b: &[f32; 4]| {
        a[0] - GAP <= b[2] && b[0] - GAP <= a[2] && a[1] - GAP <= b[3] && b[1] - GAP <= a[3]
    };
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..inks.len()).collect();
    for i in 0..inks.len() {
        for j in i + 1..inks.len() {
            if near(&inks[i].1, &inks[j].1) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                if a != b {
                    parent[b] = a;
                }
            }
        }
    }
    let mut groups: Vec<(usize, Vec<T>, [f32; 4])> = Vec::new();
    for (i, (strokes, b)) in inks.into_iter().enumerate() {
        let r = root(&mut parent, i);
        match groups.iter_mut().find(|g| g.0 == r) {
            Some(g) => {
                g.1.extend(strokes);
                g.2 = [g.2[0].min(b[0]), g.2[1].min(b[1]), g.2[2].max(b[2]), g.2[3].max(b[3])];
            }
            None => groups.push((r, strokes, b)),
        }
    }
    groups.into_iter().map(|(_, s, b)| (s, b)).collect()
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

fn stroke_size(s: &InkStroke) -> f32 {
    (s.width().max(s.height()) / INK_UNITS_PER_PX).clamp(0.5, 64.0)
}

fn flatten<'a>(items: &'a [OutlineItem], out: &mut Vec<&'a OutlineElement>) {
    for item in items {
        match item {
            OutlineItem::Element(e) => out.push(e),
            OutlineItem::Group(g) => flatten(g.outlines(), out),
        }
    }
}

fn map_tag(tag: &onenote_parser::contents::NoteTag) -> TagMapping {
    match tag.definition() {
        Some(def) => text::map_note_tag(def.label(), def.shape().is_checkable(), tag.item_status().completed()),
        None => TagMapping::Ignore,
    }
}

/// Split a paragraph into styled segments: at run boundaries and at the
/// edges of hyperlinks, with the equation object of each math run attached.
fn segments(rt: &RichText, base: &RunStyle) -> Vec<Segment> {
    let styles = rt.text_run_formatting();
    let links: Vec<(u32, u32, String)> =
        rt.hyperlinks().into_iter().map(|h| (h.start(), h.end(), h.target().to_string())).collect();

    // Math objects belong, in order, to the math-formatted runs.
    let mut math_objs = rt.math_inline_objects().iter();
    let math: HashMap<usize, MathObj> = styles
        .iter()
        .enumerate()
        .filter(|(_, s)| s.math_formatting())
        .filter_map(|(i, _)| math_objs.next().map(|m| (i, MathObj::from_parser(m))))
        .collect();

    text::split_with_links(rt.text(), rt.text_run_indices(), styles.len(), &links)
        .into_iter()
        .map(|(t, style_idx, link)| {
            let style = match style_idx {
                Some(i) => RunStyle::from_styling(&styles[i]).inherit(base),
                None => base.clone(),
            };
            Segment { text: t, style, link, math: style_idx.and_then(|i| math.get(&i).copied()) }
        })
        .collect()
}

fn is_code_paragraph(inline: &[Node]) -> bool {
    let texts: Vec<&Node> = inline.iter().filter(|n| n.is_text()).collect();
    !texts.is_empty()
        && inline.iter().all(|n| n.is_text() || n.is(kinds::HARD_BREAK))
        && texts.iter().all(|n| n.has_mark(marks::CODE) || n.text.as_deref().unwrap_or("").trim().is_empty())
        && texts.iter().any(|n| n.has_mark(marks::CODE))
}

fn code_block(inline: &[Node]) -> Node {
    let mut code = String::new();
    for n in inline {
        if n.is(kinds::HARD_BREAK) {
            code.push('\n');
        } else {
            code.push_str(&n.plain_text());
        }
    }
    let content = if code.is_empty() { vec![] } else { vec![Node::text(code)] };
    Node::element(kinds::CODE_BLOCK, content)
}

fn table_cell(blocks: Vec<Node>, width: Option<f64>) -> Node {
    Node::element(kinds::TABLE_CELL, blocks)
        .with_attr("colspan", 1)
        .with_attr("rowspan", 1)
        .with_attr("colwidth", width.filter(|w| *w > 0.0).map_or(Value::Null, |w| Value::Array(vec![w.into()])))
}

fn list_node((kind, items): (ListKind, Vec<Node>)) -> Node {
    match kind {
        ListKind::Bullet => Node::element(kinds::BULLET_LIST, items),
        ListKind::Task(_) => Node::element(kinds::TASK_LIST, items),
        ListKind::Ordered(start) => {
            let node = Node::element(kinds::ORDERED_LIST, items);
            match start.filter(|s| *s > 1) {
                Some(s) => node.with_attr("start", s),
                None => node,
            }
        }
    }
}

/// Consecutive code lines become one code block; consecutive quote
/// paragraphs become one quote.
fn merge_adjacent(blocks: &mut Vec<Node>) {
    let mut out: Vec<Node> = Vec::with_capacity(blocks.len());
    for block in std::mem::take(blocks) {
        if let Some(prev) = out.last_mut() {
            if prev.is(kinds::CODE_BLOCK)
                && block.is(kinds::CODE_BLOCK)
                && prev.attr("language") == block.attr("language")
            {
                let mut code = prev.plain_text();
                code.push('\n');
                code.push_str(&block.plain_text());
                prev.content = vec![Node::text(code)];
                continue;
            }
            if prev.is(kinds::BLOCKQUOTE) && block.is(kinds::BLOCKQUOTE) {
                prev.content.extend(block.content);
                continue;
            }
        }
        out.push(block);
    }
    *blocks = out;
}

fn read_all(mut r: Box<dyn Read>) -> Vec<u8> {
    let mut buf = Vec::new();
    // A truncated read still yields the bytes we got.
    let _ = r.read_to_end(&mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_code_lines_and_quotes() {
        let code = |t: &str| Node::element(kinds::CODE_BLOCK, vec![Node::text(t)]);
        let quote = |t: &str| Node::element(kinds::BLOCKQUOTE, vec![Node::paragraph(vec![Node::text(t)])]);
        let mut blocks = vec![code("a"), code("b"), Node::paragraph(vec![]), quote("x"), quote("y")];
        merge_adjacent(&mut blocks);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].plain_text(), "a\nb");
        assert_eq!(blocks[2].content.len(), 2);
    }

    #[test]
    fn code_paragraph_detection() {
        let code = |t: &str| Node::styled_text(t, vec![anynote_core::Mark::new(marks::CODE)]);
        assert!(is_code_paragraph(&[code("let x = 1;"), Node::text("  ")]));
        assert!(!is_code_paragraph(&[code("x"), Node::text(" is a variable")]));
        assert_eq!(code_block(&[code("a"), Node::new(kinds::HARD_BREAK), code("b")]).plain_text(), "a\nb");
    }

    #[test]
    fn nearby_ink_objects_cluster_into_drawings() {
        let inks = vec![
            (vec!["a"], [0.0, 0.0, 10.0, 10.0]),
            (vec!["far"], [500.0, 500.0, 510.0, 510.0]),
            (vec!["b"], [30.0, 0.0, 40.0, 10.0]), // within the gap of "a"
            (vec!["c"], [60.0, 5.0, 70.0, 20.0]), // chained through "b"
        ];
        let mut clusters = cluster_ink(inks);
        clusters.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].0, vec!["a", "b", "c"]);
        assert_eq!(clusters[0].1, [0.0, 0.0, 70.0, 20.0]);
    }

    #[test]
    fn lists_group_by_kind() {
        let item = |t: &str| Node::element(kinds::LIST_ITEM, vec![Node::paragraph(vec![Node::text(t)])]);
        let n = list_node((ListKind::Ordered(Some(3)), vec![item("a")]));
        assert_eq!(n.attr_i64("start"), Some(3));
        assert!(ListKind::Ordered(None).same_list(ListKind::Ordered(Some(2))));
        assert!(!ListKind::Bullet.same_list(ListKind::Task(false)));
    }
}
