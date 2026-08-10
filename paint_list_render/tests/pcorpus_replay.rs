/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Corpus replay — consumer paint lists as a regression suite.
//!
//! netrender's other ~271 tests are scenes it wrote for itself. They
//! prove the API does what the API says. They cannot catch the case
//! where a consumer drives the vocabulary in a shape nobody anticipated,
//! because no consumer wrote them.
//!
//! This harness closes that gap. Each fixture in `tests/corpus/` is a
//! postcard-encoded [`PaintEnvelope`] as some consumer actually emitted
//! it (or, for the seeds, as one demonstrably does). For each one we
//! assert:
//!
//! 1. **Wire stability** — decode then re-encode reproduces the original
//!    bytes. Catches a serde representation change that would silently
//!    invalidate every stored fixture and every IPC peer.
//! 2. **Provenance** — every fixture says where it came from. A corpus
//!    that cannot tell you whether a scene is real or invented is not
//!    evidence.
//! 3. **Translation stability** — `translate_paint_list` produces the
//!    exact `Scene` op stream recorded in the fixture's `.ops` file.
//!    This is the consumer-visible contract, and the diff is readable.
//!
//! Deliberately CPU-only: no GPU, no device boot, so it runs anywhere.
//! The rasterizer already has GPU receipts; what was missing was
//! coverage of the *ingress*.
//!
//! Recording a new or changed golden:
//!
//! ```text
//! NETRENDER_CORPUS_BLESS=1 cargo test -p paint_list_render --test pcorpus_replay
//! ```
//!
//! Review the resulting `.ops` diff like any other change. A golden that
//! moved without an intended reason is the bug this file exists to find.

use std::fs;
use std::path::{Path, PathBuf};

use paint_list_api::PaintEnvelope;
use paint_list_render::translate_paint_list;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
}

fn blessing() -> bool {
    std::env::var("NETRENDER_CORPUS_BLESS").is_ok()
}

/// Every `*.paintlist` in the corpus, sorted so failures are reported in
/// a stable order.
fn fixtures() -> Vec<PathBuf> {
    let dir = corpus_dir();
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read corpus dir {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("paintlist"))
        .collect();
    paths.sort();
    paths
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .expect("fixture stem")
        .to_string()
}

fn load(path: &Path) -> (Vec<u8>, PaintEnvelope) {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let envelope: PaintEnvelope = postcard::from_bytes(&bytes).unwrap_or_else(|e| {
        panic!(
            "{} is not a postcard-encoded PaintEnvelope: {e}\n\
             If the vocabulary changed shape, the fixture must be re-captured, \
             not hand-edited.",
            path.display()
        )
    });
    (bytes, envelope)
}

/// An empty corpus would let every other test in this file pass
/// vacuously. That failure mode is exactly what a regression suite is
/// supposed to not have.
#[test]
fn corpus_is_populated() {
    let found = fixtures();
    assert!(
        !found.is_empty(),
        "no *.paintlist fixtures in {}. Regenerate the seeds with \
         `cargo run -p paint_list_render --example seed_corpus`.",
        corpus_dir().display()
    );
}

/// A fixture without provenance is an assertion with no author. Require
/// each one to state where it came from and whether it was captured from
/// a running consumer or synthesized.
#[test]
fn every_fixture_has_provenance() {
    for path in fixtures() {
        let sidecar = path.with_extension("provenance");
        let text = fs::read_to_string(&sidecar).unwrap_or_else(|e| {
            panic!(
                "{} has no readable .provenance sidecar ({e}).\n\
                 See tests/corpus/README.md for the required keys.",
                path.display()
            )
        });
        for key in ["source:", "captured:", "describes:"] {
            assert!(
                text.lines().any(|l| l.trim_start().starts_with(key)),
                "{} is missing a `{key}` line",
                sidecar.display()
            );
        }
    }
}

/// Decode then re-encode must reproduce the stored bytes. If this breaks,
/// the paint vocabulary's serde representation changed, and every stored
/// fixture plus every IPC peer on a different build is now wrong.
#[test]
fn wire_encoding_round_trips() {
    for path in fixtures() {
        let (bytes, envelope) = load(&path);
        let reencoded = postcard::to_allocvec(&envelope)
            .unwrap_or_else(|e| panic!("re-encode {}: {e}", path.display()));
        assert_eq!(
            reencoded,
            bytes,
            "{} does not round-trip through postcard ({} bytes in, {} out). \
             The PaintEnvelope wire representation changed.",
            path.display(),
            bytes.len(),
            reencoded.len()
        );
    }
}

/// The real assertion: a given consumer paint list lowers to a specific
/// `Scene` op stream, and that stream does not drift silently.
#[test]
fn translation_matches_recorded_ops() {
    let mut blessed = Vec::new();

    for path in fixtures() {
        let (_, envelope) = load(&path);
        let scene = translate_paint_list(&envelope);
        let actual = scene.dump_ops();

        let golden_path = path.with_extension("ops");

        if blessing() {
            fs::write(&golden_path, &actual)
                .unwrap_or_else(|e| panic!("write {}: {e}", golden_path.display()));
            blessed.push(stem(&path));
            continue;
        }

        let expected = fs::read_to_string(&golden_path).unwrap_or_else(|e| {
            panic!(
                "{} has no recorded .ops ({e}).\n\
                 Record it with NETRENDER_CORPUS_BLESS=1 and review the result.",
                path.display()
            )
        });

        // `.gitattributes` marks `*.ops` as `-text` so a Windows
        // checkout cannot rewrite them to CRLF. Normalize anyway: a
        // contributor whose git predates that rule, or who edits a
        // golden by hand in a CRLF editor, should get a real diff rather
        // than a whole-file mismatch on invisible characters.
        let expected = expected.replace("\r\n", "\n");

        if actual != expected {
            let (line_no, exp_line, act_line) = first_difference(&expected, &actual);
            panic!(
                "{} translated differently than recorded.\n\
                 First difference at line {line_no}:\n  \
                 expected: {exp_line}\n  \
                 actual:   {act_line}\n\
                 ({} expected lines, {} actual)\n\n\
                 If this change is intended, re-record with \
                 NETRENDER_CORPUS_BLESS=1 and include the .ops diff in the commit.",
                stem(&path),
                expected.lines().count(),
                actual.lines().count(),
            );
        }
    }

    if !blessed.is_empty() {
        // Loud on purpose: a blessed run asserts nothing.
        panic!(
            "recorded {} golden(s): {}. \
             Re-run without NETRENDER_CORPUS_BLESS to verify.",
            blessed.len(),
            blessed.join(", ")
        );
    }
}

/// First differing line, 1-indexed, with both sides. Missing lines render
/// as `<none>` so a truncation is obvious.
fn first_difference(expected: &str, actual: &str) -> (usize, String, String) {
    let mut exp = expected.lines();
    let mut act = actual.lines();
    let mut line_no = 0;
    loop {
        line_no += 1;
        match (exp.next(), act.next()) {
            (None, None) => return (line_no, "<none>".into(), "<none>".into()),
            (e, a) if e != a => {
                return (
                    line_no,
                    e.unwrap_or("<none>").to_string(),
                    a.unwrap_or("<none>").to_string(),
                )
            }
            _ => {}
        }
    }
}
