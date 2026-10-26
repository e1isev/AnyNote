//! Markdown conversion for the open folder format.
//!
//! Export produces readable GitHub-flavoured Markdown with a few extensions
//! (`$math$`, `[[wiki links]]`, `<mark>`/`<u>`). Import parses CommonMark +
//! GFM into a schema-valid block tree. Markdown alone cannot hold everything
//! (ink, block IDs, link targets), so the exporter also writes a JSON sidecar
//! that the importer prefers when present.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use serde_json::Value;

use crate::node::{kinds, marks, Mark, Node};

/// Hooks that let the caller rewrite URLs during export.
pub trait ExportContext {
    /// Map a document URL (e.g. `anynote://asset/<hash>`) to a link in the output.
    fn url(&mut self, url: &str) -> String {
        url.to_string()
    }
    /// Markdown for an ink block (typically an image link to an exported SVG).
    fn ink(&mut self, ink_id: &str) -> Option<String> {
        let _ = ink_id;
        None
    }
}

/// Export context that leaves URLs untouched.
pub struct PlainExport;
impl ExportContext for PlainExport {}

pub fn to_markdown(doc: &Node, ctx: &mut dyn ExportContext) -> String {
    let mut w = Writer { out: String::new(), ctx };
    if doc.is_canvas() {
        w.blocks(&canvas_reading_order(doc), "");
        // The page-wide drawing comes after the text.
        if let Some(md) = w.ctx.ink(crate::node::CANVAS_INK) {
            w.out.push('\n');
            w.line("", &md);
        }
    } else {
        w.blocks(&doc.content, "");
    }
    let mut out = w.out.trim_end().to_string();
    out.push('\n');
    out
}

/// A canvas page's blocks in reading order: boxes top to bottom (boxes
/// within 24 px of each other count as one row, read left to right), empty
/// boxes skipped.
pub fn canvas_reading_order(doc: &Node) -> Vec<Node> {
    let pos = |n: &Node, k: &str| n.attr(k).and_then(Value::as_f64).unwrap_or(0.0);
    let mut boxes: Vec<&Node> = doc
        .content
        .iter()
        .filter(|n| n.is(kinds::CANVAS_BOX))
        .filter(|b| b.content.iter().any(|c| !c.is(kinds::PARAGRAPH) || !c.content.is_empty()))
        .collect();
    boxes.sort_by(|a, b| {
        let (ra, rb) = ((pos(a, "y") / 24.0).floor(), (pos(b, "y") / 24.0).floor());
        ra.total_cmp(&rb).then(pos(a, "x").total_cmp(&pos(b, "x")))
    });
    let mut blocks: Vec<Node> = boxes.into_iter().flat_map(|b| b.content.iter().cloned()).collect();
    blocks.extend(doc.content.iter().filter(|n| !n.is(kinds::CANVAS_BOX)).cloned());
    blocks
}

struct Writer<'a> {
    out: String,
    ctx: &'a mut dyn ExportContext,
}

