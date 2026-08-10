# netrender notes index

This file is the short index to the surviving plans in
`netrender-notes/`. The repo is post-cleanup: vello is the sole
rasterizer on `main`, the upstream WebRender codebase (`webrender_api/`,
`wrench/`, `wr_glyph_rasterizer/`, etc.) has been removed. To spelunk
through the original implementation, check out the
`archive/webrender-0.62-pre-phase-d` tag. (This used to say the code lived
on a `webrender-wgpu-upstream/` side worktree. That checkout is gone; see
[`archive/2026-08-10_branch_archive.md`](archive/2026-08-10_branch_archive.md).)

## Current canonical plans

Audited 2026-08-10. Four files, each with one job, plus a research
record at the end.

- [`2026-05-04_feature_roadmap.md`](2026-05-04_feature_roadmap.md)
  — **the only live checklist.** Phase R plus Phases A–G, every entry
  with a trigger and a done condition. Start here to answer "what is
  left". As of the audit, the answer is small: **D3** (native-compositor
  handoff, netrender side complete, genet adapter 5.5 outstanding and
  out-of-repo) and **R9** (linear-light blending, upstream-blocked on
  vello's compute path, R9-canary will fire when it clears). Everything
  else in Phases 0.5'–12b' has shipped with receipts.

- [`2026-05-01_vello_rasterizer_plan.md`](2026-05-01_vello_rasterizer_plan.md)
  — **the live architecture.** The vello pivot, adopted and delivered.
  Phase 7' (Masonry pattern tile cache) is its architectural heart. §12
  is the phase mapping and the accurate status table.

- [`2026-05-01_vello_verification_record.md`](2026-05-01_vello_verification_record.md)
  — **the evidence.** Split out of the plan on 2026-08-10, where it was
  62% of the file. 34 entries, 32 CLEARED, one per spike or capability.
  Section numbers unchanged, so `§11.x` still resolves. This is the
  append target when a roadmap item lands.

- [`2026-04-30_netrender_design_plan.md`](2026-04-30_netrender_design_plan.md)
  — **axioms and crate rationale only.** Partly superseded: §3 axioms
  and §4 crate structure still hold and are cited by number everywhere,
  but §5's phase plan is replaced by the vello plan's §12, §7's open
  questions are all closed or moot (one of them, the "no `hit_test()`
  entry point" rule, was reversed in the code and is now documented as
  such), and §9's time estimate is dead. Do not read it for direction.

- [`2026-08-04_rasterizer_backend_seam.md`](2026-08-04_rasterizer_backend_seam.md)
  — research record, no implementation proposed. Revisits the vello
  plan's §10 and its 2026-05-01 `vello_hybrid` rejection now that
  hybrid has published 0.1.0 on `wgpu ^29.0.3`. Capability probe
  passes every op netrender lowers, under WebGL2-class limits, on
  wgpu's GL backend. Blocker is that hybrid has no `Scene::append`,
  so Phase 7''s Masonry tile cache has no equivalent and a seam would
  have to tolerate two *retention models*. Also inventories the
  present `vello::Scene` coupling in shared retained state. Posture:
  credible future second backend, not a pending migration.

A running list of post-pivot findings used to sit in this section. It
duplicated the verification record entry by entry and went stale between
updates, so it is gone. The record is the list.

**Workspace state, 2026-08-10:** 271 tests passing, 1 ignored, 0 failed
across 57 suites, on Windows. `cargo fmt --all --check` clean.

## Host verification

- **2026-07-20 macOS / Metal:** the full workspace suite passes with
  `WGPU_BACKEND=metal`, including Apple system fonts, Apple Color Emoji,
  backdrop filters, registered textures, and compositor plumbing. The pass
  added stable Metal goldens for the two rotated Phase 3 scenes, bounded the
  small cross-submission AA variance in `compose_into`, and fixed offscreen
  filter renders incorrectly sharing retained frame tile-cache state. The
  `demo_card_grid` example also renders successfully through Metal.

## Active follow-up plans (small scope)

- [`2026-05-04_feature_roadmap.md`](2026-05-04_feature_roadmap.md)
  — Phase R (open refinements / wart fixes — was §11.99 of the
  rasterizer plan) + Phases A–G (new capability: diagnostics
  first, then consumer-pull-imminent, then SceneOp expansions,
  then architecturally-significant, then companion lanes).
- [`2026-05-05_compositor_handoff_path_b_prime.md`](2026-05-05_compositor_handoff_path_b_prime.md)
  — axiom-14 native-compositor handoff via path (b′). Sub-phases
  5.1–5.4 shipped on netrender side (commit `9447a852b`); 5.5
  genet adapter pending in separate workspace. Roadmap entry:
  D3.
- [`2026-05-06_webgl_over_wgpu_plan.md`](2026-05-06_webgl_over_wgpu_plan.md)
  — WebGL-over-wgpu companion lane. G0–G6 sequence, gated on
  Genet/Pelt consumer pull. Roadmap entry: G.
- [`wasm-portability-checklist.md`](wasm-portability-checklist.md)
  — note: this is for the WebRender wgpu-backend work (separate project,
  now the `archive/*` tags), retained for reference. A netrender-specific
  portability list will be authored when F2 (wasm) triggers.

## Archived branches

All non-`main` branches were retired to `archive/*` tags on 2026-08-10.
Index and restore instructions:
[`archive/2026-08-10_branch_archive.md`](archive/2026-08-10_branch_archive.md).

## Historical / superseded — archived

The plans below predate the vello pivot or have collapsed into other
docs. They describe approaches that are no longer the path forward
or work that has been completed and rolled into the canonical plans.
All have been moved under [`archive/`](archive/) — kept for
historical context, not for guidance.

**Activated and folded:**

- [`archive/2026-05-05_deferred_phases.md`](archive/2026-05-05_deferred_phases.md)
  — was the holding pen for three architecturally-significant
  deferrals (12c' backdrop filter, 13' compositor handoff,
  linear-light blending). All three activated 2026-05-05; canonical
  entries now live on the roadmap as D1, D3, R9. Doc retained as
  the activation history record.

**Pre-vello-pivot (the WebRender wgpu-backend lane):**

- `archive/2026-04-28_idiomatic_wgsl_pipeline_plan.md` — the
  idiomatic-wgpu-pipeline branch's approach (authored WGSL only, no GL,
  no SPIR-V intermediate). Was the active plan before the vello pivot;
  the code is preserved at the `archive/idiomatic-wgpu-pipeline` tag.
