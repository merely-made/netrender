# Classic curve and glyph diagnosis — 2026-10-11

Classic's remaining edge gate has two related mechanisms: different canonical-curve flattening across providers, and glyph representation rounding that changes adaptive flattening decisions. The new minimal receipts make ordinary glyph forwarding a weaker explanation for these selected failures. The plain-fill/single-clip equality and passing square Plus control independently narrow the remaining curve discrepancy. Production/provider source and thresholds remain unchanged.

Authoritative runtime inputs are [geometry-all/receipt.json](../geometry-all/receipt.json), [glyph-all/receipt.json](../glyph-all/receipt.json), and the final [geometry-with-polyline/receipt.json](../geometry-with-polyline/receipt.json). This report only reads those receipts/PNGs and local primary-source implementations. [glyph-coordinate-audit.json](glyph-coordinate-audit.json) holds input hashes, bounded control-point samples, pixel samples and the next-diagnostic bounds; [glyph-coordinate-audit.py](glyph-coordinate-audit.py) reproduces the simple-glyf coordinate and flatten-decision audit. No font or font-bearing scene/bundle copies are written.

## Runtime isolation

- Plain canonical cubic fill and the same path as one clip over covering paint are pixel-identical **within each backend**. Both have Classic-versus-CPU edge alpha 42. Clip-only lowering therefore does not account for that selected canonical-curve failure.
- The square accent border with Plus passes (edge RGBA/alpha at most 2/1). The curved accent and uniform-color rounded borders fail with edge alpha 42. This separates ordinary Plus joining from curve coverage.
- For actual producer `1952539235`, glyph op `25`, CPU and sparse GPU original glyph rendering are **exactly equal** to their explicit UPEM-scaled paths.
- Classic original glyph rendering differs from its explicit draw-size paths at only two pixels, maximum RGBA/alpha 4. This is accepted under the unchanged gate and is weak evidence for a forwarding bug.
- Draw-size versus UPEM paths differ within CPU/sparse GPU by alpha 53 and within Classic by alpha 36. Classic original versus CPU original has alpha 53; forcing a shared UPEM representation still leaves a Classic-versus-CPU difference of 51. Representation rounding is material, but it is not the entire provider gap.

Every original gate continues to compare premultiplied encoded-sRGB RGBA8 including alpha, with no matte: edge RGBA/alpha 32/16, interior 3/2, mean channel delta 1.0. The recorded pixel gate remains separate from successful diagnostic execution.

## Exact source of the representation difference

The captured font SHA256 is `25eceb458d4baf628ee0b6a135a9f8ac5ec7b2826646720834bd2bf00dfc825a`, matching `/System/Library/Fonts/Helvetica.ttc`, collection face 0. Run size is 14, UPEM is 2048, variation settings are empty, the world transform is identity and there is no device clip.

`DrawSettings::unhinted` defaults to `PathStyle::FreeType` (`skrifa-0.44.0/src/outline/mod.rs:189`, `outline/pen.rs:17`). The unhinted glyf dispatch still uses `FreeTypeScaler` (`outline/mod.rs:417`), which stores **F26Dot6** scaled points (`outline/glyf/mod.rs:374`). Fixed multiplication rounds at 26.6 precision (`font-types-0.12.5/src/fixed.rs:189`); implied quadratic midpoints use signed integer division at that same precision (`read-fonts-0.41.0/src/tables/glyf.rs:778`). Unhinted is not an assurance of unrounded outline coordinates.

Classic draws those outlines at size 14. Glifo caches unhinted fill outlines at UPEM 2048 and later scales by `14/2048 = 7/1024`. The draw-size Fixed scale is exactly raw bits `28672`, versus `4194304` at UPEM. The offline audit decodes only the body's 22 unique simple glyf point streams, applies that exact fixed arithmetic and FreeType midpoint convention, and compares corresponding emitted controls after scaling. Maximum coordinate difference is **0.013671875 pixels**. This is a primary-source arithmetic model, not a claim that this Python code executed Skrifa.

The baseline ratio, positions, outline coordinates and midpoints are dyadic values that fit f32 precision at these small extents. The identity matrix introduces no extra scale or skew. Thus transform precision is not a credible explanation for the large baseline differences at the two localized pixels.

## A small coordinate change crosses a discrete flattening threshold

There is one sparse simplification/subdivision decision crossing among those unique glyphs: glyph **82**, quadratic segment **10**, the outer right edge of the letter `o`.

| Representation | P0 | P1 | P2 | Control-to-chord distance | Sparse decision |
| --- | --- | --- | --- | ---: | --- |
| Draw size | (7.21875,3.8125) | (7.21875,2.046875) | (6.359375,0.890625) | 0.498199916 | One line |
| UPEM then scale | (7.2255859375,3.814453125) | (7.2255859375,2.0439453125) | (6.3642578125,0.888671875) | 0.500007355 | Two lines |

`vello_common/src/flatten_simd.rs:153` simplifies a quadratic to its chord when control-to-segment distance is at most 0.5, which bounds curve deviation by 0.25. The UPEM representation is just above that threshold. Its subdivision estimate is `1.010926903`; taking the ceiling yields two segments. Tiny fixed-point differences can therefore cause a much larger piecewise-linear coverage change.

