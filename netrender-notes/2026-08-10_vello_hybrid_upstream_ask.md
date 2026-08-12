# Draft upstream ask: scene composition in `vello_hybrid`

**Status: draft, not sent.** This is prepared for review, not filed.
Nothing has been posted to Linebender's tracker.

Companion to
[`2026-08-04_rasterizer_backend_seam.md`](2026-08-04_rasterizer_backend_seam.md),
which established the blocker. This file is the version of that finding
shaped as something to send, plus a re-verification against
`vello_hybrid` 0.2.0.

## Re-verification (2026-08-10)

The backend-seam note evaluated `vello_hybrid` 0.1.0. **0.2.0 shipped
2026-08-07**, three days after that note was written, so the blocker was
re-checked against the current release before drafting anything.

Verified against the docs.rs API listing for 0.2.0:

- No `Scene::append`, and no public `Scene` method that takes another
  `Scene`.
- No `CommandRecorder` in the public API. Still crate-private.
- Nothing exposing retained scenes, damage regions, or incremental
  rendering.
- `Scene` does now carry `take_current_state` / `save_current_state` /
  `restore_state` over a `RenderState`. That is draw-state save/restore
  (transform, clip, paint), not recorded geometry, so it does not
  substitute.

The blocker holds. What did change is that 0.2.0 added a first-class
capability-probe surface (`Probe`, `ProbeResult`, `ProbeStatistics`,
`ProbeFeature`, and the `WebGl*` probe types), which is the same question
the backend-seam note answered by hand. That is a point in favour of
asking now: upstream is actively building the surface that makes hybrid
adoptable, and this is the remaining gap.

## The ask

One method, mirroring the one `vello::Scene` already has:

```rust
impl Scene {
    pub fn append(&mut self, other: &Scene, transform: Option<Affine>);
}
```

Failing that, the minimal unblocking change is making `CommandRecorder`
(or an equivalent replay entry point) public, so a consumer can build
composition itself without upstream committing to an API shape.

### Why this is not mechanical for hybrid (source findings, 2026-08-12)

Read against `sparse_strips/vello_hybrid/src/scene.rs` on main before
drafting the offer, because the shape of the work changes the ask.
Hybrid's `Scene` holds `strip_storage` (sparse coverage strips,
**generated at record time** against the active transform and viewport),
`encoded_paints`, and a `CommandRecorder<RecordedDraw>` whose entries
hold strip *ranges* into that storage. Source paths are not retained:
by the time a draw is recorded, the geometry has already been flattened
to viewport-space strips.

So the missing `append` is not an oversight; it is the architecture.
Three implementable shapes, in ascending ambition:

1. **`append(other, None)`** — same viewport, no transform: concatenate
   strip storage (offsetting recorded strip ranges), remap encoded-paint
   indices, concatenate commands. Mechanical-ish, and already enough for
   composition patterns that pre-place content at record time.
2. **`append(other, translate)` for integer translations** — shift
   strip coordinates. Covers the retained pan/scroll case, which our
   measurements say is the case that matters, without retaining source
   geometry.
3. **Full affine `append`** — requires retaining paths (a real retained
   command list ahead of strip generation). An architectural decision
   for upstream, not a drive-by PR.

The offer should present these and ask which shape upstream wants,
rather than presuming the vello-mirror signature. Our fragment-retention
measurements (below) are the evidence for why (2) is worth having even
if (3) is out of scope.

## Why it is load-bearing rather than a convenience

Netrender caches lowered geometry per screen tile. Each frame it:

1. asks its tile cache which tiles changed,
2. re-lowers only those tiles into per-tile `vello::Scene`s,
3. builds a fresh master `vello::Scene` and `append`s every cached tile
   scene into it,
4. hands the master to vello.

Step 3 is the whole reason step 2 can be small. Without `append`, a
backend cannot retain lowered geometry between frames in a form it can
re-submit, so every frame has to re-lower everything.

That makes this not "a second lowering behind a trait" but **a second
retention model**. vello retains lowered scenes; hybrid would retain
textures or nothing. A seam wide enough to cover both has to let backends
store categorically different things and needs tile-invalidation receipts
on both paths. That is a much larger and less pleasant abstraction than
the one `append` would allow, and it is the reason netrender has not
built the seam.

## Evidence the composition path is cheap

