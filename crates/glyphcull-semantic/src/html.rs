//! The HTML5 front end: html5ever (spec-compliant parser) → Semantic Graph.
//!
//! Mapping notes:
//! - Presentational tags are canonicalized to semantics (`b`→`Strong`, `i`→`Emphasis`).
//! - Generic containers (`div`, `span`, `section`, `article`, ...) become
//!   [`SemanticKind::Transparent`] nodes: no chunk of their own, children spliced,
//!   but their classes/ids remain for CSS matching.
//! - Form/interactive elements (`input`, `button`, `form`, ...), `script`,
//!   `noscript`, and `head` metadata are dropped (they are not document prose).
//! - `<style>` blocks are extracted into [`FrontEndOutput::stylesheets`];
//!   `<title>` and `<html lang>` become package metadata.
//! - Text is whitespace-collapsed (HTML rules: runs → one space, edges trimmed at
//!   block boundaries), except inside `<pre>`/`<code>` which stay verbatim. All
//!   text is NFC-normalized (the format boundary).
//! - Colspan/rowspan from table cells are preserved.

use html5ever::interface::tree_builder::TreeSink;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;

use crate::dom::{MinimalDom, NodeKind};
use crate::model::{
    normalize_nfc, SemanticKind, SemanticNode, StyleHints, MAX_CHILDREN, MAX_DEPTH,
};

/// Errors produced by the HTML front end.
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

/// The output of a front end: the semantic tree plus document metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct FrontEndOutput {
    /// The semantic tree.
    pub tree: SemanticNode,
    /// Document title (from `<title>`), if any.
    pub title: Option<String>,
    /// Document language (from `<html lang>`), if any.
    pub lang: Option<String>,
    /// CSS text extracted from `<style>` blocks, in document order.
    pub stylesheets: Vec<String>,
}

/// Parse an HTML document into a semantic tree.
pub fn parse_html(source: &str) -> Result<FrontEndOutput, Error> {
    let dom = parse_document(MinimalDom::new(), Default::default())
        .from_utf8()
        .one(source.as_bytes());
    let mut builder = Builder {
        depth: 0,
        title: None,
        lang: None,
        stylesheets: Vec::new(),
    };
    let document = dom.get_document();
    let children = builder.build_children(&dom, document)?;
    let mut tree = SemanticNode::structural(SemanticKind::Document).with_children(children);
    flatten_hintless_transparent(&mut tree);
    crate::model::cleanup_tree(&mut tree);
    crate::model::hoist_inline_images(&mut tree);
    Ok(FrontEndOutput {
        tree,
        title: builder.title,
        lang: builder.lang,
        stylesheets: builder.stylesheets,
    })
}

/// Splice hint-less transparent containers (pure wrappers like `body` or a `div`
/// without classes/ids/styles) into their parent, so they produce no structure.
/// Transparent containers *with* style hints survive: the style resolver needs
/// them as ancestors for descendant-selector matching.
fn flatten_hintless_transparent(node: &mut SemanticNode) {
    let mut out: Vec<SemanticNode> = Vec::new();
    for mut child in std::mem::take(&mut node.children) {
        flatten_hintless_transparent(&mut child);
        let hintless = child.kind == SemanticKind::Transparent
            && child.hints.classes.is_empty()
            && child.hints.id.is_none()
            && child.hints.inline.is_none()
            && child.text.is_empty();
        if hintless {
            out.extend(child.children);
        } else {
            out.push(child);
        }
    }
    node.children = out;
}

struct Builder {
    depth: usize,
    title: Option<String>,
    lang: Option<String>,
    stylesheets: Vec<String>,
}

impl Builder {
    fn build_children(
        &mut self,
        dom: &MinimalDom,
        handle: usize,
    ) -> Result<Vec<SemanticNode>, Error> {
        let mut out = Vec::new();
        let children: Vec<usize> = dom.node(handle).children.clone();
        for child in children {
            let built = self.build_child(dom, child)?;
            if out.len() + built.len() > MAX_CHILDREN {
                return Err(Error::TooManyChildren);
            }
            out.extend(built);
        }
        Ok(out)
    }

    fn build_child(&mut self, dom: &MinimalDom, handle: usize) -> Result<Vec<SemanticNode>, Error> {
        if self.depth >= MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        self.depth += 1;
        let result = self.build_child_inner(dom, handle);
        self.depth -= 1;
        result
    }

