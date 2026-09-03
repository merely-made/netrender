# Licenses in this repository

**This repository: MPL-2.0.** netrender began as a fork of Mozilla/Servo's
WebRender and keeps that license, and every file Mark wrote carries Exhibit A
and the SPDX tag `MPL-2.0` in the house header shape, per the
[license posture brief](../mere/design_docs/2026-08-22_license_posture_brief.md)
of 2026-08-22 (mere `design_docs/2026-08-22_license_posture_brief.md`) and its
[sweep plan](../mere/design_docs/mere_docs/implementation_strategy/2026-08-22_license_sweep_plan.md)
P7. The full text is in [`LICENSE`](LICENSE).

This file is the provenance ledger. It is the authority for what the relicense
tool (mere `scripts/relicense_headers.py`) skips: the backtick-quoted paths in
the **Retained licenses** table are never touched. Provenance comes before
license: a file gets Mark's copyright line only if Mark wrote it.

## Servo heritage

netrender's history is WebRender's — 4,968 Mozilla commits under 162 of Mark's,
with the WebRender codebase itself removed during the 2026-04-30
`rename: webrender → netrender` pivot and vello installed as the sole
rasterizer. The MPL-2.0 grant is inherited from that origin; the copyright line
is not, so provenance was determined per file rather than per repository.

**Two source files keep bare Exhibit A with no Merely copyright line:**

| Path | Upstream lineage |
|---|---|
| `netrender/src/renderer/mod.rs` | `webrender/src/renderer.rs` → `renderer/mod.rs` (Kartikaya Gupta, Dzmitry Malyshau, Jamie Nicol) |
| `netrender/src/renderer/init.rs` | `webrender/src/renderer/init.rs` (Nicolas Silva, bug 1690244) |

These two are the only tracked sources whose git ancestry reaches
Mozilla-authored WebRender content, and the only two where `git blame` still
attributes lines to Mozilla addresses. What survives is thin — the header, a
`pub(crate) mod init;`, an enum declaration, blank lines and closing braces —
but the lineage is real and the sweep's rule is that an unclear file is treated
as derived. They therefore carry Exhibit A and the SPDX tag in shape C with
**no copyright line**; no Merely notice is added to code Mark did not write.
Their copyright remains with the Mozilla Foundation and the WebRender authors.

Also carried verbatim from WebRender and **not** owned work, though the header
tool never reaches them (they are not source files in its extension set):

- `.taskcluster.yml` — Mozilla's WebRender CI configuration, last touched
  upstream 2022-12-14, unmodified since;
- `servo-tidy.toml` — Servo's tidy configuration, unmodified since 2018;
- `netrender/res/area-lut.tga` — WebRender's box-shadow area lookup table,
  now unreferenced by any code in the tree (see the Retained licenses note
  about its neighbour).

Everything else in the tree — the other 106 tracked sources across `netrender`,
`netrender_device`, `netrender_text`, `paint_list_api` and `paint_list_render` —
is Mark's own work, authored 2026-04-28 onward, and carries the full shape-C
header with `Copyright 2026 Mark Alan Boykin`. Several of those files sit at
paths WebRender once used (`netrender_device/src/frame.rs`,
`readback.rs`, `core.rs` and their neighbours descend from Mark's own
`webrender/src/device/wgpu/` scaffolding of 2026-04-28, not from Mozilla's
`device/mod.rs`); `git log --follow`'s rename heuristic chains some of them back
to 2018 commits, and that chaining is spurious. `git blame` attributes zero
lines in them to any Mozilla author.

## Retained licenses

Third-party code keeps its own license and its own notices. Nothing here is
relicensed, and nothing here receives a Merely copyright line.

| Path | License | Upstream | Notice files |
|---|---|---|---|
| `netrender/res` | Proggy Clean: Tristan Grimmer's own terms; `area-lut.tga`: MPL-2.0 with WebRender | [servo/webrender](https://github.com/servo/webrender) (`webrender/res/`) | none in-tree — see below |

`netrender/res` holds two binary assets carried through the WebRender fork:
`Proggy.ttf` (the Proggy Clean bitmap font by Tristan Grimmer, which WebRender
vendored under the font's own terms rather than under the MPL) and
`area-lut.tga`. Neither is referenced by any code in the tree today — the
2026-05-01 vello rasterizer plan retired both LUT paths — and WebRender shipped
no notice file beside them. **Open for Mark:** either confirm Proggy Clean's
terms and record them here, or delete the two files, which is the simpler
disposition given that nothing loads them. They are listed here so the tool
skips the directory and so the gap is recorded rather than silently inherited.

No source file in this repository carries a `Copyright`, `Licensed under`,
`Permission is hereby granted`, or `Apache License` line, and no SPDX tag names
anything but MPL-2.0. The discovery grep of the sweep plan's invariant 1, run
unqualified over every tracked file, returns hits only inside `LICENSE` itself.

## Derivatives carrying MPL-2.0 with an upstream notice retained

**None.** No file in this repository carries a foreign copyright notice to
retain, so `--retain-notice` has nothing to do here. The two Servo-heritage
files above are handled the other way round — bare Exhibit A, no notice added —
because WebRender attached no per-file copyright line of its own.

## Exceptions under the fork/vendor criterion

**None.** The brief's §4 test — a crate stays MIT OR Apache-2.0 only when a
third party would need to *modify or vendor* it rather than merely link it —
admits nothing in this repository. All five member crates
(`netrender`, `netrender_device`, `netrender_text`, `paint_list_api`,
`paint_list_render`) declare `license = "MPL-2.0"` and did so before this sweep;
P7 changed no manifest, no `LICENSE`, and no published version.

## How to add a file from elsewhere

1. Do not delete or rewrite the upstream copyright or license notice, ever.
2. Add its path to **Retained licenses** above with its license, upstream URL,
   and where its notice text lives. The tool then skips it automatically.
3. If it is a substantial derivative rather than a verbatim import, the brief's
   rule is MPL-2.0 on the derivative *with the upstream notice retained*;
   record it in that section so the distinction is not lost.
4. If it is WebRender's own code arriving under the MPL already, it takes bare
   Exhibit A in shape C (`--renormalize --bare`) and a row in **Servo
   heritage** — never a Merely copyright line.
5. Never add `license-file` to an owned manifest.
6. Re-run `python ../mere/scripts/relicense_headers.py --repo . --audit` and
   confirm the owned source count moved by exactly what you expected.