- `archive/2026-04-28_renderer_body_wgpu_adapter_plan.md` — `WgpuDevice`
  adapter early-stage planning. Subsumed by netrender_device's
  current shape.
- `archive/2026-04-29_pipeline_first_migration_plan.md` — typed-pipeline
  migration, batch-builder discussion. Pre-cleanup.
- `archive/2026-04-30_phase_d_rollback_to_skeleton.md` — record of the
  rollback that preceded the netrender split.
- `archive/2026-04-30_servo_wgpu_integration_assessment.md` — pre-fork
  servo-integration assessment.
- `archive/2026-04-08_live_full_reftest_confirmation.md` — last
  GL/wrench reftest confirmation before the fork (412/412 passing).
  Now load-bearing prior art for the WebGL-over-wgpu plan §3.1.
- `archive/2026-04-18_spirv_shader_pipeline_plan.md` — dead direction.
- `archive/2026-04-18_upstream_cherry_pick_plan.md` — superseded by the
  fork.
- `archive/2026-04-21_spirv_pipeline_reset_execution.md` — superseded.
- `archive/2026-04-22_upstream_cherry_pick_reevaluation.md` —
  superseded.
- `archive/2026-04-24_tile_with_spacing_validation_error.md` —
  historical bug diagnostic.
- `archive/2026-04-26_track3_legacy_assembly_isolation_lane.md` —
  superseded.
- `archive/2026-04-27_dual_servo_parity_plan.md` — superseded by the
  fork.
- `archive/2026-04-28_session_brief.md` — historical session note.
- `archive/2026-03-01_webrender_wgpu_renderer_implementation_plan.md` —
  the original convergence history, no longer canonical.

**Pre-vello-pivot small-scope plans (WebRender wgpu-backend lane,
code no longer in this repo):**

- `archive/draw_context_plan.md` — `WgpuDrawContext` + encoder
  batching plan against `webrender/src/device/wgpu_device.rs` and
  `webrender/src/renderer/mod.rs`. WebRender lane.
- `archive/typed_pipeline_metadata_plan.md` — flat
  `WgpuShaderVariant` enum to replace `(name, config)` string-tuple
  pipeline keys. WebRender lane.
- `archive/texture_cache_cleanup_plan.md` — WebRender wgpu-backend
  texture subsystem cleanup (`WgpuFrameDataTextures`, dither,
  unsafe-byte-slice utility, mali workaround). WebRender lane.
- `archive/servo_wgpu_integration.md` — Servo + WebRender
  wgpu-backend integration guide (DPR=1/2 confirmed 2026-04-02).
  Pre-vello-pivot. The active netrender-side Servo integration
  story now lives in the path (b′) plan and the WebGL-over-wgpu
  plan; this guide documents the historical lane.

## Local-only

- `archive/` — dated progress snapshots and older branch-shape notes,
  kept for historical traceability.
- `logs/` — local-only, gitignored except for its `.gitignore`. Only
  retain artifacts supporting an active note or unresolved diagnostic.
