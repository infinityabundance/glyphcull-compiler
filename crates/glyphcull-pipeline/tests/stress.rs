//! Stress tests for the compiler pipeline: large generated documents must
//! compile deterministically, validate, and stay within sane resource bounds.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use glyphcull_pipeline::{compile, CompileOptions, InputKind};

/// Build a large markdown document (~300 paragraphs with mixed inline markup,
/// lists, a table, and a code block).
fn large_document(paragraphs: usize) -> String {
    let mut out = String::new();
    out.push_str("# Large Document\n\n");
    for i in 0..paragraphs {
        out.push_str(&format!(
            "Paragraph {i} with *emphasis*, **strong**, `code`, and a [link](https://e.test).\n\n"
        ));
    }
    out.push_str("- item a\n- item b\n- item c\n\n");
    out.push_str("| h1 | h2 |\n|---|---|\n| a | b |\n| c | d |\n\n");
    out.push_str("```text\nfn main() {}\n```\n");
    out
}

#[test]
fn large_document_compiles_and_validates() {
    let source = large_document(300);
    let options = CompileOptions::default();
    let (package, report) = compile(&source, InputKind::Markdown, &options).expect("compile");
    assert!(report.chunk_count > 1000);
    assert!(report.atlas_count >= 3);
    let pkg = glyphcull_format::reader::parse(&package).expect("parse");
    let issues = glyphcull_format::validate::validate_package(&pkg);
    assert!(issues.is_empty(), "issues: {issues:?}");
}

#[test]
fn large_document_is_deterministic() {
    let source = large_document(150);
    let options = CompileOptions::default();
    let a = compile(&source, InputKind::Markdown, &options).expect("a");
    let b = compile(&source, InputKind::Markdown, &options).expect("b");
    assert_eq!(a.0.len(), b.0.len());
    assert_eq!(a.0, b.0);
    assert_eq!(a.1, b.1);
}

#[test]
fn many_faces_compile() {
    // Exercise all four bundled faces (regular, bold, italic, bold+italic) in
    // one document.
    let source = "# T\n\nregular *italic* **bold** ***both***\n";
    let options = CompileOptions::default();
    let (package, report) = compile(source, InputKind::Markdown, &options).expect("compile");
    assert_eq!(report.atlas_count, 4);
    let pkg = glyphcull_format::reader::parse(&package).expect("parse");
    assert!(glyphcull_format::validate::validate_package(&pkg).is_empty());
}

#[test]
fn deep_document_bounded() {
    // A pathological nesting depth must not crash the compiler: the semantic
    // front end bounds it (MAX_DEPTH), so either a bounded error or a valid
    // compile is acceptable — never a panic.
    let mut source = String::new();
    for _ in 0..200 {
        source.push_str("> ");
    }
    source.push_str("deep\n");
    let options = CompileOptions::default();
    let _ = compile(&source, InputKind::Markdown, &options);
}

#[test]
fn html_stress() {
    let mut source = String::from("<html><head><title>S</title></head><body>");
    for i in 0..200 {
        let class = format!("p{}", i % 3);
        source.push_str(&format!(
            "<p class=\"{class}\">paragraph <strong>{i}</strong> <em>text</em></p>"
        ));
    }
    source.push_str("</body></html>");
    let options = CompileOptions::default();
    let (package, report) = compile(&source, InputKind::Html, &options).expect("compile");
    assert!(report.chunk_count > 500);
    let pkg = glyphcull_format::reader::parse(&package).expect("parse");
    assert!(glyphcull_format::validate::validate_package(&pkg).is_empty());
}
