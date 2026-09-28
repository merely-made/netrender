# Sparse outline text validation receipt

Local date: 2026-09-27 (America/New_York). Baseline commit:
`6cd346f6b0cde82f443adc106da2855d4aa0fe89`. This receipt qualifies the
bounded synthetic VB3 outline-text slice. A real Isocosm panel capture and
consumer comparison remain open; this is not a complete VB3 consumer receipt.

## Dependency and execution identity

Classic remains registry `netrender-vello 0.10.1`. CPU, Hybrid, sparse-common,
and the newly direct optional `glifo 0.3.0` use the unchanged fork revision
`ca3f40ea182216883cd543c7b9deae991268917c`. The inverse dependency trees
confirm shared Glifo, Skrifa 0.44.0, Peniko 0.6.1, and wgpu 30.0.0 identities.
`Cargo.lock` changed only by adding `glifo` to NetRender's direct dependency
list. No package revision or version changed.

The initial locked default check correctly refused the new manifest edge
(`initial-check-default.txt`). An authorized offline unlocked default check
refreshed that edge and passed (`lock-refresh-check-default.txt`). All final
commands used `--offline --locked`. There were no compiler or runtime failures.

Toolchain: rustc 1.97.1, Cargo 1.97.1, x86_64-pc-windows-msvc; full identity in
`environment.txt`. All builds reused `C:\t\cargo-targets\netrender` with `-j2`.
No separate Cargo home, target variant, or worktree was created. This stable
target remains available for NetRender development.

## Commands and results

Run from `C:\Users\mark_\Code\repos\netrender` with
`$env:CARGO_TARGET_DIR='C:\t\cargo-targets\netrender'`:

```text
cargo check --offline --locked -p netrender -j2
cargo check --offline --locked -p netrender -j2 --features vello-cpu
cargo check --offline --locked -p netrender -j2 --features vello-hybrid
cargo check --offline --locked -p netrender -j2 --features vello-all
cargo test --offline --locked -p netrender -j2 --features vello-cpu --test vb3_sparse_text --test vb2_sparse_images --test vb2_sparse_patterns -- --nocapture --test-threads=1
cargo test --offline --locked -p netrender -j2 --features vello-hybrid --test vb3_sparse_text --test vb2_sparse_images --test vb2_sparse_patterns -- --nocapture --test-threads=1
cargo test --offline --locked -p netrender -j2 --features vello-all --lib -- --nocapture --test-threads=1
cargo test --offline --locked -p netrender -j2 --features vello-all --test rg2a_scene_corpus --test p10prime_a_glyph_api --test p10prime_b_glyph_render --test pc4_variable_fonts -- --nocapture --test-threads=1
cargo tree --offline --locked -p netrender --features vello-all -i glifo
cargo tree --offline --locked -p netrender --features vello-all -i skrifa@0.44.0
cargo tree --offline --locked -p netrender --features vello-all -i wgpu
cargo tree --offline --locked -p netrender --features vello-all -i peniko
```

| Final gate | Result |
| --- | --- |
| Default / CPU / Hybrid / combined compile | All pass |
| CPU-only text | 5 passed |
| Hybrid-only text | 5 passed |
| CPU-only image / pattern regression | 3 / 2 passed |
| Hybrid-only image / pattern regression | 2 / 2 passed |
| Combined library | 75 passed, 3 intentionally ignored |
| RG2a geometry corpus and free-helper refusal table | 2 passed |
| Classic glyph API | 5 passed |
| Classic real-font glyph readback | 1 passed |
| Classic variable-font API / rendering | 7 passed |

The earlier four-test text runs are retained in `text-cpu.txt` and
`text-hybrid.txt`. The final suites added a repeated-outline frame-budget
regression and are recorded in `final-vello-cpu.txt` and
`final-vello-hybrid.txt`. The three ignored library tests are existing physical
RG2b/RG2c and repeated-plan measurement gates, not newly skipped text tests.
The known intentionally failing `linear-light-canary` feature was not enabled.

Hybrid and corpus readbacks used NVIDIA GeForce RTX 4060 Laptop GPU, Vulkan,
NVIDIA driver 610.88. CPU session tests do not request a GPU device. Classic
tests loaded real Arial (980 painted pixels) and Bahnschrift: weights 300,
400, 700 produced ink loads 1105464, 1453095, 1860417. Those system-font tests
did not take their vacuous no-font skip paths.

## What the tests and source review establish

Portable OFL fixtures and their provenance are under
`netrender/tests/fixtures/vb3_fonts/`. Pixel assertions cover caller-positioned
glyphs, premultiplied color, layer alpha, device clipping, painter order, real
variable-font axes, ignored unknown valid axis tags, caller-selected two-font
fallback, and a positive nonzero TTC face index. The TTC container is built
in memory from the two unchanged fixtures and independently parsed first.

Negative cases cover missing palette entries, malformed bytes, invalid face
index, missing head table, invalid glyph ID, non-finite glyph positions/font
size/axis values, font/variant limits, and repeated-glyph outline work. A single
box glyph fits a 64-segment frame budget; 128 copies of that same glyph must
refuse even though distinct-glyph cache limits still fit. Failed text plans
preserve text/image statistics; malformed text cannot apply a valid pending
image-source update. Hybrid submits and reads back the unchanged target after
refusal. Cache tests exercise reuse, fresh font identities, bounded epochs,
explicit clear, and image retention across text-cache clearing.

Independent source review checked the pinned Glifo unchecked font/head/draw
paths against preflight: requested face and UPEM, glyph IDs, exact unhinted
UPEM outline drawing and normalized coordinates, finite derived transforms
and bounds. Native CPU/Hybrid glyph rendering is used through a small adapter
with an owned public Glifo preparation cache. Hinting and the experimental
glyph atlas are disabled. Full cache/map replacement releases retained
capacity at explicit clear and epoch resets without reconstructing Hybrid
pipelines or discarding image textures.

Font bytes and outline segments are conservative logical accounting, not exact
allocator or process-memory measurements. Reused upstream path capacities and
parser scratch storage are not represented by the segment count. Fonts remain
trusted parser inputs: this is not a font sanitizer or CPU-time sandbox. Color,
bitmap, and SVG fonts are deliberately outside this outline subset. Shaping,
font discovery, and fallback selection remain caller responsibilities.

The existing Hybrid bilinear-pattern seam refusal remains unchanged and tested.
This receipt does not imply full sparse/Classic feature or consumer parity.

## Source provenance and hygiene

`tested-source-hashes.txt` binds the final runtime test source. Subsequently,
only standard Exhibit A license comments were added to the three session
implementation modules, plus one rustfmt-only assertion layout change in the
text test. Header removal reproduces the tested implementation hashes.
`final-source-hashes.txt` binds that final source and the portable font assets.
Those comment/format-only changes do not require repeating runtime gates.
Formatting, tracked-diff whitespace, and all newly added text
files were checked separately; final details are in `hygiene.txt`.
After staging, `python ../mere/scripts/relicense_headers.py --repo . --audit`
reported 127 owned sources (two added), zero missing Exhibit A headers and
zero Exhibit B hits. The OFL fixture subtree is retained by the license ledger.
Staged whitespace and changed-document local-link checks also passed.
