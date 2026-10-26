//! The block model: a ProseMirror-compatible node tree.
//!
//! Every page body is a tree of [`Node`]s that serialises to exactly the JSON
//! ProseMirror/TipTap uses (`{"type": "paragraph", "attrs": {...}, "content": [...]}`).
//! Keeping the model generic (type name + attribute map) rather than a closed
//! Rust enum means unknown node types and attributes survive every round trip,
//! which is the same forward-compatibility rule [MS-ONE] imposes on OneNote.
//!
//! The node and mark names the app understands are listed in [`kinds`] and
//! documented in `docs/FORMAT.md`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Node type names shared by the editor, the importers and the exporters.
pub mod kinds {
    pub const DOC: &str = "doc";
    pub const PARAGRAPH: &str = "paragraph";
    pub const HEADING: &str = "heading";
    pub const BULLET_LIST: &str = "bulletList";
    pub const ORDERED_LIST: &str = "orderedList";
    pub const LIST_ITEM: &str = "listItem";
    pub const TASK_LIST: &str = "taskList";
    pub const TASK_ITEM: &str = "taskItem";
    pub const CODE_BLOCK: &str = "codeBlock";
    pub const BLOCKQUOTE: &str = "blockquote";
    pub const HORIZONTAL_RULE: &str = "horizontalRule";
    pub const IMAGE: &str = "image";
    pub const TABLE: &str = "table";
    pub const TABLE_ROW: &str = "tableRow";
    pub const TABLE_HEADER: &str = "tableHeader";
    pub const TABLE_CELL: &str = "tableCell";
    pub const BLOCK_MATH: &str = "blockMath";
    pub const INK_BLOCK: &str = "inkBlock";
    pub const ATTACHMENT: &str = "attachment";
    /// A free-positioned text box on a canvas page (`x`, `y`, `width` in px).
    pub const CANVAS_BOX: &str = "canvasBox";
    // inline
    pub const TEXT: &str = "text";
    pub const HARD_BREAK: &str = "hardBreak";
    pub const INLINE_MATH: &str = "inlineMath";
    pub const WIKI_LINK: &str = "wikiLink";
}

/// Mark type names.
pub mod marks {
    pub const BOLD: &str = "bold";
    pub const ITALIC: &str = "italic";
    pub const UNDERLINE: &str = "underline";
    pub const STRIKE: &str = "strike";
    pub const CODE: &str = "code";
    pub const LINK: &str = "link";
    pub const HIGHLIGHT: &str = "highlight";
    pub const SUBSCRIPT: &str = "subscript";
    pub const SUPERSCRIPT: &str = "superscript";
}

/// Block types that carry a stable `id` attribute (assigned by the editor's
/// UniqueID extension, or by [`Node::ensure_block_ids`] for content created in Rust).
pub const BLOCK_ID_TYPES: &[&str] = &[
    kinds::PARAGRAPH,
    kinds::HEADING,
    kinds::BULLET_LIST,
    kinds::ORDERED_LIST,
    kinds::LIST_ITEM,
    kinds::TASK_LIST,
    kinds::TASK_ITEM,
    kinds::CODE_BLOCK,
    kinds::BLOCKQUOTE,
    kinds::HORIZONTAL_RULE,
    kinds::IMAGE,
    kinds::TABLE,
    kinds::BLOCK_MATH,
    kinds::INK_BLOCK,
    kinds::ATTACHMENT,
    kinds::CANVAS_BOX,
];

/// Key of a canvas page's page-wide drawing in the ink map.
pub const CANVAS_INK: &str = "canvas";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub attrs: Map<String, Value>,
}

impl Mark {
    pub fn new(kind: impl Into<String>) -> Self {
        Mark { kind: kind.into(), attrs: Map::new() }
    }

    pub fn with_attr(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.attrs.insert(key.to_string(), value.into());
        self
    }

    pub fn attr_str(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).and_then(Value::as_str)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub attrs: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
}

impl Node {
    pub fn new(kind: impl Into<String>) -> Self {
        Node { kind: kind.into(), ..Default::default() }
    }

    pub fn element(kind: impl Into<String>, content: Vec<Node>) -> Self {
        Node { kind: kind.into(), content, ..Default::default() }
    }

