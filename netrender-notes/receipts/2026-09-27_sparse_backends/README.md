# Sparse backend validation, 2026-09-27

## Baseline identity

- Repository HEAD: `9607d16f1907f6c2085648ae96abcaa30d7c3d41`.
- Sparse fork: `ca3f40ea182216883cd543c7b9deae991268917c` for
  `vello_cpu`, `vello_hybrid`, and `vello_common`.
- Classic: registry `netrender-vello` 0.10.1.
- Shared boundary identities: wgpu 30.0.0 and Peniko 0.6.1, confirmed by
  the inverse dependency trees beside this receipt.
- Rust: `rustc 1.97.1 (8bab26f4f 2026-07-14)`;
  Cargo: `1.97.1 (c980f4866 2026-06-30)`;
  host: `x86_64-pc-windows-msvc`.
- Working directory: `C:\Users\mark_\Code\repos\netrender`.
- Every build explicitly sets
  `$env:CARGO_TARGET_DIR='C:\t\cargo-targets\netrender'`.
  This is the existing stable repository target, retained for reuse.
- No isolated Cargo home or worktree. No compiler/test/application owner of
  this target was observed before starting. The inherited target environment
  named a different repository path and was overridden before every build.
- Source and manifest hashes are in `baseline-source-hashes.txt`. Only
  documentation was dirty at baseline start.

## Commands

```powershell
cargo check --locked -p netrender -j2
cargo check --locked -p netrender -j2 --features vello-cpu
cargo check --locked -p netrender -j2 --features vello-hybrid
cargo check --locked -p netrender -j2 --features vello-all
cargo test --locked -p netrender -j2 --features vello-all --lib vello_backends::tests -- --nocapture --test-threads=1
cargo test --locked -p netrender -j2 --features vello-all --test rg2a_scene_corpus -- --nocapture --test-threads=1
cargo tree --locked -p netrender --features vello-all -d
cargo tree --locked -p netrender --features vello-all -i wgpu
cargo tree --locked -p netrender --features vello-all -i peniko
```

## Baseline results

All commands completed with exit code zero on the unmodified baseline source.

| Gate | Result | Evidence |
| --- | --- | --- |
| Default compile | Pass | `baseline-check-default.txt` |
| CPU compile | Pass | `baseline-check-vello-cpu.txt` |
| Hybrid compile | Pass | `baseline-check-vello-hybrid.txt` |
| Combined compile | Pass | `baseline-check-vello-all.txt` |
| Focused backend tests | 6 passed, 0 failed | `baseline-backend-tests.txt` |
| RG2a corpus | 2 passed, 0 failed | `baseline-rg2a-tests.txt` |

The backend tests include exact CPU pixel checks, Hybrid readback using
NetRender's device, consuming tile-aligned append, capability checks,
gradient/layer admission, and typed image refusal. RG2a renders two semantic
fixtures through Classic, Hybrid, and CPU and verifies the refusal table.
Classic and Hybrid receive the same device handles in that corpus.

Observed GPU: **NVIDIA GeForce RTX 4060 Laptop GPU**, **Vulkan**, NVIDIA driver
**610.88**. This is automated GPU readback evidence, not a native window or
interactive application receipt.

The duplicate tree contains distinct Naga 29/30 shader-build dependencies
and Skrifa 0.42.1/0.44.0 for Parley versus the renderer. It does not duplicate
the shared wgpu or Peniko boundary identities. No pin change was necessary
for this baseline.

The RG2a test currently prints `Classic=netrender-vello@0.10.0`; that literal
diagnostic is stale. The locked dependency tree establishes 0.10.1 for this
run. This receipt does not interpret the literal as the resolved version.

## Scope

The initial resource adapter slice retains the existing source pin and its
bounded consuming, tile-aligned append prototype. An upstream candidate
upgrade is not part of this run. These tests cannot certify arbitrary affine
fragment reuse, native UI interaction, browser parity, or Isocosm performance.

## Owned image session implementation

Implementation source hashes are in `implementation-source-hashes.txt`.
The manifest, lock and sparse pin are unchanged from the baseline. These
receipts describe a working-tree change based on the baseline HEAD, not a
published package or integrated consumer release.

Commands use the same working directory, target and toolchain as above:

```powershell
cargo check --locked -p netrender -j2
cargo check --locked -p netrender -j2 --features vello-cpu
cargo check --locked -p netrender -j2 --features vello-hybrid
cargo check --locked -p netrender -j2 --features vello-all
cargo test --locked -p netrender -j2 --features vello-cpu --test vb2_sparse_images -- --nocapture --test-threads=1
cargo test --locked -p netrender -j2 --features vello-hybrid --test vb2_sparse_images -- --nocapture --test-threads=1
cargo test --locked -p netrender -j2 --features vello-all --lib -- --nocapture --test-threads=1
cargo test --locked -p netrender -j2 --features vello-all --test rg2a_scene_corpus --test p5prime_vello_image -- --nocapture --test-threads=1
cargo test --locked -p netrender -j2 --features vello-all --test rg2a_scene_corpus -- --nocapture --test-threads=1
rustfmt --check --edition 2021 --config skip_children=true netrender/src/vello_backends.rs netrender/src/vello_backends/sessions.rs netrender/src/vello_backends/sessions/validate.rs netrender/tests/vb2_sparse_images.rs
```

| Gate | Result |
| --- | --- |
| Default, CPU, Hybrid, combined compile | All pass |
| CPU-only image tests | 3 passed, 0 failed; render path does not request a device |
| Hybrid-only image tests | 2 passed, 0 failed; existing NVIDIA Vulkan device |
| Combined library tests | 75 passed, 0 failed, 3 intentionally ignored |
| Classic image regression | 3 passed, 0 failed |
| RG2a | 2 passed, 0 failed after diagnostic expectation repair |
| Formatting, four new/modified implementation/test files | Pass |

No combined-feature rerun of the new image target is claimed: the independent
CPU-only and Hybrid-only targets exercise both halves, with the combined
library and RG2a targets checking coexistence. The ignored library tests are
the physical RG2b/RG2c execution and RG1 repeated-plan measurement receipts;
this run does not refresh those measurements.

The new image tests cover sampling, crop/clamp, premultiplied tint, transform
and device-space clipping, persistent source replacement/removal, bounded
residency and refused-input target preservation. They use a synthetic image
corpus, not an Isocosm document capture. Generic geometry/gradient derived
range checks were reviewed but have no dedicated extreme-value test in this
receipt.

Review confirmed that derived image variants own CPU pixmaps and, for Hybrid,
individual GPU textures; they avoid the upstream monotonic CPU image registry
and atlas allocation path. Dropping/removing them releases session references;
GPU commands already encoded or in flight may retain resources. Reported GPU
bytes are texture payload bytes, not allocator/driver or total renderer memory.

Two first-failure logs are preserved: `implementation-cpu-constructor-failure.txt`
records the corrected pinned ImageBrush constructor mismatch, and
`implementation-stale-refusal-failure.txt` records a stale exact refusal message
assertion after the legacy helper began directing image callers to owned
sessions. The Image and Pattern expectations now match the deliberately
separate refusal guidance, and the complete RG2a rerun passed. Neither first
failure indicated a pixel mismatch. Frozen implementation/test hashes were
rechecked unchanged after the final corpus-only repair; only the corpus hash
was refreshed. The full library and CPU/Hybrid image tests already exercised
the final implementation, including the Pattern message.
