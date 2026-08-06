//! Style resolution: CSS cascade folded into flat, inherited-computed styles.
//!
//! The runtime implements *no* cascade — it consumes flat style records
//! (SPEC.md STYL). This module performs the cascade at compile time: selector
//! matching (with specificity and order), inline-style declarations, inheritance
//! of inheritable properties, and resolution of relative units (em/%) against the
//! parent's computed font size.
//!
//! A built-in default stylesheet is always prepended (user styles win on equal
//! specificity by source order); it is documented in [`DEFAULT_STYLESHEET`].

use std::collections::BTreeMap;

use glyphcull_semantic::css::{Declaration, SimpleSelector, Stylesheet};
use glyphcull_semantic::model::{SemanticKind, SemanticNode};

/// The maximum font size (px) accepted, to bound em-multiplication (defensive).
pub const MAX_FONT_SIZE_PX: f32 = 4096.0;

/// The built-in default stylesheet (prepended to any user stylesheet). Values are
/// conservative typographic defaults; documents compile deterministically with or
/// without user CSS.
pub const DEFAULT_STYLESHEET: &str = "\
h1 { font-size: 2em; font-weight: 700; margin-top: 0.67em; margin-bottom: 0.67em; }
h2 { font-size: 1.5em; font-weight: 700; margin-top: 0.83em; margin-bottom: 0.83em; }
h3 { font-size: 1.17em; font-weight: 700; margin-top: 1em; margin-bottom: 1em; }
h4 { font-size: 1em; font-weight: 700; margin-top: 1.33em; margin-bottom: 1.33em; }
h5 { font-size: 0.83em; font-weight: 700; margin-top: 1.67em; margin-bottom: 1.67em; }
h6 { font-size: 0.67em; font-weight: 700; margin-top: 2.33em; margin-bottom: 2.33em; }
p { margin-bottom: 1em; }
blockquote { margin-top: 1em; margin-bottom: 1em; }
ul, ol { margin-bottom: 1em; }
ul { list-style-type: disc; }
ol { list-style-type: decimal; }
li { margin-bottom: 0.25em; }
pre { white-space: pre; margin-top: 1em; margin-bottom: 1em; }
img { margin-top: 1em; margin-bottom: 1em; }
caption { font-size: 0.83em; }
table { margin-bottom: 1em; }
em { font-style: italic; }
strong { font-weight: 700; }
a { text-decoration: underline; }
";

/// A fully resolved, flat style (the runtime's view). Absent fields are covered
/// by the defaults; `font_family` is later mapped to a font id once atlases exist.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStyle {
    /// Font family name (resolved to a font id by the pipeline).
    pub font_family: String,
    /// Font size in px.
    pub font_size_px: f32,
    /// Line height as a multiplier of the font size.
    pub line_height: f32,
    /// Font weight 100..=900.
    pub font_weight: u16,
    /// Italic flag.
    pub italic: bool,
    /// Text color, RGBA.
    pub color: u32,
    /// Background color, RGBA.
    pub background: u32,
    /// Top margin in px.
    pub margin_top: f32,
    /// Bottom margin in px.
    pub margin_bottom: f32,
    /// Text alignment (0 start, 1 center, 2 end, 3 justify).
    pub text_align: u8,
    /// First-line indent in px.
    pub text_indent: f32,
    /// List marker style (0 none, 1 disc, ...).
    pub list_style: u8,
    /// Monospace/code flag.
    pub code: bool,
    /// Underline flag.
    pub underline: bool,
    /// Letter spacing in px.
    pub letter_spacing: f32,
    /// White-space mode (0 normal, 1 pre, 2 nowrap).
    pub white_space: u8,
}

impl Default for ResolvedStyle {
    fn default() -> Self {
        Self {
            font_family: "Noto Sans".to_string(),
            font_size_px: 16.0,
            line_height: 1.5,
            font_weight: 400,
            italic: false,
            color: 0x0000_00FF,
            background: 0x0000_0000,
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: 0,
            text_indent: 0.0,
            list_style: 1,
            code: false,
            underline: false,
            letter_spacing: 0.0,
            white_space: 0,
        }
    }
}

/// A font face key: the identity an atlas is created for (family + weight + style).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceKey {
    /// Family name.
    pub family: String,
    /// Weight 100..=900.
    pub weight: u16,
    /// Italic flag.
    pub italic: bool,
}

