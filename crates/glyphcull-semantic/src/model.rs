//! The Semantic Graph: the compiler's meaning-level intermediate representation.
//!
//! Both front ends (HTML, Markdown) converge on this model. It encodes *what the
//! document means* — headings, paragraphs, lists, tables, quotes, images, links,
//! inline emphasis — never how it is presented. Style is attached as *hints*
//! (classes, ids, inline declarations) that the style resolver (glyphcull-chunk)
//! folds into flat computed styles.
//!
//! Invariants (enforced by the front ends and property-tested):
//! - Single root (`Document`), tree structure, no cycles.
//! - Text is NFC-normalized at the boundary.
//! - Nesting depth is bounded ([`MAX_DEPTH`]) to keep recursion safe.
//! - Child kinds are constrained per parent (see [`allowed_child`]).
//! - The tree is deterministic: same input ⇒ identical tree.

use std::fmt;

use unicode_normalization::UnicodeNormalization;

/// Maximum semantic tree depth (defensive; runaway nesting is a DoS vector).
pub const MAX_DEPTH: usize = 128;

/// Maximum siblings per node (defensive).
pub const MAX_CHILDREN: usize = 1 << 20;

/// The semantic kind of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(missing_docs)]
pub enum SemanticKind {
    /// Root of the document.
    Document,
    /// A heading; level 1..=6.
    Heading(u8),
    /// A paragraph.
    Paragraph,
    /// A block quotation.
    Quote,
    /// An ordered list (marker is derived from the list's start/ordering).
    OrderedList,
    /// An unordered list.
    UnorderedList,
    /// A list item.
    ListItem,
    /// Preformatted code block (text is the verbatim content).
    CodeBlock,
    /// A table (rows are children).
    Table,
    /// A table row.
    TableRow,
    /// A table cell (block content is children; text is plain content).
    TableCell,
    /// An image (alt text; source is `image_src`).
    Image,
    /// A caption (typically child of an image).
    Caption,
    /// An inline link (target is `href`).
    Link,
    /// Inline emphasis (italic).
    Emphasis,
    /// Inline strong (bold).
    Strong,
    /// Inline code (monospace).
    InlineCode,
    /// A soft line break (renders as a space).
    SoftBreak,
    /// A hard line break (renders as a break).
    HardBreak,
    /// A horizontal rule.
    Rule,
    /// A transparent container: no chunk of its own; children are spliced into
    /// the parent's chunk stream. Used for HTML containers (div, span, section)
    /// that exist only to scope styles.
    Transparent,
}

impl SemanticKind {
    /// True for block-level kinds.
    #[must_use]
    pub const fn is_block(self) -> bool {
        matches!(
            self,
            Self::Heading(_)
                | Self::Paragraph
                | Self::Quote
                | Self::OrderedList
                | Self::UnorderedList
                | Self::ListItem
                | Self::CodeBlock
                | Self::Table
                | Self::TableRow
                | Self::TableCell
                | Self::Image
                | Self::Caption
                | Self::Rule
        )
    }

    /// True for inline kinds.
    #[must_use]
    pub const fn is_inline(self) -> bool {
        matches!(
            self,
            Self::Link
                | Self::Emphasis
                | Self::Strong
                | Self::InlineCode
                | Self::SoftBreak
                | Self::HardBreak
        )
    }

    /// A stable, human-readable name (used in diagnostics).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Heading(_) => "heading",
            Self::Paragraph => "paragraph",
            Self::Quote => "quote",
            Self::OrderedList => "ordered_list",
            Self::UnorderedList => "unordered_list",
            Self::ListItem => "list_item",
            Self::CodeBlock => "code_block",
            Self::Table => "table",
            Self::TableRow => "table_row",
            Self::TableCell => "table_cell",
            Self::Image => "image",
            Self::Caption => "caption",
            Self::Link => "link",
            Self::Emphasis => "emphasis",
            Self::Strong => "strong",
            Self::InlineCode => "inline_code",
            Self::SoftBreak => "soft_break",
            Self::HardBreak => "hard_break",
            Self::Rule => "rule",
            Self::Transparent => "transparent",
        }
    }
}