    pub fn doc(content: Vec<Node>) -> Self {
        Node::element(kinds::DOC, content)
    }

    /// A text box on a canvas page at `(x, y)`, `width` px wide.
    pub fn canvas_box(x: i64, y: i64, width: i64, blocks: Vec<Node>) -> Self {
        let blocks = if blocks.is_empty() { vec![Node::paragraph(vec![])] } else { blocks };
        Node::element(kinds::CANVAS_BOX, blocks).with_attr("x", x).with_attr("y", y).with_attr("width", width)
    }

    /// An empty canvas page: an open drawing surface (strokes live under
    /// [`CANVAS_INK`] in the ink map) with text in positioned boxes. It holds
    /// one empty box, which the editor reuses for the first text typed.
    pub fn canvas_doc() -> Self {
        Node::doc(vec![Node::canvas_box(48, 48, 480, vec![])])
    }

    /// Is this `doc` a canvas page (its blocks are positioned boxes)?
    pub fn is_canvas(&self) -> bool {
        self.content.iter().any(|n| n.is(kinds::CANVAS_BOX))
    }

    /// An empty document holds one empty paragraph, like a fresh editor.
    pub fn empty_doc() -> Self {
        Node::doc(vec![Node::paragraph(vec![])])
    }

    pub fn paragraph(inlines: Vec<Node>) -> Self {
        Node::element(kinds::PARAGRAPH, inlines)
    }

    pub fn heading(level: u8, inlines: Vec<Node>) -> Self {
        Node::element(kinds::HEADING, inlines).with_attr("level", level.clamp(1, 6))
    }

    pub fn text(text: impl Into<String>) -> Self {
        Node { kind: kinds::TEXT.into(), text: Some(text.into()), ..Default::default() }
    }

    pub fn styled_text(text: impl Into<String>, marks: Vec<Mark>) -> Self {
        Node { marks, ..Node::text(text) }
    }

    pub fn with_attr(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.attrs.insert(key.to_string(), value.into());
        self
    }

    pub fn with_mark(mut self, mark: Mark) -> Self {
        self.marks.push(mark);
        self
    }

    pub fn is(&self, kind: &str) -> bool {
        self.kind == kind
    }

    pub fn is_text(&self) -> bool {
        self.kind == kinds::TEXT
    }

    pub fn attr(&self, key: &str) -> Option<&Value> {
        self.attrs.get(key)
    }

