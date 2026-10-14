# Retained tile empty-layer qualification

Turnstone's intact Nodes frame had transparent 64-pixel tile holes and a missing
part of the Orrery header in the actual shared Frame texture. Native raw and
final captures were byte-identical, locating the damage before final composition.
The bounded repair prunes balanced empty normal source-over groups after spatial
draw filtering. Nonempty CSS clip/alpha scopes remain in painter order. Empty
masks, non-normal blends, filter/backdrop groups and malformed streams remain
unchanged.

## Qualified source and inputs

Production source was qualified on `f4147feca69f177cf8c276c130549aa6dd3e201f`.
The subsequent `484347ff846dece47fc6c6840a25e08e7e7856ec` integration contains
only unrelated RG1 documentation/receipts. Vello remains exactly
`2eae6243881d5b24c984d5ee35dde509ca5db782`; Cargo.lock and existing dependency
pins remain unchanged.

`captured/` preserves the actual same-frame Nodes and Links Scene JSON, complete
font palette, Graph producer PNG, source metadata, native raw PNG and native final
PNG. Original files and SHA-256 hashes are in `captured-inputs.json`. These came
from Turnstone's canonical scoped A4 scenario on Genet199. The actual Graph
resource is 1175×1538 Rgba8Unorm, Straight, at Image key
`15011715060995194881`, generations 17 (Nodes) and 19 (Links). The frame is
2360×1600 physical pixels, native window scale 2, raster scale 1.

Each replay process uploads that captured producer, stages it and rasterizes
through one Device/Queue. Separate control/fixed processes use the same physical
adapter and provider, **not the same physical Device object**. The adapter is
NVIDIA GeForce RTX 4060 Laptop GPU, Vulkan, vendor 4318/device 10464, driver 610.88.
No producer image is reconstructed, cropped from a damaged frame or replaced.

## Evidence

The four final focused unit tests pass under locked/offline Rust 1.99.0. The real
replay executable builds in the shipping dev profile (panic=abort). Both cold
and warm GPU frames complete within bounded readbacks, with all provider quality
counters observed. Actual `master.encoding().n_clips` is logged:

| Scene | Unpruned clips | Fixed clips | Cached tiles |
| --- | ---: | ---: | ---: |
| Nodes | 72,150 | 4,278 | 925 |
| Links | 12,950 | 3,772 | 925 |

`pixel-comparison.json` records the exact comparison. The unpruned control
reproduces 3,968 transparent header pixels and 54,720 transparent roster pixels
inside authored opaque rectangles. Both fixed frames restore alpha 255 throughout
those rectangles. Links is byte-identical in RGBA between variants. In the
3,700,736-pixel opaque overlap, 802 Nodes pixels differ by at most RGB 1, alpha 0,
within the unchanged prior interior RGB 3/alpha 2 gate. Nodes cold/warm renders
also have a one-unit blue residual at one text pixel; its position varies.
No cold/warm byte-identity claim is made for Nodes.

The baseline and fixed Nodes captures and fixed Links capture were visually
reviewed: the title and checker holes are repaired, with retained table content.
Eight final replay PNGs preserve both variants, both scenes and cold/warm frames.
`commands.json` includes exact commands, environment, source/executable hashes,
build logs and control details. The final fixed source byte hash is restored to
the same candidate that passed the four unit tests. A Windows default-decoding
comment drift was caught by that guard; the control was repeated with explicit
UTF8 byte handling, and prior metadata/logs remain available.

## Reproduce the fixed replay

Run from the repository root with `CARGO_TARGET_DIR=C:/t/cargo-targets/netrender`,
`CARGO_BUILD_BUILD_DIR=C:/t/cargo-targets/netrender/build`, and
`WGPU_BACKEND=vulkan`:

```text
cargo +1.99.0 test -p netrender --lib empty_tile_layers_tests --features serde --locked --offline -j1
cargo +1.99.0 build -p netrender --example retained_scene_replay --features serde --locked --offline -j1
retained_scene_replay captured/00_inline_nodes.scene.json captured/00_inline_nodes.source-1176657005713031169.png 15011715060995194881 17 straight OUTDIR
```

For the last command, prefix both `captured/` paths with this receipt directory
and invoke the built executable. Links uses `01_inline_roster` and generation 19.
The unpruned qualification disables only the pruning call, preserves the debug
metric, records its source/executable hashes, then restores and hash-checks the
fixed source before rebuilding. The published source contains the repair.

## Remaining scope

This qualifies the retained scene raster repair. Updated native Turnstone raw
and final captures after normal supplier adoption remain a separate consumer
gate. The original native raw/final equality is preserved as localization
evidence. The [provider prefix-scan finding](supplier-prefix-scan.md) is static
source evidence with measured clip counts recorded separately; it does not
qualify general Classic large-clip rendering or a numeric failure threshold.
Physical device loss, OS assistive actions and performance pooling are separate
gates. This change introduces no provider cap, fallback renderer or recovery
factory. The existing approved Netrender target is reused; no isolated Cargo
home or worktree was created.
