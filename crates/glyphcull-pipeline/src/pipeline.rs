//! The deterministic compile pipeline: source → `.cull` package.
//!
//! Orchestrates the stages in the mandated order:
//!
//! ```text
//! HTML / Markdown
//!   → Semantic Graph (glyphcull-semantic)
//!   → Chunk Graph + resolved styles (glyphcull-chunk)
//!   → MSDF glyph atlases per face (glyphcull-atlas)
//!   → decoded images (PNG/JPEG)
//!   → INFO / CHNK / STYL / CONT / GLYF / IMGS / SEAL (glyphcull-format)
//! ```
//!
//! Determinism is a contract: the same source + options produce byte-identical
//! packages. Faces are processed in sorted order, sections in canonical order,
//! and every emitted structure is sorted. No timestamps, no randomness, no
//! environment-dependent bytes.

use std::collections::BTreeMap;

use glyphcull_chunk::{build_chunk_model, FaceKey, ResolvedStyle};
use glyphcull_format::codec::glyph::GlyphSection;
use glyphcull_format::codec::image::ImageSection;
use glyphcull_format::codec::info::Info;
use glyphcull_format::codec::style::{
    PropertyTag, PropertyValue, StyleProperty, StyleRecord, StyleSection,
};
use glyphcull_format::reader::parse as parse_package;
use glyphcull_format::section::SectionKind;
use glyphcull_format::table::Compression;
use glyphcull_format::validate::validate_package;
use glyphcull_format::writer::PackageBuilder;
use glyphcull_semantic::css::{parse_stylesheet, Stylesheet};
use sha2::{Digest, Sha256};

use crate::fonts::{FontError, FontRegistry, FONT_SUPPLEMENTARY_BLOCKS};
use crate::images::{self, ImageError};

/// The compile input kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// HTML source.
    Html,
    /// Markdown source.
    Markdown,
}

/// A compile error.
#[derive(Debug)]
pub enum Error {
    /// The input front end rejected the source.
    FrontEnd(String),
    /// A user stylesheet failed to parse.
    Css(glyphcull_semantic::css::CssError),
    /// A font family could not be resolved.
    Font(FontError),
    /// An image could not be loaded by the host loader.
    ImageLoad(String),
    /// An image could not be decoded.
    Image(ImageError),
    /// Atlas generation failed.
    Atlas(glyphcull_atlas::error::Error),
    /// Package assembly failed.
    Format(glyphcull_format::error::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::FrontEnd(msg) => write!(f, "front end: {msg}"),
            Error::Css(e) => write!(f, "{e}"),
            Error::Font(e) => write!(f, "{e}"),
            Error::ImageLoad(msg) => write!(f, "image load: {msg}"),
            Error::Image(e) => write!(f, "{e}"),
            Error::Atlas(e) => write!(f, "atlas: {e}"),
            Error::Format(e) => write!(f, "format: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<glyphcull_format::error::Error> for Error {
    fn from(e: glyphcull_format::error::Error) -> Self {
        Error::Format(e)
    }
}

/// Compile options.
pub struct CompileOptions {
    /// Additional user stylesheets (parsed after any `<style>` blocks).
    pub user_stylesheets: Vec<String>,
    /// Atlas generation parameters.
    pub atlas: glyphcull_atlas::AtlasOptions,
    /// The font registry (defaults to the bundled Noto Sans faces).
    pub fonts: FontRegistry,
    /// The image loader: given an image source path/URL, returns raw bytes.
    /// The CLI provides a filesystem loader rooted at the input's directory.
    pub image_loader: ImageLoader,
}

/// The image-loading callback type (see [`CompileOptions::image_loader`]).
pub type ImageLoader = Box<dyn Fn(&str) -> Result<Vec<u8>, String>>;

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            user_stylesheets: Vec::new(),
            atlas: glyphcull_atlas::AtlasOptions::default(),
            fonts: FontRegistry::bundled(),
            image_loader: Box::new(|src| Err(format!("no image loader configured for {src:?}"))),
        }
    }
}