impl Writer<'_> {
    /// Write a sequence of blocks; `prefix` is prepended to every line.
    fn blocks(&mut self, blocks: &[Node], prefix: &str) {
        for (i, block) in blocks.iter().enumerate() {
            if i > 0 {
                self.out.push_str(prefix.trim_end());
                self.out.push('\n');
            }
            self.block(block, prefix);
        }
    }

    fn line(&mut self, prefix: &str, text: &str) {
        for (i, l) in text.split('\n').enumerate() {
            if i > 0 || !self.out.is_empty() && !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            let p = if l.is_empty() { prefix.trim_end() } else { prefix };
            self.out.push_str(p);
            self.out.push_str(l);
        }
        self.out.push('\n');
    }

    fn block(&mut self, node: &Node, prefix: &str) {
        match node.kind.as_str() {
            kinds::PARAGRAPH => {
                let text = self.inlines(&node.content);
                self.line(prefix, &text);
            }
            kinds::HEADING => {
                let level = node.attr_i64("level").unwrap_or(1).clamp(1, 6) as usize;
                let text = self.inlines(&node.content).replace('\n', " ");
                self.line(prefix, &format!("{} {}", "#".repeat(level), text));
            }
            kinds::BULLET_LIST | kinds::ORDERED_LIST | kinds::TASK_LIST => {
                let start = node.attr_i64("start").unwrap_or(1);
                for (i, item) in node.content.iter().enumerate() {
                    let marker = match node.kind.as_str() {
                        kinds::ORDERED_LIST => format!("{}. ", start + i as i64),
                        kinds::TASK_LIST => {
                            format!("- [{}] ", if item.attr_bool("checked") { "x" } else { " " })
                        }
                        _ => "- ".to_string(),
                    };
                    self.list_item(item, prefix, &marker);
                }
            }
            kinds::CODE_BLOCK => {
                let lang = node.attr_str("language").unwrap_or("");
                let code: String = node.content.iter().filter_map(|n| n.text.as_deref()).collect();
                let fence = if code.contains("```") { "~~~~" } else { "```" };
                self.line(prefix, &format!("{fence}{lang}\n{code}\n{fence}"));
            }
            kinds::BLOCKQUOTE => {
                let inner = format!("{prefix}> ");
                self.blocks(&node.content, &inner);
            }
            kinds::HORIZONTAL_RULE => self.line(prefix, "---"),
            kinds::IMAGE => {
                let src = node.attr_str("src").unwrap_or("");
                let url = self.ctx.url(src);
                let alt = escape(node.attr_str("alt").unwrap_or(""));
                self.line(prefix, &format!("![{alt}]({})", link_dest(&url)));
            }
            kinds::BLOCK_MATH => {
                let latex = node.attr_str("latex").unwrap_or("");
                self.line(prefix, &format!("$$\n{latex}\n$$"));
            }
            kinds::INK_BLOCK => {
                let ink_id = node.attr_str("inkId").unwrap_or("");
                let md = self.ctx.ink(ink_id).unwrap_or_else(|| "*(ink drawing)*".to_string());
                self.line(prefix, &md);
            }
            kinds::ATTACHMENT => {
                let url = self.ctx.url(node.attr_str("src").unwrap_or(""));
                let name = escape(node.attr_str("name").unwrap_or("attachment"));
                self.line(prefix, &format!("📎 [{name}]({})", link_dest(&url)));
            }
            kinds::TABLE => self.table(node, prefix),
            _ => {
                // Unknown block: keep its text rather than drop it.
                if node.content.iter().any(Node::is_inline) {
                    let text = self.inlines(&node.content);
                    self.line(prefix, &text);
                } else {
                    self.blocks(&node.content, prefix);
                }
            }
        }
    }

    fn list_item(&mut self, item: &Node, prefix: &str, marker: &str) {
        let indent = " ".repeat(marker.chars().count());
        let mut first = true;
        for (i, child) in item.content.iter().enumerate() {
            if first && child.is(kinds::PARAGRAPH) {
                let text = self.inlines(&child.content);
                let mut lines = text.split('\n');
                self.line(prefix, &format!("{marker}{}", lines.next().unwrap_or("")));
                for l in lines {
                    self.line(&format!("{prefix}{indent}"), l);
                }
            } else {
                if first {
                    self.line(prefix, marker.trim_end());
                }
                // Nested lists stay tight; other blocks need a blank line.
                if i > 0 && !matches!(child.kind.as_str(), kinds::BULLET_LIST | kinds::ORDERED_LIST | kinds::TASK_LIST)
                {
                    self.out.push('\n');
                }
                self.block(child, &format!("{prefix}{indent}"));
            }
            first = false;
        }
        if item.content.is_empty() {
            self.line(prefix, marker.trim_end());
        }
    }

    fn table(&mut self, table: &Node, prefix: &str) {
        let rows: Vec<Vec<String>> = table
            .content
            .iter()
            .map(|row| {
                row.content
                    .iter()
                    .map(|cell| {
                        let parts: Vec<String> = cell
                            .content
                            .iter()
                            .map(|b| {
                                if b.content.iter().any(Node::is_inline) {
                                    self.inlines(&b.content)
                                } else {
                                    escape(&b.plain_text())
                                }
                            })
                            .collect();
                        parts.join("<br>").replace('\n', "<br>").replace('|', "\\|")
                    })
                    .collect()
            })
            .collect();
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let fmt_row = |cells: &[String]| {
            let mut s = String::from("|");
            for c in 0..cols {
                s.push(' ');
                s.push_str(cells.get(c).map(String::as_str).unwrap_or(""));
                s.push_str(" |");
            }
            s
        };
        let header = rows.first().cloned().unwrap_or_default();
        let mut text = fmt_row(&header);
        text.push('\n');
        text.push('|');
        text.push_str(&" --- |".repeat(cols));
        for row in rows.iter().skip(1) {
            text.push('\n');
            text.push_str(&fmt_row(row));
        }
        self.line(prefix, &text);
    }

    fn inlines(&mut self, nodes: &[Node]) -> String {
        let mut out = String::new();
        let mut stack: Vec<Mark> = Vec::new();
        let mut pending_ws = String::new();
        for node in nodes {
            let target = if node.is_text() { sorted_marks(&node.marks) } else { Vec::new() };
            let keep = stack.iter().zip(&target).take_while(|(a, b)| a == b).count();
            while stack.len() > keep {
                let m = stack.pop().unwrap();
                out.push_str(&close_mark(&m, &mut *self.ctx));
            }
            out.push_str(&pending_ws);
            pending_ws.clear();

            match node.kind.as_str() {
                kinds::TEXT => {
                    let text = node.text.as_deref().unwrap_or("");
                    let in_code = target.iter().any(|m| m.kind == marks::CODE);
                    let opening = &target[keep..];
                    let (lead, core, trail) = split_ws(text);
                    if core.is_empty() {
                        out.push_str(text);
                        continue;
                    }
                    if !opening.is_empty() {
                        out.push_str(lead);
                    }
                    for m in opening {
                        out.push_str(&open_mark(m, core));
                        stack.push(m.clone());
                    }
                    let body = if opening.is_empty() { text.trim_end() } else { core };
                    out.push_str(&if in_code { body.to_string() } else { escape(body) });
                    pending_ws.push_str(trail);
                }
                kinds::HARD_BREAK => out.push_str("\\\n"),
                kinds::INLINE_MATH => {
                    out.push('$');
                    out.push_str(node.attr_str("latex").unwrap_or(""));
                    out.push('$');
                }
                kinds::WIKI_LINK => {
                    out.push_str("[[");
                    out.push_str(&node.attr_str("title").unwrap_or("").replace(']', ""));
                    out.push_str("]]");
                }
                kinds::IMAGE => {
                    let url = self.ctx.url(node.attr_str("src").unwrap_or(""));
                    out.push_str(&format!("![{}]({})", escape(node.attr_str("alt").unwrap_or("")), link_dest(&url)));
                }
                _ => out.push_str(&escape(&node.plain_text())),
            }
        }
        while let Some(m) = stack.pop() {
            out.push_str(&close_mark(&m, &mut *self.ctx));
        }
        out.push_str(&pending_ws);
        out
    }
}

