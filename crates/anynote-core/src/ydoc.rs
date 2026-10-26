//! CRDT page documents.
//!
//! Each page is one Yjs-compatible document (via `yrs`). The rich-text body is
//! stored the way the editor's binding (y-tiptap / y-prosemirror) stores it:
//!
//! * a root `Y.XmlFragment` named [`FRAGMENT`];
//! * one `Y.XmlElement` per node, named after the node type, whose attributes are
//!   the node's attrs (JSON values, `null`s omitted);
//! * runs of text as a `Y.XmlText` whose formatting attributes are the marks
//!   (`{markName: markAttrs}`).
//!
//! Ink strokes live next to the body in a root `Y.Map` (see [`crate::ink`]).
//!
//! Because the Rust side reads and writes the same structure, the importer,
//! exporter, search indexer and the editor all share one source of truth.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Map as JsonMap, Number, Value};
use yrs::types::text::YChange;
use yrs::types::{Attrs, ToJson};
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, Doc, GetString, Map, MapRef, OffsetKind, Options, Out, ReadTxn, StateVector,
    Text, Transact, TransactionMut, Update, Xml, XmlElementPrelim, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut,
    XmlTextPrelim, XmlTextRef,
};

use crate::ink::{Stroke, INK_MAP};
use crate::node::{Mark, Node};
use crate::{Error, Result};

/// Name of the root XML fragment (TipTap's Collaboration extension default).
pub const FRAGMENT: &str = "default";

pub fn new_doc() -> Doc {
    Doc::with_options(Options { offset_kind: OffsetKind::Utf16, ..Options::default() })
}

pub fn doc_from_state(state: &[u8]) -> Result<Doc> {
    let doc = new_doc();
    apply_update(&doc, state)?;
    Ok(doc)
}

pub fn apply_update(doc: &Doc, update: &[u8]) -> Result<()> {
    let update = Update::decode_v1(update).map_err(|e| Error::InvalidUpdate(e.to_string()))?;
    doc.transact_mut().apply_update(update).map_err(|e| Error::InvalidUpdate(e.to_string()))
}

/// The full document state encoded as a single update.
pub fn encode_state(doc: &Doc) -> Vec<u8> {
    doc.transact().encode_state_as_update_v1(&StateVector::default())
}

/// Build a new document holding `content` (a `doc` node) and `ink`.
pub fn doc_from_content(content: &Node, ink: &BTreeMap<String, Vec<Stroke>>) -> Doc {
    let doc = new_doc();
    let fragment = doc.get_or_insert_xml_fragment(FRAGMENT);
    let ink_map = doc.get_or_insert_map(INK_MAP);
    {
        let mut txn = doc.transact_mut();
        write_children(&mut txn, &fragment, content);
        for (ink_id, strokes) in ink {
            write_ink(&mut txn, &ink_map, ink_id, strokes);
        }
    }
    doc
}

/// Read the page body back as a `doc` node.
pub fn read_content(doc: &Doc) -> Node {
    let fragment = doc.get_or_insert_xml_fragment(FRAGMENT);
    let txn = doc.transact();
    let mut root = Node::doc(Vec::new());
    read_children(&txn, &fragment, &mut root.content);
    root
}

pub fn read_ink(doc: &Doc) -> BTreeMap<String, Vec<Stroke>> {
    let ink = doc.get_or_insert_map(INK_MAP);
    let txn = doc.transact();
    let mut out = BTreeMap::new();
    for (key, value) in ink.iter(&txn) {
        let Out::YArray(arr) = value else { continue };
        let strokes = arr
            .iter(&txn)
            .filter_map(|v| match v {
                Out::Any(any) => stroke_from_any(&any),
                _ => None,
            })
            .collect();
        out.insert(key.to_string(), strokes);
    }
    out
}

