//! Golden-package determinism test for the compiler pipeline.
//!
//! The committed fixture (`tests/fixtures/golden.cull`) is the byte-exact
//! output of the current compiler over `golden.md` + `golden.css`. Any change
//! to the pipeline — the semantic graph, the chunk partition, the cascade, the
//! atlas generator, the section codecs — that alters the compiled bytes fails
//! this test, forcing a reviewed regeneration (`scripts/regenerate-goldens.sh`).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use glyphcull_pipeline::{compile, CompileOptions, InputKind};

/// The committed golden fixture bytes.
const GOLDEN: &[u8] = include_bytes!("fixtures/golden.cull");
/// The golden source document.
const GOLDEN_MD: &str = include_str!("fixtures/golden.md");
/// The golden user stylesheet.
const GOLDEN_CSS: &str = "p { color: #336699; }\n";

#[test]
fn golden_package_is_byte_exact() {
    let options = CompileOptions {
        user_stylesheets: vec![GOLDEN_CSS.to_string()],
        ..CompileOptions::default()
    };
    let (package, report) = compile(GOLDEN_MD, InputKind::Markdown, &options).expect("compile");
    assert_eq!(report.chunk_count, 18);
    assert_eq!(report.atlas_count, 3);
    assert_eq!(
        package, GOLDEN,
        "compiled bytes differ from the committed golden fixture; run \
         scripts/regenerate-goldens.sh and review the diff"
    );
}

/// Regenerate the committed fixture (used by the scripts; ignored by default).
#[test]
#[ignore]
fn regenerate_fixture() {
    let options = CompileOptions {
        user_stylesheets: vec![GOLDEN_CSS.to_string()],
        ..CompileOptions::default()
    };
    let (package, _) = compile(GOLDEN_MD, InputKind::Markdown, &options).expect("compile");
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden.cull"),
        &package,
    )
    .expect("write fixture");
}

/// The committed fixture itself must parse and validate (independent of the
/// compiler that produced it — the reader must not trust its own output).
#[test]
fn golden_fixture_validates_independently() {
    let pkg = glyphcull_format::reader::parse(GOLDEN).expect("parse golden");
    let issues = glyphcull_format::validate::validate_package(&pkg);
    assert!(issues.is_empty(), "golden fixture issues: {issues:?}");
    let seal = pkg
        .section(glyphcull_format::section::SectionKind::Seal)
        .expect("seal");
    let seal_section =
        glyphcull_format::codec::seal::SealSection::decode(seal).expect("seal decode");
    let covered: Vec<(glyphcull_format::section::SectionKind, &[u8])> = pkg
        .sections
        .iter()
        .filter(|(kind, _)| *kind != &glyphcull_format::section::SectionKind::Seal)
        .map(|(kind, section)| (*kind, section.payload.as_slice()))
        .collect();
    assert!(
        glyphcull_format::codec::seal::verify_seal(&seal_section, &pkg.header, &covered).is_ok()
    );
}
