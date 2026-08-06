//! The `cull` command-line tool — library surface.
//!
//! `cull` compiles, validates, and inspects `.cull` packages. The library exposes
//! [`run`] so hosts can embed the tool programmatically; the `cull` binary is a
//! thin wrapper (see `main.rs`).
//!
//! Subcommands (Phase 1):
//! - `cull validate <file>` — structural + semantic validation; exit 0 valid, 1 invalid, 2 error.
//! - `cull inspect <file>` — deterministic package diagnostics.
//!
//! `cull compile` arrives with the compiler pipeline (Phase 2).

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::io::Write;
use std::process::ExitCode;

use glyphcull_format::codec::chunk::{ChunkExtraKind, ChunkSection};
use glyphcull_format::codec::content::{ContentSection, PayloadKind};
use glyphcull_format::codec::glyph::GlyphSection;
use glyphcull_format::codec::image::ImageSection;
use glyphcull_format::codec::info::Info;
use glyphcull_format::codec::seal::SealSection;
use glyphcull_format::codec::style::StyleSection;
use glyphcull_format::reader::parse;
use glyphcull_format::section::SectionKind;
use glyphcull_format::table::Compression;
use glyphcull_format::validate::validate_package;

/// Run the `cull` tool with program arguments (excluding the program name).
///
/// Returns the process exit code: `0` success, `1` invalid input/package,
/// `2` usage or I/O error.
pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("validate") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: cull validate <file.cull>");
                return ExitCode::from(2);
            };
            validate(path)
        }
        Some("compile") => {
            let Some(_path) = args.get(1) else {
                eprintln!(
                    "usage: cull compile <input.md|input.html> [-s style.css]... [-o out.cull]"
                );
                return ExitCode::from(2);
            };
            let rest = args.get(1..).unwrap_or(&[]);
            compile_cmd(rest)
        }
        Some("inspect") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: cull inspect <file.cull>");
                return ExitCode::from(2);
            };
            inspect(path)
        }
        Some("--help") | Some("-h") => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some("--version") | Some("-V") => {
            println!("cull {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown subcommand: {other}");
            print_usage();
            ExitCode::from(2)
        }
        None => {
            print_usage();
            ExitCode::from(2)
        }
    }
}

/// Print the usage text.
fn print_usage() {
    println!(
        "cull — the GlyphCull document package tool\n\
         \n\
         USAGE:\n\
         \x20   cull compile <input.md|input.html>   compile a document to a .cull package\n\
         \x20          [-s style.css]... [-o out.cull]\n\
         \x20   cull validate <file.cull>            structural + semantic validation\n\
         \x20   cull inspect  <file.cull>            package diagnostics\n\
         \x20   cull --version                       print the version\n"
    );
}

fn read_package(path: &str) -> Result<glyphcull_format::ParsedPackage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    parse(&bytes).map_err(|e| format!("{path}: invalid package: {e}"))
}

fn validate(path: &str) -> ExitCode {
    let pkg = match read_package(path) {
        Ok(pkg) => pkg,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(1);
        }
    };
    let issues = validate_package(&pkg);
    if issues.is_empty() {
        println!("valid: {}", path);
        ExitCode::SUCCESS
    } else {
        for issue in &issues {
            eprintln!("{}: {}", issue.section, issue.message);
        }
        eprintln!("invalid: {} ({} issue(s))", path, issues.len());
        ExitCode::from(1)
    }
}

