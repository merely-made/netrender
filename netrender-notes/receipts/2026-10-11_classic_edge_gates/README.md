# Classic edge isolation — 2026-10-11

The canonical curved fixture's Classic gap is flattening-dominated. Identical
fine line endpoints pass across Classic, CPU and Sparse GPU with maximum alpha
difference zero and premultiplied RGBA difference one; the unchanged canonical
cubic still differs by 42 alpha levels. The actual body glyph has a separate
representation threshold crossing: draw-size versus UPEM outlines differ by at
most 0.013671875 pixels, but sparse subdivision changes and amplifies that to
53/51 alpha levels. Classic still differs with the shared UPEM outline.

These are diagnosis receipts. Overall Classic pixel acceptance remains false.
No production renderer/provider behavior, canonical geometry or thresholds
changed. Classic remains the shipping host renderer. The fine polyline is only
a bounded diagnostic control, not a production workaround.

- [Diagnosis and exact source arithmetic](offline-analysis/classic-curve-glyph-diagnosis.md).
- [Safe arithmetic and pixel evidence](offline-analysis/glyph-coordinate-audit.json).
- [Final 11-case geometry receipt](geometry-with-polyline/receipt.json).
- [Actual isolated glyph receipt](glyph-all/receipt.json).
- [Four passing source/CPU controls](checks/classic-polyline-controls.log).
- [Diagnostic source and execution protocol](../../../paint_list_render/examples/classic_edge_probe/README.md).

Source base is Netrender ef5412d760a978584c8c1801e8a765c51aecd403;
provider is 5862bba26c8a9f56f6a5fdc8ea6966b360a5d63a. Owned source hashes,
input identity, intact scene digests, timings, transparent PNGs and output hashes
are recorded. The actual caller-shaped body run remains 48 glyphs with its
original font, axes, positions and transform. No fonts or font-bearing source
bundles are committed here; the complete input bundle stays local.

Both coordinating tasks explicitly handed off the finite runtime lane. Only
root built, at two jobs; all GPU/browser processes ran serially with finite
outer deadlines and five-second GPU copy/readback deadlines. Root preserved
borrowed Cargo caches and released the lane/target after the batch. Matching
[diagnostic filename metadata](gpu-diagnostics-comparison.json) did not change;
that observation is not forced-reset or systemwide health proof.

Production browser presentation has a separate Mere receipt. Browser portable
producer export and cross-renderer parity, OS assistive actions, native text
caret/selection pixels and real host device-loss replacement remain separate
open gates. The completed earlier orderly two-device recreation is not driver
reset recovery. Production flattening remediation remains pending.
