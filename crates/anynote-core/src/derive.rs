//! Data derived from a page body for indexing: plain text, outgoing wiki
//! links, tasks and `#tags`. Recomputed whenever a page changes.

use std::collections::BTreeSet;

use crate::node::{kinds, Node};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Derived {
    pub text: String,
    pub links: BTreeSet<String>,
    pub tasks: Vec<Task>,
    pub tags: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub block_id: String,
    pub text: String,
    pub checked: bool,
}

pub fn derive(doc: &Node) -> Derived {
    let text = doc.plain_text();
    let mut links = BTreeSet::new();
    let mut tasks = Vec::new();
    doc.walk(&mut |n| {
        if n.is(kinds::WIKI_LINK) {
            if let Some(id) = n.attr_str("pageId").filter(|s| !s.is_empty()) {
                links.insert(id.to_string());
            }
        } else if n.is(kinds::TASK_ITEM) {
            // The task's own text is its first paragraph; nested items are separate tasks.
            let text = n.content.iter().find(|c| c.is(kinds::PARAGRAPH)).map(Node::plain_text).unwrap_or_default();
            tasks.push(Task {
                block_id: n.attr_str("id").unwrap_or_default().to_string(),
                text,
                checked: n.attr_bool("checked"),
            });
        }
    });
    // Code blocks are not prose: `#include` is not a tag.
    let mut prose = String::new();
    collect_prose(doc, &mut prose);
    let tags = extract_tags(&prose);
    Derived { text, links, tasks, tags }
}

fn collect_prose(node: &Node, out: &mut String) {
    if node.is(kinds::CODE_BLOCK) {
        return;
    }
    if node.is_text() && !node.has_mark(crate::node::marks::CODE) {
        out.push_str(node.text.as_deref().unwrap_or(""));
    }
    for child in &node.content {
        collect_prose(child, out);
    }
    if !node.is_inline() {
        out.push('\n');
    }
}

/// `#tag`, `#multi-word_tag`, `#area/sub`. A tag must follow whitespace or
/// the start of the text and must contain at least one non-digit.
pub fn extract_tags(text: &str) -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' && (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(') {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && (chars[end].is_alphanumeric() || matches!(chars[end], '_' | '-' | '/')) {
                end += 1;
            }
            let tag: String = chars[start..end].iter().collect();
            let tag = tag.trim_end_matches(['-', '/']).to_lowercase();
            if !tag.is_empty() && !tag.chars().all(|c| c.is_ascii_digit()) {
                tags.insert(tag);
            }
            i = end;
        } else {
            i += 1;
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Mark;

    #[test]
    fn finds_tags() {
        let tags = extract_tags("Meeting #Work and #project/alpha, not a#tag, not #123 (#paren)");
        let tags: Vec<_> = tags.into_iter().collect();
        assert_eq!(tags, vec!["paren", "project/alpha", "work"]);
    }

    #[test]
    fn derives_links_tasks_tags() {
        let doc = Node::doc(vec![
            Node::paragraph(vec![
                Node::text("see "),
                Node::new(kinds::WIKI_LINK).with_attr("pageId", "p2").with_attr("title", "Other"),
                Node::text(" #idea "),
                Node::styled_text("#notatag", vec![Mark::new("code")]),
            ]),
            Node::element(
                kinds::TASK_LIST,
                vec![Node::element(kinds::TASK_ITEM, vec![Node::paragraph(vec![Node::text("Buy milk")])])
                    .with_attr("checked", true)
                    .with_attr("id", "t1")],
            ),
            Node::element(kinds::CODE_BLOCK, vec![Node::text("#include <stdio.h>")]),
        ]);
        let d = derive(&doc);
        assert_eq!(d.links.into_iter().collect::<Vec<_>>(), vec!["p2"]);
        assert_eq!(d.tasks, vec![Task { block_id: "t1".into(), text: "Buy milk".into(), checked: true }]);
        assert_eq!(d.tags.into_iter().collect::<Vec<_>>(), vec!["idea"]);
        assert!(d.text.contains("Buy milk"));
    }
}