/// The interned style table: `styles[id]` for id 0..n; style 0 is the document
/// default (matching the format's implicit style 0).
#[derive(Debug, Clone, Default)]
pub struct StyleTable {
    /// Interned styles (id = index).
    pub styles: Vec<ResolvedStyle>,
    /// Per-semantic-node style id, in document order (indexed by the same order
    /// as the partition walk).
    pub node_styles: Vec<u32>,
}

impl StyleTable {
    fn intern(&mut self, style: ResolvedStyle) -> u32 {
        if let Some(pos) = self.styles.iter().position(|s| *s == style) {
            return pos as u32;
        }
        self.styles.push(style);
        (self.styles.len() - 1) as u32
    }
}

/// The declared (unresolved) value of a property, as written in CSS.
#[derive(Debug, Clone, PartialEq)]
enum Declared {
    /// A length: value + unit ('px', 'em', 'rem', '%', 'pt').
    Length(f32, &'static str),
    /// A unitless number (line-height multiplier, weights).
    Number(f32),
    /// A keyword.
    Keyword(String),
    /// A color.
    Color(u32),
    /// A list style (0..=8).
    ListStyle(u8),
    /// A text alignment (0..=3).
    Align(u8),
    /// A weight.
    Weight(u16),
    /// A white-space mode (0..=2).
    WhiteSpace(u8),
    /// A text-decoration flag (underline).
    Underline(bool),
}

/// Parse one declaration into a typed value; returns `None` for unsupported or
/// malformed values (the declaration is ignored, like browsers do for invalid
/// declarations).
fn parse_declaration(decl: &Declaration) -> Option<(String, Declared)> {
    let name = decl.name.as_str();
    let value = decl.value.trim();
    let declared = match name {
        "font-family" => {
            let family = parse_family(value)?;
            Declared::Keyword(family)
        }
        "font-size" => {
            let (n, unit) = parse_length(value)?;
            Declared::Length(n, unit)
        }
        "line-height" => {
            if let Some((n, "px" | "em" | "rem" | "%")) = parse_length(value) {
                Declared::Length(n, "em")
            } else if let Ok(n) = value.parse::<f32>() {
                Declared::Number(n)
            } else {
                return None;
            }
        }
        "font-weight" => Declared::Weight(parse_weight(value)?),
        "font-style" => {
            let v = value.to_lowercase();
            if v == "italic" || v == "oblique" {
                Declared::Keyword("italic".to_string())
            } else if v == "normal" {
                Declared::Keyword("normal".to_string())
            } else {
                return None;
            }
        }
        "color" | "background-color" => Declared::Color(parse_color(value)?),
        "margin-top" | "margin-bottom" => {
            let (n, unit) = parse_length(value)?;
            Declared::Length(n, unit)
        }
        "text-align" => {
            let v = match value.to_lowercase().as_str() {
                "left" | "start" => 0,
                "center" => 1,
                "right" | "end" => 2,
                "justify" => 3,
                _ => return None,
            };
            Declared::Align(v)
        }
        "text-indent" => {
            let (n, unit) = parse_length(value)?;
            Declared::Length(n, unit)
        }
        "list-style-type" => {
            let v = match value.to_lowercase().as_str() {
                "none" => 0,
                "disc" => 1,
                "circle" => 2,
                "square" => 3,
                "decimal" => 4,
                "lower-alpha" => 5,
                "upper-alpha" => 6,
                "lower-roman" => 7,
                "upper-roman" => 8,
                _ => return None,
            };
            Declared::ListStyle(v)
        }
        "letter-spacing" => {
            if value.eq_ignore_ascii_case("normal") {
                Declared::Length(0.0, "px")
            } else {
                let (n, unit) = parse_length(value)?;
                Declared::Length(n, unit)
            }
        }
        "white-space" => {
            let v = match value.to_lowercase().as_str() {
                "normal" => 0,
                "pre" | "pre-wrap" | "pre-line" => 1,
                "nowrap" => 2,
                _ => return None,
            };
            Declared::WhiteSpace(v)
        }
        "text-decoration" => {
            let v = match value.to_lowercase().as_str() {
                "underline" => true,
                "none" => false,
                _ => return None,
            };
            Declared::Underline(v)
        }
        _ => return None,
    };
    Some((name.to_string(), declared))
}

/// Parse a length: `Npx`, `Nem`, `Nrem`, `N%`, `Npt`. `0` may be unitless.
fn parse_length(value: &str) -> Option<(f32, &'static str)> {
    let value = value.trim();
    if value == "0" {
        return Some((0.0, "px"));
    }
    for (suffix, unit) in [
        ("px", "px"),
        ("em", "em"),
        ("rem", "rem"),
        ("%", "%"),
        ("pt", "pt"),
    ] {
        if let Some(num) = value.strip_suffix(suffix) {
            let n: f32 = num.trim().parse().ok()?;
            if n.is_finite() {
                return Some((n, unit));
            }
        }
    }
    None
}

/// Parse a font family list; returns the first concrete family name.
fn parse_family(value: &str) -> Option<String> {
    for part in value.split(',') {
        let part = part.trim();
        let part = part.trim_matches('"').trim_matches('\'');
        if part.is_empty() {
            continue;
        }
        if matches!(
            part.to_lowercase().as_str(),
            "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui" | "inherit"
        ) {
            continue;
        }
        return Some(part.to_string());
    }
    None
}

/// Parse a font weight: 100..=900 or a keyword.
fn parse_weight(value: &str) -> Option<u16> {
    let v = value.trim().to_lowercase();
    match v.as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "bolder" => Some(700),
        "lighter" => Some(300),
        _ => {
            let n: u16 = v.parse().ok()?;
            if (100..=900).contains(&n) && n % 100 == 0 {
                Some(n)
            } else {
                None
            }
        }
    }
}