/// Replace the page body and ink with new content inside one transaction, and
/// return the resulting incremental update (to broadcast to open editors).
///
/// This is how version restore and Rust-side rewrites stay CRDT-friendly: the
/// change is an ordinary edit that merges with concurrent edits elsewhere.
pub fn replace_all(doc: &Doc, content: &Node, ink: &BTreeMap<String, Vec<Stroke>>) -> Vec<u8> {
    let fragment = doc.get_or_insert_xml_fragment(FRAGMENT);
    let ink_map = doc.get_or_insert_map(INK_MAP);
    let mut txn = doc.transact_mut();
    let len = fragment.len(&txn);
    if len > 0 {
        fragment.remove_range(&mut txn, 0, len);
    }
    write_children(&mut txn, &fragment, content);
    let existing: Vec<String> = ink_map.keys(&txn).map(str::to_string).collect();
    for key in existing {
        if !ink.contains_key(&key) {
            ink_map.remove(&mut txn, &key);
        }
    }
    for (ink_id, strokes) in ink {
        write_ink(&mut txn, &ink_map, ink_id, strokes);
    }
    txn.encode_update_v1()
}

/// Set an attribute on the block whose `id` attribute equals `block_id`.
/// Returns the incremental update, or `None` if no such block exists.
pub fn set_block_attr(doc: &Doc, block_id: &str, key: &str, value: Value) -> Option<Vec<u8>> {
    let fragment = doc.get_or_insert_xml_fragment(FRAGMENT);
    let target = {
        let txn = doc.transact();
        find_element(&txn, &fragment, block_id)?
    };
    let mut txn = doc.transact_mut();
    target.insert_attribute(&mut txn, key, json_to_any(&value));
    Some(txn.encode_update_v1())
}