/// Style hints attached to a node: what the style resolver matches against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleHints {
    /// CSS classes (in document order, deduplicated).
    pub classes: Vec<String>,
    /// An id, if any.
    pub id: Option<String>,
    /// Inline style declarations (`style="..."`), parsed later by the CSS parser.
    pub inline: Option<String>,
    /// The HTML `hidden` attribute (semantic culling flag).
    pub hidden: bool,
}

/// One node in the semantic tree.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticNode {
    /// The semantic kind.
    pub kind: SemanticKind,
    /// Plain text carried by this node (leaf text; NFC-normalized).
    pub text: String,
    /// Link target (for `Link`; for an `Image` that was inside a link, the
    /// hoisted target — the compiler translates image links this way).
    pub href: Option<String>,
    /// Image source (for `Image`).
    pub image_src: Option<String>,
    /// Image alt text (for `Image`).
    pub alt: Option<String>,
    /// Table cell spans (for `TableCell`).
    pub colspan: u32,
    /// Table cell row span (for `TableCell`).
    pub rowspan: u32,
    /// Explicit start value for ordered lists (`OrderedList`); `None` = 1.
    pub list_start: Option<u64>,
    /// Style hints.
    pub hints: StyleHints,
    /// Children (document order).
    pub children: Vec<SemanticNode>,
}

impl SemanticNode {
    /// Create a leaf node with text.
    #[must_use]
    pub fn text(kind: SemanticKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: normalize_nfc(&text.into()),
            href: None,
            image_src: None,
            alt: None,
            colspan: 1,
            rowspan: 1,
            list_start: None,
            hints: StyleHints::default(),
            children: Vec::new(),
        }
    }

    /// Create a structural (child-less) node.
    #[must_use]
    pub fn structural(kind: SemanticKind) -> Self {
        Self::text(kind, "")
    }

    /// Create a node with children.
    #[must_use]
    pub fn with_children(mut self, children: Vec<SemanticNode>) -> Self {
        self.children = children;
        self
    }

    /// Create a node with style hints.
    #[must_use]
    pub fn with_hints(mut self, hints: StyleHints) -> Self {
        self.hints = hints;
        self
    }

    /// Append text to the node's text (normalization happens in a final tree pass;
    /// see [`normalize_tree`]).
    pub fn push_text(&mut self, text: &str) {
        self.text.push_str(text);
    }

    /// Normalize every node's text to NFC in one pass (O(total text)).
    pub fn normalize_tree(root: &mut SemanticNode) {
        fn walk(node: &mut SemanticNode) {
            node.text = normalize_nfc(&node.text);
            for child in &mut node.children {
                walk(child);
            }
        }
        walk(root);
    }

    /// The total text length of this node and its descendants.
    #[must_use]
    pub fn text_len(&self) -> usize {
        self.text.chars().count()
            + self
                .children
                .iter()
                .map(SemanticNode::text_len)
                .sum::<usize>()
    }

    /// Walk the tree in document order, visiting every node.
    pub fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a SemanticNode)) {
        visitor(self);
        for child in &self.children {
            child.visit(visitor);
        }
    }
}

impl fmt::Display for SemanticNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind.is_inline() || self.kind == SemanticKind::Transparent {
            write!(f, "{}", self.text)?;
        }
        for child in &self.children {
            write!(f, "{child}")?;
        }
        Ok(())
    }
}

/// Normalize text to NFC (the format boundary; SPEC.md §1).
#[must_use]
pub fn normalize_nfc(text: &str) -> String {
    text.nfc().collect()
}