The two run occurrences have origins `(243.5439453125,195)` and `(297.2470703125,195)`. Their world-space curves cover the exact previous body-line failure pixels `(250,192)` and `(304,192)`.

| Pixel | CPU original/UPEM alpha | CPU draw-size alpha | Classic original/draw-size alpha | Classic UPEM alpha | Sparse GPU original/UPEM alpha |
| --- | ---: | ---: | ---: | ---: | ---: |
| (250,192) | 149 | 96 | 96 | 98 | 149 |
| (304,192) | 73 | 22 | 22 | 23 | 73 |

These alpha values are read directly from existing transparent PNGs and incur no RGB round-trip ambiguity. At the first pixel, numerical integration of the isolated outer quadratic gives ideal area alpha **157.276551** for draw-size geometry and **158.999256** for UPEM geometry: only 1.72 units apart. The inner-hole control-point hull ends at world x 249.484375 for the first occurrence and 303.1875 for the second, wholly left of each sampled pixel; adjacent outer edges lie above/below its y interval. The isolated edge-area model therefore applies at these pixels. Replacing the draw-size quadratic by its chord gives alpha **96.049805**, matching observed alpha 96. At the second pixel the corresponding draw-size chord is **22.308256**, matching observed alpha 22. The 53/51-unit runtime jump is therefore explained by adaptive flattening amplifying the small representation difference at these pixels, rather than by a 53-unit change in the underlying smooth glyph geometry.

Classic still renders the shared UPEM edge at alpha 98/23 rather than sparse 149/73. Its 0.25 shader flattening uses a different approximation and subdivision scheme. The initial glyph receipts alone do not separate that line selection from fine-area coverage for identical glyph segment lists. The completed font-free polyline control below isolates the canonical cubic fixture; a corresponding identical-line glyph control was not executed.

## Executed fine-polyline control, without a production geometry change

The final root-run diagnostic lowers the **same original canonical cubic contour once** to an explicit fine polyline in world space and gives the exact same f32 endpoint vector to every backend under identity transforms. Existing canonical curve fill/clip cases remain unchanged. The two additions are `polyline-fill` and `polyline-clip-only`.

A uniform parameter partition has Euclidean chord error bounded by `M/(8 N²)`, where `M` bounds the norm of the cubic's second derivative. For the radius-5×3 quarter ellipse, `M = 13.562702`; a conservative bound 13.563 and **32 dyadic segments per quarter** yield error at most **0.001655640 pixels**, below 1/512. Endpoint rounding at extent 24 adds less than 1e-6 pixels per axis (less than 1.35e-6 in Euclidean norm), so the total remains below **1/256 pixel**. Four quarters give 128 curve segments, plus four straight edges: **132 line segments** total. The closed endpoint already equals the start, so Close adds no new geometric edge.

The executed results are:

| Final control | Classic vs CPU edge RGBA/alpha | Sparse GPU vs CPU edge RGBA/alpha | Acceptance |
| --- | ---: | ---: | --- |
| Original `curve-fill` | 42 / 42 | 1 / 0 | Classic red |
| Original `curve-clip-only` | 42 / 42 | 1 / 0 | Classic red |
| Diagnostic `polyline-fill` | 1 / 0 | 1 / 0 | Both GPU families accepted |
| Diagnostic `polyline-clip-only` | 1 / 0 | 1 / 0 | Both GPU families accepted |

Both polyline cases have exact interior matches against CPU. Their mean channel delta is 0.00375. Polyline fill and single clip are pixel-identical within **every** backend. The final diagnostic completed in approximately 3.3 seconds; its overall acceptance flag remains false because the retained canonical curve cases still fail. Process completion is not a three-renderer parity claim.

**The canonical fixture gap is flattening-dominated.** When all backends receive identical line endpoints, fine area coverage agrees exactly in alpha and within one premultiplied RGB unit. This isolates one fixture; it does not qualify every glyph or justify a production canonical-geometry rewrite. A generalized follow-up should cap 4096 segments per curve and 16384 per case, refusing diagnostics that exceed those bounds.

The root-built source/CPU controls passed **4/4**, with no failures ([control log](../checks/classic-polyline-controls.log)); this report also records the [runtime completion log](../checks/classic-polyline-final-runtime.log). Source base is `ef5412d760a978584c8c1801e8a765c51aecd403`, and the read-only provider remains `5862bba26c8a9f56f6a5fdc8ea6966b360a5d63a`. JSON records SHA256s for those logs, the authoritative receipt, the selected PNGs, and exact source files:

- `/Users/markik/Code/worktrees/netrender-cambium-resource-bridge/paint_list_render/examples/classic_edge_probe.rs`
- `/Users/markik/Code/worktrees/netrender-cambium-resource-bridge/paint_list_render/examples/classic_edge_probe/fixtures.rs`
- `/Users/markik/Code/worktrees/netrender-cambium-resource-bridge/paint_list_render/tests/classic_edge_probe.rs`
- `/Users/markik/Code/worktrees/netrender-cambium-resource-bridge/paint_list_render/examples/classic_edge_probe/README.md`

**Production remediation remains explicitly pending.** There are no production/backend/provider changes and no widened thresholds. The root task owns all executed builds and GPU runs. This offline receipt update performed no renderer calls or adapter initialization.
