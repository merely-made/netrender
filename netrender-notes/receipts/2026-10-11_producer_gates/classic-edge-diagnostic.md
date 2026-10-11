# Classic edge diagnostic — 2026-10-11

Classic pixel acceptance remains open with the existing thresholds. Complete-resource identity admission passed. The synthetic rounded border exposes an alpha-coverage difference at curved edges; actual bound producer scenes independently expose glyph-edge RGB differences. Sparse CPU/GPU acceptance passed for both captured runs. This audit read existing receipts and PNGs and inspected source; it did not execute a renderer.

The authoritative receipts are [synthetic-parity-final/receipt.json](synthetic-parity-final/receipt.json) and [production-wide-cold-parity/receipt.json](production-wide-cold-parity/receipt.json). [classic-edge-diagnostic.json](classic-edge-diagnostic.json) contains their SHA256s, exact PNG SHA256s, localized pixel samples, and the analytic method. It contains no font or callback scene blobs.

## Exact harness results

Comparison uses premultiplied encoded-sRGB RGBA8 including alpha, without a background matte. Thresholds remain edge RGBA 32 / alpha 16, interior RGBA 3 / alpha 2, mean channel delta 1.0. Edge classification uses CPU neighbor variation above 8. Exact source/resource identity is a separate admission result.

| Run / renderer | Edge RGBA / alpha | Interior RGBA / alpha | Mean channel delta | Pixel acceptance |
| --- | ---: | ---: | ---: | --- |
| Synthetic composition / Classic | 42 / 42 | 1 / 1 | 0.673177 | Failed |
| Synthetic composition / sparse GPU | 2 / 1 | 2 / 1 | 0.161892 | Passed |
| Synthetic producer / Classic | 1 / 0 | 1 / 0 | 0.023438 | Passed |
| Actual wide-cold composition / Classic | 45 / 0 | 1 / 0 | 0.030058 | Failed |
| Actual producer 1952539235 / Classic | 44 / 1 | 0 / 0 | 0.016327 | Failed |
| Actual producer 1952539250 / Classic | 46 / 0 | 0 / 0 | 0.137276 | Failed |

Both actual direct producer sparse GPU comparisons passed. Their interior deltas are zero, and edge RGB/alpha deltas are at most 2/1. The synthetic direct source is also accepted by both GPU renderer families, so CPU materialization cannot explain the synthetic composition-only failure.

## Curved border localization

Coordinates below are zero-based and values are **straight PNG RGBA**, which makes the alpha difference directly observable. The authoritative gate uses original premultiplied raw bytes. Reconstructing premultiplied RGB from a PNG can introduce a one-unit round-trip difference and is used only for localization.

| Synthetic pixel | CPU PNG | Classic PNG | Sparse GPU PNG |
| --- | --- | --- | --- |
| (21, 0) | [108, 141, 171, 139] | [107, 141, 170, 97] | [106, 141, 171, 139] |
| (21, 23) | [108, 141, 171, 139] | [107, 141, 170, 97] | [106, 141, 171, 139] |
| (22, 2) | [107, 141, 171, 76] | [106, 141, 170, 44] | [107, 141, 171, 76] |
| (20, 0) | [107, 142, 170, 189] | [107, 141, 170, 160] | [107, 140, 170, 189] |

At (21, 0), the top-side polygon covers the pixel, the inner hole does not intersect it, and the side opacity is one. The point is away from the side-join diagonal. Thus Plus accumulation at the side seam or higher-level opacity does not explain this particular 42-unit alpha difference. Nearly equal straight RGB with differing alpha also rules out black-matting or unpremultiplication as an explanation for this sample.

The shared top-right outer cubic of the 24×24 fixture has radii 5×3 and control points (19,0), (21.761424,0), (24,1.3431456), (24,3). Offline integration used 10,000 midpoint x samples across the pixel, each with 50 binary-search iterations for monotone x(t), integrating clamp(1−y(t),0,1). The ideal cubic area at (21,0), scaled to alpha 255, is **150.678349**; the corresponding outer area at (20,0) is **218.494775**. Source f32 coordinate rounding changes control points by less than 1e-6. Both providers undershoot the ideal outer cubic area at these samples, Classic more than sparse. This supports a difference in flattening or area coverage; it does not uniquely prove which internal provider step causes the difference.

The source supports that interpretation: Classic `research/vello_shaders/shader/flatten.wgsl:376` uses tolerance 0.25 with its Euler-spiral cubic flattening scheme; sparse `vello_common/src/flatten.rs:17` also defines tolerance 0.25, but `flatten_simd.rs` implements the different kurbo-derived CPU SIMD approach. Equal numeric tolerances do not guarantee identical segments or coverage. Netrender copies all cubic control points into Classic paths at `netrender/src/vello_rasterizer/emit.rs:170`; sparse path translation similarly preserves them. Classic uses `AaConfig::Area` (`netrender/src/vello_tile_rasterizer/mod.rs:572`). Canonical border cubics and thresholds remain unchanged.

## Independent production glyph edges

The 931×328 source PNG is the Garden notebook reader. Visual inspection places (250,192) and (304,192) on the body line “Every path begins with a small act of attention.” and (118,43) on the toolbar text. These are direct callback scene renders, independent of the composed border path.

| Producer / pixel | CPU PNG | Classic PNG | Sparse GPU PNG |
| --- | --- | --- | --- |
| 1952539235 / (250,192) | [114,120,132,255] | [158,164,176,255] | [114,120,132,255] |
| 1952539235 / (304,192) | [177,184,195,255] | [219,226,237,255] | [177,184,195,255] |
| 1952539235 / (118,43) | [86,91,107,255] | [122,128,146,255] | [85,91,107,255] |
| 1952539250 / (319,212) | [175,182,193,255] | [130,136,148,255] | [176,182,193,255] |

The first source has edge RGB 44 / alpha 1 and the second edge RGB 46 / alpha 0, while both have exact interior matches. These opaque glyph-edge samples cannot be explained by straight/premultiplied alpha conversion. Netrender uses the same captured glyph IDs, positions, and fonts; its Classic `draw_glyphs` path (`netrender/src/vello_rasterizer/emit.rs:123`) does not opt into hinting. Classic provider default hinting is false, and the sparse session documents unhinted outlines. No hint-toggle or dropped-control-point mismatch was identified in this inspection. A provider glyph-outline/flattening/AA difference remains an inference requiring a focused provider investigation.

Classic fine shading explicitly stores straight RGB with original alpha (`research/vello_shaders/shader/fine.wgsl:1432`), matching the harness conversion contract. Source identity and alpha encoding admission remain intact.

## Scope and disposition

Keep the Classic gate red. Do not modify canonical ellipse geometry, matte output, remove commands, or widen thresholds to make this receipt pass. Sparse CPU/GPU acceptance and real producer capture are separate qualified lanes. Provider-specific Classic edge coverage and glyph AA need further work before claiming three-renderer pixel parity.

The final owned Rust files were formatted with a scoped `rustfmt --edition 2021` invocation and frozen after formatting. The earlier root-run `checks/bundle-and-translator-controls.log` records **20 passed, 0 failed**. A subsequent final narrow guard refuses NormalBorder.do_aa=false because its semantics are not lowered; the additional negative control first admits the same AA=true accent border, then requires command-specific refusal after flipping the flag. The final source target is **21 controls**, with the new control awaiting the root-owned final rebuild/run. Root owns the final post-format rebuild and runtime receipts. This diagnostic creates no new build, adapter, or GPU submission.
