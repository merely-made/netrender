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

## Two things to decide before sending

1. **Issue or discussion.** This is an API-shape request with a design
   consequence for upstream, not a bug. A discussion thread probably
   fits better, with the concrete signature as a starting proposal
   rather than a demand.
2. **Whether to offer the PR.** `vello::Scene::append` already exists,
   so there is a reference implementation in the same workspace. If
   upstream is receptive to the shape, offering to write it is cheap and
   makes the ask concrete.
