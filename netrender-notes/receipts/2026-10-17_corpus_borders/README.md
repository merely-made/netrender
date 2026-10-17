# Captured border corpus verification, 2026-10-17

The article-card mismatch was a stale translation expectation. Commit
`ef5412d76` replaced solid border strokes/rectangles with joined filled rings,
but the August corpus goldens were not updated. Fixing the first expectation
exposed the same issue in `livery_nested_rows`. Both goldens now describe the
existing renderer. Production code, dependency pins and captured command bytes
are unchanged.

The [receipt](receipt.json) records **48 passes, zero failures or ignores**:
17 translator unit tests, 3 captured-border tests, 4 corpus tests, 3 retained
fragment tests and 21 resource-bundle tests. The captured solid borders were
rasterized on CPU. The article border also passed actual AMD RADV GPU
readback, retained placement and a layout move, with exactly one lowering.
Checks cover thickness, bounds, holes, rounded corners and side colors.

Opaque-background flat/retained images match byte for byte, before and after
the move. Transparent images match in every alpha byte and every visible RGB
byte. Raw equality initially exposed residual RGB at alpha zero: pinned Vello
writes straight RGBA after dividing by `max(alpha, 1e-6)`. A direct-border
master-path control reproduced this without retained border content. The
regression explicitly excludes only invisible RGB, retaining exact visible
comparisons. Failed attempts remain in the raw receipt directory.

The complete captured pages cannot be rasterized with their elided font
payloads. Tests extract unchanged border commands; the article target is
widened to include its right edge beyond the original viewport. The article's
second nonuniform dashed border remains unchanged and is not pixel-qualified
here. These checks do not establish browser or cross-platform conformance.

All gates ran serially under the shared machine lock, 5 GiB memory cap, zero
swap allowance, 150% CPU, nice 10, one Cargo job and one test thread. The final
gate peaked at 342172 KiB process RSS; the cold baseline peaked at 888324 KiB.
No gate reported swapping. No consumer pin update is needed for this test-only
repair.
