# Borrow source data during Classic lowering

Dirty tiles previously built an owned temporary Scene by copying every selected operation and both palettes. Retained direct-operation runs did the same, and first-time fragment lowering cloned the registered fragment and remapped its already-local IDs through another temporary Scene.

Classic lowering now consumes borrowed palettes and an ordered stream of operation references. FrameIndex remains the tile-membership owner. Tile filtering retains only references while preserving empty-layer pruning. Retained runs borrow their parent; registered fragments lower against their own local palettes and image sources. Ordinary valid palettes are borrowed. Malformed or missing fragment slot-zero sentinels keep the old normalization behavior through a narrow fallback.

Public translation APIs, image override precedence, cache-only lookup, nominal fragment viewport, painter order, and retained cache invalidation stay intact. The owned Vello encoding remains the backend cache. This removes source copies visible in the implementation; it makes no measured frame-time or GPU-upload claim.

Done conditions: selected operation and palette identity checks, fragment-local IDs and sentinel compatibility, cache-only behavior, actual retained tile and fragment pixel tests, filtering and layers, feature-off Wasm compilation, then downstream retained-path qualification. All heavy gates use the shared machine lock, 3 GiB memory cap, no swap, 150 percent CPU, one build job, one test thread, nice 10, and a 600-second timeout per gate.
