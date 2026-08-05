//! The Markdown front end: CommonMark (pulldown-cmark) → Semantic Graph.
//!
//! Mapping notes (scope boundaries, documented in DESIGN.md):
//! - Strikethrough/tasklists are not enabled: per CommonMark defaults, `~~` and
//!   `[ ]` render as literal text. The v1 format has no strikethrough or task
//!   semantics; this is an explicit boundary, not a bug.
//! - Footnotes are not enabled.
//! - Block-level raw HTML is skipped (the HTML front end owns HTML).
//! - Inline raw HTML is preserved as literal text.
//! - Tight lists are normalized: every list item's inline content becomes a
//!   `Paragraph` child (semantically equivalent to CommonMark's tight rendering).
//! - Code content is NFC-normalized like all text (the format mandates NFC).

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::model::{SemanticKind, SemanticNode, MAX_CHILDREN, MAX_DEPTH};

/// Errors produced by the Markdown front end.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Error {
    /// Nesting exceeded [`MAX_DEPTH`].
    TooDeep,
    /// A node exceeded [`MAX_CHILDREN`] children.
    TooManyChildren,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooDeep => write!(f, "document nested too deeply"),
            Self::TooManyChildren => write!(f, "document has too many siblings"),
        }
    }
}

impl std::error::Error for Error {}