fn split_ws(s: &str) -> (&str, &str, &str) {
    let core_start = s.len() - s.trim_start().len();
    let core_end = s.trim_end().len().max(core_start);
    (&s[..core_start], &s[core_start..core_end], &s[core_end..])
}

/// Outermost first: links wrap emphasis, code is innermost.
fn sorted_marks(marks: &[Mark]) -> Vec<Mark> {
    let rank = |m: &Mark| match m.kind.as_str() {
        marks::LINK => 0,
        marks::BOLD => 1,
        marks::ITALIC => 2,
        marks::STRIKE => 3,
        marks::UNDERLINE => 4,
        marks::HIGHLIGHT => 5,
        marks::SUBSCRIPT | marks::SUPERSCRIPT => 6,
        marks::CODE => 9,
        _ => 8,
    };
    let mut v = marks.to_vec();
    v.sort_by_key(rank);
    v
}

fn open_mark(m: &Mark, text: &str) -> String {
    match m.kind.as_str() {
        marks::BOLD => "**".into(),
        marks::ITALIC => "*".into(),
        marks::STRIKE => "~~".into(),
        marks::UNDERLINE => "<u>".into(),
        marks::HIGHLIGHT => "<mark>".into(),
        marks::SUBSCRIPT => "<sub>".into(),
        marks::SUPERSCRIPT => "<sup>".into(),
        marks::CODE => {
            if text.contains('`') {
                "`` ".into()
            } else {
                "`".into()
            }
        }
        marks::LINK => "[".into(),
        _ => String::new(),
    }
}

