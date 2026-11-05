# Sparse text glyph culling

CPU, native Hybrid and browser sparse sessions now skip drawing positioned glyphs
whose validated outline bounds cannot reach the target. The classic tiled path
already culled by tile (2026-11-03); the sparse sessions drew every admitted glyph.

Culling happens in `TextState::plan`, after each glyph has passed its ID,
outline, finite-coordinate, segment-budget and derived-transform range checks.
Those checks, face admission, identity-collision refusal, font/outline budgets,
outline preparation, cache epochs and commit semantics are unchanged. A culled
glyph still counts against positioned-glyph and frame-segment budgets, and its
outline is still prepared and retained. An invalid glyph far outside the target
still refuses the frame.

The test rectangle is the viewport intersected with the run's device-space clip
(rounded corners only shrink it). Sparse sessions refuse layer filters and
backdrop filters, so no admitted operation samples pixels outside the target, and
blend/compose layers are pixel-local. Bounds are the control-point box of the
unhinted outline at the run's variation location, in font units, mapped through
the same affine the range check uses; control points contain the filled outline.
The rejection margin is two device pixels plus a 1e-6 relative allowance for the
renderer recomputing the transform in f32 at a scaled font size. Empty outlines
are culled. A fully culled run still builds a Glifo run with no glyphs, so
render-time font errors surface as before.

`SparseTextStats::culled_glyphs` reports cumulative culled glyphs.

Tests cover culling after every glyph check, budgets counting culled glyphs,
off-target refusal, fully culled runs retaining their prepared outline, and exact
CPU pixel equality with culling disabled on a fixture that straddles every edge
at subpixel offsets, under rotation, a rounded run clip, 3.5 px and 41 px sizes,
and a non-default variation location. That test fails when the margin is made
negative. Native Hybrid pixel equality is qualified by Cue's paired references.
