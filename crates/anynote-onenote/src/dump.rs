//! `--dump-json`: the parsed OneNote object tree as JSON.
//!
//! This is the instrumentation step of the reverse-engineering plan: make one
//! change in OneNote, dump before and after, and diff the JSON to see which
//! objects and properties the change touched.

use std::path::Path;

use onenote_parser::contents::{
    Content, EmbeddedFile, EmbeddedObject, Image, Ink, InkStroke, NoteTag, Outline, OutlineElement, OutlineItem,
    ParagraphStyling, RichText, Table,
};
use onenote_parser::page::{Page, PageContent};
use onenote_parser::property::common::{Color, ColorRef};
use onenote_parser::section::{Section, SectionEntry};
use serde_json::{json, Map, Value};

use crate::source::{self, Parsed};
use crate::text;

pub fn dump(path: &Path) -> anyhow::Result<Value> {
    let parsed = source::parse(path)?;
    Ok(match parsed {
        Parsed::Notebook { title, notebook } => json!({
            "kind": "notebook",
            "path": path.display().to_string(),
            "title": title,
            "color": notebook.color().map(color),
            "warnings": notebook.report().warnings().iter().map(|w| w.message().to_string()).collect::<Vec<_>>(),
            "entries": notebook.entries().iter().map(entry).collect::<Vec<_>>(),
        }),
        Parsed::Section(s) => {
            let mut v = section(&s);
            v["path"] = json!(path.display().to_string());
            v
        }
    })
}

fn entry(e: &SectionEntry) -> Value {
    match e {
        SectionEntry::Section(s) => section(s),
        SectionEntry::SectionGroup(g) => json!({
            "kind": "sectionGroup",
            "name": g.display_name(),
            "entries": g.entries().iter().map(entry).collect::<Vec<_>>(),
        }),
    }
}

fn section(s: &Section) -> Value {
    json!({
        "kind": "section",
        "name": s.display_name(),
        "color": s.color().map(color),
        "warnings": s.report().warnings().iter().map(|w| w.message().to_string()).collect::<Vec<_>>(),
        "pageSeries": s.page_series().iter().map(|ps| ps.pages().iter().map(page).collect::<Vec<_>>()).collect::<Vec<_>>(),
    })
}

fn page(p: &Page) -> Value {
    json!({
        "kind": "page",
        "id": p.link_target_id(),
        "title": p.title_text(),
        "level": p.level(),
        "created": text::datetime_ms(p.created_time()),
        "updated": text::datetime_ms(p.updated_time()),
        "author": p.author(),
        "height": p.height(),
        "titleOutlines": p.title().map(|t| t.contents().iter().map(outline).collect::<Vec<_>>()),
        "inkRecognition": p.ink_recognition().map(|r| r.text()),
        "contents": p.contents().iter().map(page_content).collect::<Vec<_>>(),
    })
}

fn page_content(c: &PageContent) -> Value {
    match c {
        PageContent::Outline(o) => outline(o),
        PageContent::Image(i) => image(i),
        PageContent::EmbeddedFile(f) => embedded_file(f),
        PageContent::Ink(i) => ink(i),
        PageContent::Unknown => json!({"kind": "unknown"}),
    }
}

fn outline(o: &Outline) -> Value {
    json!({
        "kind": "outline",
        "offset": [o.offset_horizontal(), o.offset_vertical()],
        "maxWidth": o.layout_max_width(),
        "childLevel": o.child_level(),
        "indents": o.indents(),
        "items": o.items().iter().map(outline_item).collect::<Vec<_>>(),
    })
}

fn outline_item(i: &OutlineItem) -> Value {
    match i {
        OutlineItem::Element(e) => element(e),
        OutlineItem::Group(g) => json!({
            "kind": "group",
            "childLevel": g.child_level(),
            "items": g.outlines().iter().map(outline_item).collect::<Vec<_>>(),
        }),
    }
}

fn element(e: &OutlineElement) -> Value {
    json!({
        "kind": "element",
        "childLevel": e.child_level(),
        "list": e.list_contents().iter().map(|l| json!({
            "format": l.list_format().iter().map(|c| format!("U+{:04X}", *c as u32)).collect::<Vec<_>>(),
            "font": l.list_font(),
            "restart": l.list_restart(),
        })).collect::<Vec<_>>(),
        "contents": e.contents().iter().map(content).collect::<Vec<_>>(),
        "children": e.children().iter().map(outline_item).collect::<Vec<_>>(),
    })
}

fn content(c: &Content) -> Value {
    match c {
        Content::RichText(t) => rich_text(t),
        Content::Table(t) => table(t),
        Content::Image(i) => image(i),
        Content::EmbeddedFile(f) => embedded_file(f),
        Content::Ink(i) => ink(i),
        Content::Unknown => json!({"kind": "unknown"}),
    }
}

