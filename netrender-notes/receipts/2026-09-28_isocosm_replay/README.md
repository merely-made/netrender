# Intact Isocosm panel replay receipt

Local date: 2026-09-28 (America/New_York). NetRender baseline:
`da229106de51a2f939546d673039d490690ca32c`.

The original Isocosm Simulation/Inspect panel rendered intact through CPU,
Hybrid, and Classic. No operation was removed, font substituted, glyph
reshaped, or session admission rule relaxed. Classic reproduces the paired
reference PNG exactly, including its encoded-file hash. CPU and Hybrid pass
the independent visual/regional review below with small edge-localized raster
differences. This qualifies this paused tick-0 panel, not interactive operation
or universal renderer parity.

## Input and source identity

Original receipt:
`repos/isometry/mesocosm/testing/bench/receipts/2026-09-27/paint-capture/source.json`.
Its SHA-256 and verified input hashes are in `input-provenance.json`.

The source packet is
`C:\Users\mark_\Code\testing\isometry\receipts\2026-09-27\paint-capture\sim-sim-founded.paintlist`,
2,511,691 bytes, SHA-256
`73c4a9ce901957bcccdc371cdede05f12c671d7780376be71f2c2a02ff3ddd7d`.
The paired PNG is 709,375 bytes, SHA-256
`9061fb4263eead6cd540af4d53760ae4566c00947172485d522532edc66b2cfb`.
Both match the original receipt and remained unchanged after every run/control.

The packet contains 221 commands, including 73 text runs and 1,228 positioned
glyphs. Three embedded fonts total 2,489,052 bytes. Translation produces 260
scene operations: 73 glyph runs, 90 rectangles, 23 strokes, and 37 push/pop
layer pairs. Logical viewport 1232 x 752 is presented at 2x, yielding
2464 x 1504 pixels. Lossless postcard roundtrip, font bytes/face indices,
resource counts, and positioned glyph identity are checked before rendering.
All three runs report the same positioned-text digest.

Source hashes are in `source-hashes.txt`; per-feature executable hashes are in
`executable-hashes.txt`. No renderer/session implementation changed in this
slice. The replay example and its package's optional feature forwarding and
development dependencies were added. The lock update adds four direct edges
and cached `sha2 0.10.9`; no existing package version or fork revision changed.
The initial expected locked refusal and offline refresh are retained.

Sparse CPU/Hybrid/Glifo remain at fork
`ca3f40ea182216883cd543c7b9deae991268917c`, Classic is registry
`netrender-vello 0.10.1`, and both share wgpu 30.0.0. See `dependency-tree.txt`.
Toolchain is rustc/Cargo 1.97.1, x86_64-pc-windows-msvc, dev profile.

## Replay and controls

All builds reuse `C:\t\cargo-targets\netrender` with `-j2`. From the NetRender
checkout, set `$env:CARGO_TARGET_DIR='C:\t\cargo-targets\netrender'` and run:

```text
cargo test --offline --locked -p paint_list_render --example sparse_panel_replay --features replay-cpu -j2 -- --nocapture
cargo build --offline --locked -p paint_list_render --example sparse_panel_replay --features replay-cpu -j2
sparse_panel_replay.exe PACKET REFERENCE OUTDIR cpu
cargo build --offline --locked -p paint_list_render --example sparse_panel_replay --features replay-hybrid -j2
sparse_panel_replay.exe PACKET REFERENCE OUTDIR hybrid
cargo build --offline --locked -p paint_list_render --example sparse_panel_replay -j2
sparse_panel_replay.exe PACKET REFERENCE OUTDIR classic
sparse_panel_replay.exe PACKET REFERENCE OUTDIR classic
sparse_panel_replay.exe PACKET REFERENCE OUTDIR/wrong-scale classic --scale 1
cargo test --offline --locked -p paint_list_render --lib --test pcorpus_replay -j2 -- --nocapture --test-threads=1
```

`PACKET` and `REFERENCE` are the verified files above. The executable is
`C:\t\cargo-targets\netrender\debug\examples\sparse_panel_replay.exe`.
`OUTDIR` is
`C:\Users\mark_\Code\testing\netrender\receipts\2026-09-28_isocosm_replay`.
Each backend was executed immediately after its own feature build; CPU had no
Hybrid feature and returned before GPU initialization. Default Classic had
neither sparse feature enabled.

