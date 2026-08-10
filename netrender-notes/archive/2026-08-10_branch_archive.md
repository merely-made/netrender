# Branch archive (2026-08-10)

`origin` carried thirteen branches and the local clone five. All of the
non-`main` ones were dead: last commits between 2026-03-23 and 2026-05-04,
all predating the vello pivot, none mergeable. They are now **annotated
tags** rather than branches, so `git branch -a` is readable again while
every commit stays reachable and pushed.

Tags are on `origin`. Nothing was discarded.

## Why tags and not deletion

Most of these are the wgpu-backend-for-WebRender line of work, which was
aimed at **servo issue #37149** as an unsolicited contribution: a wgpu
backend that keeps GL alive and can be adopted as a non-hack drop-in.
That work used to live in a separate `webrender-wgpu-upstream` checkout.
That checkout no longer exists on disk, and there is no repository by
that name on GitHub. **These tags are the only surviving copy.** Treat
them accordingly.

## Restoring one

```bash
git checkout -b <name> archive/<name>
```

The tag message on each carries the same summary as the table below.

## The upstream-contribution line

Ordered roughly by how close each got to something offerable.

| Tag | Tip | Last commit | Commits off `main` |
| --- | --- | --- | --- |
| `archive/spirv-shader-pipeline` | `40c0b90ed` | 2026-05-04 | 86 |
| `archive/wgpu-device-renderer-gl-parity` | `ee6262aac` | 2026-04-08 | 74 |
| `archive/wgpu-backend-0.62` | `03a065e1a` | 2026-03-23 | 11061 |
| `archive/wgpu-hal-backend` | `3cbb2ca79` | 2026-04-08 | 84 |
| `archive/wgpu-device-sharing` | `5615909f2` | 2026-04-08 | 79 |
| `archive/wgpu-backend-0.68-minimal` | `1de049155` | 2026-04-08 | 80 |
| `archive/wgpu-backend-0.68-experimental` | `449eb907e` | 2026-04-18 | 140 |

**`spirv-shader-pipeline`** is the one to start from if this is ever
revived. It was the designated offering branch. SPIR-V baked into the
binary via `build.rs` + `include_bytes!`, `wgpu.rs` (~1100 lines)
decomposed into a `wgpu/` module, `GpuShaders` program create/link
working end to end, P5 rendering `ps_clear` through `WgpuDevice`, and
wgpu-hal trait-surface prep. Design constraints recorded at the time:
keep GL alive, gate the public `Compositor` trait on `gl_backend` so
Gecko's downstream impl survives, and make `--features wgpu_backend`
work standalone.

**`wgpu-device-renderer-gl-parity`** holds the strongest evidence:
413/413 reftests passing with backend-aware fuzzy tolerances on Windows,
plus picture-cache tile `dirty_rect` clearing and `resolve_ops` /
picture-cache blits. If the question is ever "did this actually work",
this tag is the answer.

**`wgpu-backend-0.62`** is the widest, on the original WebRender tree
before the history rewrite. High-water mark: Stage 4f, 61/61 GLSL to
WGSL shader translations passing, headless `WgpuDevice` under a
`GpuDevice` skeleton. Its 11061-commit distance from `main` is not
divergence in any meaningful sense; it simply shares no ancestry with
the rewritten history.

The remaining three are narrower slices of the same effort:
`wgpu-hal-backend` (a `WgpuHal` variant, `composite_output_hal`, a
`--wgpu-hal` wrench flag, device-level and pixel-parity tests),
`wgpu-device-sharing` (host-app shared-device API, exposed composite
output texture, smoke tests, a demo), and `wgpu-backend-0.68-minimal`
(the same shared-device API rebased onto 0.68). `wgpu-backend-0.68-experimental`
is the broader 0.68 attempt, including cherry-picked upstream fixes.

## Netrender's own dead ends

| Tag | Tip | Last commit | Commits off `main` |
| --- | --- | --- | --- |
| `archive/idiomatic-wgpu-pipeline` | `86cb23ae8` | 2026-05-04 | 7 |
| `archive/webrender-0.62-pre-phase-d` | `e1c924eba` | 2025-10-02 | 11052 |

**`idiomatic-wgpu-pipeline`** is the pre-vello text stack on netrender's
own WGSL rasterizer: phases 10a.2 through 10b, `swash::Scaler` glyph
rasterization via `RasterContext`, a `FontHandle` / `BoundRaster`
shaped-run API, an opt-in subpixel-AA dual-source pipeline, text in
tiled scenes, and a subpixel glyph atlas with eviction and a
transform-aware policy. Fully superseded by the vello pivot plus
`netrender_text` on parley. Kept because the atlas eviction policy is
the sort of thing worth re-reading rather than re-deriving.

**`webrender-0.62-pre-phase-d`** is the pre-rewrite snapshot: the full
upstream WebRender tree plus netrender work up to the Phase D rollback.
It is the only in-repo copy of the original `webrender_api/`, `wrench/`,
and `wr_glyph_rasterizer/` sources. `PROGRESS.md` used to point at a
`webrender-wgpu-upstream/` side worktree for this; that worktree is gone,
so this tag is the replacement.

## Deleted outright

`refactor/split-files-under-600loc` was fully merged into `main` (zero
commits ahead), so it got no tag.