fn rich_text(t: &RichText) -> Value {
    let runs: Vec<Value> = text::split_utf16(t.text(), t.text_run_indices(), t.text_run_formatting().len())
        .into_iter()
        .map(|(s, style)| {
            let mut v = json!({ "text": s });
            if let Some(i) = style {
                v["style"] = styling(&t.text_run_formatting()[i]);
            }
            v
        })
        .collect();
    json!({
        "kind": "richText",
        "text": t.text(),
        "paragraphStyle": styling(t.paragraph_style()),
        "runs": runs,
        "hyperlinks": t.hyperlinks().iter().map(|h| json!({"target": h.target(), "start": h.start(), "end": h.end()})).collect::<Vec<_>>(),
        "noteTags": t.note_tags().iter().map(note_tag).collect::<Vec<_>>(),
        "math": t.math_inline_objects().iter().map(|m| json!({
            "type": format!("{:?}", m.object_type()),
            "args": m.arg_count(),
            "char": m.char().map(String::from),
            "char1": m.char1().map(String::from),
            "char2": m.char2().map(String::from),
        })).collect::<Vec<_>>(),
        "embedded": t.embedded_objects().iter().map(|o| match o {
            EmbeddedObject::Ink(c) => json!({"kind": "ink", "ink": ink(c.ink()), "box": c.bounding_box().map(|b| [b.x(), b.y(), b.width(), b.height()])}),
            EmbeddedObject::InkSpace(s) => json!({"kind": "inkSpace", "width": s.width(), "height": s.height()}),
            EmbeddedObject::InkLineBreak => json!({"kind": "inkLineBreak"}),
        }).collect::<Vec<_>>(),
    })
}

fn styling(s: &ParagraphStyling) -> Value {
    let mut m = Map::new();
    let mut flag = |k: &str, v: bool| {
        if v {
            m.insert(k.into(), Value::Bool(true));
        }
    };
    flag("bold", s.bold());
    flag("italic", s.italic());
    flag("underline", s.underline());
    flag("strike", s.strikethrough());
    flag("superscript", s.superscript());
    flag("subscript", s.subscript());
    flag("math", s.math_formatting());
    flag("hyperlink", s.hyperlink());
    flag("hidden", s.hidden());
    if let Some(v) = s.style_id() {
        m.insert("styleId".into(), json!(v));
    }
    if let Some(v) = s.font() {
        m.insert("font".into(), json!(v));
    }
    if let Some(v) = s.font_size() {
        m.insert("fontSize".into(), json!(f32::from(v) / 2.0));
    }
    if let Some(v) = s.font_color() {
        m.insert("color".into(), color_ref(v));
    }
    if let Some(v) = s.highlight() {
        m.insert("highlight".into(), color_ref(v));
    }
    Value::Object(m)
}

fn note_tag(t: &NoteTag) -> Value {
    let def = t.definition();
    json!({
        "label": def.map(|d| d.label()),
        "shape": def.map(|d| format!("{:?}", d.shape())),
        "checkable": def.map(|d| d.shape().is_checkable()),
        "completed": t.item_status().completed(),
        "taskTag": t.item_status().task_tag(),
    })
}

fn table(t: &Table) -> Value {
    json!({
        "kind": "table",
        "rows": t.rows(),
        "cols": t.cols(),
        "colWidths": t.col_widths(),
        "bordersVisible": t.borders_visible(),
        "contents": t.contents().iter().map(|r| r.contents().iter().map(|c| json!({
            "background": c.background_color().map(color),
            "elements": c.contents().iter().map(element).collect::<Vec<_>>(),
        })).collect::<Vec<_>>()).collect::<Vec<_>>(),
    })
}

fn image(i: &Image) -> Value {
    json!({
        "kind": "image",
        "offset": [i.offset_horizontal(), i.offset_vertical()],
        "extension": i.extension(),
        "filename": i.image_filename(),
        "size": i.size(),
        "status": format!("{:?}", i.data_status()),
        "altText": i.alt_text(),
        "ocrText": i.text(),
        "maxWidth": i.layout_max_width(),
        "pictureWidth": i.picture_width(),
        "pictureHeight": i.picture_height(),
        "hyperlink": i.hyperlink_url(),
        "background": i.is_background(),
        "webPicture": i.web_picture().map(|p| json!({"extension": p.extension(), "size": p.size()})),
    })
}

fn embedded_file(f: &EmbeddedFile) -> Value {
    json!({
        "kind": "embeddedFile",
        "offset": [f.offset_horizontal(), f.offset_vertical()],
        "filename": f.filename(),
        "size": f.size(),
        "type": format!("{:?}", f.file_type()),
        "status": format!("{:?}", f.data_status()),
    })
}

fn ink(i: &Ink) -> Value {
    json!({
        "kind": "ink",
        "offset": [i.offset_horizontal(), i.offset_vertical()],
        "box": i.bounding_box().map(|b| [b.x(), b.y(), b.width(), b.height()]),
        "strokes": i.ink_strokes().iter().map(stroke).collect::<Vec<_>>(),
        "groups": i.child_groups().iter().map(ink).collect::<Vec<_>>(),
    })
}

fn stroke(s: &InkStroke) -> Value {
    let path = s.absolute_path();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &path {
        min_x = min_x.min(p.x());
        min_y = min_y.min(p.y());
        max_x = max_x.max(p.x());
        max_y = max_y.max(p.y());
    }
    json!({
        "points": path.len(),
        "bounds": if path.is_empty() { Value::Null } else { json!([min_x, min_y, max_x, max_y]) },
        "width": s.width(),
        "height": s.height(),
        "color": s.color().map(|c| format!("{:#08X}", c)),
        "transparency": s.transparency(),
        "penTip": s.pen_tip(),
        "word": s.recognized_word().and_then(|w| w.text()),
    })
}

fn color(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn color_ref(c: ColorRef) -> Value {
    match c {
        ColorRef::Auto => json!("auto"),
        ColorRef::Manual { r, g, b } => json!(format!("#{r:02x}{g:02x}{b:02x}")),
    }
}