/// The allowed child kinds for a parent kind (semantic shape invariants).
#[must_use]
pub fn allowed_child(parent: SemanticKind, child: SemanticKind) -> bool {
    use SemanticKind::*;
    match parent {
        Document => child.is_block() || child == Transparent,
        Heading(_) | Paragraph | Caption | TableCell => child.is_inline() || child == Transparent,
        Quote => child.is_block() || child == Transparent,
        OrderedList | UnorderedList => child == ListItem || child == Transparent,
        ListItem => {
            matches!(
                child,
                OrderedList | UnorderedList | Paragraph | Quote | CodeBlock | Table | Image
            ) || child == Transparent
        }
        CodeBlock => false,
        Table => child == TableRow || child == Transparent,
        TableRow => child == TableCell || child == Transparent,
        Image => child == Caption || child == Transparent,
        Link | Emphasis | Strong | InlineCode => child.is_inline() || child == Transparent,
        SoftBreak | HardBreak | Rule => false,
        Transparent => true, // containers may hold anything
    }
}

/// Post-process a finished tree: for every block node, collapse whitespace across
/// its inline content the way CSS `white-space: normal` does — whitespace runs
/// become one space, edge whitespace is dropped at block and line boundaries,
/// and whitespace keeps the style ownership it had in the source (the leaf that
/// contained it). Empty text leaves and empty inline containers are dropped;
/// containers with children are never dropped. Deterministic and O(total nodes).
pub fn cleanup_tree(root: &mut SemanticNode) {
    fn walk(node: &mut SemanticNode) {
        for child in &mut node.children {
            walk(child);
        }
        if node.kind.is_block() {
            collapse_block_inline_ws(node);
        }
    }
    walk(root);
}

