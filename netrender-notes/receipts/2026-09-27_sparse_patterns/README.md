# Sparse pattern session validation, 2026-09-27

## Identity and scope

Base: NetRender `402a925e3c6a6430bfd8226a3989e3aec3e72b2b`, clean before
this pattern change. Local receipt date: September 27, America/New_York.
Sparse CPU/Hybrid/common remain pinned to
`ca3f40ea182216883cd543c7b9deae991268917c`; Classic remains
`netrender-vello` 0.10.1. The dependency manifests and lock are unchanged;
`dependency-hashes.txt` and the preceding image-session receipt preserve
their identities. Shared boundary versions remain wgpu 30.0.0 and Peniko 0.6.1.

Toolchain: rustc 1.97.1 (`8bab26f4f`, 2026-07-14), cargo 1.97.1
(`c980f4866`, 2026-06-30), `x86_64-pc-windows-msvc`.
Every command runs from `C:\Users\mark_\Code\repos\netrender` with
`$env:CARGO_TARGET_DIR='C:\t\cargo-targets\netrender'` and `-j2`.
No isolated Cargo home, worktree, or target was created. The stable target is
retained for repository reuse. No live NetRender build owner existed at start.

An initial online locked check waited for the shared package-cache lock;
only this lane's verified waiting Cargo processes were stopped. The same
commands with `--offline --locked` proceeded using existing cached sources.
Other workspace build owners were left alone. `check-package-cache-wait.txt`
records the interrupted wait, not a successful check.

## Measured bilinear repeat defect

The first implementation admitted both samplers on both sparse backends.
CPU's two pattern tests passed. Hybrid's lifecycle test passed, but the
semantic test failed at a repeat seam:

- Source: two opaque pixels, red then blue, scaled by 8 on each axis.
- Pattern extent begins at x=5, so the repeated tile boundary is x=21.
- At x=20, y=24 Hybrid returned **[0, 0, 255, 255]**.
- The independent seam assertion requires both red and blue channels above
  80 and alpha above 250, reflecting interpolation across the repeated edge.
- The same assertion passes on CPU at x=20 and x=21. Exact CPU bytes were
  not logged, so this receipt makes no claim about them.

Evidence: `initial-cpu-patterns.txt`, `initial-hybrid-patterns.txt`,
`initial-source-hashes.txt`, and the byte-preserved initial test source
`initial-pattern-fixture.rs.txt` (SHA256
`E58B036486564421F364068FA4EF6F2DED2C236E911B69C5601E129849E01CD5`).

Pinned shader inspection explains the failure: `render.wesl` wraps the
central image coordinate, then `external_texture.wesl` clamps bilinear taps
to source edges. CPU wraps each neighboring tap separately. The atlas path
uses the same bounded sampling helper, so switching to atlas uploads would
not repair the seam. No dependency patch or pin change is claimed.

## Final admitted subset

CPU sessions admit nearest and bilinear repeated patterns. Hybrid sessions
admit **explicit nearest** patterns and return a typed Pattern refusal for
bilinear before source updates, cache changes, uploads, or target commands.
There is no automatic sampler substitution.

Both retain Classic's extent-top-left phase, independent axis scale,
nonpositive-axis normalization to 1, affine placement, device-space rounded
clips, and nested alpha behavior. Neutral full-tile pattern pixels share
cached variants with ordinary untinted full images. Admission bounds resource
counts/bytes, source payloads, parameters, transformed bounds, and affine
inverses. Resources remain owned by the existing image sessions.

This is a bounded adapter receipt. It does not close full VB2 sampler parity,
shaped text, actual Isocosm panel acceptance, native interaction, performance,
or browser backend interchangeability. Hybrid bilinear repeat remains open.

## Final commands

```powershell
cargo check --offline --locked -p netrender -j2
cargo check --offline --locked -p netrender -j2 --features vello-cpu
cargo check --offline --locked -p netrender -j2 --features vello-hybrid
cargo check --offline --locked -p netrender -j2 --features vello-all
cargo test --offline --locked -p netrender -j2 --features vello-cpu --test vb2_sparse_patterns --test vb2_sparse_images -- --nocapture --test-threads=1
cargo test --offline --locked -p netrender -j2 --features vello-hybrid --test vb2_sparse_patterns --test vb2_sparse_images -- --nocapture --test-threads=1
cargo test --offline --locked -p netrender -j2 --features vello-all --lib -- --nocapture --test-threads=1
cargo test --offline --locked -p netrender -j2 --features vello-all --test rg2a_scene_corpus --test pc2_pattern_op --test pc2b_pattern_scale_render -- --nocapture --test-threads=1
```

All final commands completed with exit code zero. Final source hashes are
recorded in `final-source-hashes.txt` before these commands.

| Final gate | Result |
| --- | --- |
| Default / CPU / Hybrid / combined compile | All pass |
| CPU-only patterns | 2 passed; includes bilinear seam interpolation |
| CPU-only image regression | 3 passed |
| Hybrid-only patterns | 2 passed; includes atomic bilinear refusal |
| Hybrid-only image regression | 2 passed |
| Combined library | 75 passed, 0 failed, 3 intentionally ignored |
| RG2a geometry corpus and legacy refusal table | 2 passed |
| Classic pattern op/invalidation tests | 8 passed |
| Classic pattern scale GPU readback | 2 passed |

Physical GPU evidence uses **NVIDIA GeForce RTX 4060 Laptop GPU**, **Vulkan**,
**NVIDIA 610.88**, as recorded by RG2a and the Hybrid tests. CPU sessions do
not request a GPU device. The ignored library tests are existing physical
RG2b/RG2c and repeated-plan timing receipts; their results were not refreshed.
These are automated pixel/readback tests, not a native application receipt.

Hybrid refusal tests submit/read back the preserved target, compare resource
stats before/after rejection, and verify an attempted green source replacement
did not replace the previously admitted red/blue tile. New pattern and existing
image tests were run independently with CPU-only and Hybrid-only features;
combined compilation/library/RG2a checks cover feature coexistence.

Final hygiene checks passed: `rustfmt --check --edition 2021 --config
skip_children=true` for the three implementation modules and two VB2 test
files; `git diff --check` for tracked edits; and `git diff --no-index --check`
against `NUL` for every new receipt file and the new pattern test. Log-only
trailing whitespace/blank EOF lines were normalized; the initial fixture
snapshot was left byte-identical and its hash reverified. All final source
hashes remained unchanged after tests. No live NetRender Cargo/compiler/test
process remained when the receipt finished.