fn inspect(path: &str) -> ExitCode {
    let pkg = match read_package(path) {
        Ok(pkg) => pkg,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(1);
        }
    };

    let mut out = String::new();
    out.push_str(&format!("package: {path}\n"));
    out.push_str(&format!(
        "header: version {} flags {:#06x} sections {}\n",
        pkg.header.version, pkg.header.flags, pkg.header.section_count
    ));
    out.push_str("sections:\n");
    for entry in &pkg.entries {
        let kind = entry.known_kind().map_or_else(
            || format!("reserved({})", entry.kind),
            |k| k.name().to_string(),
        );
        let compression = match entry.compression {
            Compression::None => "raw",
            Compression::Zlib => "zlib",
        };
        out.push_str(&format!(
            "  {kind:<6} offset={:<8} stored={:<8} decoded={:<8} crc={:08x} {compression}\n",
            entry.offset, entry.stored_len, entry.decoded_len, entry.crc32
        ));
    }

    if let Some(payload) = pkg.section(SectionKind::Info) {
        match Info::decode(payload) {
            Ok(info) => {
                out.push_str("info:\n");
                out.push_str(&format!(
                    "  generator: {} {}\n",
                    info.generator, info.generator_version
                ));
                out.push_str(&format!("  document_id: {}\n", info.document_id));
                out.push_str(&format!("  source_digest: {}\n", info.source_digest));
                if let Some(title) = &info.title {
                    out.push_str(&format!("  title: {title}\n"));
                }
                if let Some(lang) = &info.lang {
                    out.push_str(&format!("  lang: {lang}\n"));
                }
                out.push_str(&format!(
                    "  counts: chunks={} styles={} content={} atlases={} images={}\n",
                    info.chunk_count,
                    info.style_count,
                    info.content_count,
                    info.atlas_count,
                    info.image_count
                ));
            }
            Err(e) => out.push_str(&format!("info: undecodable ({e})\n")),
        }
    }

    if let Some(payload) = pkg.section(SectionKind::Chunk) {
        match ChunkSection::decode(payload) {
            Ok(chunks) => {
                out.push_str(&format!(
                    "chunks: {} records, {} extras\n",
                    chunks.chunks.len(),
                    chunks.extras.len()
                ));
                let mut counts: std::collections::BTreeMap<&'static str, usize> =
                    Default::default();
                for chunk in &chunks.chunks {
                    *counts.entry(chunk.kind.name()).or_insert(0) += 1;
                }
                for (kind, count) in counts {
                    out.push_str(&format!("  {kind}: {count}\n"));
                }
                let mut extras: std::collections::BTreeMap<&'static str, usize> =
                    Default::default();
                for extra in &chunks.extras {
                    let name = match extra.kind {
                        ChunkExtraKind::LinkTarget => "link_target",
                        ChunkExtraKind::CellSpan => "cell_span",
                        ChunkExtraKind::ListItemValue => "list_item_value",
                        ChunkExtraKind::ImageAlt => "image_alt",
                    };
                    *extras.entry(name).or_insert(0) += 1;
                }
                for (kind, count) in extras {
                    out.push_str(&format!("  extra {kind}: {count}\n"));
                }
            }
            Err(e) => out.push_str(&format!("chunks: undecodable ({e})\n")),
        }
    }

    if let Some(payload) = pkg.section(SectionKind::Style) {
        match StyleSection::decode(payload) {
            Ok(styles) => {
                let total_props: usize = styles.styles.iter().map(|s| s.properties.len()).sum();
                out.push_str(&format!(
                    "styles: {} records, {total_props} properties\n",
                    styles.styles.len()
                ));
            }
            Err(e) => out.push_str(&format!("styles: undecodable ({e})\n")),
        }
    }

    if let Some(payload) = pkg.section(SectionKind::Content) {
        match ContentSection::decode(payload) {
            Ok(content) => {
                let text_bytes: usize = content
                    .payloads
                    .iter()
                    .filter(|p| p.kind == PayloadKind::TextUtf8)
                    .map(|p| p.data.len())
                    .sum();
                out.push_str(&format!(
                    "content: {} payloads ({} text bytes)\n",
                    content.payloads.len(),
                    text_bytes
                ));
            }
            Err(e) => out.push_str(&format!("content: undecodable ({e})\n")),
        }
    }

    if let Some(payload) = pkg.section(SectionKind::Glyph) {
        match GlyphSection::decode(payload) {
            Ok(glyphs) => {
                for atlas in &glyphs.atlases {
                    let page_bytes: usize = atlas.pages.iter().map(|p| p.len()).sum();
                    out.push_str(&format!(
                        "atlas: font_id={} family={} weight={} italic={} glyphs={} kerning={} pages={} page={}x{} {} texels/em, {} page bytes\n",
                        atlas.font_id,
                        atlas.family,
                        atlas.weight,
                        atlas.italic,
                        atlas.glyphs.len(),
                        atlas.kerning.len(),
                        atlas.pages.len(),
                        atlas.page_width,
                        atlas.page_height,
                        atlas.texels_per_em as f32 / 1024.0,
                        page_bytes
                    ));
                }
            }
            Err(e) => out.push_str(&format!("atlas: undecodable ({e})\n")),
        }
    }

    if let Some(payload) = pkg.section(SectionKind::Images) {
        match ImageSection::decode(payload) {
            Ok(images) => {
                let total_pixels: u64 = images
                    .images
                    .iter()
                    .map(|i| u64::from(i.width) * u64::from(i.height))
                    .sum();
                out.push_str(&format!(
                    "images: {} images, {total_pixels} pixels\n",
                    images.images.len()
                ));
            }
            Err(e) => out.push_str(&format!("images: undecodable ({e})\n")),
        }
    }

    if let Some(payload) = pkg.section(SectionKind::Seal) {
        match SealSection::decode(payload) {
            Ok(seal) => {
                out.push_str(&format!("seal: {} covered sections\n", seal.entries.len()));
            }
            Err(e) => out.push_str(&format!("seal: undecodable ({e})\n")),
        }
    }

    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(out.as_bytes())
        .and_then(|_| stdout.flush())
        .is_err()
    {
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

/// `cull compile <input> [-s style.css]... [-o out.cull]`.
///
/// The input kind is inferred from the extension (`.md`/`.markdown` →
/// Markdown; `.html`/`.htm` → HTML). Stylesheet files are loaded in order
/// after any in-document `<style>` blocks; images are loaded relative to the
/// input file's directory. The output defaults to stdout.
fn compile_cmd(args: &[String]) -> ExitCode {
    let mut input: Option<String> = None;
    let mut stylesheets: Vec<String> = Vec::new();
    let mut output: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-s" | "--style" => {
                let Some(path) = iter.next() else {
                    eprintln!("error: -s requires a stylesheet path");
                    return ExitCode::from(2);
                };
                stylesheets.push(path.clone());
            }
            "-o" | "--output" => {
                let Some(path) = iter.next() else {
                    eprintln!("error: -o requires an output path");
                    return ExitCode::from(2);
                };
                output = Some(path.clone());
            }
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            other => {
                if input.is_some() {
                    eprintln!("error: unexpected argument {other}");
                    return ExitCode::from(2);
                }
                input = Some(other.to_string());
            }
        }
    }
    let Some(input) = input else {
        eprintln!("usage: cull compile <input.md|input.html> [-s style.css]... [-o out.cull]");
        return ExitCode::from(2);
    };

    let source = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {input}: {e}");
            return ExitCode::from(1);
        }
    };
    let kind = match input
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown") => glyphcull_pipeline::InputKind::Markdown,
        Some("html" | "htm") => glyphcull_pipeline::InputKind::Html,
        other => {
            eprintln!("error: cannot infer input kind from extension {other:?} (.md / .html)");
            return ExitCode::from(2);
        }
    };

    let mut user_sheets: Vec<String> = Vec::new();
    for sheet in &stylesheets {
        match std::fs::read_to_string(sheet) {
            Ok(s) => user_sheets.push(s),
            Err(e) => {
                eprintln!("error: cannot read stylesheet {sheet}: {e}");
                return ExitCode::from(1);
            }
        }
    }

    // Images resolve relative to the input file's directory.
    let base = std::path::Path::new(&input)
        .parent()
        .map_or_else(std::path::PathBuf::new, std::path::Path::to_path_buf);
    let options = glyphcull_pipeline::CompileOptions {
        user_stylesheets: user_sheets,
        image_loader: Box::new(move |src: &str| {
            let path = base.join(src);
            std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
        }),
        ..glyphcull_pipeline::CompileOptions::default()
    };

    match glyphcull_pipeline::compile(&source, kind, &options) {
        Ok((package, report)) => {
            if let Some(out_path) = output {
                if let Err(e) = std::fs::write(&out_path, &package) {
                    eprintln!("error: cannot write {out_path}: {e}");
                    return ExitCode::from(1);
                }
                println!(
                    "compiled {input} → {out_path} ({} bytes; {} chunks, {} styles, {} content, {} atlases, {} images)",
                    package.len(),
                    report.chunk_count,
                    report.style_count,
                    report.content_count,
                    report.atlas_count,
                    report.image_count
                );
            } else {
                let stdout = std::io::stdout().lock();
                let mut stdout = std::io::BufWriter::new(stdout);
                if stdout
                    .write_all(&package)
                    .and_then(|_| stdout.flush())
                    .is_err()
                {
                    eprintln!("error: cannot write stdout");
                    return ExitCode::from(1);
                }
            }
            for cp in &report.missing_codepoints {
                eprintln!(
                    "warning: codepoint U+{cp:04X} ({:?}) has no glyph in the bundled fonts",
                    char::from_u32(*cp)
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
