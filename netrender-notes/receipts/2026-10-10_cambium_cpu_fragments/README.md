# Cambium CPU source and fragment bridge receipt

Date: 2026-10-10 (America/New_York). This slice implements a bounded source
registry and proves CPU rendering of production Tabard preview components.
It does not qualify ordinary application routing, automatic fallback, Hybrid
GPU pixels, host upload/presentation, or recovery after a driver reset.

## Source and isolation

Netrender baseline: `99d8d71ec527a2ee5f32aa1e4d5ef197c4e71131`.
Mere production preview source: `e95326dc08446a9256d3e340c63e571684afe697`,
in a clean detached checkout at
`/Users/markik/Code/worktrees/mere-vello-cpu-source`.
Changes live in Netrender's isolated `codex/vello-cambium-resource-bridge`
checkout, `/Users/markik/Code/worktrees/netrender-cambium-resource-bridge`.
Neither primary checkout nor the Gemot/Murm/Moot lane was changed.

Both repositories were fetched/pruned before assessment. At that point the
clean Netrender primary was seven commits behind its remote; Mere's primary
had five unpushed commits and was 378 commits behind its remote. No reset,
merge, stash or fast-forward was performed on those primary checkouts.

Mere's section 11.1 correctly describes its pinned `9607d16f` lowerers. Newer
Netrender already has owned image/pattern/outline-text sessions and the
September Isocosm replay. This slice builds on those sessions. It does not
advance Mere/Genet dependency pins or migrate the Vello provider to 0.11.

CPU/Hybrid/common/Glifo stay at `ca3f40ea182216883cd543c7b9deae991268917c`.
The production-source probe uses Mere's coherent Classic patches at
`10f01d6d88e94eac087daf033b24895cf97b8e82`; it does not execute Classic.
The provider tests use Netrender's existing lock and registry Classic source.
The probe's separate manifest/lock are included, with path patches selecting
this Netrender checkout. No workspace manifest or lock was edited.

