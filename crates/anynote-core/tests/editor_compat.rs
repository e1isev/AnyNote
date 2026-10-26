//! Contract tests between the Rust CRDT layer and the editor's Yjs binding.
//!
//! `fixtures/editor-compat/` holds one sample page that uses every node and
//! mark the editor knows:
//! * `sample.json` – the page as ProseMirror JSON (written by this test);
//! * `rust-state.bin` – that page written by Rust (`ydoc::doc_from_content`);
//! * `js-state.bin` – the same page written by y-tiptap (by `app/src/editor/compat.test.ts`).
//!
//! Rust must read what the editor writes, and the editor test checks that it
//! can load what Rust writes. Regenerate with `ANYNOTE_UPDATE_FIXTURES=1`
//! (`cargo test -p anynote-core --test editor_compat`, then `npm test` in `app/`).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anynote_core::ink::Stroke;
use anynote_core::node::{kinds, Mark, Node};
use anynote_core::ydoc;
use serde_json::{json, Value};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/editor-compat")
}

fn updating() -> bool {
    std::env::var_os("ANYNOTE_UPDATE_FIXTURES").is_some()
}

fn text(s: &str) -> Node {
    Node::text(s)
}

fn p(inlines: Vec<Node>) -> Node {
    Node::paragraph(inlines)
}

fn sample() -> Node {
    let mut doc = Node::doc(vec![
        Node::heading(2, vec![text("Every block and mark")]),
        p(vec![
            Node::styled_text("bold", vec![Mark::new("bold")]),
            text(" "),
            Node::styled_text("italic", vec![Mark::new("italic")]),
            text(" "),
            Node::styled_text("under", vec![Mark::new("underline")]),
            text(" "),
            Node::styled_text("strike", vec![Mark::new("strike")]),
            text(" "),
            Node::styled_text("code", vec![Mark::new("code")]),
            text(" "),
            Node::styled_text("link", vec![Mark::new("link").with_attr("href", "https://example.com")]),
            text(" "),
            Node::styled_text("marked", vec![Mark::new("highlight").with_attr("color", "#fff3a3")]),
            text(" "),
            Node::styled_text("red", vec![Mark::new("textStyle").with_attr("color", "#d93f3f")]),
            text(" H"),
            Node::styled_text("2", vec![Mark::new("subscript")]),
            text("O x"),
            Node::styled_text("2", vec![Mark::new("superscript")]),
            Node::new(kinds::HARD_BREAK),
            text("ünïcødé 😀 "),
            Node::new(kinds::INLINE_MATH).with_attr("latex", "a^2+b^2"),
            text(" see "),
            Node::new(kinds::WIKI_LINK)
                .with_attr("pageId", "0190f0e0-0000-7000-8000-000000000001")
                .with_attr("title", "Other page"),
        ]),
        Node::element(
            kinds::BULLET_LIST,
            vec![Node::element(
                kinds::LIST_ITEM,
                vec![
                    p(vec![text("bullet")]),
                    Node::element(
                        kinds::ORDERED_LIST,
                        vec![Node::element(kinds::LIST_ITEM, vec![p(vec![text("third")])])],
                    )
                    .with_attr("start", 3),
                ],
            )],
        ),
        Node::element(
            kinds::TASK_LIST,
            vec![
                Node::element(kinds::TASK_ITEM, vec![p(vec![text("done")])]).with_attr("checked", true),
                Node::element(kinds::TASK_ITEM, vec![p(vec![text("todo")])]).with_attr("checked", false),
            ],
        ),
        Node::element(kinds::CODE_BLOCK, vec![text("fn main() {\n    println!(\"hi\");\n}")])
            .with_attr("language", "rust"),
        Node::element(kinds::BLOCKQUOTE, vec![p(vec![text("quoted")])]),
        Node::new(kinds::HORIZONTAL_RULE),
        Node::new(kinds::IMAGE)
            .with_attr("src", "anynote://asset/0000000000000000000000000000000000000000000000000000000000000000")
            .with_attr("alt", "A figure")
            .with_attr("width", 320),
        Node::element(
            kinds::TABLE,
            vec![
                Node::element(
                    kinds::TABLE_ROW,
                    vec![
                        Node::element(kinds::TABLE_HEADER, vec![p(vec![text("A")])])
                            .with_attr("colspan", 1)
                            .with_attr("rowspan", 1),
                        Node::element(kinds::TABLE_HEADER, vec![p(vec![text("B")])])
                            .with_attr("colspan", 1)
                            .with_attr("rowspan", 1),
                    ],
                ),
                Node::element(
                    kinds::TABLE_ROW,
                    vec![Node::element(kinds::TABLE_CELL, vec![p(vec![text("wide")])])
                        .with_attr("colspan", 2)
                        .with_attr("rowspan", 1)
                        .with_attr("colwidth", json!([120, 80]))],
                ),
            ],
        ),
        Node::new(kinds::BLOCK_MATH).with_attr("latex", "\\int_0^1 x^2 dx"),
        Node::new(kinds::INK_BLOCK).with_attr("inkId", "ink-1").with_attr("height", 240),
        Node::new(kinds::ATTACHMENT)
            .with_attr("src", "anynote://asset/1111111111111111111111111111111111111111111111111111111111111111")
            .with_attr("name", "report.pdf")
            .with_attr("size", 12345)
            .with_attr("mime", "application/pdf"),
        Node::canvas_box(
            40,
            60,
            320,
            vec![p(vec![text("on a canvas")]), Node::element(kinds::BLOCKQUOTE, vec![p(vec![text("nested")])])],
        ),
        p(vec![]),
    ]);
    // Deterministic block ids so the fixture is stable.
    let mut n = 0;
    doc.walk_mut(&mut |node| {
        if anynote_core::node::BLOCK_ID_TYPES.contains(&node.kind.as_str()) {
            n += 1;
            node.attrs.insert("id".into(), Value::String(format!("block-{n:02}")));
        }
    });
    doc
}