/// The compile report (deterministic diagnostics for the CLI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileReport {
    /// Document title, if any.
    pub title: Option<String>,
    /// Document language tag, if any.
    pub lang: Option<String>,
    /// Codepoints present in the document but absent from the resolved fonts.
    pub missing_codepoints: Vec<u32>,
    /// The number of glyph atlases.
    pub atlas_count: u32,
    /// The number of decoded images.
    pub image_count: u32,
    /// CHNK record count.
    pub chunk_count: u32,
    /// STYL record count.
    pub style_count: u32,
    /// CONT payload count.
    pub content_count: u32,
}

/// Compile `source` into a `.cull` package.
pub fn compile(
    source: &str,
    kind: InputKind,
    options: &CompileOptions,
) -> Result<(Vec<u8>, CompileReport), Error> {
    // 1. Front end → semantic graph.
    let (tree, front_styles, title, lang) = match kind {
        InputKind::Html => {
            let out = glyphcull_semantic::parse_html(source)
                .map_err(|e| Error::FrontEnd(format!("{e}")))?;
            (out.tree, out.stylesheets, out.title, out.lang)
        }
        InputKind::Markdown => {
            let tree = glyphcull_semantic::parse_markdown(source)
                .map_err(|e| Error::FrontEnd(format!("{e}")))?;
            (tree, Vec::new(), None, None)
        }
    };

    // 2. Stylesheets: `<style>` blocks first, then user sheets (cascade order).
    let mut sheets: Vec<Stylesheet> = Vec::new();
    for raw in front_styles.iter().chain(options.user_stylesheets.iter()) {
        sheets.push(parse_stylesheet(raw).map_err(Error::Css)?);
    }

    // 3. Chunk graph + resolved styles.
    let model = build_chunk_model(&tree, &[], &sheets);

    // 4. Glyph atlases, one per used face (sorted → deterministic font ids).
    //    Pages are sized to the face's content (power-of-two, capped) so small
    //    documents produce small packages; the packer spills to further pages.
    let mut atlases: Vec<GlyphSection> = Vec::new();
    let mut face_to_font: BTreeMap<FaceKey, u32> = BTreeMap::new();
    let mut missing: Vec<u32> = Vec::new();
    for (font_id, (face, codepoints)) in model.used_codepoints.iter().enumerate() {
        let bytes = options
            .fonts
            .resolve(&face.family, face.weight, face.italic)
            .map_err(Error::Font)?;
        let mut atlas_options = options.atlas;
        let page = suggest_page_size(codepoints.len());
        atlas_options.page_width = page;
        atlas_options.page_height = page;
        let result = glyphcull_atlas::build_atlas_with(
            bytes,
            Some(FONT_SUPPLEMENTARY_BLOCKS),
            codepoints,
            font_id as u32,
            &atlas_options,
        )
        .map_err(Error::Atlas)?;
        face_to_font.insert(face.clone(), font_id as u32);
        missing.extend(result.missing);
        atlases.push(GlyphSection {
            atlases: vec![result.atlas],
        });
    }
    missing.sort_unstable();
    missing.dedup();

    // 5. STYL: flat records with the font id resolved per face.
    let style_section = StyleSection {
        styles: model
            .resolved_styles
            .iter()
            .enumerate()
            .map(|(id, style)| StyleRecord {
                id: id as u32,
                properties: style_properties(style, &face_to_font),
            })
            .collect(),
    };

    // 6. CONT.
    let content_section = model.content_section.clone();

    // 7. IMGS: decode every image in document order (IMGS ids match the
    //    image_ref payload indices by construction).
    let mut image_section = ImageSection { images: Vec::new() };
    for image in &model.images {
        let bytes = (options.image_loader)(&image.src).map_err(Error::ImageLoad)?;
        let decoded = images::decode(&bytes).map_err(Error::Image)?;
        image_section.images.push(decoded);
    }

    // 8. GLYF: one section with all atlases.
    let glyph_section = GlyphSection {
        atlases: atlases.into_iter().flat_map(|s| s.atlases).collect(),
    };

    // 9. INFO (deterministic; the document id is content-addressed over the
    //    decoded content sections, excluding INFO/SEAL — non-circular).
    let chunk_bytes = model.chunk_section.encode();
    let style_bytes = style_section.encode();
    let content_bytes = content_section.encode();
    let glyph_bytes = glyph_section.encode();
    let image_bytes = image_section.encode();
    let document_id = content_id(&[
        &chunk_bytes,
        &style_bytes,
        &content_bytes,
        &glyph_bytes,
        &image_bytes,
    ]);
    let source_digest = hex(&Sha256::digest(source.as_bytes()));
    let info = Info {
        format_version: 1,
        generator: "glyphcull-compiler".to_string(),
        generator_version: env!("CARGO_PKG_VERSION").to_string(),
        source_digest,
        document_id,
        title,
        lang,
        chunk_count: model.chunk_section.len() as u32,
        style_count: style_section.styles.len() as u32,
        content_count: content_section.payloads.len() as u32,
        atlas_count: glyph_section.atlases.len() as u32,
        image_count: image_section.images.len() as u32,
    };
    let info_bytes = info.encode();

    // 10. Assemble in canonical order with the SEAL appended.
    let mut builder = PackageBuilder::new().with_seal(true);
    builder.add(SectionKind::Info, info_bytes, Compression::Zlib)?;
    builder.add(SectionKind::Chunk, chunk_bytes, Compression::Zlib)?;
    builder.add(SectionKind::Style, style_bytes, Compression::Zlib)?;
    builder.add(SectionKind::Content, content_bytes, Compression::Zlib)?;
    builder.add(SectionKind::Glyph, glyph_bytes, Compression::None)?;
    if !image_section.images.is_empty() {
        builder.add(SectionKind::Images, image_bytes, Compression::None)?;
    }
    let package = builder.build()?;

    // 11. Self-check: the assembled package must parse and validate.
    let pkg = parse_package(&package).map_err(Error::Format)?;
    let issues = validate_package(&pkg);
    if !issues.is_empty() {
        let detail = issues
            .iter()
            .take(8)
            .map(|issue| format!("  - [{}] {}", issue.section, issue.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::Format(glyphcull_format::error::Error::Validation {
            detail: format!("self-validation found {} issue(s):\n{detail}", issues.len()),
        }));
    }

    let report = CompileReport {
        title: info.title.clone(),
        lang: info.lang.clone(),
        missing_codepoints: missing,
        atlas_count: info.atlas_count,
        image_count: info.image_count,
        chunk_count: info.chunk_count,
        style_count: info.style_count,
        content_count: info.content_count,
    };
    Ok((package, report))
}

/// Build the STYL properties for a resolved style. Only non-default values are
/// emitted (the codec documents the defaults); the font id is emitted when the
/// face is not the default atlas.
fn style_properties(style: &ResolvedStyle, faces: &BTreeMap<FaceKey, u32>) -> Vec<StyleProperty> {
    let default = ResolvedStyle::default();
    let mut props = Vec::new();
    let font_id = faces
        .get(&FaceKey {
            family: style.font_family.clone(),
            weight: style.font_weight,
            italic: style.italic,
        })
        .copied()
        .unwrap_or(0);
    if font_id != 0 {
        props.push(StyleProperty {
            tag: PropertyTag::FontId,
            value: PropertyValue::U32(font_id),
        });
    }
    if style.font_size_px != default.font_size_px {
        props.push(StyleProperty {
            tag: PropertyTag::FontSizePx,
            value: PropertyValue::F32(style.font_size_px),
        });
    }
    if style.line_height != default.line_height {
        props.push(StyleProperty {
            tag: PropertyTag::LineHeight,
            value: PropertyValue::F32(style.line_height),
        });
    }
    if style.font_weight != default.font_weight {
        props.push(StyleProperty {
            tag: PropertyTag::FontWeight,
            value: PropertyValue::U16(style.font_weight),
        });
    }
    if style.italic != default.italic {
        props.push(StyleProperty {
            tag: PropertyTag::Italic,
            value: PropertyValue::U8(u8::from(style.italic)),
        });
    }
    if style.color != default.color {
        props.push(StyleProperty {
            tag: PropertyTag::Color,
            value: PropertyValue::U32(style.color),
        });
    }
    if style.background != default.background {
        props.push(StyleProperty {
            tag: PropertyTag::BackgroundColor,
            value: PropertyValue::U32(style.background),
        });
    }
    if style.margin_top != default.margin_top {
        props.push(StyleProperty {
            tag: PropertyTag::MarginTop,
            value: PropertyValue::F32(style.margin_top),
        });
    }
    if style.margin_bottom != default.margin_bottom {
        props.push(StyleProperty {
            tag: PropertyTag::MarginBottom,
            value: PropertyValue::F32(style.margin_bottom),
        });
    }
    if style.text_align != default.text_align {
        props.push(StyleProperty {
            tag: PropertyTag::TextAlign,
            value: PropertyValue::U8(style.text_align),
        });
    }
    if style.text_indent != default.text_indent {
        props.push(StyleProperty {
            tag: PropertyTag::TextIndent,
            value: PropertyValue::F32(style.text_indent),
        });
    }
    if style.list_style != default.list_style {
        props.push(StyleProperty {
            tag: PropertyTag::ListStyle,
            value: PropertyValue::U8(style.list_style),
        });
    }
    if style.code != default.code {
        props.push(StyleProperty {
            tag: PropertyTag::Code,
            value: PropertyValue::U8(u8::from(style.code)),
        });
    }
    if style.underline != default.underline {
        props.push(StyleProperty {
            tag: PropertyTag::Underline,
            value: PropertyValue::U8(u8::from(style.underline)),
        });
    }
    if style.letter_spacing != default.letter_spacing {
        props.push(StyleProperty {
            tag: PropertyTag::LetterSpacing,
            value: PropertyValue::F32(style.letter_spacing),
        });
    }
    if style.white_space != default.white_space {
        props.push(StyleProperty {
            tag: PropertyTag::WhiteSpace,
            value: PropertyValue::U8(style.white_space),
        });
    }
    props
}

/// The content-addressed document id: the first 16 bytes of SHA-256 over the
/// decoded content sections in canonical order (CHNK..IMGS), hex-encoded.
fn content_id(payloads: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for payload in payloads {
        hasher.update(payload);
    }
    let digest = hasher.finalize();
    hex_limited(&digest, 16)
}

/// Hex-encode the first `limit` bytes.
fn hex_limited(bytes: &[u8], limit: usize) -> String {
    let mut out = String::with_capacity(limit * 2);
    for b in bytes.iter().take(limit) {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Hex-encode bytes.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Choose a power-of-two atlas page size for a face's glyph count: the smallest
/// power of two (64..=2048) whose area can hold the estimated glyph footprints
/// at a 75% fill factor. Deterministic; the packer opens further pages when a
/// face outgrows the cap.
fn suggest_page_size(glyph_count: usize) -> u32 {
    const AVG_FOOTPRINT: f64 = 1600.0; // ~40×40 texels per glyph at 32 texels/em
    const FILL: f64 = 0.75;
    const MAX_PAGE: u32 = 2048; // 16 MiB per page
    let area = glyph_count as f64 * AVG_FOOTPRINT / FILL;
    let side = area.sqrt().ceil().max(64.0);
    let mut page = 64_u32;
    while page < side as u32 && page < MAX_PAGE {
        page *= 2;
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md_options() -> CompileOptions {
        CompileOptions::default()
    }

    #[test]
    fn compiles_markdown_end_to_end() {
        let source = "# Title\n\nHello *world*!\n";
        let (package, report) =
            compile(source, InputKind::Markdown, &md_options()).expect("compile");
        assert!(!package.is_empty());
        assert!(report.chunk_count > 0);
        assert!(report.atlas_count >= 1);
        assert_eq!(report.title, None);
        // The package round-trips and validates.
        let pkg = parse_package(&package).expect("parse");
        assert!(validate_package(&pkg).is_empty());
    }

    #[test]
    fn compiles_html_with_style() {
        let source = "<html><head><title>T</title><style>p { color: #ff0000; }</style></head>\
                      <body><p>red text</p></body></html>";
        let (package, report) = compile(source, InputKind::Html, &md_options()).expect("compile");
        assert_eq!(report.title.as_deref(), Some("T"));
        let pkg = parse_package(&package).expect("parse");
        assert!(validate_package(&pkg).is_empty());
    }

    #[test]
    fn deterministic_bytes() {
        let source = "# T\n\n- a\n- b\n\n> quote *em*\n";
        let a = compile(source, InputKind::Markdown, &md_options()).expect("a");
        let b = compile(source, InputKind::Markdown, &md_options()).expect("b");
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1);
    }

    #[test]
    fn italic_face_produces_second_atlas() {
        let source = "# T\n\nSome *italic* and **bold** text.\n";
        let (_, report) = compile(source, InputKind::Markdown, &md_options()).expect("compile");
        assert!(report.atlas_count >= 2, "italic + bold faces");
    }

    #[test]
    fn missing_codepoints_reported() {
        // U+10FFFF (outside Noto Sans) must be reported, not crash.
        let source = "# T\n\n\u{10FFFF} text\n";
        let (_, report) = compile(source, InputKind::Markdown, &md_options()).expect("compile");
        assert!(report.missing_codepoints.contains(&0x10FFFF));
    }

    #[test]
    fn block_glyphs_compile_via_the_supplementary_face() {
        // The chart-bar vocabulary (U+2588 FULL BLOCK et al.) is not in the
        // bundled Noto Sans faces but is supplied by the Block Elements
        // supplementary face: compiling a document that uses it must not
        // report missing codepoints.
        let source = "<p>\u{2588}\u{2588}\u{2588}\u{2588}</p>";
        let (package, report) = compile(source, InputKind::Html, &md_options()).expect("compile");
        assert!(
            !report.missing_codepoints.contains(&0x2588),
            "U+2588 must resolve via the supplementary face"
        );
        // And the compiled package actually carries the glyph record.
        let pkg = parse_package(&package).expect("parse");
        let glyf = pkg.section(SectionKind::Glyph).expect("glyf section");
        let section =
            glyphcull_format::codec::glyph::GlyphSection::decode(glyf).expect("decode glyf");
        assert!(
            section
                .atlases
                .iter()
                .flat_map(|a| &a.glyphs)
                .any(|g| g.codepoint == 0x2588),
            "U+2588 must be present in the compiled atlas"
        );
    }

    #[test]
    fn unknown_family_errors() {
        let source = "# T\n\nx\n";
        let mut options = md_options();
        options.user_stylesheets = vec!["p { font-family: NoSuchFont; }".to_string()];
        let result = compile(source, InputKind::Markdown, &options);
        assert!(matches!(result, Err(Error::Font(_))), "{result:?}");
    }

    #[test]
    fn invalid_css_errors() {
        let source = "# T\n";
        let mut options = md_options();
        options.user_stylesheets = vec!["p { color: ".to_string()];
        assert!(matches!(
            compile(source, InputKind::Markdown, &options),
            Err(Error::Css(_))
        ));
    }

    #[test]
    fn image_loader_wired() {
        let source = "![alt](img.png)\n";
        let mut options = md_options();
        options.image_loader =
            Box::new(|_| Ok(include_bytes!("../assets/test/2x1-rgba.png").to_vec()));
        let (package, report) = compile(source, InputKind::Markdown, &options).expect("compile");
        assert_eq!(report.image_count, 1);
        let pkg = parse_package(&package).expect("parse");
        assert!(validate_package(&pkg).is_empty());
    }

    #[test]
    fn image_load_error_propagates() {
        let source = "![alt](missing.png)\n";
        let mut options = md_options();
        options.image_loader = Box::new(|_| Err("file not found".to_string()));
        assert!(matches!(
            compile(source, InputKind::Markdown, &options),
            Err(Error::ImageLoad(_))
        ));
    }

    #[test]
    fn empty_document_compiles() {
        let (package, report) = compile("", InputKind::Markdown, &md_options()).expect("compile");
        assert_eq!(report.chunk_count, 1); // the document chunk
        let pkg = parse_package(&package).expect("parse");
        assert!(validate_package(&pkg).is_empty());
    }
}