fn close_mark(m: &Mark, ctx: &mut dyn ExportContext) -> String {
    match m.kind.as_str() {
        marks::BOLD => "**".into(),
        marks::ITALIC => "*".into(),
        marks::STRIKE => "~~".into(),
        marks::UNDERLINE => "</u>".into(),
        marks::HIGHLIGHT => "</mark>".into(),
        marks::SUBSCRIPT => "</sub>".into(),
        marks::SUPERSCRIPT => "</sup>".into(),
        // A code span opened with "`` " is closed with " ``"; a lone backtick
        // would end it early, so both forms are accepted by CommonMark.
        marks::CODE => "`".into(),
        marks::LINK => {
            let href = ctx.url(m.attr_str("href").unwrap_or(""));
            format!("]({})", link_dest(&href))
        }
        _ => String::new(),
    }
}

fn link_dest(url: &str) -> String {
    if url.contains([' ', '(', ')', '<', '>']) {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_string()
    }
}

/// Escape characters that would otherwise start Markdown syntax.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.chars().enumerate() {
        match c {
            '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '$' | '|' => {
                out.push('\\');
                out.push(c);
            }
            '#' | '-' | '+' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            '~' if text[text.char_indices().nth(i).map(|(b, _)| b).unwrap_or(0)..].starts_with("~~") => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    // "1. text" at the start of a paragraph would become a list.
    if let Some(dot) = out.find(". ") {
        if dot > 0 && out[..dot].chars().all(|c| c.is_ascii_digit()) {
            out.insert(dot, '\\');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// import

/// Parse Markdown into a `doc` node. `resolve` maps link and image targets
/// (e.g. relative file paths) to document URLs; return `None` to keep them.
pub fn from_markdown(source: &str, resolve: &mut dyn FnMut(&str) -> Option<String>) -> Node {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_MATH);
    opts.insert(Options::ENABLE_WIKILINKS);
    opts.insert(Options::ENABLE_GFM);

    let mut b = Builder::new();
    let mut in_table_head = false;
    let mut image: Option<(String, String)> = None; // (src, alt)
    let mut wiki: Option<(String, String)> = None; // (target, label)
    let mut code_lang: Option<String> = None;

    for event in Parser::new_ext(source, opts) {
        if let Some((_, alt)) = image.as_mut() {
            match &event {
                Event::Text(t) | Event::Code(t) => {
                    alt.push_str(t);
                    continue;
                }
                Event::End(TagEnd::Image) => {}
                _ => continue,
            }
        }
        if let Some((_, label)) = wiki.as_mut() {
            match &event {
                Event::Text(t) | Event::Code(t) => {
                    label.push_str(t);
                    continue;
                }
                Event::End(TagEnd::Link) => {}
                _ => continue,
            }
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => b.open(Node::new(kinds::PARAGRAPH)),
                Tag::Heading { level, .. } => b.open(Node::heading(heading_level(level), vec![])),
                Tag::BlockQuote(_) => b.open(Node::new(kinds::BLOCKQUOTE)),
                Tag::CodeBlock(kind) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    let mut node = Node::new(kinds::CODE_BLOCK);
                    if !lang.is_empty() {
                        node = node.with_attr("language", lang.clone());
                    }
                    code_lang = Some(lang);
                    b.open(node);
                }
                Tag::List(Some(start)) => {
                    let mut node = Node::new(kinds::ORDERED_LIST);
                    if start != 1 {
                        node = node.with_attr("start", start);
                    }
                    b.open(node)
                }
                Tag::List(None) => b.open(Node::new(kinds::BULLET_LIST)),
                Tag::Item => b.open(Node::new(kinds::LIST_ITEM)),
                Tag::Table(_) => b.open(Node::new(kinds::TABLE)),
                Tag::TableHead => {
                    in_table_head = true;
                    b.open(Node::new(kinds::TABLE_ROW))
                }
                Tag::TableRow => b.open(Node::new(kinds::TABLE_ROW)),
                Tag::TableCell => {
                    b.open(Node::new(if in_table_head { kinds::TABLE_HEADER } else { kinds::TABLE_CELL }))
                }
                Tag::Emphasis => b.marks.push(Mark::new(marks::ITALIC)),
                Tag::Strong => b.marks.push(Mark::new(marks::BOLD)),
                Tag::Strikethrough => b.marks.push(Mark::new(marks::STRIKE)),
                Tag::Superscript => b.marks.push(Mark::new(marks::SUPERSCRIPT)),
                Tag::Subscript => b.marks.push(Mark::new(marks::SUBSCRIPT)),
                Tag::Link { link_type: LinkType::WikiLink { .. }, dest_url, .. } => {
                    wiki = Some((dest_url.to_string(), String::new()));
                }
                Tag::Link { dest_url, .. } => {
                    let href = resolve(&dest_url).unwrap_or_else(|| dest_url.to_string());
                    b.marks.push(Mark::new(marks::LINK).with_attr("href", href));
                }
                Tag::Image { dest_url, .. } => {
                    let src = resolve(&dest_url).unwrap_or_else(|| dest_url.to_string());
                    image = Some((src, String::new()));
                }
                Tag::HtmlBlock => b.open(Node::new(kinds::PARAGRAPH)),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::Table
                | TagEnd::TableRow
                | TagEnd::TableCell
                | TagEnd::HtmlBlock => b.close(),
                TagEnd::TableHead => {
                    in_table_head = false;
                    b.close()
                }
                TagEnd::CodeBlock => {
                    code_lang = None;
                    b.close()
                }
                TagEnd::Emphasis => b.pop_mark(marks::ITALIC),
                TagEnd::Strong => b.pop_mark(marks::BOLD),
                TagEnd::Strikethrough => b.pop_mark(marks::STRIKE),
                TagEnd::Superscript => b.pop_mark(marks::SUPERSCRIPT),
                TagEnd::Subscript => b.pop_mark(marks::SUBSCRIPT),
                TagEnd::Link => {
                    if let Some((target, label)) = wiki.take() {
                        let title = if label.is_empty() { target.clone() } else { label };
                        b.inline(Node::new(kinds::WIKI_LINK).with_attr("title", title).with_attr("target", target));
                    } else {
                        b.pop_mark(marks::LINK)
                    }
                }
                TagEnd::Image => {
                    if let Some((src, alt)) = image.take() {
                        let mut node = Node::new(kinds::IMAGE).with_attr("src", src);
                        if !alt.is_empty() {
                            node = node.with_attr("alt", alt);
                        }
                        b.inline(node);
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if code_lang.is_some() {
                    b.raw_text(&text);
                } else {
                    b.text(&text);
                }
            }
            Event::Code(code) => {
                b.marks.push(Mark::new(marks::CODE));
                b.text(&code);
                b.pop_mark(marks::CODE);
            }
            Event::InlineMath(latex) => b.inline(Node::new(kinds::INLINE_MATH).with_attr("latex", latex.to_string())),
            Event::DisplayMath(latex) => {
                b.inline(Node::new(kinds::BLOCK_MATH).with_attr("latex", latex.trim().to_string()))
            }
            Event::Html(html) | Event::InlineHtml(html) => b.html(&html),
            Event::SoftBreak => b.text(" "),
            Event::HardBreak => b.inline(Node::new(kinds::HARD_BREAK)),
            Event::Rule => {
                b.open(Node::new(kinds::HORIZONTAL_RULE));
                b.close();
            }
            Event::TaskListMarker(checked) => b.task_marker(checked),
            Event::FootnoteReference(name) => b.text(&format!("[^{name}]")),
        }
    }
    b.finish()
}

fn heading_level(level: HeadingLevel) -> u8 {
    level as u8
}

struct Builder {
    /// Open nodes; `stack[0]` is the document. The flag marks paragraphs the
    /// builder opened itself to hold loose inline content.
    stack: Vec<(Node, bool)>,
    marks: Vec<Mark>,
}

/// Nodes whose content is blocks; inline content inside them gets wrapped.
fn holds_blocks(kind: &str) -> bool {
    matches!(
        kind,
        kinds::DOC | kinds::BLOCKQUOTE | kinds::LIST_ITEM | kinds::TASK_ITEM | kinds::TABLE_CELL | kinds::TABLE_HEADER
    )
}

/// Block-level nodes that may not stay inside a paragraph.
fn is_block_leaf(kind: &str) -> bool {
    matches!(kind, kinds::IMAGE | kinds::BLOCK_MATH)
}

impl Builder {
    fn new() -> Self {
        Builder { stack: vec![(Node::doc(vec![]), false)], marks: Vec::new() }
    }

    fn top(&mut self) -> &mut Node {
        &mut self.stack.last_mut().expect("document frame").0
    }

    fn open(&mut self, node: Node) {
        self.close_auto();
        self.stack.push((node, false));
    }

    /// Close the implicit paragraph, if one is open.
    fn close_auto(&mut self) {
        if self.stack.len() > 1 && self.stack.last().is_some_and(|(_, auto)| *auto) {
            let (node, _) = self.stack.pop().unwrap();
            self.attach(node);
        }
    }

    fn close(&mut self) {
        self.close_auto();
        if self.stack.len() > 1 {
            let (node, _) = self.stack.pop().unwrap();
            self.attach(node);
        }
    }

    fn attach(&mut self, mut node: Node) {
        node.normalize_text();
        let mut out = Vec::new();
        match node.kind.as_str() {
            kinds::PARAGRAPH | kinds::HEADING => {
                // Hoist images and display math out of text blocks.
                let shell = node_shell(&node);
                let mut current = shell.clone();
                for child in node.content {
                    if is_block_leaf(&child.kind) {
                        if current
                            .content
                            .iter()
                            .any(|n| !n.is_text() || !n.text.as_deref().unwrap_or("").trim().is_empty())
                        {
                            out.push(std::mem::replace(&mut current, shell.clone()));
                        } else {
                            current.content.clear();
                        }
                        out.push(child);
                    } else {
                        current.content.push(child);
                    }
                }
                trim_inline_edges(&mut current.content);
                if !current.content.is_empty() || out.is_empty() {
                    out.push(current);
                }
            }
            kinds::CODE_BLOCK => {
                if let Some(last) = node.content.last_mut() {
                    if let Some(t) = last.text.as_mut() {
                        while t.ends_with('\n') {
                            t.pop();
                        }
                    }
                }
                node.normalize_text();
                out.push(node);
            }
            kinds::LIST_ITEM | kinds::TASK_ITEM => {
                if !node.content.first().is_some_and(|n| n.is(kinds::PARAGRAPH)) {
                    node.content.insert(0, Node::paragraph(vec![]));
                }
                out.push(node);
            }
            kinds::BLOCKQUOTE | kinds::TABLE_CELL | kinds::TABLE_HEADER => {
                if node.content.is_empty() {
                    node.content.push(Node::paragraph(vec![]));
                }
                out.push(node);
            }
            kinds::TASK_LIST => {
                for item in &mut node.content {
                    if item.is(kinds::LIST_ITEM) {
                        item.kind = kinds::TASK_ITEM.into();
                        item.attrs.insert("checked".into(), Value::Bool(false));
                    }
                }
                out.push(node);
            }
            kinds::BULLET_LIST | kinds::ORDERED_LIST | kinds::TABLE | kinds::TABLE_ROW => {
                if !node.content.is_empty() {
                    out.push(node);
                }
            }
            _ => out.push(node),
        }
        self.top().content.extend(out);
    }

    fn ensure_textblock(&mut self) {
        let kind = self.top().kind.clone();
        if holds_blocks(&kind) {
            self.stack.push((Node::new(kinds::PARAGRAPH), true));
        }
    }

    fn inline(&mut self, node: Node) {
        self.ensure_textblock();
        self.top().content.push(node);
    }

    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let marks = self.marks.clone();
        self.inline(Node::styled_text(text, marks));
    }

    fn raw_text(&mut self, text: &str) {
        self.top().content.push(Node::text(text));
    }

    fn pop_mark(&mut self, kind: &str) {
        if let Some(i) = self.marks.iter().rposition(|m| m.kind == kind) {
            self.marks.remove(i);
        }
    }

    fn html(&mut self, html: &str) {
        let tag = html.trim().to_ascii_lowercase();
        let simple = |name: &str| (format!("<{name}>"), format!("</{name}>"));
        for (name, mark) in [
            ("u", marks::UNDERLINE),
            ("mark", marks::HIGHLIGHT),
            ("sub", marks::SUBSCRIPT),
            ("sup", marks::SUPERSCRIPT),
        ] {
            let (open, close) = simple(name);
            if tag == open {
                self.marks.push(Mark::new(mark));
                return;
            }
            if tag == close {
                self.pop_mark(mark);
                return;
            }
        }
        if matches!(tag.as_str(), "<br>" | "<br/>" | "<br />") {
            self.inline(Node::new(kinds::HARD_BREAK));
            return;
        }
        if tag.starts_with("<!--") {
            return;
        }
        // Anything else is kept as literal text.
        self.text(html.trim_end_matches('\n'));
    }

    fn task_marker(&mut self, checked: bool) {
        // The marker arrives inside the item (possibly inside its paragraph).
        let n = self.stack.len();
        for i in (1..n).rev() {
            if self.stack[i].0.is(kinds::LIST_ITEM) {
                self.stack[i].0.kind = kinds::TASK_ITEM.into();
                self.stack[i].0.attrs.insert("checked".into(), Value::Bool(checked));
                if self.stack[i - 1].0.is(kinds::BULLET_LIST) || self.stack[i - 1].0.is(kinds::ORDERED_LIST) {
                    self.stack[i - 1].0.kind = kinds::TASK_LIST.into();
                    self.stack[i - 1].0.attrs.remove("start");
                }
                break;
            }
        }
    }

    fn finish(mut self) -> Node {
        while self.stack.len() > 1 {
            self.close();
        }
        let mut doc = self.stack.pop().unwrap().0;
        // A task list may hold items parsed before the first marker.
        doc.walk_mut(&mut |n| {
            if n.is(kinds::TASK_LIST) {
                for item in &mut n.content {
                    if item.is(kinds::LIST_ITEM) {
                        item.kind = kinds::TASK_ITEM.into();
                        item.attrs.insert("checked".into(), Value::Bool(false));
                    }
                }
            }
        });
        if doc.content.is_empty() {
            doc.content.push(Node::paragraph(vec![]));
        }
        doc
    }
}

