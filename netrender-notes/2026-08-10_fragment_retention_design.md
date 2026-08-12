# Fragment retention: incremental scene construction (2026-08-10)

**Status**: design note, not scheduled. The trigger gate is at the end.
Nothing here is implemented; code samples are illustrative only.

Companion measurements live in
[`../netrender/examples/e1_damage_profile.rs`](../netrender/examples/e1_damage_profile.rs).
Prior art in this repo: E3's op-to-tile index
([verification record §11.35](2026-05-01_vello_verification_record.md)),
E2's `SceneFragment` (§11.32), and the Path B `register_texture`
registry pattern.

## 1. The two measurements that motivate this

Both taken 2026-08-10, RTX 4060 / Vulkan, release, median of 30 frames,
microseconds. Same document-shaped scene in both tables; the only
difference is a 7px/frame camera translate in the second.

**Static page, one dirty tile per frame (caret blink):**

| viewport | tiles | ops | dirty | invalidate | rebuild | compose | vello | total |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 512² | 4 | 62 | 1 | 6.4 | 5.6 | 3.7 | 250.6 | 267.0 |
| 4096² | 256 | 4422 | 1 | 472.2 | 36.0 | 121.1 | 558.1 | 1187.5 |

**Same page under a camera pan:**

| viewport | tiles | ops | dirty | invalidate | rebuild | compose | vello | total |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 512² | 4 | 62 | 4 | 6.5 | 18.1 | 3.5 | 260.7 | 290.3 |
| 1024² | 16 | 296 | 16 | 28.5 | 103.0 | 8.2 | 272.7 | 409.7 |
| 2048² | 64 | 1094 | 64 | 111.3 | 866.9 | 35.9 | 409.2 | 1446.1 |
| 4096² | 256 | 4422 | 256 | 487.3 | **11634.8** | 155.5 | 676.1 | **12936.4** |

The static case is fine post-E3. The pan case is not: identical content,
placement moved, and the frame costs 11x more. 90% of it is
`dirty_tile_rebuild`, which is `filter_scene_to_tile` +
`scene_to_vello` for every dirty tile, and under pan every tile is
dirty. `filter_scene_to_tile` walks the whole op list per tile, so this
is the same O(tiles × ops) complexity class E3 removed from detection,
alive in lowering. E3 could not touch it because detection was doing its
job: everything really did change, by the only definition the cache has.

