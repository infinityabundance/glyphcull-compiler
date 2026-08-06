//! End-to-end CLI tests: `cull compile` → `cull validate` → `cull inspect`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::process::ExitCode;

/// Run the CLI with args.
fn run(args: &[&str]) -> ExitCode {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    glyphcull_cli::run(&args)
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cull-cli-test-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

#[test]
fn compile_validate_inspect_roundtrip() {
    let dir = temp_dir("roundtrip");
    let md = dir.join("doc.md");
    let out = dir.join("doc.cull");
    std::fs::write(&md, "# Title\n\nHello *world*.\n").expect("write md");

    // compile
    assert_eq!(
        run(&["compile", md.to_str().unwrap(), "-o", out.to_str().unwrap()]),
        ExitCode::SUCCESS
    );
    assert!(out.exists());
    let bytes = std::fs::read(&out).expect("read cull");
    assert!(bytes.len() > 1000);

    // validate
    assert_eq!(run(&["validate", out.to_str().unwrap()]), ExitCode::SUCCESS);

    // inspect
    assert_eq!(run(&["inspect", out.to_str().unwrap()]), ExitCode::SUCCESS);

    // compile to stdout (no -o): exit 0 (bytes go to the harness's stdout).
    assert_eq!(run(&["compile", md.to_str().unwrap()]), ExitCode::SUCCESS);

    // usage errors
    assert_eq!(run(&["compile"]), ExitCode::from(2));
    assert_eq!(run(&["validate"]), ExitCode::from(2));
    assert_eq!(run(&["nonexistent-subcommand"]), ExitCode::from(2));

    // nonexistent input
    assert_eq!(
        run(&["compile", "/nonexistent/input.md"]),
        ExitCode::from(1)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn html_compile_with_style() {
    let dir = temp_dir("html");
    let html = dir.join("doc.html");
    let css = dir.join("style.css");
    let out = dir.join("doc.cull");
    std::fs::write(
        &html,
        "<html><head><title>T</title></head><body><p>styled</p></body></html>",
    )
    .expect("write html");
    std::fs::write(&css, "p { color: #123456; }").expect("write css");
    assert_eq!(
        run(&[
            "compile",
            html.to_str().unwrap(),
            "-s",
            css.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ]),
        ExitCode::SUCCESS
    );
    assert_eq!(run(&["validate", out.to_str().unwrap()]), ExitCode::SUCCESS);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_extension_rejected() {
    let dir = temp_dir("ext");
    let file = dir.join("doc.txt");
    std::fs::write(&file, "x").expect("write");
    assert_eq!(run(&["compile", file.to_str().unwrap()]), ExitCode::from(2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_image_fails_compile() {
    let dir = temp_dir("image");
    let md = dir.join("doc.md");
    let out = dir.join("o.cull");
    std::fs::write(&md, "![alt](missing.png)\n").expect("write");
    assert_eq!(
        run(&["compile", md.to_str().unwrap(), "-o", out.to_str().unwrap()]),
        ExitCode::from(1)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