fn node_shell(node: &Node) -> Node {
    Node { kind: node.kind.clone(), attrs: node.attrs.clone(), ..Default::default() }
}

fn trim_inline_edges(content: &mut Vec<Node>) {
    if let Some(first) = content.first_mut() {
        if let Some(t) = first.text.as_mut() {
            *t = t.trim_start().to_string();
        }
    }
    if let Some(last) = content.last_mut() {
        if let Some(t) = last.text.as_mut() {
            *t = t.trim_end().to_string();
        }
    }
    content.retain(|n| !n.is_text() || !n.text.as_deref().unwrap_or("").is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(doc: &Node) -> String {
        to_markdown(doc, &mut PlainExport)
    }

    fn parse(s: &str) -> Node {
        from_markdown(s, &mut |_| None)
    }

    #[test]
    fn exports_common_blocks() {
        let doc = Node::doc(vec![
            Node::heading(2, vec![Node::text("Plan")]),
            Node::paragraph(vec![
                Node::text("Some "),
                Node::styled_text("bold ", vec![Mark::new("bold")]),
                Node::styled_text("both", vec![Mark::new("bold"), Mark::new("italic")]),
                Node::text(" and "),
                Node::styled_text("code_x", vec![Mark::new("code")]),
                Node::text(" 5*3"),
            ]),
            Node::element(
                kinds::BULLET_LIST,
                vec![
                    Node::element(kinds::LIST_ITEM, vec![Node::paragraph(vec![Node::text("one")])]),
                    Node::element(
                        kinds::LIST_ITEM,
                        vec![
                            Node::paragraph(vec![Node::text("two")]),
                            Node::element(
                                kinds::ORDERED_LIST,
                                vec![Node::element(
                                    kinds::LIST_ITEM,
                                    vec![Node::paragraph(vec![Node::text("nested")])],
                                )],
                            ),
                        ],
                    ),
                ],
            ),
            Node::element(
                kinds::TASK_LIST,
                vec![Node::element(kinds::TASK_ITEM, vec![Node::paragraph(vec![Node::text("done")])])
                    .with_attr("checked", true)],
            ),
            Node::element(kinds::CODE_BLOCK, vec![Node::text("fn main() {}")]).with_attr("language", "rust"),
            Node::new(kinds::BLOCK_MATH).with_attr("latex", "E = mc^2"),
        ]);
        let out = md(&doc);
        assert_eq!(
            out,
            "## Plan\n\nSome **bold *both*** and `code_x` 5\\*3\n\n- one\n- two\n  1. nested\n\n- [x] done\n\n```rust\nfn main() {}\n```\n\n$$\nE = mc^2\n$$\n"
        );
    }

    #[test]
    fn markdown_round_trip_is_stable() {
        let source = "# Title\n\nHello **world** and *you*, see [docs](https://example.com) and $x^2$.\n\n> quoted\n> text\n\n1. first\n2. second\n\n- [ ] open\n- [x] closed\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```js\nlet a = 1;\n```\n\n---\n\n[[Other page]]\n";
        let doc = parse(source);
        let again = parse(&md(&doc));
        assert_eq!(doc, again);
        assert_eq!(md(&doc), md(&again));
    }

    #[test]
    fn import_builds_schema_valid_tree() {
        let doc = parse("- [x] done\n- [ ] todo\n\n* tight\n  * nested\n\n![alt text](pic.png) after\n\n$$\na+b\n$$\n");
        let kinds_: Vec<_> = doc.content.iter().map(|n| n.kind.as_str()).collect();
        assert_eq!(kinds_, ["taskList", "bulletList", "image", "paragraph", "blockMath"]);
        let task = &doc.content[0].content[0];
        assert_eq!(task.kind, "taskItem");
        assert!(task.attr_bool("checked"));
        assert_eq!(task.content[0].plain_text(), "done");
        let nested_item = &doc.content[1].content[0];
        assert_eq!(nested_item.content[0].kind, "paragraph");
        assert_eq!(nested_item.content[1].kind, "bulletList");
        assert_eq!(doc.content[2].attr_str("alt"), Some("alt text"));
        assert_eq!(doc.content[4].attr_str("latex"), Some("a+b"));
    }

    #[test]
    fn html_marks_and_wikilinks() {
        let doc = parse("<u>under</u> <mark>hi</mark> [[Target|Shown]]");
        let p = &doc.content[0];
        assert!(p.content[0].has_mark("underline"));
        assert!(p.content.iter().any(|n| n.has_mark("highlight")));
        let link = p.content.iter().find(|n| n.is(kinds::WIKI_LINK)).unwrap();
        assert_eq!(link.attr_str("target"), Some("Target"));
        assert_eq!(link.attr_str("title"), Some("Shown"));
    }

    #[test]
    fn canvas_pages_export_in_reading_order() {
        let doc = Node::doc(vec![
            Node::canvas_box(300, 400, 200, vec![Node::paragraph(vec![Node::text("last")])]),
            Node::canvas_box(400, 10, 200, vec![Node::paragraph(vec![Node::text("top right")])]),
            Node::canvas_box(0, 0, 200, vec![]),
            Node::canvas_box(20, 20, 200, vec![Node::paragraph(vec![Node::text("top left")])]),
        ]);
        assert_eq!(md(&doc), "top left\n\ntop right\n\nlast\n");
    }

    #[test]
    fn escapes_markdown_syntax() {
        let doc = Node::doc(vec![Node::paragraph(vec![Node::text("# not a heading [x] *y* $5")])]);
        let out = md(&doc);
        assert_eq!(parse(&out).plain_text(), "# not a heading [x] *y* $5");
    }
}
