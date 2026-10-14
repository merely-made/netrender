# Shadow helper image custody

The public `build_box_shadow_mask` helper calls `insert_image_vello`. On the
qualified prior supplier `43c5fc0e3e4d75f1eb708bda7f2b08f31df7fddc`, reusing a
mask key registered a new Vello identity without retiring the previous one.
Cached scenes still referenced that previous identity, so changing only blur
parameters could leave old pixels while GPU source registrations grew.

The bounded repair reuses the existing same-size refresh path and marks atlas
pixels dirty. Resizing retires the old identity. Helper identity creation,
replacement and explicit retirement invalidate global, per-surface and retained
scene caches whose scene hashes do not include override identity. The public
helper signatures and mask/filter namespaces remain unchanged. Explicit helper
images are not enrolled in filter-slot retirement or automatic collection.

## Actual GPU controls

`shadow_mask_lifecycle.rs` boots one Device/Queue; reused and fresh oracle
renderers receive those same handles. The Scene and image key remain constant.
Global rendering, two independently cached surface stores and retained-fragment
rendering all match fresh oracle RGBA bytes under six mask updates:

| Case | Dimensions | Blur | Inverted | Change |
| --- | ---: | ---: | --- | --- |
| 0 | 64 | 2 | false | Initial mask |
| 1 | 64 | 2 | false | Repeated identical parameters |
| 2 | 64 | 8 | false | Blur only |
| 3 | 64 | 8 | true | Inversion only |
| 4 | 96 | 8 | false | Resized source |
| 5 | 64 | 2 | false | Smaller source and changed bounds |

The reused renderer owns exactly one live registration throughout. Explicit
retirement and duplicate retirement leave zero live registrations. Four cached
rendering lanes then paint zero bytes despite unchanged Scene references.
Re-admission under the same key matches the fresh oracle in every lane, including
retained rendering, and final explicit retirement leaves zero registrations.
Every output readback is bounded and provider quality counters complete.

The exact previous supplier is restored as a negative control using byte-safe
Git source extraction. It **fails as expected**: handle counts grow to two and
three; the blur-only update first differs at `(19,0)`, with actual transparent
RGBA versus oracle `[51,127,203,1]`. The bad output differs at 3,408 of 4,096
pixels. Source guards restore the qualified helper byte hashes before the final
positive run. Both failed control captures and logs remain here.

Final gates on NVIDIA GeForce RTX4060 Laptop GPU / Vulkan, vendor4318/device10464:

- Shadow helper GPU lifecycle: **1 passed**. Twenty-four captured reused/oracle
  pairs have exact RGBA equality; four retired captures are zero.
- Existing filter texture lifecycle: **8 passed**, including bounded handles,
  moving content, explicit retirement and static tile reuse.
- Existing registered texture round trip: **1 passed**.

Commands are locked/offline Rust1.99.0, default features, `-j1`, with the existing
`C:/t/cargo-targets/netrender` target and matching build directory. Exact commands,
source hashes, negative control, PNGs and comparison records accompany this
receipt. Vello remains `2eae6243881d5b24c984d5ee35dde509ca5db782` and Cargo.lock
is unchanged. No isolated Cargo home or worktree was created.

## Scope

This is a new helper resource/pixel qualification. The prior
[empty-tile receipt](../2026-10-14_empty_tile_layers/README.md) remains evidence
for its original supplier and is not relabeled. Caller mask keys must identify
their logical producer and callers must explicitly retire withdrawn helpers.
Native Turnstone mask custody/adoption, driver loss, physical pooling and generic
module retirement remain separate owner gates. No GC rule, rendering fallback or
new recovery factory is introduced.