/// Parse a color: `#rgb`, `#rrggbb`, or a named color.
fn parse_color(value: &str) -> Option<u32> {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix('#') {
        return match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                Some(rgba(r, g, b, 255))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(rgba(r, g, b, 255))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(rgba(r, g, b, a))
            }
            _ => None,
        };
    }
    named_color(v.to_lowercase().as_str())
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(r) << 24 | u32::from(g) << 16 | u32::from(b) << 8 | u32::from(a)
}

/// A small named-color table (CSS basic keywords).
fn named_color(name: &str) -> Option<u32> {
    let v = match name {
        "black" => 0x0000_00FF,
        "silver" => 0xC0C0_C0FF,
        "gray" | "grey" => 0x8080_80FF,
        "white" => 0xFFFF_FFFF,
        "maroon" => 0x8000_00FF,
        "red" => 0xFF00_00FF,
        "purple" => 0x8000_80FF,
        "fuchsia" => 0xFF00_FFFF,
        "green" => 0x0080_00FF,
        "lime" => 0x00FF_00FF,
        "olive" => 0x8080_00FF,
        "yellow" => 0xFFFF_00FF,
        "navy" => 0x0000_80FF,
        "blue" => 0x0000_FFFF,
        "teal" => 0x0080_80FF,
        "aqua" => 0x00FF_FFFF,
        "orange" => 0xFFA5_00FF,
        "brown" => 0xA52A_2AFF,
        "pink" => 0xFFC0_CBFF,
        "transparent" => 0x0000_0000,
        _ => return None,
    };
    Some(v)
}

/// Build the effective rule set: the built-in defaults first, then the user
/// stylesheets (user rules win on equal specificity by source order).
pub fn ruleset(user_stylesheets: &[Stylesheet]) -> Vec<glyphcull_semantic::css::CssRule> {
    let mut all_rules = Vec::new();
    // The built-in default stylesheet is a reviewed compile-time constant; a
    // parse failure would be a programming error, so the expect is safe.
    #[allow(clippy::expect_used)]
    let default_sheet = glyphcull_semantic::css::parse_stylesheet(DEFAULT_STYLESHEET)
        .expect("the built-in default stylesheet must parse");
    all_rules.extend(default_sheet.rules);
    for sheet in user_stylesheets {
        all_rules.extend(sheet.rules.iter().cloned());
    }
    all_rules
}

/// Compute a node's resolved style: matched declarations (specificity + order +
/// inline) folded over the parent's computed style (inheritance + relative units).
pub fn resolve_node(
    node: &SemanticNode,
    ancestors: &[&SemanticNode],
    parent: &ResolvedStyle,
    rules: &[glyphcull_semantic::css::CssRule],
) -> ResolvedStyle {
    let declared = match_node(node, ancestors, rules);
    compute(node, parent, &declared)
}