/// Collapse inline whitespace within a block, matching CSS `white-space: normal`.
///
/// The block's inline content is flattened to a leaf sequence (text leaves and
/// line breaks, recursing through inline/transparent containers), processed with
/// a CSS-like state machine, and written back. Key rules:
///
/// - Internal whitespace runs collapse to a single space, and a single leading
///   or trailing space is preserved on the leaf that owned it in the source —
///   so `Hello *world*` keeps the separator inside `Hello` (regular weight)
///   while `Hello* world*` keeps it inside the emphasis, exactly like browsers.
/// - Consecutive whitespace across a leaf boundary collapses to one space that
///   stays with the earlier text node (the CSS behavior for boundary whitespace).
/// - Leading whitespace is dropped at the start of a block or line; trailing
///   whitespace is dropped at the end of a block or line.
/// - Pure-whitespace leaves between content leaves become a single space leaf.
///
/// Deterministic; O(total inline text). No recursion into block children.
fn collapse_block_inline_ws(node: &mut SemanticNode) {
    /// A text leaf addressed by its child-index path from the block.
    struct Leaf {
        path: Vec<usize>,
        text: String,
    }
    /// An inline event in document order.
    enum Event {
        /// A text leaf (original text; final text written back on the path).
        Text(Leaf),
        /// A line boundary: line-start rules apply after it, and the preceding
        /// leaf's trailing whitespace is dropped.
        Break,
    }

    /// Collect inline events in document order, recursing through inline and
    /// transparent containers. Block children are not part of this block's
    /// inline stream and are left untouched; an image is a boundary (its chunk
    /// is block-level, so the surrounding text forms separate lines).
    fn collect(node: &SemanticNode, path: &mut Vec<usize>, out: &mut Vec<Event>) {
        for (i, child) in node.children.iter().enumerate() {
            path.push(i);
            if matches!(
                child.kind,
                SemanticKind::HardBreak | SemanticKind::SoftBreak | SemanticKind::Image
            ) {
                out.push(Event::Break);
            } else if child.kind.is_inline() || child.kind == SemanticKind::Transparent {
                if child.children.is_empty() {
                    if !child.text.is_empty() {
                        out.push(Event::Text(Leaf {
                            path: path.clone(),
                            text: child.text.clone(),
                        }));
                    }
                } else {
                    collect(child, path, out);
                }
            }
            path.pop();
        }
    }

    /// Write a leaf's final text back along its path.
    fn write_leaf(node: &mut SemanticNode, path: &[usize], text: &str) {
        if let Some((head, rest)) = path.split_first() {
            if let Some(child) = node.children.get_mut(*head) {
                if rest.is_empty() {
                    child.text = text.to_string();
                } else {
                    write_leaf(child, rest, text);
                }
            }
        }
    }

    /// Drop empty text leaves and empty inline containers recursively; line
    /// breaks and anything with content or children survive.
    fn retain_content(node: &mut SemanticNode) {
        for child in &mut node.children {
            retain_content(child);
        }
        node.children.retain(|c| {
            let inline_like = c.kind.is_inline() || c.kind == SemanticKind::Transparent;
            let structural = matches!(c.kind, SemanticKind::HardBreak | SemanticKind::SoftBreak);
            !(inline_like && !structural && c.text.is_empty() && c.children.is_empty())
        });
    }

    let mut events = Vec::new();
    collect(node, &mut Vec::new(), &mut events);

    // Process the event stream. `prev_ws` — the stream so far ends with
    // whitespace; `line_start` — at a block/line start, where leading whitespace
    // is dropped. `line_ends` pre-computes, for every event, whether it sits at
    // the end of a line (last event or immediately before a break).
    let line_ends: Vec<bool> = events
        .iter()
        .enumerate()
        .map(|(i, _)| i + 1 == events.len() || matches!(events.get(i + 1), Some(Event::Break)))
        .collect();
    let mut prev_ws = false;
    let mut line_start = true;
    for (event, &at_line_end) in events.iter_mut().zip(line_ends.iter()) {
        match event {
            Event::Break => {
                line_start = true;
                prev_ws = false;
            }
            Event::Text(leaf) => {
                // Collapse internal whitespace runs to one space, preserving a
                // single leading and a single trailing space.
                let mut collapsed = String::with_capacity(leaf.text.len());
                let mut pending = false;
                for c in leaf.text.chars() {
                    if c.is_whitespace() {
                        pending = true;
                    } else {
                        if pending {
                            collapsed.push(' ');
                        }
                        pending = false;
                        collapsed.push(c);
                    }
                }
                if pending && !collapsed.is_empty() {
                    collapsed.push(' ');
                }

                let mut text = if collapsed.is_empty() {
                    // Pure whitespace: a separator unless at a line start or
                    // already separated by the previous leaf's whitespace.
                    if line_start || prev_ws {
                        String::new()
                    } else {
                        String::from(" ")
                    }
                } else {
                    // Drop the single leading space at a line start, or when
                    // the previous leaf already ended with whitespace (the
                    // boundary collapse keeps the space with the earlier node).
                    let starts_ws = collapsed.starts_with(' ');
                    if starts_ws && (line_start || prev_ws) {
                        collapsed.remove(0);
                    }
                    collapsed
                };

                // Trailing whitespace is dropped at the end of the block or at
                // a line boundary (CSS: trailing spaces at end of line render
                // nothing).
                if at_line_end && text.ends_with(' ') {
                    text.pop();
                }

                if text.is_empty() {
                    // Dropped leaf: state is unchanged (the previous state still
                    // describes the stream).
                    leaf.text = String::new();
                } else {
                    leaf.text = text;
                    prev_ws = leaf.text.ends_with(' ');
                    line_start = false;
                }
            }
        }
    }

    // Trailing whitespace at the end of the block is dropped even when the last
    // content leaf is separated from the block end by dropped whitespace leaves
    // or trailing line breaks (CSS: trailing spaces at end of line render
    // nothing). Walk backwards to the last leaf with content and trim it.
    for event in events.iter_mut().rev() {
        match event {
            Event::Break => {}
            Event::Text(leaf) => {
                if leaf.text.is_empty() {
                    continue;
                }
                if leaf.text.ends_with(' ') {
                    leaf.text.pop();
                }
                break;
            }
        }
    }

    for event in &events {
        if let Event::Text(leaf) = event {
            write_leaf(node, &leaf.path, &leaf.text);
        }
    }
    retain_content(node);
}

