# Rasterizer backend seam — vello_hybrid findings (2026-08-04)

**Status**: Research record. No implementation proposed yet. Follow-on to
[`2026-05-01_vello_rasterizer_plan.md`](2026-05-01_vello_rasterizer_plan.md)
(hereafter "the rasterizer plan"), which remains the canonical live
architecture.

> **Re-verified 2026-08-10 against `vello_hybrid` 0.2.0**, which shipped
> 2026-08-07, three days after this note was written. §4's blocker holds:
> still no `Scene::append`, still no public `CommandRecorder`, still
> nothing retained or incremental. 0.2.0 did add a first-class
> capability-probe surface (`Probe`, `ProbeResult`, `ProbeFeature`, and
> the `WebGl*` probe types), which covers by API the question §3
> answered by hand.
>
> §4 is now drafted as something to send:
> [`2026-08-10_vello_hybrid_upstream_ask.md`](2026-08-10_vello_hybrid_upstream_ask.md).
> Not filed. Version numbers below refer to 0.1.0 unless stated.
>
> **Deepened 2026-08-12 by a source read** (`scene.rs` on vello main):
> hybrid generates sparse strips *at record time* against the active
> transform, retaining strip ranges + encoded paints, not source paths.
> So the missing `append` is architectural, not an omission: no-transform
> append is mechanical-ish, integer-translate append is a strip-shift,
> and full-affine append would require retaining paths. The ask now
> presents those three shapes; the "two retention models" framing in §4
> stands confirmed from the inside.

Question that prompted it: could `vello_hybrid` serve as a second
rasterizer behind netrender's seam, to reach WebGL2-only browsers that
mainline vello cannot?

---

## 1. What changed since the 2026-05-01 evaluation

The rasterizer plan evaluated `vello_hybrid` and rejected it: workspace-
internal `v0.0.7`, "not yet suitable for production", no multi-region /
multi-target / scissor / partial updates.

Two of those premises moved:

- **Published.** `vello_hybrid` 0.1.0 shipped 2026-07-29 on crates.io.
  It depends on `wgpu ^29.0.3`, matching this workspace's pin exactly.
- **Caller-supplied encoder.** `Renderer::render` now takes the caller's
  `CommandEncoder`, target `TextureView`, and `RenderSize`. The
  rasterizer plan named this the one ergonomic win it wanted from the
  sparse-strips family.

A third premise is new: the browser-reach argument did not exist in May.
Mainline vello is 20-of-20 compute shaders with no downlevel handling, so
it is WebGPU-only. Global reach as of 2026-08-04 is 83.63% for WebGPU
against 94.67% for WebGL2, and the gap is Firefox categorically (disabled
by default through v156), Safari desktop partial, and pre-150 Chrome for
Android. For a Rust/Linux-leaning audience the practical gap is wider
than the global average implies.

## 2. §10 does not forbid this

The rasterizer plan's §10 ("the two backends trap") argues against
keeping the parent plan's hand-authored batched WGSL alive beside vello,
and closes by deleting those shaders. Its reasons transfer unevenly to a
second *dependency* that shares vello's API family:

| §10 reason | Transfers to `vello_hybrid`? |
| --- | --- |
| Every primitive change lands twice | Weakly. The cost it names is authoring a WGSL shader, pipeline factory, and batch slot per family. Against hybrid it is a second lowering over the same peniko/kurbo types, four call sites. |
| Plan-time savings need vello to be *the* path | No. Entirely about shader code we author. |
| Color contracts diverge | Open question, not a known divergence. Both consume `peniko::Color`; whether their storage and blend conventions agree is unmeasured. See §5. |
| Test matrix doubles | **Yes, intact, and it is the real price.** See §4. |

§10 also already blesses a trait for "testability and option value" and
keeps it as a documented escape hatch. Phase 7' dropped the §2.2
`Rasterizer` trait for want of a consumer, not on principle.

Kinship is at the API layer only: shared `peniko` and `kurbo` types and a
near-identical `Scene` vocabulary, which is what collapses reasons 1 and
2. The rasterizers are unrelated — vello is `vello_encoding` plus
compute, hybrid is `vello_common` sparse strips plus fragment, and
`vello_common`'s own docs state that vello does not use it.

## 3. Capability probe receipt

Scratchpad probe, RTX 4060, under `Limits::downlevel_webgl2_defaults()`
(storage buffers and compute zeroed, max texture 2048). Run twice: once
on Vulkan, once forced onto wgpu's GL backend, which exercises naga's
WGSL-to-GLSL emission on a real OpenGL context. Identical results.

Passing, zero validation errors: fills, strokes, filled path, linear
gradient, clip layer, all six `SceneBlendMode` mixes under `SrcOver`,
opacity layer, single-primitive blur filter layer, glyphs (via `glifo`,
atlas caching off).

