# Classic edge isolation protocol

This is diagnostic source for the Classic gate left open by the producer-bundle
qualification. It changes no production lowering, renderer provider, canonical
border geometry, or pixel threshold. Provider inspection used commit
`5862bba26c8a9f56f6a5fdc8ea6966b360a5d63a`; the Netrender starting point was
`ef5412d760a978584c8c1801e8a765c51aecd403`.

Builds and all runtime selections belong to the root task. Keep the existing
dedicated target/environment and two-job limit. Example build and CPU controls:

```sh
cargo build --locked -p paint_list_render --features resource-bundle,replay-cpu,replay-hybrid --example classic_edge_probe -j 2
cargo test --locked -p paint_list_render --features resource-bundle,replay-cpu --test classic_edge_probe -j 2
```

The test file has three source controls and one CPU control. It verifies that the
isolated cubic contour is exactly the border's outer contour; separates fill,
single clip, and coincident fill/clip scopes; and checks transparent exterior,
border hole, and translucent accent alpha. The additional polyline control
checks identical backend input, exact shared f32 fill/clip endpoints, identity
transforms, the 132-segment bound and the geometric approximation bound. It never
initializes an adapter.

Invoke the resulting `classic_edge_probe` executable directly. For the existing
actual wide-cold bundle, source-only inventory is:

```sh
classic_edge_probe /Users/markik/Code/output/vello-classic-browser-gates-20261011/inventory cpu --list --bundle /Users/markik/Code/output/vello-producer-gates-20261011/production-wide-cold-parity/complete-bundle.postcard --producer 1952539235
```

The inventory validates complete identity and ledger admission, then records
glyph operation indices, first glyph ID/position, font ID, and size. It performs
no rasterization. Choose the body-line run near the earlier failure at
`(250,192)` using those positions and the transform recorded by a selected run.
Pass its index as `--glyph-op INDEX`. A new output directory is required for
each run.

```sh
classic_edge_probe /Users/markik/Code/output/vello-classic-browser-gates-20261011/geometry-cpu cpu
classic_edge_probe /Users/markik/Code/output/vello-classic-browser-gates-20261011/geometry-all all
classic_edge_probe /Users/markik/Code/output/vello-classic-browser-gates-20261011/glyph-all all --bundle /Users/markik/Code/output/vello-producer-gates-20261011/production-wide-cold-parity/complete-bundle.postcard --producer 1952539235 --glyph-op INDEX
```

`classic` selects CPU plus Classic; `gpu` selects CPU plus sparse GPU; `all`
selects all three. `--case NAME` narrows the run to one listed case. CPU selection
does not boot GPU. Each selected GPU case uses a dedicated device and an exact
copy submission/readback with a shared five-second deadline, copied from the
previously qualified finite parity protocol. GPU runs require the root-owned
serialized handoff. No headed application or browser is launched.

The executable exits zero when diagnostics finish, including when the recorded
pixel gate is red. Refusal, preparation, and runtime errors exit one. Inspect
`all_selected_cross_renderer_pixel_acceptance` and each `comparison.accepted`;
do not interpret process success as pixel acceptance. The source-only inventory
does not contain that runtime acceptance field.

The eleven geometry cases are:

- `curve-fill`: exact canonical four-cubic ellipse contour, direct solid fill.
- `curve-clip-only`: one copy of that contour clips covering rectangular paint.
- `curve-fill-and-coincident-clip`: identical contour as both paint and clip.
- `curve-double-coincident-clip`: two identical clips over covering paint.
- `curve-fill-fractional-translation`: same path translated by `(0.25,0.375)`.
- `curve-fill-uniform-scale`: same path scaled by `1.25`.
- `border-ellipse-uniform`: nonuniform border widths, uniform opaque color.
- `border-ellipse-accent-plus`: actual translucent accent/Plus joined geometry.
- `border-square-accent-plus`: same side colors/widths with square corners.
- `polyline-fill`: diagnostic fine polyline approximation of the original
  canonical contour, prepared once with the same f32 endpoints for all backends.
- `polyline-clip-only`: exact same polyline endpoints as one clip over covering
  paint, with identity transforms for both cases.

The polyline cases isolate curve flattening from area coverage. They are
diagnostic approximations, never production geometry. Four quarter cubics each
use 32 dyadic parameter subdivisions, plus the four existing straight edges:
132 line segments in total. The conservative world-space second-derivative
bound is 13.563. The chord error bound `M/(8*N*N)` is 0.001655640 pixels;
rounding the common endpoints to f32 adds less than 1e-6 pixels per axis at this
extent (less than 1.35e-6 in Euclidean norm), retaining a total bound below
1/256 pixel. Canonical curve controls and all pixel thresholds remain unchanged.
The receipt compares polyline fill/clip within each backend, and canonical
curve fill against the diagnostic polyline. For bounded root-owned runtime:

```sh
classic_edge_probe /Users/markik/Code/output/vello-classic-browser-gates-20261011/polyline-fill-all all --case polyline-fill
classic_edge_probe /Users/markik/Code/output/vello-classic-browser-gates-20261011/polyline-clip-all all --case polyline-clip-only
```

Running without `--case` includes both cases and records their within-backend
comparison. If exact line inputs match across providers, the canonical fixture
gap is flattening-dominated; if not, coverage/composition remains implicated.
This result alone cannot establish pixel acceptance for all production glyphs.

The optional five glyph cases are:

- `glyph-original-run`: one unchanged caller-shaped `SceneGlyphRun`, isolated
  from unrelated source painting while preserving its font, positions,
  variation settings, transform, device clip, and premultiplied color.
- `glyph-skrifa-draw-size`: explicit unhinted Skrifa outlines drawn at the run's
  font size, then positioned/Y-flipped/transformed as filled paths.
- `glyph-skrifa-upem-scaled`: explicit unhinted outlines drawn at UPEM and scaled
  by the run's font-size/UPEM ratio, then positioned/Y-flipped/transformed.
- `glyph-run-fractional-translation`: unchanged glyph representation with a
  `(0.25,0.375)` post-transform, device clip disabled explicitly for this control.
- `glyph-run-uniform-scale`: unchanged glyph representation with a `1.25`
  post-scale, device clip disabled explicitly for this control.

Glyph selection is limited to a run containing 1–256 glyphs. No shaping,
fallback font, glyph substitution, or font-byte output is introduced. Captured
bundle and source font hashes, variation coordinates, glyph positions, scene
digests, PNGs, raw/premultiplied pixel hashes, and exact timings are recorded.
Font-bearing scene or bundle copies are not written. Baseline glyph comparisons
use the callback's logical viewport; source layout scale is recorded rather
than silently applied.

Every cross-renderer comparison retains edge RGBA/alpha thresholds 32/16,
interior thresholds 3/2, and mean channel delta 1.0. CPU one-pixel neighbor
variation above 8 defines an edge. Classic raw straight-alpha output is
converted to premultiplied encoded-sRGB bytes; sparse bytes are already
premultiplied. PNG output preserves transparency. Within-backend isolation
comparisons use the same metrics but are diagnostic, not additional identity
gates: repeated antialiased clipping can expose multiplied coverage.

## Source hypotheses to resolve with those receipts

Classic copies `ScenePath` cubic/quad control points without discarding them
(`netrender/src/vello_rasterizer/emit.rs`). Its shader flattening tolerance is
0.25 with an Euler-spiral approximation
(`research/vello_shaders/shader/flatten.wgsl:376` in the provider checkout).
Sparse fill invokes the common SIMD flattener
(`vello_common/src/flatten.rs:209`), using the same numeric 0.25 tolerance with a
different kurbo-derived algorithm. Identical tolerances alone do not establish
identical coverage. Direct fill versus clip isolates that from nested
composition; the square accent case separately controls Plus side seams.

Classic's glyph cache uses unhinted Skrifa outlines at requested draw size
(`research/vello_encoding/src/glyph_cache.rs:204`). Its global uniform scale is
absorbed into font size only for hinted runs
(`research/vello_encoding/src/resolve.rs:445`). The Netrender Classic caller does
not enable hinting. Sparse explicitly requests `hint(false)` and disables atlas
caching (`netrender/src/vello_backends/sessions/text.rs:423`); its Glifo callback
delegates to the outline backend. Glifo caches unhinted fill outlines at UPEM
(`glifo/src/glyph.rs:1263`) and also absorbs positive uniform scale into font
size for unhinted runs (`glifo/src/glyph.rs:1680`). Both use the same caller glyph
IDs/positions and normalize axis settings through Skrifa. Their representation,
scale absorption, floating-point transform precision, and subsequent flattening
are distinguishable hypotheses, not an established production bug.

Do not change provider or backend behavior until the small runtime receipts
identify which distinction matters. If original glyphs match explicit draw-size
paths within Classic but both diverge from sparse, glyph API forwarding is a
weaker explanation than provider flattening/coverage. If explicit draw-size and
UPEM paths differ substantially within one backend, first measure outline and
transform representation rather than widening pixel tolerance. If plain cubic
fill passes but curve clipping fails, investigate clip lowering before changing
canonical geometry.