/// Hoist images to block level, per the format contract: `image` chunks are
/// block-level, and a block whose children are inline-only (paragraph, heading,
/// caption) must never contain an image. The transform (deterministic, applied
/// after [`cleanup_tree`] by both front ends):
///
/// - Images nested in inline containers (link/emphasis/strong/inline code)
///   bubble up through the containers; when an image passes through a `Link`,
///   the link target moves onto the image (`href`), so image links survive.
/// - An inline-content block containing images splits around them: the run
///   before each image stays in a block of the same kind, the image becomes a
///   sibling, and the run after starts a new block. Empty segments are dropped.
/// - Transparent containers (figure, div, span) keep their images — the chunk
///   partition splices them, so an image under a transparent container is
///   already block-level. Images nested in transparent containers *inside* an
///   inline-content block are extracted by the split.
///
/// This keeps the tree shape valid ([`allowed_child`]) and makes the chunk
/// partition image placement unambiguous.
pub fn hoist_inline_images(root: &mut SemanticNode) {
    /// Pull every image out of an inline/transparent container, recursively;
    /// returns the images in document order. Link targets transfer to images.
    /// Line breaks survive (they are content, not images).
    fn extract_images(container: &mut SemanticNode) -> Vec<SemanticNode> {
        let mut images = Vec::new();
        let mut kept = Vec::new();
        for mut child in std::mem::take(&mut container.children) {
            if child.kind == SemanticKind::Image {
                if container.kind == SemanticKind::Link && child.href.is_none() {
                    child.href = container.href.clone();
                }
                images.push(child);
            } else if child.kind.is_inline() || child.kind == SemanticKind::Transparent {
                let nested = extract_images(&mut child);
                let is_break = matches!(
                    child.kind,
                    SemanticKind::HardBreak | SemanticKind::SoftBreak
                );
                if is_break || !child.children.is_empty() || !child.text.is_empty() {
                    kept.push(child);
                }
                images.extend(nested);
            } else {
                kept.push(child);
            }
        }
        container.children = kept;
        images
    }

    /// Split an inline-content block around its images; returns the replacement
    /// sibling sequence (segments of the same kind plus the images).
    fn split_block_images(block: SemanticNode) -> Vec<SemanticNode> {
        // Fast path: a block without images is returned unchanged.
        fn has_image(node: &SemanticNode) -> bool {
            node.children
                .iter()
                .any(|c| c.kind == SemanticKind::Image || has_image(c))
        }
        if !has_image(&block) {
            return vec![block];
        }

        fn fresh_segment(kind: SemanticKind, hints: &StyleHints) -> SemanticNode {
            let mut segment = SemanticNode::structural(kind);
            segment.hints = hints.clone();
            segment
        }

        let mut out: Vec<SemanticNode> = Vec::new();
        let mut segment = fresh_segment(block.kind, &block.hints);

        for mut child in block.children {
            if child.kind == SemanticKind::Image {
                if !segment.children.is_empty() {
                    out.push(segment);
                    segment = fresh_segment(block.kind, &block.hints);
                }
                out.push(child);
            } else if child.kind.is_inline() || child.kind == SemanticKind::Transparent {
                let images = extract_images(&mut child);
                let is_break = matches!(
                    child.kind,
                    SemanticKind::HardBreak | SemanticKind::SoftBreak
                );
                if is_break || !child.children.is_empty() || !child.text.is_empty() {
                    segment.children.push(child);
                }
                if !images.is_empty() {
                    if !segment.children.is_empty() {
                        out.push(segment);
                        segment = fresh_segment(block.kind, &block.hints);
                    }
                    out.extend(images);
                }
            } else {
                segment.children.push(child);
            }
        }
        if !segment.children.is_empty() {
            out.push(segment);
        }
        out
    }

    /// Transform a child list; returns the replacement list.
    fn hoist(children: Vec<SemanticNode>) -> Vec<SemanticNode> {
        let mut out: Vec<SemanticNode> = Vec::new();
        for mut child in children {
            child.children = hoist(std::mem::take(&mut child.children));
            match child.kind {
                // Inline containers: images bubble to the current level.
                SemanticKind::Link
                | SemanticKind::Emphasis
                | SemanticKind::Strong
                | SemanticKind::InlineCode => {
                    let images = extract_images(&mut child);
                    if !child.children.is_empty() || !child.text.is_empty() {
                        out.push(child);
                    }
                    out.extend(images);
                }
                // Inline-content blocks: split around any remaining images.
                SemanticKind::Paragraph | SemanticKind::Caption | SemanticKind::Heading(_) => {
                    out.extend(split_block_images(child));
                }
                _ => out.push(child),
            }
        }
        out
    }

    root.children = hoist(std::mem::take(&mut root.children));
}
#[must_use]
/// Validate the tree invariants; returns a list of violations (empty = valid).
pub fn validate_tree(root: &SemanticNode) -> Vec<String> {
    let mut issues = Vec::new();
    fn walk(node: &SemanticNode, depth: usize, issues: &mut Vec<String>) {
        if depth > MAX_DEPTH {
            issues.push(format!("depth {} exceeds MAX_DEPTH", depth));
            return;
        }
        if node.children.len() > MAX_CHILDREN {
            issues.push(format!(
                "node has {} children (limit {MAX_CHILDREN})",
                node.children.len()
            ));
            return;
        }
        if node.text != normalize_nfc(&node.text) {
            issues.push("text is not NFC-normalized".to_string());
        }
        for child in &node.children {
            if !allowed_child(node.kind, child.kind) {
                issues.push(format!(
                    "{} cannot contain {}",
                    node.kind.name(),
                    child.kind.name()
                ));
            }
            walk(child, depth + 1, issues);
        }
    }
    if root.kind != SemanticKind::Document {
        issues.push(format!("root is {}, not document", root.kind.name()));
    }
    walk(root, 0, &mut issues);
    issues
}