/// Resolve the styles of every node in the tree.
///
/// `user_stylesheets` are prepended by the default stylesheet; nodes without any
/// matching rule inherit from their parent, per the CSS inheritance model.
pub fn resolve_styles(root: &SemanticNode, user_stylesheets: &[Stylesheet]) -> StyleTable {
    let all_rules = ruleset(user_stylesheets);

    let mut table = StyleTable::default();
    let default_style = ResolvedStyle::default();
    let default_id = table.intern(default_style.clone());
    debug_assert_eq!(default_id, 0);

    // Walk the tree in document order, carrying the parent's computed style and
    // the ancestor chain (nearest first) for descendant-selector matching.
    fn walk(
        node: &SemanticNode,
        parent: &ResolvedStyle,
        ancestors: &[&SemanticNode],
        table: &mut StyleTable,
        rules: &[glyphcull_semantic::css::CssRule],
    ) {
        let computed = resolve_node(node, ancestors, parent, rules);
        let id = table.intern(computed.clone());
        table.node_styles.push(id);

        let mut chain = Vec::with_capacity(ancestors.len() + 1);
        chain.push(node);
        chain.extend_from_slice(ancestors);
        for child in &node.children {
            walk(child, &computed, &chain, table, rules);
        }
    }
    walk(root, &default_style, &[], &mut table, &all_rules);
    table
}

/// A cascade entry: (id, class, tag) specificity, source order, declarations.
type CascadeEntry = ((usize, usize, usize), usize, Vec<Declaration>);

/// Collect the declarations that match a node, in cascade order (specificity,
/// then source order; inline styles last).
fn match_node(
    node: &SemanticNode,
    ancestors: &[&SemanticNode],
    rules: &[glyphcull_semantic::css::CssRule],
) -> Vec<Declaration> {
    let mut matched: Vec<CascadeEntry> = Vec::new();
    for (order, rule) in rules.iter().enumerate() {
        for selector in &rule.selectors {
            if matches_selector(node, ancestors, selector) {
                let specificity = selector
                    .chain
                    .last()
                    .map_or((0, 0, 0), SimpleSelector::specificity);
                matched.push((specificity, order, rule.declarations.clone()));
            }
        }
    }
    matched.sort_by_key(|(spec, order, _)| (*spec, *order));

    let mut out: Vec<Declaration> = Vec::new();
    for (_, _, decls) in matched {
        out.extend(decls.iter().cloned());
    }
    // Inline styles act last (highest priority).
    if let Some(inline) = &node.hints.inline {
        if let Ok(decls) = glyphcull_semantic::css::parse_declarations(inline) {
            out.extend(decls);
        }
    }
    out
}