fn find_element<T: ReadTxn>(txn: &T, parent: &impl XmlFragment, block_id: &str) -> Option<XmlElementRef> {
    for child in parent.children(txn) {
        if let XmlOut::Element(el) = child {
            if matches!(el.get_attribute(txn, "id"), Some(Out::Any(Any::String(ref s))) if s.as_ref() == block_id) {
                return Some(el);
            }
            if let Some(found) = find_element(txn, &el, block_id) {
                return Some(found);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// writing

fn write_children(txn: &mut TransactionMut, parent: &impl XmlFragment, node: &Node) {
    let mut i = 0;
    while i < node.content.len() {
        if node.content[i].is_text() {
            // Consecutive text nodes become one Y.XmlText, as y-prosemirror does.
            let start = i;
            while i < node.content.len() && node.content[i].is_text() {
                i += 1;
            }
            let text: XmlTextRef = parent.push_back(txn, XmlTextPrelim::new(""));
            let mut offset = 0;
            for run in &node.content[start..i] {
                let chunk = run.text.as_deref().unwrap_or("");
                if chunk.is_empty() {
                    continue;
                }
                text.insert_with_attributes(txn, offset, chunk, marks_to_attrs(&run.marks));
                offset += chunk.encode_utf16().count() as u32;
            }
        } else {
            let child = &node.content[i];
            let el: XmlElementRef = parent.push_back(txn, XmlElementPrelim::empty(child.kind.as_str()));
            for (key, value) in &child.attrs {
                if !value.is_null() {
                    el.insert_attribute(txn, key.as_str(), json_to_any(value));
                }
            }
            write_children(txn, &el, child);
            i += 1;
        }
    }
}

fn marks_to_attrs(marks: &[Mark]) -> Attrs {
    let mut attrs: Attrs = HashMap::new();
    for mark in marks {
        attrs.insert(Arc::from(mark.kind.as_str()), json_to_any(&Value::Object(mark.attrs.clone())));
    }
    attrs
}

fn write_ink(txn: &mut TransactionMut, ink_map: &MapRef, ink_id: &str, strokes: &[Stroke]) {
    let arr: ArrayRef = ink_map.insert(txn, ink_id, ArrayPrelim::default());
    for stroke in strokes {
        arr.push_back(txn, stroke_to_any(stroke));
    }
}

pub fn stroke_to_any(stroke: &Stroke) -> Any {
    let mut m = HashMap::new();
    m.insert("id".to_string(), Any::from(stroke.id.as_str()));
    m.insert("tool".to_string(), Any::from(stroke.tool.as_str()));
    m.insert("color".to_string(), Any::from(stroke.color.as_str()));
    m.insert("size".to_string(), Any::from(stroke.size));
    m.insert("pts".to_string(), Any::Buffer(Stroke::encode_points(&stroke.points).into()));
    Any::Map(Arc::new(m))
}

fn stroke_from_any(any: &Any) -> Option<Stroke> {
    let Any::Map(m) = any else { return None };
    let s = |k: &str| match m.get(k) {
        Some(Any::String(s)) => Some(s.to_string()),
        _ => None,
    };
    let points = match m.get("pts") {
        Some(Any::Buffer(b)) => Stroke::decode_points(b),
        // Tolerate a plain number array as well.
        Some(Any::Array(a)) => {
            a.chunks_exact(3).map(|c| [any_f64(&c[0]) as f32, any_f64(&c[1]) as f32, any_f64(&c[2]) as f32]).collect()
        }
        _ => Vec::new(),
    };
    Some(Stroke {
        id: s("id").unwrap_or_default(),
        tool: s("tool").unwrap_or_else(|| "pen".into()),
        color: s("color").unwrap_or_else(|| "#000000".into()),
        size: m.get("size").map(any_f64).unwrap_or(2.0),
        points,
    })
}

fn any_f64(a: &Any) -> f64 {
    match a {
        Any::Number(yrs::any::Number::Int(n)) => *n as f64,
        Any::Number(yrs::any::Number::Float(f)) => *f,
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// reading

fn read_children<T: ReadTxn>(txn: &T, parent: &impl XmlFragment, out: &mut Vec<Node>) {
    for child in parent.children(txn) {
        match child {
            XmlOut::Element(el) => {
                let mut node = Node::new(el.tag().to_string());
                for (key, value) in el.attributes(txn) {
                    node.attrs.insert(key.to_string(), out_to_json(txn, &value));
                }
                read_children(txn, &el, &mut node.content);
                out.push(node);
            }
            XmlOut::Text(text) => {
                for chunk in text.diff(txn, YChange::identity) {
                    let s = match &chunk.insert {
                        Out::Any(Any::String(s)) => s.to_string(),
                        other => other.clone().to_string(txn),
                    };
                    if s.is_empty() {
                        continue;
                    }
                    let mut marks: Vec<Mark> = chunk
                        .attributes
                        .map(|attrs| attrs.iter().map(|(k, v)| attr_to_mark(k, v)).collect())
                        .unwrap_or_default();
                    marks.sort_by(|a, b| a.kind.cmp(&b.kind));
                    out.push(Node::styled_text(s, marks));
                }
            }
            XmlOut::Fragment(frag) => read_children(txn, &frag, out),
        }
    }
}

/// y-prosemirror suffixes overlapping marks with `--<8 char hash>`.
fn attr_to_mark(key: &str, value: &Any) -> Mark {
    let kind = match key.rfind("--") {
        Some(i) if key.len() - i == 10 => &key[..i],
        _ => key,
    };
    let attrs = match any_to_json(value) {
        Value::Object(m) => m,
        _ => JsonMap::new(),
    };
    Mark { kind: kind.to_string(), attrs }
}

fn out_to_json<T: ReadTxn>(txn: &T, out: &Out) -> Value {
    match out {
        Out::Any(any) => any_to_json(any),
        other => any_to_json(&other.to_json(txn)),
    }
}

pub fn any_to_json(any: &Any) -> Value {
    match any {
        Any::Null | Any::Undefined => Value::Null,
        Any::Bool(b) => Value::Bool(*b),
        Any::Number(yrs::any::Number::Int(n)) => Value::Number((*n).into()),
        Any::Number(yrs::any::Number::Float(f)) => {
            if f.fract() == 0.0 && f.abs() < 9.0e15 {
                Value::Number((*f as i64).into())
            } else {
                Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null)
            }
        }
        Any::String(s) => Value::String(s.to_string()),
        Any::Buffer(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
        Any::Array(a) => Value::Array(a.iter().map(any_to_json).collect()),
        Any::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), any_to_json(v))).collect()),
    }
}

pub fn json_to_any(value: &Value) -> Any {
    match value {
        Value::Null => Any::Null,
        Value::Bool(b) => Any::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Any::from(i),
            None => Any::from(n.as_f64().unwrap_or_default()),
        },
        Value::String(s) => Any::from(s.as_str()),
        Value::Array(a) => Any::Array(a.iter().map(json_to_any).collect::<Vec<_>>().into()),
        Value::Object(m) => Any::Map(Arc::new(m.iter().map(|(k, v)| (k.clone(), json_to_any(v))).collect())),
    }
}

/// Convenience: text of a whole doc's body.
pub fn plain_text(doc: &Doc) -> String {
    read_content(doc).plain_text()
}

#[allow(dead_code)]
fn debug_xml(doc: &Doc) -> String {
    let fragment: XmlFragmentRef = doc.get_or_insert_xml_fragment(FRAGMENT);
    let txn = doc.transact();
    fragment.get_string(&txn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::kinds;
    use serde_json::json;

    fn sample() -> Node {
        let mut doc = Node::doc(vec![
            Node::heading(1, vec![Node::text("Title")]),
            Node::paragraph(vec![
                Node::text("plain "),
                Node::styled_text("bold", vec![Mark::new("bold")]),
                Node::text(" and "),
                Node::styled_text("link", vec![Mark::new("link").with_attr("href", "https://example.com")]),
                Node::new(kinds::HARD_BREAK),
                Node::new(kinds::INLINE_MATH).with_attr("latex", "x^2"),
                Node::text(" ünïcødé 😀 tail"),
            ]),
            Node::element(
                kinds::TASK_LIST,
                vec![Node::element(kinds::TASK_ITEM, vec![Node::paragraph(vec![Node::text("todo")])])
                    .with_attr("checked", false)],
            ),
            Node::new(kinds::INK_BLOCK).with_attr("inkId", "ink1").with_attr("height", 240),
        ]);
        doc.ensure_block_ids();
        doc
    }

    #[test]
    fn content_round_trips_through_crdt() {
        let content = sample();
        let mut ink = BTreeMap::new();
        ink.insert(
            "ink1".to_string(),
            vec![Stroke {
                id: "s1".into(),
                tool: "pen".into(),
                color: "#112233".into(),
                size: 3.0,
                points: vec![[0.0, 0.0, 0.5], [10.0, 5.0, 0.7]],
            }],
        );
        let doc = doc_from_content(&content, &ink);
        let state = encode_state(&doc);
        let reloaded = doc_from_state(&state).unwrap();
        assert_eq!(read_content(&reloaded), content);
        assert_eq!(read_ink(&reloaded), ink);
    }

    #[test]
    fn replace_and_set_attr_produce_mergeable_updates() {
        let content = sample();
        let a = doc_from_content(&content, &BTreeMap::new());
        let b = doc_from_state(&encode_state(&a)).unwrap();

        let task_id = {
            let mut id = None;
            content.walk(&mut |n| {
                if n.is(kinds::TASK_ITEM) {
                    id = n.attr_str("id").map(str::to_string);
                }
            });
            id.unwrap()
        };
        let update = set_block_attr(&a, &task_id, "checked", json!(true)).unwrap();
        apply_update(&b, &update).unwrap();
        let mut checked = false;
        read_content(&b).walk(&mut |n| {
            if n.is(kinds::TASK_ITEM) {
                checked = n.attr_bool("checked");
            }
        });
        assert!(checked);

        let replacement = Node::doc(vec![Node::paragraph(vec![Node::text("restored")])]);
        let update = replace_all(&a, &replacement, &BTreeMap::new());
        apply_update(&b, &update).unwrap();
        assert_eq!(read_content(&b), replacement);
    }
}