#[cfg(test)]
mod tests {
    use super::{normalize_nfc, validate_tree, SemanticKind, SemanticNode};

    #[test]
    fn nfc_normalization_at_boundary() {
        // "é" as e + combining acute (NFC: single codepoint U+00E9).
        let decomposed = "e\u{0301}";
        assert_ne!(decomposed, "\u{00e9}");
        assert_eq!(normalize_nfc(decomposed), "\u{00e9}");
        let node = SemanticNode::text(SemanticKind::Paragraph, decomposed);
        assert_eq!(node.text, "\u{00e9}");
    }

    #[test]
    fn tree_validation_passes() {
        let doc = SemanticNode::structural(SemanticKind::Document).with_children(vec![
            SemanticNode::structural(SemanticKind::Heading(1))
                .with_children(vec![SemanticNode::text(SemanticKind::Emphasis, "hi")]),
            SemanticNode::text(SemanticKind::Paragraph, "body"),
        ]);
        assert!(validate_tree(&doc).is_empty());
    }

    #[test]
    fn tree_validation_catches_bad_shape() {
        // A paragraph containing a table is invalid.
        let doc = SemanticNode::structural(SemanticKind::Document)
            .with_children(vec![SemanticNode::structural(SemanticKind::Paragraph)
                .with_children(vec![SemanticNode::structural(SemanticKind::Table)])]);
        let issues = validate_tree(&doc);
        assert!(issues.iter().any(|i| i.contains("cannot contain")));
    }

    #[test]
    fn root_must_be_document() {
        let p = SemanticNode::text(SemanticKind::Paragraph, "x");
        assert!(validate_tree(&p).iter().any(|i| i.contains("root")));
    }

    #[test]
    fn display_concatenates_text() {
        let para = SemanticNode::structural(SemanticKind::Paragraph).with_children(vec![
            SemanticNode::text(SemanticKind::Emphasis, "bold "),
            SemanticNode::text(SemanticKind::SoftBreak, "\n"),
            SemanticNode::text(SemanticKind::Strong, "words"),
        ]);
        assert_eq!(para.to_string(), "bold \nwords");
    }
}