    fn build_child_inner(
        &mut self,
        dom: &MinimalDom,
        handle: usize,
    ) -> Result<Vec<SemanticNode>, Error> {
        let node = dom.node(handle);
        match &node.kind {
            NodeKind::Document => {
                let children = node.children.clone();
                drop(node);
                let mut out = Vec::new();
                for child in children {
                    out.extend(self.build_child(dom, child)?);
                }
                Ok(out)
            }
            NodeKind::Text { contents } => {
                let collapsed = collapse_ws(contents);
                if collapsed.is_empty() {
                    return Ok(Vec::new());
                }
                Ok(vec![SemanticNode::text(
                    SemanticKind::Transparent,
                    collapsed,
                )])
            }
            NodeKind::Element { name, attrs, .. } => {
                let tag = name.local.as_ref().to_string();
                let attrs: Vec<(String, String)> = attrs
                    .iter()
                    .map(|a| (a.name.local.as_ref().to_string(), a.value.to_string()))
                    .collect();
                drop(node);
                self.build_element(&tag, &attrs, dom, handle)
            }
            NodeKind::Comment { .. } | NodeKind::Doctype { .. } | NodeKind::Pi { .. } => {
                Ok(Vec::new())
            }
        }
    }

    fn build_element(
        &mut self,
        tag: &str,
        attrs: &[(String, String)],
        dom: &MinimalDom,
        handle: usize,
    ) -> Result<Vec<SemanticNode>, Error> {
        // Metadata that never becomes content.
        match tag {
            "title" => {
                let text = element_text(dom, handle);
                if !text.trim().is_empty() {
                    self.title = Some(normalize_nfc(text.trim()));
                }
                return Ok(Vec::new());
            }
            "style" => {
                self.stylesheets.push(element_text(dom, handle));
                return Ok(Vec::new());
            }
            "html" => {
                if let Some(lang) = attr(attrs, "lang") {
                    self.lang = Some(lang.to_string());
                }
            }
            _ => {}
        }

        // Elements whose content is not document prose.
        if is_dropped(tag) {
            return Ok(Vec::new());
        }

        // `<head>` contributes only metadata (title/style), extracted above;
        // its other children are dropped.
        if tag == "head" {
            let children = dom.node(handle).children.clone();
            for child in children {
                let _ = self.build_child(dom, child)?;
            }
            return Ok(Vec::new());
        }

        let hints = hints(attrs);
        let children = self.build_children(dom, handle)?;

        let kind = match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                // The level is the digit in the tag name (h1..h6 by construction).
                let level = tag.as_bytes().get(1).copied().map_or(1, |b| b - b'0');
                SemanticKind::Heading(level)
            }
            "p" => SemanticKind::Paragraph,
            "blockquote" => SemanticKind::Quote,
            "ul" => SemanticKind::UnorderedList,
            "ol" => SemanticKind::OrderedList,
            "li" => SemanticKind::ListItem,
            "pre" => SemanticKind::CodeBlock,
            "table" => SemanticKind::Table,
            "thead" | "tbody" | "tfoot" | "colgroup" | "figure" | "div" | "span" | "section"
            | "article" | "main" | "header" | "footer" | "nav" | "aside" | "address"
            | "details" | "summary" | "mark" | "u" | "s" | "small" | "sub" | "sup" | "time"
            | "abbr" | "q" | "cite" | "kbd" | "samp" | "var" | "dfn" | "del" | "ins" | "wbr"
            | "ruby" | "rt" | "rp" | "bdi" | "bdo" | "data" | "output" | "progress" | "meter"
            | "fieldset" | "legend" | "label" | "picture" | "source" => SemanticKind::Transparent,
            "tr" => SemanticKind::TableRow,
            "th" | "td" => SemanticKind::TableCell,
            "img" => {
                let mut node = SemanticNode::structural(SemanticKind::Image);
                node.image_src = attr(attrs, "src").map(str::to_string);
                node.alt = attr(attrs, "alt").map(normalize_nfc);
                node.hints = hints;
                return Ok(vec![node]);
            }
            "figcaption" => SemanticKind::Caption,
            "a" => SemanticKind::Link,
            "em" | "i" => SemanticKind::Emphasis,
            "strong" | "b" => SemanticKind::Strong,
            "code" => SemanticKind::InlineCode,
            "br" => return Ok(vec![SemanticNode::structural(SemanticKind::HardBreak)]),
            "hr" => return Ok(vec![SemanticNode::structural(SemanticKind::Rule)]),
            _ => SemanticKind::Transparent,
        };

        let mut node = SemanticNode::structural(kind).with_hints(hints);
        node.children = children;

        if kind == SemanticKind::CodeBlock {
            // Verbatim text: the block's text is the concatenated descendant text.
            node.text = normalize_nfc(&element_text(dom, handle));
            node.children.clear();
            return Ok(vec![node]);
        }

        // Inline links carry their target; cell spans are preserved.
        if kind == SemanticKind::Link {
            node.href = attr(attrs, "href").map(str::to_string);
        }
        if kind == SemanticKind::TableCell {
            node.colspan = attr(attrs, "colspan")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1)
                .max(1);
            node.rowspan = attr(attrs, "rowspan")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1)
                .max(1);
        }
        if kind == SemanticKind::OrderedList {
            node.list_start = attr(attrs, "start").and_then(|v| v.parse().ok());
        }

        Ok(vec![node])
    }
}

