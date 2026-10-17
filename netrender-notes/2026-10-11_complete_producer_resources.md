# Complete producer resources, 2026-10-11

`paint_list_render`'s opt-in `resource-bundle` feature captures an intact
`PaintEnvelope` and actual bound producer `Scene` snapshots. Producer keys,
content generations, source identity/revision, geometry, layout scale, alpha
convention and encoding remain explicit. The translated scene retains external
draw placement, UVs, opacity and operation boundaries, plus complete shadow
requests. Unavailable resources and unsupported semantics return typed errors.

The portable capture does not initialize a GPU. A complete bundle adds pixel
payloads with bindings to those sources or shadow requests. `admit_bundle`
validates the expected capture identity, canonical translation, complete ledger,
namespace collisions, payload extents, alpha/encoding and aggregate budgets
before constructing an image-bearing scene. Limits apply to nested structure,
font/image bytes and all sources together. Identity admission binds declarations;
it does not independently prove which renderer produced supplied pixels.

Cambium's production capture stages actual producer callback output, preflights
the entire borrowed plan before hashing or proposing stamps, and admits the
whole envelope before committing registry cache/lifecycle state. Its portable
lookup never substitutes cached live GPU commands. Producer-internal preview
layout caches may update during callbacks; application authoring authority
remains with the existing application state.

The `resource_bundle_parity` example materializes admitted producer scenes using
CPU, then independently compares each source scene and the composed workshop
against the selected renderer. It compares premultiplied encoded-sRGB RGBA8,
including alpha, without matting. Classic's straight output is converted
explicitly. CPU mode never boots a GPU. GPU modes use a five-second shared
submission/readback deadline; the coordinator also supplies a finite process
deadline. PNGs and JSON survive pixel refusal. Font-bearing postcard captures
and complete bundles are local artifacts, excluded from published receipts.

```sh
cargo build -p paint_list_render --example resource_bundle_parity \
  --features resource-bundle,replay-cpu,replay-hybrid -j 2
resource_bundle_parity CAPTURE.postcard FRESH_OUTPUT gpu
resource_bundle_parity --fixture FRESH_OUTPUT all
```

The shared border translation now represents normalized elliptical outer and
inner rings, including authored asymmetric widths. Uniform colors fill one
ring; different sides accumulate premultiplied coverage using explicit `Plus`
composition inside the outer ring. This avoids the alpha deficit produced by
independent SrcOver side fills at a diagonal seam. Existing serialized compose
indices remain SrcOver=0 and DestIn=1; Plus is appended at 2. Decorated styles
and `do_aa=false` remain typed refusals in complete-capture admission until their
semantics have a faithful lowerer.

See the [gate receipt](receipts/2026-10-11_producer_gates/README.md) for actual
execution results and remaining acceptance boundaries. Classic curved-edge and
glyph-edge pixel parity remains open with the original thresholds. Sparse GPU
evidence does not establish shipping host renderer selection, browser
qualification, OS assistive-technology interaction or forced driver-loss
recovery. Shadow requests require authoritative complete payloads; the CPU
materializer refuses missing masks rather than approximating them.

The image side-table now carries decoded RGBA8 pixels as `Arc<Vec<u8>>`.
Cloning an image for a retained frame or a composited layer shares the pixel
allocation, and translation wraps that same allocation in a renderer blob.
The JSON image payload remains an inline byte array. Supplier tests cover
clone identity, serialization, composite key remapping, retained fragments,
and a CPU raster of opaque and translucent pixels. Post-cut gates passed 46
API/renderer/resource-bundle tests, including the CPU pixel draw. The existing
corpus gate passed three tests and failed `translation_matches_recorded_ops`
on `livery_article_card` border Stroke-versus-Shape output. A freshly compiled
unchanged `f4f9c0d87` control reproduces that exact failure; expected fixtures
were not rewritten. This is allocation sharing, not stable renderer Blob
identity across translations or new GPU qualification.

Gates ran offline/locked with one Cargo job, serial tests, 5 GiB memory cap,
zero swap allowance, 150% CPU and nice 10 under the shared machine lock. Raw
logs, a fresh-control dependency receipt, and hashes are retained at
`/home/markik/Code/testing/audits/genet-architecture-20261016/followups/`.
Genet's architecture audit records the paired consumer adoption and final
source hashes. Both supplier and consumer are local-only until published.

The corpus failure above is superseded by the [2026-10-17 border verification](receipts/2026-10-17_corpus_borders/README.md): captured border pixels and retained GPU output justify correcting two stale goldens; all 48 selected tests pass. Production translation and captured commands are unchanged.