/// Parse Markdown into a semantic tree.
pub fn parse_markdown(source: &str) -> Result<SemanticNode, Error> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(source, options);

    let mut stack: Vec<SemanticNode> = vec![SemanticNode::structural(SemanticKind::Document)];
    let mut pending_text = String::new();
    let mut code_buffer: Option<String> = None; // verbatim text while inside CodeBlock
    let mut image_alt: Option<String> = None; // alt text while inside Image

    for event in parser {
        match event {
            Event::Start(tag) => {
                flush_text(&mut stack, &mut pending_text)?;
                match tag {
                    Tag::Paragraph => push(&mut stack, SemanticKind::Paragraph)?,
                    Tag::Heading { level, .. } => {
                        let level = match level {
                            HeadingLevel::H1 => 1,
                            HeadingLevel::H2 => 2,
                            HeadingLevel::H3 => 3,
                            HeadingLevel::H4 => 4,
                            HeadingLevel::H5 => 5,
                            HeadingLevel::H6 => 6,
                        };
                        push(&mut stack, SemanticKind::Heading(level))?;
                    }
                    Tag::BlockQuote(_) => push(&mut stack, SemanticKind::Quote)?,
                    Tag::CodeBlock(kind) => {
                        let _ = kind; // language info is not part of the v1 format
                        push(&mut stack, SemanticKind::CodeBlock)?;
                        code_buffer = Some(String::new());
                    }
                    Tag::List(start) => {
                        let kind = if start.is_some() {
                            SemanticKind::OrderedList
                        } else {
                            SemanticKind::UnorderedList
                        };
                        let mut node = SemanticNode::structural(kind);
                        node.list_start = start;
                        push_node(&mut stack, node)?;
                    }
                    Tag::Item => push(&mut stack, SemanticKind::ListItem)?,
                    Tag::Table(_) => push(&mut stack, SemanticKind::Table)?,
                    Tag::TableHead => push(&mut stack, SemanticKind::TableRow)?,
                    Tag::TableRow => push(&mut stack, SemanticKind::TableRow)?,
                    Tag::TableCell => push(&mut stack, SemanticKind::TableCell)?,
                    Tag::Link { dest_url, .. } => {
                        let mut node = SemanticNode::structural(SemanticKind::Link);
                        node.href = Some(dest_url.to_string());
                        push_node(&mut stack, node)?;
                    }
                    Tag::Image { dest_url, .. } => {
                        let mut node = SemanticNode::structural(SemanticKind::Image);
                        node.image_src = Some(dest_url.to_string());
                        push_node(&mut stack, node)?;
                        image_alt = Some(String::new());
                    }
                    Tag::Emphasis => push(&mut stack, SemanticKind::Emphasis)?,
                    Tag::Strong => push(&mut stack, SemanticKind::Strong)?,
                    other => {
                        // Unknown tags (e.g. Strikethrough when enabled by callers):
                        // treated as transparent containers so their content is kept.
                        let mut node = SemanticNode::structural(SemanticKind::Transparent);
                        node.hints.inline = Some(format!("{other:?}"));
                        push_node(&mut stack, node)?;
                    }
                }
            }
            Event::End(tag) => {
                flush_text(&mut stack, &mut pending_text)?;
                if let Some(buf) = &mut code_buffer {
                    if matches!(tag, TagEnd::CodeBlock) {
                        let verbatim = std::mem::take(buf);
                        let top = stack.last_mut().ok_or(Error::TooDeep)?;
                        top.text = crate::model::normalize_nfc(&verbatim);
                        code_buffer = None;
                    }
                }
                if let Some(alt) = &mut image_alt {
                    if matches!(tag, TagEnd::Image) {
                        let alt_text = std::mem::take(alt);
                        let top = stack.last_mut().ok_or(Error::TooDeep)?;
                        top.alt = Some(alt_text);
                        image_alt = None;
                    }
                }
                finish_node(&mut stack)?;
            }
            Event::Text(text) => {
                if let Some(buf) = &mut code_buffer {
                    buf.push_str(&text);
                } else if let Some(alt) = &mut image_alt {
                    alt.push_str(&text);
                } else {
                    pending_text.push_str(&text);
                }
            }
            Event::SoftBreak => {
                if image_alt.is_some() || code_buffer.is_some() {
                    // treated as literal text
                    pending_text.push(' ');
                } else {
                    pending_text.push(' ');
                }
            }
            Event::HardBreak => {
                flush_text(&mut stack, &mut pending_text)?;
                let top = stack.last_mut().ok_or(Error::TooDeep)?;
                if top.children.len() >= MAX_CHILDREN {
                    return Err(Error::TooManyChildren);
                }
                top.children
                    .push(SemanticNode::structural(SemanticKind::HardBreak));
            }
            Event::Rule => {
                flush_text(&mut stack, &mut pending_text)?;
                let top = stack.last_mut().ok_or(Error::TooDeep)?;
                if top.children.len() >= MAX_CHILDREN {
                    return Err(Error::TooManyChildren);
                }
                top.children
                    .push(SemanticNode::structural(SemanticKind::Rule));
            }
            Event::InlineHtml(html) => pending_text.push_str(&html),
            Event::Html(_) => {} // block HTML: skipped (the HTML front end owns HTML)
            Event::FootnoteReference(_) => {} // footnotes not enabled
            Event::TaskListMarker(_) => {} // tasklists not enabled
            Event::Code(code) => {
                flush_text(&mut stack, &mut pending_text)?;
                let top = stack.last_mut().ok_or(Error::TooDeep)?;
                if top.children.len() >= MAX_CHILDREN {
                    return Err(Error::TooManyChildren);
                }
                top.children.push(SemanticNode::text(
                    SemanticKind::InlineCode,
                    code.to_string(),
                ));
            }
            _ => {}
        }
    }

    // The stack must collapse to exactly the root.
    while stack.len() > 1 {
        flush_text(&mut stack, &mut pending_text)?;
        finish_node(&mut stack)?;
    }
    flush_text(&mut stack, &mut pending_text)?;

    let mut root = stack.pop().ok_or(Error::TooDeep)?;
    debug_assert!(stack.is_empty());
    SemanticNode::normalize_tree(&mut root);
    crate::model::cleanup_tree(&mut root);
    crate::model::hoist_inline_images(&mut root);
    Ok(root)
}

fn push(stack: &mut Vec<SemanticNode>, kind: SemanticKind) -> Result<(), Error> {
    push_node(stack, SemanticNode::structural(kind))
}

fn push_node(stack: &mut Vec<SemanticNode>, node: SemanticNode) -> Result<(), Error> {
    if stack.len() >= MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    stack.push(node);
    Ok(())
}

/// Attach accumulated inline text to the current node, then pop the node onto its
/// parent.
fn finish_node(stack: &mut Vec<SemanticNode>) -> Result<(), Error> {
    let node = stack.pop().ok_or(Error::TooDeep)?;
    let parent = stack.last_mut().ok_or(Error::TooDeep)?;
    if parent.children.len() >= MAX_CHILDREN {
        return Err(Error::TooManyChildren);
    }
    parent.children.push(node);
    Ok(())
}

