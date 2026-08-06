//! Memory regression test: peak allocation during a compile must stay within
//! the committed budget (PERFORMANCE.md §3: peak < 16 × decoded package size).
//!
//! Measurement: the process RSS high-water mark (`VmHWM` from `/proc/self/status`,
//! Linux) before and after the compile; the delta is the compile's peak footprint
//! (including allocator arena growth — a conservative overestimate, which is the
//! right direction for a regression gate). Deterministic and noise-free for the
//! same binary + input.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

/// The committed peak-memory multiplier (PERFORMANCE.md §3).
const PEAK_MULTIPLIER: usize = 16;

/// The process RSS high-water mark in bytes (Linux).
fn vmhwm_bytes() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kb: usize = rest
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse()
                .expect("parse VmHWM");
            return kb * 1024;
        }
    }
    panic!("VmHWM not found in /proc/self/status");
}

#[test]
fn compile_peak_memory_within_budget() {
    // A document exercising every stage: many paragraphs (styles, runs), a
    // table, lists, a code block, an image.
    let mut source = String::from("# Memory\n\n");
    for i in 0..400 {
        source.push_str(&format!(
            "Paragraph {i} with *em* **strong** `code` and a [link](https://e.test).\n\n"
        ));
    }
    source.push_str(
        "| a | b |\n|---|---|\n| 1 | 2 |\n\n- x\n- y\n\n```text\ncode\n```\n\n![alt](img.png)\n",
    );

    let baseline = vmhwm_bytes();
    let options = glyphcull_pipeline::CompileOptions {
        image_loader: Box::new(|_| Ok(include_bytes!("../assets/test/2x1-rgba.png").to_vec())),
        ..glyphcull_pipeline::CompileOptions::default()
    };
    let (package, _) =
        glyphcull_pipeline::compile(&source, glyphcull_pipeline::InputKind::Markdown, &options)
            .expect("compile");
    let peak = vmhwm_bytes() - baseline;
    let budget = package.len() * PEAK_MULTIPLIER;
    assert!(
        peak < budget,
        "peak footprint {peak} bytes exceeds the committed budget {budget} \
         (package {} bytes × {PEAK_MULTIPLIER})",
        package.len()
    );
}