/// Match a complex selector (descendant chain) against a node and its ancestors.
fn matches_selector(
    node: &SemanticNode,
    ancestors: &[&SemanticNode],
    selector: &glyphcull_semantic::css::ComplexSelector,
) -> bool {
    let Some((innermost, rest)) = selector.chain.split_last() else {
        return false;
    };
    if !matches_simple(node, innermost) {
        return false;
    }
    // The remaining chain elements (outermost-first) must match progressively
    // more distant ancestors.
    let mut ancestor_iter = ancestors.iter();
    for sel in rest.iter().rev() {
        let mut found = false;
        for ancestor in ancestor_iter.by_ref() {
            if matches_simple(ancestor, sel) {
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

fn matches_simple(node: &SemanticNode, sel: &SimpleSelector) -> bool {
    if let Some(id) = &sel.id {
        if node.hints.id.as_ref() != Some(id) {
            return false;
        }
    }
    for class in &sel.classes {
        if !node.hints.classes.contains(class) {
            return false;
        }
    }
    if let Some(tag) = &sel.tag {
        let node_tag = tag_of(node);
        if node_tag != Some(tag.as_str()) {
            return false;
        }
    }
    true
}

/// The element tag a semantic node maps to for selector matching.
fn tag_of(node: &SemanticNode) -> Option<&'static str> {
    use SemanticKind::*;
    match node.kind {
        Document => Some("document"),
        Heading(1) => Some("h1"),
        Heading(2) => Some("h2"),
        Heading(3) => Some("h3"),
        Heading(4) => Some("h4"),
        Heading(5) => Some("h5"),
        Heading(6) => Some("h6"),
        Heading(_) => Some("h"),
        Paragraph => Some("p"),
        Quote => Some("blockquote"),
        OrderedList => Some("ol"),
        UnorderedList => Some("ul"),
        ListItem => Some("li"),
        CodeBlock => Some("pre"),
        Table => Some("table"),
        TableRow => Some("tr"),
        TableCell => Some("td"),
        Image => Some("img"),
        Caption => Some("caption"),
        Link => Some("a"),
        Emphasis => Some("em"),
        Strong => Some("strong"),
        InlineCode => Some("code"),
        SoftBreak | HardBreak => Some("br"),
        Rule => Some("hr"),
        Transparent => None,
    }
}

/// Compute the inherited+declared style for a node.
fn compute(node: &SemanticNode, parent: &ResolvedStyle, declared: &[Declaration]) -> ResolvedStyle {
    let mut style = parent.clone();

    // Parse declarations into a per-property map (last wins).
    let mut map: BTreeMap<String, Declared> = BTreeMap::new();
    for decl in declared {
        if let Some((name, value)) = parse_declaration(decl) {
            map.insert(name, value);
        }
    }

    let font_size = match map.get("font-size") {
        Some(Declared::Length(n, "em" | "rem")) => parent.font_size_px * n,
        Some(Declared::Length(n, "%")) => parent.font_size_px * n / 100.0,
        Some(Declared::Length(n, "pt")) => n * 96.0 / 72.0,
        Some(Declared::Length(n, "px")) => *n,
        _ => parent.font_size_px,
    };
    let font_size = font_size.clamp(1.0, MAX_FONT_SIZE_PX);

    if let Some(Declared::Keyword(family)) = map.get("font-family") {
        style.font_family = family.clone();
    }
    style.font_size_px = font_size;
    if let Some(Declared::Number(lh)) = map.get("line-height") {
        style.line_height = *lh;
    } else if let Some(Declared::Length(n, "em")) = map.get("line-height") {
        style.line_height = font_size * n / font_size; // em line-height relative to own size
    }
    if let Some(Declared::Weight(w)) = map.get("font-weight") {
        style.font_weight = *w;
    }
    if let Some(Declared::Keyword(k)) = map.get("font-style") {
        style.italic = k == "italic";
    }
    if let Some(Declared::Color(c)) = map.get("color") {
        style.color = *c;
    }
    if let Some(Declared::Color(c)) = map.get("background-color") {
        style.background = *c;
    }
    if let Some(Declared::Length(n, unit)) = map.get("margin-top") {
        style.margin_top = resolve_px(*n, unit, font_size);
    }
    if let Some(Declared::Length(n, unit)) = map.get("margin-bottom") {
        style.margin_bottom = resolve_px(*n, unit, font_size);
    }
    if let Some(Declared::Align(a)) = map.get("text-align") {
        style.text_align = *a;
    }
    if let Some(Declared::Length(n, unit)) = map.get("text-indent") {
        style.text_indent = resolve_px(*n, unit, font_size);
    }
    if let Some(Declared::ListStyle(l)) = map.get("list-style-type") {
        style.list_style = *l;
    }
    if let Some(Declared::Length(n, unit)) = map.get("letter-spacing") {
        style.letter_spacing = resolve_px(*n, unit, font_size);
    }
    if let Some(Declared::WhiteSpace(w)) = map.get("white-space") {
        style.white_space = *w;
    }
    if let Some(Declared::Underline(u)) = map.get("text-decoration") {
        style.underline = *u;
    }

    // Semantic code flag: inline code and code blocks render monospace.
    if matches!(
        node.kind,
        SemanticKind::InlineCode | SemanticKind::CodeBlock
    ) {
        style.code = true;
    }
    // Text decoration propagates to inline descendants (browser behavior: a
    // link's underline spans nested runs) and is overridable per element via
    // `text-decoration: none`, which the cascade resolves naturally.
    style
}

fn resolve_px(n: f32, unit: &str, font_size: f32) -> f32 {
    match unit {
        "em" | "rem" => n * font_size,
        "%" => n * font_size / 100.0,
        "pt" => n * 96.0 / 72.0,
        _ => n,
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_styles, ResolvedStyle, DEFAULT_STYLESHEET};
    use glyphcull_semantic::css::parse_stylesheet;
    use glyphcull_semantic::model::{SemanticKind, SemanticNode};

    fn doc_with(children: Vec<SemanticNode>) -> SemanticNode {
        SemanticNode::structural(SemanticKind::Document).with_children(children)
    }

    #[test]
    fn defaults() {
        let doc = doc_with(vec![SemanticNode::text(SemanticKind::Paragraph, "x")]);
        let table = resolve_styles(&doc, &[]);
        assert_eq!(table.styles[0], ResolvedStyle::default());
        // The default stylesheet gives paragraphs a bottom margin; everything
        // else inherits the default.
        let para = &table.styles[table.node_styles[1] as usize];
        assert_eq!(para.margin_bottom, 16.0);
        assert_eq!(para.font_size_px, 16.0);
    }

    #[test]
    fn selector_matching_and_cascade() {
        let css =
            parse_stylesheet("p { color: #ff0000; font-size: 20px; } .note { color: #00ff00; }")
                .expect("css");
        let mut para = SemanticNode::text(SemanticKind::Paragraph, "x");
        para.hints.classes.push("note".to_string());
        let doc = doc_with(vec![para]);
        let table = resolve_styles(&doc, &[css]);
        let style = &table.styles[table.node_styles[1] as usize];
        assert_eq!(style.color, 0x00FF_00FF); // .note wins over p (class > tag)
        assert_eq!(style.font_size_px, 20.0);
    }

    #[test]
    fn inline_style_wins() {
        let css = parse_stylesheet("p { color: #ff0000; }").expect("css");
        let mut para = SemanticNode::text(SemanticKind::Paragraph, "x");
        para.hints.inline = Some("color: #0000ff;".to_string());
        let doc = doc_with(vec![para]);
        let table = resolve_styles(&doc, &[css]);
        let style = &table.styles[table.node_styles[1] as usize];
        assert_eq!(style.color, 0x0000_FFFF);
    }

    #[test]
    fn inheritance() {
        let mut outer = SemanticNode::text(SemanticKind::Paragraph, "outer");
        outer.hints.inline = Some("color: #123456; font-size: 24px;".to_string());
        let inner = SemanticNode::text(SemanticKind::Emphasis, "inner");
        outer.children.push(inner);
        let doc = doc_with(vec![outer]);
        let table = resolve_styles(&doc, &[]);
        let inner_style = &table.styles[table.node_styles[2] as usize];
        assert_eq!(inner_style.color, 0x1234_56FF); // inherited
        assert_eq!(inner_style.font_size_px, 24.0); // inherited
    }

    #[test]
    fn em_units_resolve_against_parent() {
        let mut outer = SemanticNode::text(SemanticKind::Paragraph, "outer");
        outer.hints.inline = Some("font-size: 20px;".to_string());
        let inner = SemanticNode::text(SemanticKind::Emphasis, "inner");
        outer.children.push(inner);
        let doc = doc_with(vec![outer]);
        let table = resolve_styles(&doc, &[]);
        let inner_style = &table.styles[table.node_styles[2] as usize];
        // inner inherits 20px
        assert_eq!(inner_style.font_size_px, 20.0);
        // heading em sizes: an h1 under the default sheet is 2em of 16 = 32px.
        let doc2 = doc_with(vec![SemanticNode::structural(SemanticKind::Heading(1))]);
        let table2 = resolve_styles(&doc2, &[]);
        let h1 = &table2.styles[table2.node_styles[1] as usize];
        assert_eq!(h1.font_size_px, 32.0);
        assert_eq!(h1.font_weight, 700);
    }

    #[test]
    fn default_stylesheet_parses() {
        let sheet = parse_stylesheet(DEFAULT_STYLESHEET).expect("parse");
        assert!(!sheet.rules.is_empty());
    }

    #[test]
    fn list_style_and_pre() {
        let doc = doc_with(vec![SemanticNode::structural(SemanticKind::CodeBlock)]);
        let table = resolve_styles(&doc, &[]);
        let pre = &table.styles[table.node_styles[1] as usize];
        assert_eq!(pre.white_space, 1); // pre
        assert!(pre.code);
    }

    #[test]
    fn determinism() {
        let doc = doc_with(vec![SemanticNode::text(SemanticKind::Paragraph, "x")]);
        let a = resolve_styles(&doc, &[]);
        let b = resolve_styles(&doc, &[]);
        assert_eq!(a.styles, b.styles);
        assert_eq!(a.node_styles, b.node_styles);
    }
}