    pub fn attr_str(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).and_then(Value::as_str)
    }

    pub fn attr_bool(&self, key: &str) -> bool {
        self.attrs.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn attr_i64(&self, key: &str) -> Option<i64> {
        self.attrs.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    }

    pub fn has_mark(&self, kind: &str) -> bool {
        self.marks.iter().any(|m| m.kind == kind)
    }

    pub fn mark(&self, kind: &str) -> Option<&Mark> {
        self.marks.iter().find(|m| m.kind == kind)
    }

    /// The node's text with all formatting removed. Blocks are separated by
    /// newlines so the result is suitable for full-text indexing.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        self.write_plain_text(&mut out);
        while out.ends_with('\n') {
            out.pop();
        }
        out
    }

    fn write_plain_text(&self, out: &mut String) {
        match self.kind.as_str() {
            kinds::TEXT => out.push_str(self.text.as_deref().unwrap_or("")),
            kinds::HARD_BREAK => out.push('\n'),
            kinds::INLINE_MATH | kinds::BLOCK_MATH => {
                out.push_str(self.attr_str("latex").unwrap_or(""));
                if self.kind == kinds::BLOCK_MATH {
                    out.push('\n');
                }
            }
            kinds::WIKI_LINK => out.push_str(self.attr_str("title").unwrap_or("")),
            kinds::IMAGE => {
                if let Some(alt) = self.attr_str("alt").filter(|a| !a.is_empty()) {
                    out.push_str(alt);
                    out.push('\n');
                }
            }
            kinds::ATTACHMENT => {
                out.push_str(self.attr_str("name").unwrap_or(""));
                out.push('\n');
            }
            _ => {
                let is_textblock = self.content.iter().any(Node::is_inline);
                for child in &self.content {
                    child.write_plain_text(out);
                }
                if is_textblock || self.content.is_empty() && self.kind != kinds::DOC {
                    out.push('\n');
                }
            }
        }
    }

    pub fn is_inline(&self) -> bool {
        matches!(self.kind.as_str(), kinds::TEXT | kinds::HARD_BREAK | kinds::INLINE_MATH | kinds::WIKI_LINK)
    }

    /// Depth-first, pre-order visit of this node and all its descendants.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        for child in &self.content {
            child.walk(f);
        }
    }

    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Node)) {
        f(self);
        for child in &mut self.content {
            child.walk_mut(f);
        }
    }

    /// Give every block that should have a stable ID one, keeping existing IDs.
    pub fn ensure_block_ids(&mut self) {
        self.walk_mut(&mut |n| {
            if BLOCK_ID_TYPES.contains(&n.kind.as_str()) && n.attr_str("id").is_none_or(str::is_empty) {
                n.attrs.insert("id".into(), Value::String(crate::ids::new_id()));
            }
        });
    }

    /// Put a `doc` in the exact shape the editor would leave it in: block ids
    /// everywhere and a trailing empty paragraph after a non-paragraph last
    /// block (the editor's TrailingNode). Otherwise merely opening a page
    /// would write a change.
    pub fn prepare_for_editor(&mut self) {
        // Canvas pages end with a box; the editor adds nothing after one.
        if !self.content.last().is_some_and(|n| n.is(kinds::PARAGRAPH) || n.is(kinds::CANVAS_BOX)) {
            self.content.push(Node::paragraph(vec![]));
        }
        self.ensure_block_ids();
    }

    /// Merge adjacent text nodes with identical marks and drop empty ones.
    pub fn normalize_text(&mut self) {
        let mut merged: Vec<Node> = Vec::with_capacity(self.content.len());
        for mut child in std::mem::take(&mut self.content) {
            child.normalize_text();
            if child.is_text() {
                if child.text.as_deref().unwrap_or("").is_empty() {
                    continue;
                }
                if let Some(prev) = merged.last_mut() {
                    if prev.is_text() && prev.marks == child.marks {
                        prev.text.get_or_insert_with(String::new).push_str(child.text.as_deref().unwrap_or(""));
                        continue;
                    }
                }
            }
            merged.push(child);
        }
        self.content = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_like_prosemirror() {
        let doc = Node::doc(vec![Node::heading(2, vec![Node::styled_text("Hi", vec![Mark::new("bold")])])]);
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "heading",
                    "attrs": {"level": 2},
                    "content": [{"type": "text", "text": "Hi", "marks": [{"type": "bold"}]}]
                }]
            })
        );
        let back: Node = serde_json::from_value(json).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn plain_text_separates_blocks() {
        let doc = Node::doc(vec![
            Node::paragraph(vec![Node::text("one "), Node::styled_text("two", vec![Mark::new("bold")])]),
            Node::element(
                kinds::BULLET_LIST,
                vec![Node::element(kinds::LIST_ITEM, vec![Node::paragraph(vec![Node::text("three")])])],
            ),
        ]);
        assert_eq!(doc.plain_text(), "one two\nthree");
    }

    #[test]
    fn prepare_for_editor_adds_trailing_paragraph_and_ids() {
        let mut doc = Node::doc(vec![Node::new(kinds::HORIZONTAL_RULE)]);
        doc.prepare_for_editor();
        assert_eq!(doc.content.len(), 2);
        assert!(doc.content[1].is(kinds::PARAGRAPH));
        assert!(doc.content.iter().all(|n| n.attr_str("id").is_some()));
        doc.prepare_for_editor();
        assert_eq!(doc.content.len(), 2, "idempotent");
    }

    #[test]
    fn canvas_pages_need_no_trailing_paragraph() {
        let mut doc = Node::canvas_doc();
        assert!(doc.is_canvas());
        doc.prepare_for_editor();
        assert_eq!(doc.content.len(), 1);
        assert_eq!(doc.content[0].attr_i64("x"), Some(48));
        assert!(!Node::empty_doc().is_canvas());
    }

    #[test]
    fn normalize_merges_runs() {
        let mut p = Node::paragraph(vec![Node::text("a"), Node::text("b"), Node::text(""), Node::text("c")]);
        p.normalize_text();
        assert_eq!(p.content, vec![Node::text("abc")]);
    }
}