Measured on netrender's side 2026-08-10, RTX 4060 / Vulkan, release,
median of 30 frames, holding damage at one dirty tile
([`../netrender/examples/e1_damage_profile.rs`](../netrender/examples/e1_damage_profile.rs)):

| viewport | tiles | ops | `master_compose` | `vello_render` | total |
| --- | --- | --- | --- | --- | --- |
| 512² | 4 | 62 | 3.1 | 205.5 | 219.3 |
| 1024² | 16 | 296 | 6.7 | 221.0 | 275.2 |
| 2048² | 64 | 1094 | 18.6 | 226.1 | 545.8 |
| 4096² | 256 | 4422 | 100.9 | 404.4 | 3939.0 |

Microseconds. `master_compose` is the build-master-and-append step: 100 µs
at 256 tiles, about 2.5% of the frame. Appending retained sub-scenes is
not where the cost is, which is worth stating plainly because the natural
objection to adding `append` is that it invites an expensive pattern. In
netrender's use it is the cheap step, and it is what allows the expensive
step to be skipped.

(The 4096² total is dominated by netrender's own dirty detection, tracked
as roadmap E3. That is our bug, not upstream's, and it is not part of
this ask.)

## Where this is heading on our side (strengthens the ask)

Netrender's next planned architecture step is **fragment retention**
([design note](2026-08-10_fragment_retention_design.md)): consumers
register retained sub-scenes behind stable handles, the renderer caches
each fragment's digest and lowered form across frames, and a camera pan
becomes O(fragments) re-composition instead of a full re-lower. The
motivating measurement: a placement-only pan over a static 4096² page
currently costs 12.9 ms/frame, 11× the static case, all CPU.

This matters for the ask because it turns "two retention models" from a
blocker into a non-issue. Under fragment identity, the seam contract is:
the renderer names fragments and generations, and **each backend retains
its own native form** keyed by `(fragment, generation)` — vello a
lowered `Scene`, hybrid a texture or whatever fits. Invalidation
receipts are fragment-level on both paths. The one thing a backend must
support to participate at all is composing retained pieces into a frame:
for vello that is `Scene::append`, which exists; for hybrid it is the
missing method this note asks for.

So the ask is not "add a convenience so netrender's current architecture
ports over". It is: `append` is the minimal primitive for *any* consumer
that retains lowered content across frames, and we have measurements
showing why consumers end up needing to.

## What we can offer

- A capability probe covering every `SceneOp` netrender lowers, passing
  under WebGL2-class limits on wgpu's GL backend. Details in the
  backend-seam note §3.
- A rasterizer-independent regression corpus
  ([`../paint_list_render/tests/corpus/`](../paint_list_render/tests/corpus/README.md))
  that pins consumer-visible behaviour without assuming vello. If a
  hybrid backend landed, this is how we would show it preserves output.
- A real consumer. Netrender is the renderer for eight downstream repos,
  including a full-web engine, and hybrid's WebGL2 target is exactly the
  case we cannot currently reach.

## Fork versus patch, if we prototype first

No standing fork is needed to build the prototype. `vello_hybrid` lives
in the `linebender/vello` monorepo, so the working posture is the same
one this workspace already uses for the genet family: a branch on a
GitHub fork of the monorepo carrying the append commit(s), consumed via

```toml
[patch.crates-io]
vello_hybrid = { git = "https://github.com/mark-ik/vello", branch = "mark-ik/hybrid-scene-append" }
```

in whichever workspace hosts the hybrid-backend experiment. The delta
rebases forward on upstream; if the PR lands, the patch entry deletes
itself. A heavy fork (own lineage, like netrender's origin) only enters
the picture if upstream rejects the direction *and* the backend still
needs it, which is a bridge to not build in advance. Note netrender
itself needs no patch at all today: nothing depends on `vello_hybrid`
until the backend seam work actually starts.

## Two things to decide before sending

1. **Issue or discussion.** This is an API-shape request with a design
   consequence for upstream, not a bug. A discussion thread probably
   fits better, with the source findings above as the opening analysis
   and shape (2) as the starting proposal rather than a demand.
2. **Whether to offer the PR.** For shapes (1) and (2) the offer is
   credible and cheap: strip-range and paint-index remapping against
   their own encoding, with `vello::Scene::append` as the semantic
   reference. Shape (3) should be offered as design participation, not
   a patch.