fn sample_ink() -> BTreeMap<String, Vec<Stroke>> {
    BTreeMap::from([(
        "ink-1".to_string(),
        vec![Stroke {
            id: "stroke-1".into(),
            tool: "pen".into(),
            color: "#2f6fdf".into(),
            size: 4.0,
            points: vec![[10.0, 20.0, 0.5], [30.5, 40.25, 0.75], [60.0, 42.0, 0.5]],
        }],
    )])
}

/// Every key in `expected` is present in `actual` with an equal (recursively
/// subset-equal) value; arrays must have the same length. The editor adds
/// default attributes (e.g. link `target`), which this tolerates.
fn assert_subset(expected: &Value, actual: &Value, path: &str) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, v) in e {
                let av = a.get(k).unwrap_or_else(|| panic!("{path}.{k} missing in {actual}"));
                assert_subset(v, av, &format!("{path}.{k}"));
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            assert_eq!(e.len(), a.len(), "{path}: length differs\nexpected {expected}\nactual   {actual}");
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                assert_subset(ev, av, &format!("{path}[{i}]"));
            }
        }
        _ => assert_eq!(expected, actual, "{path}"),
    }
}

#[test]
fn rust_written_fixture_is_current() {
    let doc = sample();
    let ink = sample_ink();
    let json_text = serde_json::to_string_pretty(&doc).unwrap() + "\n";
    if updating() {
        fs::create_dir_all(dir()).unwrap();
        fs::write(dir().join("sample.json"), &json_text).unwrap();
        fs::write(dir().join("rust-state.bin"), ydoc::encode_state(&ydoc::doc_from_content(&doc, &ink))).unwrap();
    }
    assert_eq!(
        fs::read_to_string(dir().join("sample.json")).unwrap(),
        json_text,
        "sample.json is stale; regenerate the fixtures"
    );
    let state = fs::read(dir().join("rust-state.bin")).unwrap();
    let decoded = ydoc::doc_from_state(&state).unwrap();
    assert_eq!(ydoc::read_content(&decoded), doc);
    assert_eq!(ydoc::read_ink(&decoded), ink);
}

#[test]
fn reads_documents_written_by_the_editor() {
    let path = dir().join("js-state.bin");
    let state = fs::read(&path).unwrap_or_else(|_| {
        panic!("{} missing: run `npm test` in app/ with ANYNOTE_UPDATE_FIXTURES=1", path.display())
    });
    let doc = ydoc::doc_from_state(&state).unwrap();
    let content = serde_json::to_value(ydoc::read_content(&doc)).unwrap();
    let expected = serde_json::to_value(sample()).unwrap();
    assert_subset(&expected, &content, "doc");
    assert_eq!(ydoc::read_ink(&doc), sample_ink());
}
