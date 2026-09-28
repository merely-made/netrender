# Vello sparse backends: compatibility and consumer plan

**Status:** source and registry audit and VB1 dependency gate complete,
2026-09-27. The bounded VB2 CPU-owned image/session slice is implemented and
passes CPU pixels and Hybrid GPU readback. VB2 remains open for patterns.
See the [baseline and implementation receipts](receipts/2026-09-27_sparse_backends/README.md).
No new dependency pin or alternate default has been promoted.

**Purpose:** make CPU and Hybrid useful selectable realizations of NetRender's
existing `Scene`, driven by a real specimen document. Classic stays the shipping
path while each alternate earns an explicit scope. The canonical task checklist
is [feature roadmap, VB0-VB6](2026-05-04_feature_roadmap.md#vello-backend-completion).

## 1. Source audit

Inspected NetRender `9607d16f1907f6c2085648ae96abcaa30d7c3d41`, the cached
fork source at `ca3f40ea182216883cd543c7b9deae991268917c`, and upstream
`a9f11bbe752c391a2e420657d65cbd1fc97f4fae` (commit dated 2026-09-25).
Registry metadata was queried on 2026-09-27. Version numbers below describe
that snapshot, not a rolling dependency recommendation.

| Source | Actual state | Consequence |
| --- | --- | --- |
| NetRender Classic | `netrender-vello` 0.10.1; wgpu 30 | Preserve existing renderer and receipts. |
| NetRender CPU/Hybrid | Both pinned to fork `ca3f40ea`; `vello_common` from the same source | Keep the shared sparse type/resource identity. |
| Published `vello_hybrid` 0.2.0 | Requires wgpu `^29.0.3` | Not a drop-in shared-device dependency for NetRender's wgpu 30. |
| Published `vello_cpu` / `vello_common` | 0.2.0 | Same version number does not prove identity with the fork's Git packages. |
| Published `vello_gpu` 0.1.0 | Description says name reservation; no dependencies or features | Do not install it as the renamed working renderer. |
| Upstream Git at the audited commit | Working package renamed to `vello_gpu`; declares 0.2.0; wgpu 30.0.0 | A plausible source candidate, not compile-tested by this audit. |

Immutable upstream [workspace manifest][upstream-manifest] records Rust 1.89,
Peniko 0.6.1, Kurbo 0.13.1, Skrifa 0.44.0, and Glifo 0.3.0. The existing fork
already has the same declared wgpu/Peniko/Kurbo/Skrifa rows. Source identity,
resolved versions, feature flags, and API changes still need a Cargo gate.
The new package name is documented in the [GPU README][upstream-gpu].
Registry evidence: [GPU reservation][registry-gpu], [Hybrid dependency metadata][registry-hybrid],
[CPU dependency metadata][registry-cpu].

The fork consists of upstream `f6e4999c`, wgpu-30 upgrade `a34adac3`, then
append prototype `ca3f40ea`. Inspecting the latter's `append.rs` establishes:

- `append_scene` consumes the donor; it is not reusable by-reference retention.
- Translation must be aligned to the sparse tile dimensions in both axes;
  arbitrary affine transforms and sub-tile scrolling are not implemented.
- Viewports must match and layers must be balanced. Filter layers and
  non-default-thread clip storage are refused. Image atlas namespaces must agree.
- Upstream's audited GPU scene and source tree have no public `append_scene`
  counterpart. A package rename does not preserve this local extension.

**Working dependency decision:** retain the current pin for the first resource
adapter slice, now build/readback-tested in VB1. Evaluate the audited upstream
commit separately before any future adoption. A move
must either preserve and re-test the bounded append experiment or explicitly
retire its API/test/capability claim together. Never drop it incidentally in a
dependency bump. No upstream issue or PR is authorized by this plan.

## 2. The actual adapter gaps

See [`vello_backends.rs`](../netrender/src/vello_backends.rs),
[`SceneGlyphRun`](../netrender/src/scene/elements.rs),
[`SceneImage` and `ScenePattern`](../netrender/src/scene/geometry.rs), and
[`Renderer`](../netrender/src/renderer/mod.rs).

| Operation | Current sparse adapter | Required work |
| --- | --- | --- |
| Geometry, gradients, transforms, clips, nested alpha | Lowered; bounded RG2a receipts exist | Preserve semantic anchors and refusal tests. |
| CPU-owned image | Owned `CpuSession` / `HybridSession`; synthetic CPU/GPU acceptance passed | Real consumer capture remains open. Free lowerers still refuse images. |
| Pattern | Typed refusal | Build on the owned image lifecycle; prove repeat/transform semantics separately. |
| Glyph run | Typed refusal; sparse `text` features disabled | Enable text explicitly, supply persistent Resources, map existing shaped glyphs and font variations. |
| Registered fragment | Typed refusal | Resolve registry and generations; design backend-specific reuse and invalidation. |
| Element / backdrop filters | Typed refusal | Admit exact supported operations with painter-order and alpha evidence. |
| External GPU scene image | Classic's staging path exists | VB4b gives sparse GPU a same-device route; CPU requires an explicit CPU source or refuses. |
| Host backend selection | Cargo gates, free lowerers and opt-in owned image sessions | Connect host selection and explicit requested/effective backend reporting. |

Both the pinned source and upstream expose glyph builders requiring mutable
backend `Resources`. The owned image sessions use direct `Arc<Pixmap>` sources
on CPU and session-owned textures on Hybrid, avoiding monotonically growing
image registrations or atlas fragmentation. Hybrid's internal texture bindings
hydrate CPU-owned pixels; they do not implement the external producer import
gate VB4b. The free `Scene -> packet` helpers stay resource-free. Text shaping
remains in the caller/Parley.

`SparseResourceLimits` bounds source count/bytes, derived image count/bytes,
viewport pixels, input complexity and layer depth. Sources persist until
explicit removal; omitted Scene entries do not unregister them. Derived
crop/tint variants survive only while used by the latest admitted frame.
Unchanged frames reuse them; returning to an evicted variant rehydrates it.
Hybrid retains both CPU derived pixels and matching GPU payloads, reported
separately by `SparseResourceStats`. These are logical image limits, not a
total-memory or hostile-code sandbox: backend scratch, driver allocation and
host-retained in-flight commands are outside the accounting.

Preflight must become resource-aware: missing keys, invalid fonts/collection
indices, malformed image payloads, non-finite dimensions/transforms, resource
budgets, and unsupported blend/filter modes must have defined outcomes before
mutating the visible target. Existing operation booleans are insufficient to
claim every parameter combination. Upstream documents panic paths for some
operations; this workspace uses `panic = "abort"`, so catch-unwind is not a
fallback strategy. Preserve Classic's existing missing-image policy unless a
separately reviewed common policy changes it.

## 3. First consumer and acceptance

Use the current Mesocosm specimen bench's **Isocosm panel**, implemented at
`isometry/mesocosm/crates/mesocosm-genet/src/app/bench/sim.rs::view`:
world controls, Tick/status text, Step, Play/pause, and individual inspection.
The existing body specimen supplies the image/viewport lifecycle target.
These are existing consumer surfaces, not new simulation rules.

Capture a bounded panel region in initial, selected, and post-step states,
plus specimen image replacement and resize. Store source revisions, viewport,
scale, font/assets, image generations and capture provenance. First inventory
the emitted operations; do not strip unsupported commands from a real capture
and call the result a successful consumer render. Resource-complete capture
may need a Scene/resource bundle in addition to PaintEnvelope. A synthetic
card can develop the adapter, but cannot close consumer acceptance.

The Isocosm review thread has endorsed keeping Isometer and testing the full
interaction loop. Its latest response reports paging integrated at `497e5fe`;
this is coordination context, not a fresh acceptance run here. The sim lane
owns connected body/ecology behavior and per-tick accepted-flow handoff.
Capture and host integration must coordinate with that owner rather than
change the panel or sim concurrently.

The reviewer checked consumer source `ec43e60`: Play advances once per host
frame, and `Session::advance(ticks)` rolls back atomically across its entire
span. Backend comparisons must replay identical logical ticks/actions, not run
Play for equal wall time. Do not replace multi-tick advances with per-tick
calls to obtain measurements; that changes simulation rollback semantics.
The sim panel and body specimen are currently separate views. Passing both
independently does not establish the integrated-scene gate.

## 4. Implementation sequence and done-conditions

**VB1: dependency gate.** Check default, CPU-only feature, Hybrid-only feature,
and combined feature configurations on the existing pin. Run RG2a and focused
CPU/readback/append tests, then evaluate the candidate source against the same
gates before adopting it. Record `cargo tree -d` and the wgpu/Peniko identities
that cross APIs. One physical device must serve both GPU paths. This gate
closes with a tested chosen source and explicit append disposition, not a
manifest-only compatibility claim.

**Completed:** existing `ca3f40ea` pin retained; four feature configurations,
focused append/readback, full library, RG2a and Classic image gates recorded
in the receipt. Shared wgpu/Peniko identities are unchanged. The audited
upstream candidate was not built or adopted.

**VB2: image resources and owned sessions.** Start on the existing pin with
ordinary CPU-owned RGBA scene images and a bounded resource cache. Preserve
source identity, replacement, removal, UV crop/clamp, nearest/bilinear sampling,
alpha, tint and transformed device-space clips. Unsupported combinations stay
typed refusals. Add patterns after the underlying image semantics pass. Close
with CPU pixels and GPU readback, stale-generation negative controls, and
bounded cache counts across replacement/removal cycles. No per-frame GPU
readback is permitted as an implicit CPU fallback.

**Image subgate completed:** three CPU-only tests and two Hybrid-only tests
cover sampling, crop/clamp, tint/alpha, transforms/clips/layers, painter order,
replacement/removal, cache reuse/budgets and refusal preserving the target.
These are synthetic fixtures, not panel acceptance. Admitted UVs must be
finite, ordered and within [0,1]; clamp crops round to whole source pixels,
and empty rounded crops refuse. Parameters outside the documented subset
return typed errors. Patterns remain the next VB2 subgate.

**VB3: shaped text.** Enable `text` deliberately for each sparse dependency.
Pass existing glyph positions and font data into the backend builders; preserve
collection index, variation settings, transforms and clips. Close with real
panel text, a variable-font fixture, a fallback-font fixture, clear handling
of missing/invalid resources, and bounded resource lifetime. Use semantic
pixel regions/tolerances, not universal byte identity between rasterizers.
Color/bitmap glyph cases remain explicit subcapabilities until tested.

**VB4a: host selection and real panel.** Expose requested and effective backend,
compiled availability, and diagnostic refusal. Default stays Classic. Begin
with explicit selection at session creation; changing it recreates owned
backend resources from authoritative Scene/assets. An automatic fallback must
be a host policy and reported, never a silent per-op mix. CPU pixel rendering
must run without requesting a GPU device; CPU presentation through a GPU host
is a different receipt. Close with actual selection/step/resize/resource-update
behavior and the real capture corpus. A panel capture that excludes the spatial
viewport must say so. Do not label this full browser parity or live specimen
image acceptance; the latter also requires VB4b.

**VB4b: external specimen image.** Connect the live Isometer output to Hybrid
on the host's existing device, preserving producer revision, replacement,
resize/removal, painter order and enclosing clips/opacity. Verify premultiplied
alpha and source/target color formats using actual tenant pixels. Prove an
unchanged generation avoids redundant staging and changed pixels become visible.
CPU has a separately declared CPU-owned snapshot/source mode or a typed refusal
for GPU-only images; it must not introduce an implicit frame readback. Close
with a real live specimen GPU receipt and an honest CPU-mode outcome. This is
the explicit prerequisite for any full specimen acceptance claim in VB4/VB6.

**VB5: retention and effects.** Treat semantic fragment support separately from
reuse speed. Retaining source ops and re-lowering may establish correctness,
but must be reported and measured as re-lowering. Compare that baseline with
any sparse packet/texture cache on non-tile-aligned pan, scale, clip, nested
opacity and resource mutation. Preserve generation/namespace invalidation.
Admit filters individually using the existing image execution layer where
appropriate; prove backdrop prefix/order and element isolation. The existing
RG2 downstream blur receipt is not Scene filter parity. Close each declared
subcapability with semantic and invalidation evidence before marking it true.

**VB6: integrated consumer budget.** With the sim owner, measure accepted sim
advance, projection, document updates, CPU preprocessing, uploads, tenant GPU
work, NetRender composition and input-to-present behavior in one scene. Report
initial and steady frames, medians/tails, viewport/device/source identities,
cache residency and quality settings. Isometer remains the spatial producer.
Use existing configurable residency policies; record what happens when a
budget is exceeded. Changing appearance/detail must leave authoritative sim
outcomes unchanged. A renderer completion timer alone cannot close this gate.

## 5. Ownership and useful outcomes

NetRender owns adapters, resource lifetime, scene admission and composition.
The host owns device lifetime, backend policy, scheduling and recovery. Genet
owns document paint/layout/input. Isometer owns spatial presentation and
producer depth. Isocosm owns simulation truth; the product maps accepted
records to appearance and offers controls. Renderling replacement, voxel
physiology, and simulation scheduling are outside this plan.

CPU can support headless documents, previews and comparison diagnostics. GPU
sparse rendering offers a different CPU/GPU work balance and broader upstream
hardware support. Neither establishes better Isocosm performance without VB6.
CPU Vello does not make the spatial tenant, GPU filters or window presentation
software-only; a schematic or sprite fallback is a separate consumer choice.

Configurable flow marks, material colouring, cutaways and replay exports are
useful consumers, but take their meaning from accepted simulation records.
Trusted renderer implementations consume bounded data; handing arbitrary mod
code the shared device is not a sandbox. Preserve the existing host-owned
device-loss and validation contract rather than inventing tenant-local recovery.

## 6. Evidence discipline

The initial audit used read-only local source, GitHub immutable source and
crates.io metadata; that audit created no build outputs or dependency checkout.
Runtime evidence from subsequent implementation is recorded separately. Builds reuse the
approved stable `C:\t\cargo-targets\netrender` target (or the repository's
existing approved target). Receipts distinguish source audit, compile, CPU
raster, GPU readback and native interaction. Record successful implementation
gates in the [verification record](2026-05-01_vello_verification_record.md).

[upstream-manifest]: https://github.com/linebender/vello/blob/a9f11bbe752c391a2e420657d65cbd1fc97f4fae/Cargo.toml
[upstream-gpu]: https://github.com/linebender/vello/blob/a9f11bbe752c391a2e420657d65cbd1fc97f4fae/vello_gpu/README.md
[registry-gpu]: https://crates.io/api/v1/crates/vello_gpu/0.1.0
[registry-hybrid]: https://crates.io/api/v1/crates/vello_hybrid/0.2.0/dependencies
[registry-cpu]: https://crates.io/api/v1/crates/vello_cpu/0.2.0/dependencies