/// Elements whose subtree is not document prose (dropped entirely).
fn is_dropped(tag: &str) -> bool {
    matches!(
        tag,
        "script"
            | "noscript"
            | "meta"
            | "link"
            | "base"
            | "form"
            | "input"
            | "button"
            | "select"
            | "textarea"
            | "option"
            | "iframe"
            | "svg"
            | "canvas"
            | "audio"
            | "video"
            | "embed"
            | "object"
            | "template"
            | "dialog"
            | "math"
    )
}

/// Collapse whitespace runs to a single space (HTML text rule). Edge spaces are
/// preserved; block-edge trimming happens in `cleanup_tree`.
fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    if pending_space {
        out.push(' ');
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn hints(attrs: &[(String, String)]) -> StyleHints {
    let mut classes: Vec<String> = attr(attrs, "class")
        .map(|s| s.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut seen = std::collections::BTreeSet::new();
    classes.retain(|c| seen.insert(c.clone()));
    StyleHints {
        classes,
        id: attr(attrs, "id").map(str::to_string),
        inline: attr(attrs, "style").map(str::to_string),
        hidden: attr(attrs, "hidden").is_some(),
    }
}

/// The concatenated text of an element's subtree (used for `<pre>`, `<title>`,
/// and `<style>` where verbatim extraction is required).
fn element_text(dom: &MinimalDom, handle: usize) -> String {
    fn walk(dom: &MinimalDom, handle: usize, out: &mut String) {
        let node = dom.node(handle);
        match &node.kind {
            NodeKind::Text { contents } => out.push_str(contents),
            NodeKind::Element { .. } | NodeKind::Document => {
                let children = node.children.clone();
                drop(node);
                for child in children {
                    walk(dom, child, out);
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    walk(dom, handle, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::parse_html;
    use crate::model::{validate_tree, SemanticKind};

    #[test]
    fn basic_document() {
        let out = parse_html(
            "<!doctype html><html lang=\"en\"><head><title>T</title></head><body><h1>Hi</h1><p>Body <em>text</em>.</p></body></html>",
        )
        .expect("parse");
        assert!(validate_tree(&out.tree).is_empty());
        assert_eq!(out.title.as_deref(), Some("T"));
        assert_eq!(out.lang.as_deref(), Some("en"));
        let root = &out.tree;
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].kind, SemanticKind::Heading(1));
        assert_eq!(root.children[0].to_string(), "Hi");
        let para = &root.children[1];
        assert_eq!(para.kind, SemanticKind::Paragraph);
        assert_eq!(para.to_string(), "Body text.");
        assert!(para
            .children
            .iter()
            .any(|c| c.kind == SemanticKind::Emphasis));
    }

    #[test]
    fn style_extracted() {
        let out = parse_html("<style>p { color: red; }</style><p>x</p>").expect("parse");
        assert_eq!(out.stylesheets.len(), 1);
        assert!(out.stylesheets[0].contains("color: red"));
    }

    #[test]
    fn table_spans() {
        let out = parse_html("<table><tr><td colspan=\"2\" rowspan=\"3\">x</td></tr></table>")
            .expect("parse");
        let table = &out.tree.children[0];
        assert_eq!(table.kind, SemanticKind::Table);
        let cell = &table.children[0].children[0];
        assert_eq!(cell.kind, SemanticKind::TableCell);
        assert_eq!(cell.colspan, 2);
        assert_eq!(cell.rowspan, 3);
    }

    #[test]
    fn whitespace_collapse_and_trim() {
        let out = parse_html("<p>  a\n\n   b  <em>  c  </em>  </p>").expect("parse");
        let para = &out.tree.children[0];
        assert_eq!(para.to_string(), "a b c");
    }

    #[test]
    fn whitespace_keeps_style_ownership() {
        // The separator keeps the style of the text node that contained it:
        // here the space after `Hello` is inside the emphasis, so it renders
        // italic, exactly like browsers.
        let out = parse_html("<p>Hello<em> world</em></p>").expect("parse");
        let para = &out.tree.children[0];
        assert_eq!(para.children[0].text, "Hello");
        assert_eq!(para.children[1].kind, SemanticKind::Emphasis);
        assert_eq!(para.children[1].children[0].text, " world");
        assert_eq!(para.to_string(), "Hello world");
    }

    #[test]
    fn hint_bearing_span_keeps_content() {
        // A styled span is a transparent container that must survive cleanup
        // with its children (regression: empty-text containers were dropped).
        let out = parse_html(r#"<p>before <span class="x">hi</span> after</p>"#).expect("parse");
        let para = &out.tree.children[0];
        assert_eq!(para.to_string(), "before hi after");
        let span = para
            .children
            .iter()
            .find(|c| c.kind == SemanticKind::Transparent && !c.hints.classes.is_empty())
            .expect("span");
        assert_eq!(span.hints.classes, vec!["x"]);
        assert_eq!(span.children.len(), 1);
        assert_eq!(span.children[0].text, "hi");
    }

    #[test]
    fn inline_image_hoisted_from_paragraph() {
        let out = parse_html(r#"<p>a <img src="x.png" alt="pic"> b</p>"#).expect("parse");
        let children = &out.tree.children;
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].kind, SemanticKind::Paragraph);
        assert_eq!(children[0].to_string(), "a");
        assert_eq!(children[1].kind, SemanticKind::Image);
        assert_eq!(children[1].image_src.as_deref(), Some("x.png"));
        assert_eq!(children[2].kind, SemanticKind::Paragraph);
        assert_eq!(children[2].to_string(), "b");
    }

    #[test]
    fn figure_keeps_image_and_caption() {
        let out = parse_html(
            "<figure><img src=\"a.png\" alt=\"pic\"><figcaption>Cap</figcaption></figure>",
        )
        .expect("parse");
        // The hintless figure wrapper is spliced; the image and caption become
        // block-level siblings.
        let children = &out.tree.children;
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].kind, SemanticKind::Image);
        assert_eq!(children[0].image_src.as_deref(), Some("a.png"));
        assert_eq!(children[1].kind, SemanticKind::Caption);
        assert_eq!(children[1].to_string(), "Cap");
    }

    #[test]
    fn image_link_target_preserved() {
        let out = parse_html(r#"<p><a href="https://e.test"><img src="x.png" alt="pic"></a></p>"#)
            .expect("parse");
        let img = &out.tree.children[0];
        assert_eq!(img.kind, SemanticKind::Image);
        assert_eq!(img.href.as_deref(), Some("https://e.test"));
    }

    #[test]
    fn pre_verbatim() {
        let out = parse_html("<pre>  keep   spaces  \n  </pre>").expect("parse");
        let pre = &out.tree.children[0];
        assert_eq!(pre.kind, SemanticKind::CodeBlock);
        assert!(pre.text.contains("  keep   spaces"));
    }

    #[test]
    fn script_and_forms_dropped() {
        let out =
            parse_html("<script>var x = 1;</script><form><input value=\"x\"></form><p>keep</p>")
                .expect("parse");
        assert_eq!(out.tree.children.len(), 1);
        assert_eq!(out.tree.children[0].kind, SemanticKind::Paragraph);
    }

    #[test]
    fn img_and_figcaption() {
        let out = parse_html(
            "<figure><img src=\"a.png\" alt=\"pic\"><figcaption>Cap</figcaption></figure>",
        )
        .expect("parse");
        let children = &out.tree.children;
        assert!(children.iter().any(|c| c.kind == SemanticKind::Image));
        assert!(children.iter().any(|c| c.kind == SemanticKind::Caption));
        let img = children
            .iter()
            .find(|c| c.kind == SemanticKind::Image)
            .expect("img");
        assert_eq!(img.image_src.as_deref(), Some("a.png"));
        assert_eq!(img.alt.as_deref(), Some("pic"));
    }

    #[test]
    fn classes_hints() {
        let out = parse_html("<p class=\"note intro\" id=\"p1\">x</p>").expect("parse");
        let para = &out.tree.children[0];
        assert_eq!(
            para.hints.classes,
            vec!["note".to_string(), "intro".to_string()]
        );
        assert_eq!(para.hints.id.as_deref(), Some("p1"));
    }

    #[test]
    fn deterministic() {
        let src = "<p>a <b>b</b> c</p>";
        assert_eq!(parse_html(src).expect("a"), parse_html(src).expect("b"));
    }

    #[test]
    fn malformed_html_is_tolerated() {
        let out = parse_html("<p>unclosed <em>and <b>nested</p>").expect("parse");
        assert!(validate_tree(&out.tree).is_empty());
    }

    #[test]
    fn link_href_preserved() {
        let out = parse_html("<p><a href=\"https://e.test\">x</a></p>").expect("parse");
        let link = &out.tree.children[0].children[0];
        assert_eq!(link.kind, SemanticKind::Link);
        assert_eq!(link.href.as_deref(), Some("https://e.test"));
    }
}