All three primary replays exited zero and rendered, with no typed refusal.
Two focused example tests passed: alpha-convention conversion and one-time
world/device-clip presentation scaling. Classic's exact reference match also
independently validates the full physical-scene rewrite for this capture.
The existing translator library (14 tests) and corpus replay suite (4 tests)
also passed with default features.

Every replay includes a deliberately broken missing-font palette control,
which refuses before rendering. The repeated Classic command exits 1 through
`create_new` protection; its existing JSON/PNG remain unchanged. Wrong scale
also exits 1, before GPU initialization, and creates no PNG. See
`negative-controls.json` and corresponding logs.

All backends first render to transparent output. Classic's straight-alpha
pixels are multiplied by alpha with rounded integer conversion; CPU/Hybrid
already return premultiplied pixels. Common opaque-black presentation then
sets alpha to 255. The initial code-review correction was made before any
replay execution and covered by the nonopaque unit control. Raw nonopaque
pixel counts were CPU 152, Hybrid 0, Classic 0. CPU's presentation therefore
actually exercised nonopaque sparse output.

## Image evidence

| Backend | Changed pixels | Maximum channel delta | Mean absolute RGBA channel delta |
| --- | ---: | ---: | ---: |
| Classic | 0 | 0 | 0 |
| CPU | 70,429 | 64 | 0.0282991973 |
| Hybrid | 71,886 | 64 | 0.0281225849 |

Channel units are 0..255. These are measured differences, not a global pass
tolerance. The independent reviewer selected six physical regions before
seeing outputs: title, controls, status, site cards, individual inspector,
and footer. Their rectangles, dark-pixel definition, counts, and difference
localization are in `semantic-review.json`. Both outputs preserve visible and
aligned content across all regions. For CPU and Hybrid, every pixel whose
maximum channel delta exceeds 1 lies on a reference color edge; none lies off
those edges. Thresholds 8 and 32 also have zero off-edge differences.

The edge diagnostic uses immediate horizontal/vertical RGB neighbors without
wrapping at boundaries. It supports the observed edge-localized differences;
it does not prove universal semantic equivalence. CPU/Hybrid are not byte
identical to Classic or each other.

Hybrid and Classic used NVIDIA GeForce RTX 4060 Laptop GPU, Vulkan, NVIDIA
610.88. CPU reports `gpu_initialized: false` and null GPU wait/readback times.
Both sparse sessions retain 111 glyph variants, 2,427 outline segments, and
three fonts after this frame, without a cache reset.

## Timing scope and retained artifacts

| Backend | Preparation ms | Initialization ms | Render/encode/submit ms | GPU wait ms | Readback ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| CPU | 317.426 | 0.083 | 533.397 | n/a | n/a |
| Hybrid | 314.023 | 1630.076 | 226.448 | 1.291 | 8.911 |
| Classic | 314.541 | 1395.957 | 25.321 | 8.354 | 5.504 |

These are single cold-frame dev-profile diagnostics on a shared machine with
other build/test work. CPU time includes synchronous rasterization; GPU submit
time is separate from completion/readback. Preparation also includes validation
and reference loading. These numbers do not establish interactive frame rate,
steady-state performance, or comparative production efficiency.

Raw capture/font bytes and rendered PNGs remain local. Git receives only the
harness and text/JSON receipts, with artifact paths/hashes in
`artifact-hashes.json`. No system font data was copied into repository fixtures.
The output directory contains the three replay PNG/JSON pairs and the failed
scale-control JSON. No extra Cargo home, target variant, or worktree was created;
the stable NetRender target is retained for reuse.

This panel has no images, patterns, external producer textures, or generated
shadow masks. Their real-consumer acceptance remains separate. Existing
synthetic variable-font/fallback/refusal/lifecycle tests supply their own
bounded evidence; this capture does not replace them. Font parsing still uses
trusted assets. Live producer composition, device loss, warm reuse, motion,
input latency, and whole-loop performance remain separate gates.