Panicking: mask layers (`unimplemented!`, `scene.rs:734`), as documented
upstream. Irrelevant here — netrender never sends a mask to the
rasterizer. Masks, backdrop filters, and color matrices are netrender's
own fragment-shader wgpu passes.

This retires the *capability* risk for the nine `SceneOp` variants
netrender actually lowers. It does not retire §4.

## 4. The blocking finding: two retention models

`vello_hybrid` has **no `Scene::append`**. No public method on its
`Scene` accepts another `Scene`; the `CommandRecorder` holding recorded
draws is `pub(crate)`.

Phase 7' — the Masonry pattern, the rasterizer plan's "architectural
heart" — caches a `vello::Scene` per tile and composes them into a master
scene for one `render_to_texture`. That pattern has no hybrid equivalent.

Hybrid does render scissored regions with `LoadOp::Load` internally for
its glyph atlas ("Don't clear entire texture, just the scissor region").
The capability exists; it is not exposed on the main target path.

Three routes:

1. **Re-record every tile per frame into one master scene.** Discard.
   Loses picture caching, and costs more on hybrid than on vello because
   hybrid flattens paths on the CPU — this re-pays the expensive half of
   its design every frame.
2. **Cache rasterized tile textures instead of scenes.** Render each
   dirty tile to its own texture and composite through netrender's
   existing external-texture path; clean tiles cost nothing. Fits
   netrender's shape, and hybrid's per-render `TextureBindings` model
   suits it. Costs N render passes for dirty tiles plus texture memory.
3. **Ask upstream** for scene composition or scissored main-target
   rendering. Architecturally close to what hybrid already does for
   atlases, so this is API surface rather than a redesign. 0.1.0 is the
   right moment to ask.

**Consequence for any seam design:** this is not "a second lowering
behind a trait." It is a second *retention model*. vello retains lowered
scenes; hybrid would retain textures or nothing. A backend seam must be
wide enough that backends legitimately store different kinds of thing,
and tile-invalidation receipts would then be needed on both paths. That
is §10's surviving reason in its most expensive form.

## 5. Present coupling to unwind first

Regardless of which route wins, netrender today stores the lowered type
in shared retained state. A proper seam moves these behind backend-owned
state, leaving `netrender::Scene`, tile invalidation, the filter and
render-task graph, external-texture identity, and presentation shared and
backend-blind.

- `SurfaceTileState.tile_scenes: HashMap<TileCoord, vello::Scene>`
  (`renderer/mod.rs:99`) — retained state holds the lowered type.
- `render_vello_inner` carries that map in its signature
  (`renderer/mod.rs:294`).
- **External-texture ownership is a model mismatch, not a rename.**
  netrender calls `rast.register_texture(key, texture)` from four sites,
  including inside the filter chain, and depends on vello *retaining*
  that registration across render calls: a filter pass mints a blur
  result, registers it, and a later scene references it by `ImageKey`.
  Hybrid has no registry; it takes `TextureBindings` per `render` call.
  A seam must therefore express "make this texture addressable under
  this key for the frame" and let each backend satisfy it from its own
  retention model.
- **`ColorLoad::Load` is unsupported for a vello-specific reason** —
  vello always overwrites the whole target. If hybrid differs, that is
  rasterizer semantics leaking through netrender's public API, and it
  should become a backend capability rather than a fixed comment.
- Timing spans are `vello_render` and `master_compose` (documented as
  "building the master `vello::Scene`"). Both want backend-neutral
  names, and receipts should record the selected rasterizer and its
  version.
- Public API carries the name too: `render_vello`, `enable_vello`,
  `unregister_image_vello`. Renaming is breaking for genet and mesocosm
  both, so it wants one deliberate pass rather than drift.

## 6. Open questions

- **Colour parity.** Do vello and hybrid agree closely enough on stored
  colour, premultiplication, and gradient interpolation for goldens to
  be shared, or does each path need its own? Unmeasured. This decides
  how badly §10's test-matrix reason bites.
- **Route 2 cost.** Per-tile render-to-texture against the current
  single master render, measured on a real scene.
- **Upstream appetite** for exposing scene composition or scissored
  main-target rendering.
- **Browser receipt.** Everything above is native evidence. Browser
  support is admitted by a headed run, never by `cargo check` or a
  native GL backend. ANGLE-backed WebGL2 validates more strictly than a
  desktop GL driver.

## 7. Posture

`vello_hybrid` is a **credible future second backend and not a pending
migration.** Mainline vello remains the sole rasterizer. Reopen this when
a browser target is actually being shipped, and gate adoption on §6's
colour-parity answer plus one of route 2 or route 3 in §4.

Nothing here blocks or redirects voxel or games-wing work. The games
lane's world passes are ordinary fragment pipelines that already fit
WebGL2-class limits (mesocosm landscape doc §8.8) and never route
through the 2D rasterizer.
