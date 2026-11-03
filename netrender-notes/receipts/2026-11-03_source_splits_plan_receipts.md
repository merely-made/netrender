# Source splits: H5 Netrender receipts

## 2026-11-03: twelve rendering and fixture paths

Owner: house-style H5 / 01a19f86-cb71-7820-a6ac-aaec39d8bfd4.
Baseline source: `58d80d8aee042d014d7c935803f8fdd4f81efaa9`. Compact text only, under H5's shared
100 MB retained cap. Frozen historical receipts and corpus captures intact.

| Original source | Before | After |
|---|---:|---:|
| `netrender/src/external_texture.rs` | 646 | 559 |
| `netrender/src/renderer/rg2c.rs` | 640 | 590 |
| `netrender/src/scene/build.rs` | 1052 | 554 |
| `netrender/src/tile_cache/index.rs` | 868 | 304 |
| `netrender/src/vello_backends.rs` | 1368 | 455 |
| `netrender/src/vello_backends/sessions.rs` | 907 | 594 |
| `netrender/tests/rg2a_scene_corpus.rs` | 654 | 428 |
| `netrender/tests/vb3_sparse_text.rs` | 620 | 521 |
| `paint_list_api/src/lib.rs` | 613 | 456 |
| `paint_list_render/src/lib.rs` | 751 | 283 |
| `paint_list_render/src/tests.rs` | 925 | 362 |
| `paint_list_render/tests/resource_bundle.rs` | 737 | 454 |

All new modules also fit 600 physical lines. Twelve exceptions retired,
seven remain. Root API paths, shader bytes, rendering arithmetic, cache
identity, budgets, painter order, fixture data and pixel thresholds are
preserved. There are no manifest or lockfile changes.

- Baseline `cargo test --workspace --locked`: existing failure in
  `compose_into_01_identity_matches_render`, max_diff 14/diff_count 48,
  against its unchanged ±4/16-channel threshold. This batch does not repair
  that unrelated pixel mismatch or claim a passing full workspace suite.
- Scoped post commands, all passing, total 156 test cases; three existing
  ignored cases remain ignored:
  `cargo test --locked -p netrender --lib --features vello-all`;
  `cargo test --locked -p paint_list_api --lib`;
  `cargo test --locked -p paint_list_render --lib --test resource_bundle --features resource-bundle,replay-cpu`;
  `cargo test --locked -p netrender --test rg2a_scene_corpus --test vb3_sparse_text --features vello-all`.
- `cargo check --workspace --all-targets --locked --features netrender/vello-all,paint_list_render/resource-bundle`: pass.
  This exposed a pre-existing non-exhaustive native RG2c fixture match;
  its explicit WebGl2 arm now explains that the native fixture only
  schedules Classic, Hybrid and CPU. No native browser route is added.
- `cargo doc --workspace --locked --no-deps --features netrender/vello-all,paint_list_render/resource-bundle`:
  pass, with unrelated existing rustdoc warnings outside the extracted code.
- `prek run --all-files`: ceiling and Clippy pass.
- Structural review compared 167 old/new function bodies: source-scope
  adjustments and rustfmt blocks/separators, plus the explicit native-only
  fixture arm. Existing source notices retained on extracted files.
- This is native Windows CPU/offscreen GPU evidence. No new headed,
  browser, macOS/Linux or performance acceptance is claimed.