Upstream primary documentation was read live on this date:
[renderer guidance](https://github.com/linebender/vello#choose-a-renderer),
[architecture](https://github.com/linebender/vello/blob/main/ARCHITECTURE.md),
[GPU README](https://github.com/linebender/vello/blob/main/vello_gpu/README.md),
and [CPU README](https://github.com/linebender/vello/blob/main/vello_cpu/README.md).
Upstream distinguishes the more mature CPU renderer, GPU rendering with CPU
path preprocessing and no compute requirement, and the experimental original
compute renderer. It documents the Hybrid-to-GPU rename and resource/API
limitations. Those current APIs were neither assumed compatible with this
pin nor adopted by this proof. The separate upstream-adoption plan remains
the authority for provider migration.

## Implemented boundary

`SparseFragmentRegistry` is an application/session-owned source ledger.
It stores non-nested `SceneFragment` content, allocates process-local IDs
that are not recycled, exposes generations, supports atomic replacement and
removal, and bounds count, source complexity, font bytes and image bytes.
Shared blobs are conservatively counted once per reference. These are logical
residency bounds, not allocator/RSS limits or a hostile-code sandbox.

`resolve(scene, backend, limits)` expands placements in painter order. It
remaps local font and transform IDs, composes local geometry transforms with
placement, and moves primitive device-space clips with the fragment through
explicit clip layers. Parent clip/opacity layers and root state survive.
The resolved Scene goes to the existing owned `CpuSession` or `HybridSession`.
Both receive the same positioned glyphs, font blobs and CPU image sources.
Resource hydration/cache lifetime remains with those existing sessions.

This is source expansion followed by per-frame re-lowering. It makes no
packet-cache, tile-aligned append, retention-speed or application-performance
claim. Classic renderer-local fragment IDs need explicit mapping when a host
migrates; the new registry is not a cross-process or persistent resource
protocol. Registry removal releases its source content; already hydrated
session image/font resources have separate explicit removal/clear APIs.

Missing/foreign/removed IDs, nested fragments, invalid palette references,
unbalanced layers, conflicting shared image definitions and budgets refuse.
Unsupported filters, native compositor surfaces, malformed font/image data,
and the pinned Hybrid bilinear-pattern subset keep their existing refusal
boundaries. Resolution leaves the ledger unchanged; owned sessions still
validate full resource admission before cache or target mutation. Free
lowerers and sessions called directly with Fragment ops continue to refuse.
Capability booleans were not broadened by this explicit source bridge.

## Production CPU proof

`probe.rs` invokes the actual `WorkshopState::in_memory`, `reader_preview`,
`stylesheet_preview`, and each preview's `frame` method. Reader content uses
the production extraction/document-canvas path. The stylesheet preview uses
the real HTML fixture, Livery cascade/layout and caller-shaped text. No font
substitution, glyph reshaping by the adapter, command stripping or native
application launch is involved.

It renders light, dark, narrow and authored states, eight component frames:

| Component | Wide viewport | Narrow viewport | Fonts | Positioned glyphs |
| --- | --- | --- | --- | --- |
| Reader | 640 x 400 | 320 x 500 | 2 | 318 |
| Stylesheet preview | 640 x 400 | 320 x 500 | 3 | 146 |

For every component frame, direct CPU pixels equal pixels after source
registration, placement, expansion and rendering through the same persistent
CPU session. Generations advance from 0 through 3. The repeated retained
render adds no hydrated image or cached glyph. Operation/parameter admission
also succeeds when resolving for Hybrid; that is a source check, not a
Hybrid resource or GPU execution receipt.

Four additional composition frames place the retained reader above an image
whose bytes came from CPU rendering of the actual stylesheet preview. The
image is explicitly an opaque CPU snapshot, so its premultiplied and straight
RGBA representations coincide. There is no GPU readback. These frames exercise
real shaped text, real rendered-image payloads, placement and source retention
in one registry/session, with one cached image and exact direct/retained CPU
pixel identity. They are a proof composition of production components, not
the workshop's native top-level layout or an external-texture import receipt.

Rendering preserves the draft theme, selected mode, status, dirty state and
pending fields, checked before/after each state. Authored CSS changes only
the stylesheet preview: the reader-authored PNG equals reader-light, while
stylesheet-authored differs from light/dark. Local grant/session routing code
is not involved. Input, selection movement and accessibility adapter behavior
have not been exercised by this raster proof; semantic names are recorded.

All twelve PNGs were inspected. Text, body layout, narrow reflow, colors and
composed image content are visible. The authored CSS deliberately changes the
body without updating every fixture surface, so its light text on light cards
is an author-input consequence, not a contrast qualification. The reader link
prefix is a square in this CPU output; no Classic control was rendered here to
attribute that glyph's appearance. These observations remain part of the
receipt rather than being hidden by pixel-identity assertions.

`production-preview-receipt.json` records frame counts, identities and
diagnostic times. macOS 15.8.1 / x86_64-apple-darwin, rustc/Cargo 1.97.1,
dev profile with debug info disabled for the separate probe. The shared host
and debug build prevent performance conclusions. `artifact-hashes.json`
records binary, PNG and local Scene bundle hashes. `.scene` files retain raw
system-font resources locally under
`/Users/markik/Code/output/vello-cambium-cpu-20261010/production-previews-v2`;
they are not committed.

### Post-proof process startup limitation

After the completed twelve-frame v2 proof and focused test matrix, an extra
locked-build refresh and rerender stalled before producing output. The probe
had not created its new output directory; a macOS sample showed an 8 KiB
process footprint at `_dyld_start`, with no application call stack. A Git read
also waited. The redundant processes were interrupted and the accepted v2
artifacts were kept intact. This observation does not establish a renderer,
GPU, driver or font-parser cause. The sample remains local at
`/Users/markik/Code/output/vello-cambium-cpu-20261010/probe-startup-sample.txt`.

Before that startup stall, an offline rebuild completed. Its scratch lock
refresh only reordered `patch.unused` entries; no resolved package or version
changed. The copied `probe.Cargo.lock` and binary hash describe the completed
v2 run, not a claim that the interrupted refresh passed. Subsequent source
changes were documentation/comments and a test import cleanup; provider
behavior did not change after the v2 proof. A fresh runtime refresh remains
appropriate before promoting this branch into consumer pins.

## Whole-workshop negative controls

The original authored/narrow native-11 workshop captures were read without
changing the owning lane's artifacts. They contain 434/430 commands,
127/125 text runs, 2,042/1,987 glyphs and three embedded fonts, plus two
`DrawExternalTexture` commands each. Serialized packets contain no corresponding
external preview pixel payloads. The existing intact replay example refuses
at command 253/249, before GPU initialization or rasterization.

`workshop_authored-refusal.json` and `workshop_narrow-refusal.json` preserve
input/reference hashes, counts and the explicit absent-producer error. Neither
packet was stripped or modified. The source-component proof above does not
repair or close this full-application resource-capture gate. A complete bundle
must carry declared producer snapshots/scenes with resource identity,
generation, size, alpha and placement; live GPU imports remain a separate gate.

## Validation and reproduction

`checks.json` contains exact commands and exits. The default build and all
focused feature/test gates pass. Strict Clippy with `-D warnings` stops on six
pre-existing lints in unchanged `netrender_device` code. Scoped Clippy with
`--no-deps` completes with four pre-existing warnings in `hit_test`,
`interpolate`, and `vello_rasterizer/emit`; the new fragment code has none.
These baseline lints were not repaired in this renderer-resource slice. Focused tests cover retained
text/images, source replacement, missing-resource atomicity, cache reuse,
ID isolation/removal, generation changes, budgets, rotated rounded clips,
parent clips, opacity and painter order. Existing CPU image/pattern/text and
ingress/corpus regressions run without GPU initialization.

Run the checked-in provider suite from the isolated Netrender checkout with:

```sh
export CARGO_TARGET_DIR=/Users/markik/Code/targets/netrender-cambium-cpu-20261010
cargo test --locked -p netrender --features vello-cpu --test vb5_sparse_fragments --test vb2_sparse_images --test vb2_sparse_patterns --test vb3_sparse_text -j 2
cargo test --locked -p netrender --features vello-hybrid --test vb5_sparse_fragments -j 2
cargo test --locked -p netrender --features vello-all --test vb5_sparse_fragments -j 2
```

The `vb5_sparse_fragments` test file is CPU-only in every feature configuration.
Do not substitute full library/integration suites: some of those boot a GPU.

The separate probe package is
`/Users/markik/Code/output/vello-cambium-cpu-20261010/probe`. Its exact source,
manifest and lock are copied here. Paths in the manifest are the isolated
checkouts above; change those paths when reproducing elsewhere. Build and run:

```sh
CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked --manifest-path /Users/markik/Code/output/vello-cambium-cpu-20261010/probe/Cargo.toml -j 2
/Users/markik/Code/targets/netrender-cambium-cpu-20261010/debug/cambium-cpu-source-proof NEW_OUTPUT_DIRECTORY
```

PNG creation refuses replacement; choose a fresh output directory. The probe
contains no device, queue, adapter, window or browser initialization calls.
GPU-producing tests, native GPU applications and browser qualification were
not launched. They remain gated on the explicit handoff from the GPU-slot owner.
CPU raster, host upload/presentation and driver-reset recovery stay separate
acceptance boundaries; no backend-selection or fallback policy was added.