/// Attach accumulated inline text to the current node as a transparent text child
/// (the unified inline model: all inline content lives in children; block nodes
/// carry no direct text).
fn flush_text(stack: &mut [SemanticNode], pending: &mut String) -> Result<(), Error> {
    if pending.is_empty() {
        return Ok(());
    }
    let top = stack.last_mut().ok_or(Error::TooDeep)?;
    if top.children.len() >= MAX_CHILDREN {
        return Err(Error::TooManyChildren);
    }
    let text = std::mem::take(pending);
    top.children
        .push(SemanticNode::text(SemanticKind::Transparent, text));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_markdown;
    use crate::model::{validate_tree, SemanticKind};

    fn kinds(node: &crate::model::SemanticNode) -> Vec<SemanticKind> {
        let mut out = Vec::new();
        node.visit(&mut |n| out.push(n.kind));
        out
    }

    #[test]
    fn headings_and_paragraphs() {
        let root = parse_markdown("# Title\n\nSome *emphasis* text.\n").expect("parse");
        assert!(validate_tree(&root).is_empty());
        let ks = kinds(&root);
        assert_eq!(
            ks,
            vec![
                SemanticKind::Document,
                SemanticKind::Heading(1),
                SemanticKind::Transparent, // plain text inside the heading
                SemanticKind::Paragraph,
                SemanticKind::Transparent,
                SemanticKind::Emphasis,
                SemanticKind::Transparent, // text inside the emphasis
                SemanticKind::Transparent,
            ]
        );
        assert_eq!(root.children[0].children[0].text, "Title");
        let para = &root.children[1];
        assert_eq!(para.to_string(), "Some emphasis text.");
        assert_eq!(para.children[1].kind, SemanticKind::Emphasis);
    }

    #[test]
    fn lists_nested() {
        let root = parse_markdown("- a\n- b\n  - c\n").expect("parse");
        assert!(validate_tree(&root).is_empty());
        let list = &root.children[0];
        assert_eq!(list.kind, SemanticKind::UnorderedList);
        assert_eq!(list.children.len(), 2);
        assert!(list.children[1]
            .children
            .iter()
            .any(|c| c.kind == SemanticKind::UnorderedList));
    }

    #[test]
    fn ordered_list_start() {
        let root = parse_markdown("3. third\n4. fourth\n").expect("parse");
        let list = &root.children[0];
        assert_eq!(list.kind, SemanticKind::OrderedList);
        // Explicit list start values are preserved through the item order; the
        // runtime derives markers from position. The semantic graph records order.
        assert_eq!(list.children.len(), 2);
    }

    #[test]
    fn tables() {
        let src = "| a | b |\n|---|---|\n| 1 | 2 |\n";
        let root = parse_markdown(src).expect("parse");
        assert!(validate_tree(&root).is_empty());
        let table = &root.children[0];
        assert_eq!(table.kind, SemanticKind::Table);
        assert_eq!(table.children.len(), 2); // header row + body row
        assert!(table.children[0]
            .children
            .iter()
            .all(|c| c.kind == SemanticKind::TableCell));
    }

    #[test]
    fn code_block_verbatim() {
        let root = parse_markdown("```rust\nfn main() {}\n```\n").expect("parse");
        let code = &root.children[0];
        assert_eq!(code.kind, SemanticKind::CodeBlock);
        assert!(code.text.contains("fn main()"));
    }

    #[test]
    fn links_and_images() {
        let root = parse_markdown("[text](https://e.test) and ![alt](img.png)\n").expect("parse");
        let para = &root.children[0];
        assert_eq!(para.kind, SemanticKind::Paragraph);
        assert_eq!(para.children[0].kind, SemanticKind::Link);
        assert_eq!(para.children[0].href.as_deref(), Some("https://e.test"));
        // The inline image is hoisted out of the paragraph (block-level image).
        assert_eq!(root.children[1].kind, SemanticKind::Image);
        assert_eq!(root.children[1].image_src.as_deref(), Some("img.png"));
        assert_eq!(root.children[1].alt.as_deref(), Some("alt"));
    }

    #[test]
    fn hard_breaks_and_rules() {
        let root = parse_markdown("a\\\nb\n\n---\n").expect("parse");
        assert!(root.children[0]
            .children
            .iter()
            .any(|c| c.kind == SemanticKind::HardBreak));
        assert!(root.children.iter().any(|c| c.kind == SemanticKind::Rule));
    }

    #[test]
    fn blockquote() {
        let root = parse_markdown("> quoted\n").expect("parse");
        assert_eq!(root.children[0].kind, SemanticKind::Quote);
        assert_eq!(root.children[0].children[0].kind, SemanticKind::Paragraph);
    }

    #[test]
    fn inline_code() {
        let root = parse_markdown("use `std::fs`;\n").expect("parse");
        let para = &root.children[0];
        assert!(para
            .children
            .iter()
            .any(|c| c.kind == SemanticKind::InlineCode));
    }

    #[test]
    fn deterministic() {
        let src = "# T\n\nbody *x*.\n";
        assert_eq!(
            parse_markdown(src).expect("a"),
            parse_markdown(src).expect("b")
        );
    }

    #[test]
    fn empty_document() {
        let root = parse_markdown("").expect("parse");
        assert_eq!(root.kind, SemanticKind::Document);
        assert!(root.children.is_empty());
    }

    #[test]
    fn strikethrough_is_literal() {
        // Strikethrough is not enabled: tildes stay literal (CommonMark default).
        let root = parse_markdown("~~nope~~\n").expect("parse");
        assert!(root.children[0].to_string().contains("~~"));
    }

    #[test]
    fn whitespace_keeps_source_ownership() {
        // The separator space stays in the leaf that owned it: for
        // `Hello *world*` the space is in the plain text run (regular weight),
        // exactly like CSS/browser rendering.
        let a = parse_markdown("Hello *world*\n").expect("parse");
        let para_a = &a.children[0];
        assert_eq!(para_a.children[0].text, "Hello ");
        assert_eq!(para_a.children[1].kind, SemanticKind::Emphasis);
        assert_eq!(para_a.children[1].children[0].text, "world");
        assert_eq!(para_a.to_string(), "Hello world");
    }

    #[test]
    fn trailing_whitespace_trimmed_at_block_end() {
        let root = parse_markdown("Some *emphasis* text.  \n").expect("parse");
        let para = &root.children[0];
        assert_eq!(para.to_string(), "Some emphasis text.");
        // The last run has no trailing space.
        let last = para.children.last().expect("last child");
        assert!(!last.text.ends_with(' '));
    }

    #[test]
    fn inline_image_hoisted_and_split() {
        let root = parse_markdown("text ![alt](img.png) more\n").expect("parse");
        assert!(validate_tree(&root).is_empty());
        // The paragraph splits around the hoisted (block-level) image.
        assert_eq!(root.children.len(), 3);
        assert_eq!(root.children[0].kind, SemanticKind::Paragraph);
        assert_eq!(root.children[0].to_string(), "text");
        assert_eq!(root.children[1].kind, SemanticKind::Image);
        assert_eq!(root.children[1].image_src.as_deref(), Some("img.png"));
        assert_eq!(root.children[2].kind, SemanticKind::Paragraph);
        assert_eq!(root.children[2].to_string(), "more");
    }

    #[test]
    fn image_link_target_transfers() {
        let root = parse_markdown("[![alt](img.png)](https://e.test)\n").expect("parse");
        assert!(validate_tree(&root).is_empty());
        let img = &root.children[0];
        assert_eq!(img.kind, SemanticKind::Image);
        assert_eq!(img.image_src.as_deref(), Some("img.png"));
        assert_eq!(img.href.as_deref(), Some("https://e.test"));
    }

    #[test]
    fn hard_break_survives_cleanup() {
        let root = parse_markdown("a\\\nb\n").expect("parse");
        let para = &root.children[0];
        assert!(para
            .children
            .iter()
            .any(|c| c.kind == SemanticKind::HardBreak));
    }
}