That definition is the root cause. The per-op digest folds the world
AABB in (the anti-ghosting rule, `hash_aabb`'s doc comment), so the
cache cannot distinguish "this op changed" from "this op moved". Pan and
scroll are placement-only changes, and mere's graph canvas, B2 scroll
frames, and any host-chrome panel drag are all this shape.

A second, smaller motivation: the ~95 ns/op digest floor from E3 exists
because the consumer rebuilds the `Scene` from scratch every frame, so
there is nothing to cache a digest against. Both problems have the same
fix: give scene content an identity that survives across frames.

## 2. The kernel: split content from placement

One sentence: digest a fragment's content in fragment-local space, keep
the digest while the content generation is unchanged, and mix placement
in at bin time.

Then:

- **Content change** re-digests and re-lowers one fragment, O(that
  fragment's ops).
- **Placement change** (pan, scroll, drag) re-bins fragments and
  re-composes, O(fragments), and invalidates nothing that was lowered.
- The ghosting rule survives: placement participates in every covering
  tile's hash, so a moved fragment still dirties the tiles it leaves and
  enters. What changes is what "dirty" costs: re-compose, not re-lower.

## 3. Shape of the API

Follows the Path B `register_texture` precedent: the `Renderer` owns a
keyed registry of retained state; the `Scene` stays a flat value that
references it by id. Painter order stays consumer push order; a fragment
placement is one op in the stream, so fragments interleave freely with
direct ops and with each other.

```rust
// Illustrative only, not implementation-ready.

// Retained side (Renderer-owned registry, like image Path B):
let id: FragmentId = renderer.register_fragment(fragment);  // SceneFragment, E2's type
renderer.update_fragment(id, new_fragment);                 // bumps content generation
renderer.remove_fragment(id);

// Per-frame side (Scene stays a cheap flat value):
scene.place_fragment(id, transform);   // pushes SceneOp::Fragment { id, transform_id }
```

`SceneOp::Fragment` is the one new op variant. On `update_fragment` the
renderer computes, once: the fragment's local per-op digests, its local
AABB, and (lazily) its lowered `vello::Scene`. On `place_fragment` the
frame index bins the fragment by its placed AABB and mixes
`(content_generation, placement_transform_bits)` into covering tiles'
hashes. `compose_into` appends the cached lowered scene under the
placement transform; `vello::Scene::append` already takes exactly that
`(scene, Option<Affine>)` pair, and it is what `compose_into` does with
tile scenes today.

What this does to the pan numbers, mechanically: `rebuild` drops from
O(tiles × ops) to zero for unchanged fragments; per-frame cost becomes
invalidate O(direct ops + fragments) + compose O(fragments + tiles) +
vello. At 4096² that is the ~12.9 ms pan frame falling back to roughly
the ~1.2 ms static frame. GPU cost is untouched: vello still re-encodes
and re-renders the whole master (that is E1, upstream, unchanged).

## 4. Granularity: fragment-unit binning first

Two options for the frame index:

1. **Fragment-unit** (recommended first): bin the fragment's placed AABB
   as one entry; a covering tile's hash takes one
   `(generation, placement)` mix per fragment. O(fragments) index cost.
   Coarse: any content change inside a fragment dirties every tile the
   fragment covers.
2. **Per-op**: keep E3's per-op bins, in fragment-local space with a
   placement offset composed at hash time. Fine-grained, but placement
   change forces re-binning every op, and the arithmetic for composing
   offsets into binned AABBs has to be exactly `aabb_intersects`-
   equivalent a second time.

Fragment-unit wins on cost and on matching the consumer's mental model:
a fragment should be the unit of independent change (a gnode card, a
panel, a paragraph block). If a consumer's fragments turn out too
coarse, the fix is more fragments, not a finer index. Per-op stays
available if a profile ever demands it.

The tile cache itself does not go away. Direct (non-fragment) ops keep
the exact E3 path, which also remains the fallback for consumers who
never touch fragments. Within a changed fragment, lowering is per
fragment, not per tile; tiles remain the invalidation and A3-overlay
vocabulary, and the unit any future partial-present or OS-compositor
work (D3) speaks.

## 5. Who diffs: the xilem question

Netrender should grow identity and lifecycle, and nothing else. The
diffing brain (which fragment changed, what the new content is) belongs
to the layer that owns a tree, and that layer already exists or is
growing in this ecosystem:

- `sprigging` describes itself as retained custom-paint leaves; a leaf
  maps to a fragment naturally.
- `xilem_serval` is the host-framework direction and `xilem_core` is the
  renderer-agnostic diffing half, already in `crates/xilem`.
- genet's DOM and mere's graph model are trees with change tracking of
  their own.

Importing `View`-tree machinery into netrender would duplicate all of
that, one layer down, against the propagate-capability-up-the-stack
rule. A flat op stream plus stable fragment handles is the whole
renderer-side contract. Consumers that want xilem semantics build them
on top; consumers that want immediate mode keep it.

The wire layer is deliberately out of scope for v1: `paint_list_api`
speaks whole envelopes with a whole-list `generation`. Fragment refs in
the envelope vocabulary (so IPC consumers can send deltas) is a real
future item with its own design questions (key allocation across
processes, lifetime), and nothing in v1 should preclude it. First
consumers are in-process (sprigging, chisel, hosts), so the Scene API is
the right seam to start at.

## 6. Convergence with the backend seam

The [backend-seam note](2026-08-04_rasterizer_backend_seam.md) blocked a
`vello_hybrid` backend on "two retention models": vello retains lowered
scenes, hybrid would retain textures or nothing, and a seam would have
to tolerate both. Fragment identity dissolves most of that objection.
The seam contract becomes: the renderer names fragments and their
generations; each backend retains whatever its native form is, keyed by
`(FragmentId, generation)`, vello a lowered scene, hybrid a texture, and
invalidation receipts are fragment-level on both paths.

This also sharpens the upstream ask
([draft](2026-08-10_vello_hybrid_upstream_ask.md)): `Scene::append` is
the one primitive that lets a backend participate in fragment retention
at all.

Lineage note, for honesty: WebRender had this shape as pipelines +
spatial nodes + picture-cache slices, and netrender deliberately
flattened it away at the fork. The pressure to regrow it was
predictable; the regrowth is leaner (handles and generations, no IPC
epochs, no spatial tree in the renderer).

## 7. What this does not fix

- **GPU cost.** vello re-encodes and re-renders the whole master every
  frame regardless (E1, upstream-gated). Fragments fix the CPU side.
- **Consumer emit cost.** A consumer that rebuilds its `PaintList` and
  re-translates every frame pays that before netrender is involved.
  Fragments give consumers a reason and a place to stop doing that, but
  the stopping is their work.
- **First-frame cost.** Everything still lowers once.

## 8. Trigger and done conditions

Per the roadmap's rule (items come from real consumer needs) and the E1
lesson (measure before designing further):

*Trigger, both required:*

1. A consumer commits to driving fragments (sprigging / a cambium host
   panel, or a mere canvas surface). "Commits" means a named integration
   target, not interest.
2. An end-to-end profile of that consumer's frame (emit + translate +
   render) exists, so the fragment win is sized against the whole loop,
   not just netrender's slice. A4 spans cover the render half;
   the emit half needs consumer-side timing.

*Done conditions:*

- `e1_damage_profile`'s pan table: per-frame cost independent of op
  count for unchanged-content fragments; the 4096² pan row lands within
  ~2x of the static row instead of 11x.
- E3's differential tests extended to cover `SceneOp::Fragment`
  (placement change, content-generation change, fragment interleaved
  with direct ops, fragment crossing a layer boundary), still asserting
  identical dirty sets against a reference.
- `pa3_tile_dirty_tracking` green unchanged, plus a new receipt: a moved
  fragment dirties exactly the tiles it left and entered.
- Hit testing accounts for fragments (`hit_test` walks a scene with
  `SceneOp::Fragment` correctly, resolving into fragment-local ops).

*Explicit non-goals for v1:* wire-level fragment refs in
`paint_list_api`; any diffing machinery in netrender; per-op binning
inside fragments; changing the direct-op path.
