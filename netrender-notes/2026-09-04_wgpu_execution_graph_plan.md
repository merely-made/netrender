# Netrender wgpu execution graph plan

**Date:** 2026-09-04

**Status:** RG0 and RG1 delivered; repeated-plan measurement defers RG5;
RG2a is next

**Prior art:**

- [`vk-graph`](https://github.com/attackgoat/vk-graph) for compiled GPU work;
- [`AnyRender`](https://github.com/DioxusLabs/anyrender) for the semantic
  paint/backend seam above it.

**Owns:** frame-local GPU dependency planning in Netrender

**Does not own:** scene meaning, product state, physical GPU barriers,
completion policy or fences, or device creation

## Decision

Evolve Netrender's existing `RenderGraph` into a validated, inspectable wgpu
execution graph. Keep one `WgpuHandles` instance as the physical device and
queue authority. The graph owns logical work order and the placement of its
submission boundaries. Let wgpu own physical barriers, resource transitions,
backend choice, and command submission validity. The host declares and
observes completion boundaries through wgpu's completion primitives.

`vk-graph` is design prior art, not a dependency. Its useful contribution is
the separation between graph construction, typed resource handles, explicit
resource access, finalization into an execution plan, and reusable prepared
command streams. Its Vulkan device, synchronization, pools, and submission
types cannot sit underneath or beside Netrender without creating a second GPU
authority and losing wgpu portability.

AnyRender is also design prior art, not a dependency. Its useful contribution
is a common semantic paint vocabulary with separate Classic Vello, Vello
Hybrid, and Vello CPU adapters. Netrender already owns the stronger
authoritative model in `Scene`, including retained fragments, registered
resources, external textures, hit testing, and consumer capture. Do not add an
AnyRender-shaped second scene authority. Borrow the adapter contract: one
scene meaning enters every backend, and each backend either lowers it
faithfully or returns a typed admission error.

The current graph stays in `netrender` while its only real operations are
Netrender filters and raster/composite work. A move into `netrender_device`, or
into a sibling crate, requires a second independent execution producer to
insert real work through the same public graph contract. A second renderer is
enough evidence for a render-only executor. A resident compute producer such
as Conatus/CubeCL writing a versioned buffer is stronger evidence for the
broader unified-GPU-graph ambition. Sharing a device alone is not that proof.

## Why this is a continuation, not a new direction

The existing architecture already says:

- Netrender owns the render-task topology.
- Vello rasterizes into graph-allocated targets rather than owning the graph.
- Filter tasks consume raster outputs.
- `WgpuHandles` gives Netrender and tenant renderers one device and queue.
- Mesocosm's candidate render lane places its world pass between Vello layers
  and keeps Netrender as frame owner.

The implementation is the deliberately small Phase 6 first cut:

- `TaskId = u64` names output textures.
- `Task.inputs` supplies dependency edges.
- Kahn sorting orders one-pass callbacks.
- Every task creates a new 2D texture with render-attachment,
  texture-binding, and copy-source usage.
- Every graph execution creates one command encoder and submits it once.
- Filter, clip-mask, box-shadow, and backdrop-filter paths consume it.

The historical 2026-09-04 baseline receipt was:

```text
cargo test -p netrender --test p6_render_graph -j 1
2 passed; 0 failed
```

After RG1 retired the compatibility API, its replacement receipt is:

```text
cargo test -p netrender --lib render_graph_tests -j 1
8 passed; 0 failed
```

RG0 landed in `fa5526051`. Its focused receipt is recorded in the Vello
verification record §11.38.

## Probe findings

### Correctness gaps in the first cut

These are admission work, rather than optional optimization:

1. Task IDs are collected into a `HashMap`; duplicate IDs replace earlier
   tasks without an error.
2. An unknown input is omitted by `filter_map`, so a malformed task can encode
   with fewer inputs than it declared.
3. A cycle produces an incomplete sorted list and silently leaves tasks
   unexecuted.
4. The stated deterministic tie order is not guaranteed because the initial
   ready queue is populated through `HashMap` iteration.
5. Graph construction, validation, allocation, encoding, and submission are
   fused inside `execute`, leaving no plan to inspect or test independently.
6. Every task is assumed to produce one newly allocated texture and encode
   exactly one render pass. Buffers, imported output targets, compute/copy
   commands, multiple outputs, and side-effect-only tasks have no vocabulary.
7. Every live task executes, even when the caller needs only one output.

### Existing execution boundaries

The three Vello realizations cannot honestly be represented as identical
callbacks:

| Realization | Current execution shape | Graph treatment |
| --- | --- | --- |
| Classic | Vello creates and submits its own GPU work | explicit opaque submission step |
| Hybrid | accepts Netrender's device, queue, and command encoder | encoder-participating task |
| CPU | produces host pixels | out-of-band host producer, followed by a ready upload/import step |

Netrender's filter callbacks and a future Mesocosm march pass can participate
in a caller-owned encoder. `compose_external_texture` currently creates and
submits another encoder; it is a good first function to split into
"encode into this encoder" and convenience "encode + submit" forms.

One logical graph therefore does not imply one command encoder or one queue
submission. Finalization partitions ordered work into encoder batches around
opaque submission boundaries. Queue order supplies the physical sequencing;
the graph makes that order deliberate and visible. An opaque submission does
not imply a CPU wait; only an explicit poll, map, or other completion fence
does.

CPU rasterization is not a graph task in the first implementation. RG2 may run
it synchronously as a bounded proof, but the graph receives only ready pixels
through an upload/import operation. Worker scheduling, deadlines, stale-frame
fallback, and CPU fences require a real host scheduler and stay outside this
graph until one exists.

### Semantic raster boundary

AnyRender's [`PaintScene`](https://github.com/DioxusLabs/anyrender/blob/562c03ea6a657f158e5ac7601450f3194e110fa1/crates/anyrender/src/lib.rs)
and [recording scene](https://github.com/DioxusLabs/anyrender/blob/562c03ea6a657f158e5ac7601450f3194e110fa1/crates/anyrender/src/recording.rs)
clarify the layer immediately above the execution graph. The useful flow for
Netrender is:

```text
PaintList / producer
        -> authoritative Netrender Scene
        -> backend admission and lowering
        -> backend-native packet plus execution declaration
        -> compiled execution graph
        -> wgpu device and queue
```

The stages have different authority:

| Stage | Owns | Must not decide |
| --- | --- | --- |
| `Scene` | painter order, layers, resources, effects, and retained identity | Vello realization or command submission |
| backend adapter | faithful lowering and explicit capability admission | scene meaning or device creation |
| raster execution declaration | whether work joins an encoder, submits opaquely, or returns CPU pixels | graph scheduling |
| execution graph | resources, dependencies, liveness, batches, and boundaries | drawing semantics or physical barriers |

`BackendCapabilities` and `BackendAdmissionError` are the beginning of this
contract. RG2 may add a small internal `RasterExecution`-shaped enum for the
three execution forms. It does not need a new public drawing trait. If a
streaming paint sink becomes useful later, `Scene` must be one implementation
and a second real producer must justify making that seam public.

AnyRender also supplies three useful cautions, verified against source commit
[`562c03ea`](https://github.com/DioxusLabs/anyrender/tree/562c03ea6a657f158e5ac7601450f3194e110fa1):

- its [`WGPUContext`](https://github.com/DioxusLabs/anyrender/blob/562c03ea6a657f158e5ac7601450f3194e110fa1/crates/wgpu_context/src/lib.rs)
  owns a device pool rather than adopting Netrender's `WgpuHandles`;
- custom resources cross the core trait as `Box<dyn Any>`, leaving adapters to
  downcast at runtime;
- its [Classic adapter](https://github.com/DioxusLabs/anyrender/blob/562c03ea6a657f158e5ac7601450f3194e110fa1/crates/anyrender_vello/src/scene.rs)
  can turn unsupported custom or missing resources transparent and currently
  ignores layer filters.

Those are deliberate non-patterns here. Netrender keeps one device authority,
typed graph-local resources, and explicit refusals. AnyRender's current
[compatibility table](https://github.com/DioxusLabs/anyrender/blob/562c03ea6a657f158e5ac7601450f3194e110fa1/README.md)
also ends at wgpu 29 while Netrender is on wgpu 30, reinforcing the decision
to learn from the shape without taking the dependency.

### Semantic effect graphs are not execution graphs

AnyRender's declarative [`Filter`](https://github.com/DioxusLabs/anyrender/blob/562c03ea6a657f158e5ac7601450f3194e110fa1/crates/anyrender/src/filters.rs)
is useful prior art for a future Netrender effect graph. It names semantic
inputs such as source graphic, source alpha, backdrop, fill, and stroke, and
tracks the output-bounds expansion caused by effects. Those are content facts,
not GPU resource accesses.

The present `Vec<SceneFilter>` remains the correct representation for linear
CSS filter chains. A semantic DAG is triggered only when a real multi-input
effect such as blend, composite, or displacement enters Netrender's scene
contract. That DAG must:

- use typed semantic source and effect IDs;
- reject missing references and cycles;
- expose one explicit selected output;
- propagate conservative transformed bounds before allocation;
- lower into execution-graph image nodes and tasks without exposing its
  semantic node types to the executor.

AnyRender's source does not currently expose a graph validation pass, so it is
vocabulary prior art rather than a construction or correctness model. RG0 and
RG1's validation rules remain the foundation.

### Existing tenant evidence

Paredros already boots Renderling and Netrender through one `WgpuHandles`,
renders the room into a tenant-owned texture, and composites that texture below
Netrender chrome. Mesocosm has the same shared-handle and external-texture
shape in several headed probes. They prove device tenancy and zero-copy texture
composition. They do not yet prove a shared execution graph because neither
tenant contributes a graph operation.

## Authority boundaries

Keep these graphs distinct even when one compiles into another:

| Layer | Lifetime | Authority |
| --- | --- | --- |
| Netrender `Scene` / paint list | retained or captured | painter order, layers, resources, and retained identity |
| Semantic effect description | retained with the scene | effect inputs, selected output, and affected bounds |
| Rasterizer adapter / native packet | fragment generation or frame | backend-specific realization and capability admission |
| Mere projection/content graphs | durable application state | content relationships and projection choices |
| Execution graph | one frame or workload | logical GPU dependencies, lifetimes, and execution boundaries |
| Frame host | frame and device lifetime | inter-tenant submission order, completion and presentation policy, and shared-device recovery |
| wgpu | device lifetime | resource validity, barriers, backend synchronization, and queue submission |
| Mesocosm simulation | saved/replayed world | ecology, matter, causality, and deterministic outcomes |

The execution graph may project durable state into work. It never becomes the
record of that state. A completed GPU pass cannot author a Mesocosm ecological
fact or mutate a Mere content graph by implication.

## Working vocabulary

Names are provisional until RG1 proves them in code.

```rust
GraphId
ImageNode
BufferNode
TaskNode
TransientImageDesc
ImageAccess
BufferAccess
ExecutionPlan
ExecutionStep
EncoderBatch
SubmissionBoundary
ExecutionReport
GraphBuildError
GraphExecutionError
```

RG2 may additionally need an internal `RasterExecution` with three explicit
forms: encoder participation, opaque submission, and CPU pixels. That name and
shape remain provisional until the conformance work proves what data crosses
the boundary. `BackendCapabilities` and `BackendAdmissionError` already name
the semantic side and should be extended instead of duplicated.

`ImageNode` and later `BufferNode` are abstract logical resources, never
physical wgpu allocations. Every graph owns a process-local monotonic
`GraphId`; node handles carry it and foreign-handle checks remain active in
release builds. Imported images use the same `ImageNode` type as transients,
but bind a caller-owned physical texture through a per-execution table. A
stable prepared-template parameter is a different future type,
`TemplateImageSlot`, introduced only in RG4.

Future `BufferNode` support must distinguish a logical value from its physical
allocation. Each admitted write produces a new logical buffer version even if
the executor reuses the same wgpu buffer underneath. An imported resident
buffer binding may carry the producer's revision/read epoch as an opaque
equality token. A consumer declares the exact token it expects and execution
refuses a mismatched binding. The graph does not compare, order, or advance
those tokens, mint that durable stamp, or depend directly on Conatus's
`ChunkStamp`; Conatus or the product record remains its authority.

Tasks declare accesses rather than only predecessor IDs. The initial portable
access vocabulary should match decisions Netrender can actually validate:

- sampled read;
- color-attachment read/write with load and store policy;
- storage read/write;
- copy source/destination;
- buffer uniform/storage/copy use.

These declarations derive dependency edges, liveness, and required wgpu usage
flags. They are not Vulkan pipeline stages or barriers. wgpu retains that
physical responsibility.

Every executable step owns and closes one complete command scope. A render or
compute pass begins and ends inside its step; copy operations are closed
encoder operations. `EncoderBatch` means an ordered sequence of those closed
steps recorded into one command encoder. A pass never spans task callbacks.
Pass merging would be a separate measured compiler transformation and remains
excluded. Until a scoped encoder facade is proved, raw encoder callbacks stay
trusted internal machinery rather than becoming a tenant API. RG1 must not
export its new step builder with raw encoder access; the existing public
`Task`/`EncodeCallback` surface remains compatibility-only until its in-repo
callers migrate.

Access order to the same resource remains stable. The compiler may reorder
independent work, but it cannot use graph optimization to change two reads or
writes whose relative order was declared by the builder. Scene painter order
has already been compiled inside the raster task and is not reconstructed here.

RG1's `ExecutionPlan` packages scheduling metadata with one-shot opaque encode
operations. It has a deterministic text dump containing names, resource
descriptions, accesses, edges, selected outputs, encoder batches, and
submission boundaries. The dump excludes raw handles, callback contents, and
backend-specific state. Before RG4 caches a topology, split the reusable
`PlanStructure` from a per-execution `BoundPlan` (provisional plain names).
Templates may retain the former; `FnOnce` operations and physical bindings
belong only to the latter.

## Work sequence

### RG0: Make the existing graph refuse malformed work — delivered 2026-09-04

Preserve the public `Task` shape long enough to harden the foundation.

- Replace `HashMap`-iteration scheduling with insertion-indexed storage and a
  stable ready queue.
- Make duplicate task IDs, missing inputs, and cycles typed errors.
- Make `execute` return `Result` and verify that every requested input appears
  in callback order.
- Add CPU-only unit receipts for deterministic scheduling and every refusal.
- Keep the current filter and box-shadow outputs pixel-identical.

Delivered in `fa5526051`. The implementation also refuses an external texture
whose ID collides with a task output; the old behavior silently replaced the
external in the returned map. Six CPU-only tests pin push-order scheduling,
all four refusal classes, and repeated-input dependency handling. The public
plan dump belongs to RG1; RG0's deterministic receipt is the ordered task-ID
sequence returned by its pure scheduler test.

**Done condition met:** identical insertion produces the same scheduled order;
each malformed case names the offending task/resource; all current
render-graph and filter receipts remain green.

### RG1: Separate build, compile, and execute — production path delivered 2026-09-04

The pre-RG1 probe found that every current raw `RenderGraph` consumer is a
unary chain: blur, color matrix, and box-shadow/clip work. Netrender does have
one real logical fork/join when a `SceneLayer` carries both `backdrop_filter`
and element `filters`:

```text
prefix Vello -> backdrop filters -> backdrop image --+
                                                    +-> final Vello -> target
layer content Vello -> element filters -> image ----+
```

That capability is currently exercised only through direct `Scene` tests;
`PaintList::LayerSpec` cannot yet express a backdrop filter. Classic Vello's
internal submission also means an encoder-only RG1 cannot execute the whole
shape honestly. Keep it as the documented target through RG1; RG2b owns its
first honest compiled and executed receipt through explicit Vello boundaries.
Until then, describe production graph use as unary rather than presenting a
synthetic fork/join as consumer proof. The live seams are
[`filter_passes.rs`](../netrender/src/renderer/filter_passes.rs),
[`filter_chain.rs`](../netrender/src/renderer/filter_chain.rs), and
[`filters.rs`](../netrender/src/renderer/filters.rs).

- Introduce graph-local `GraphId` and `ImageNode` handles plus explicit
  imported/transient image registration. Keep buffers out of the first patch.
- Introduce named task access declarations.
- Compile a graph for one or more requested output nodes into an
  `ExecutionPlan` before allocating or encoding.
- Cull disconnected tasks that cannot affect a requested output.
- Keep one image-only encoder-step kind as the first executable form. Every
  callback owns a closed render pass.
- Produce an `ExecutionReport` with compile, allocate, encode, and submit CPU
  durations; transient creation counts; descriptor-estimated logical bytes;
  and peak-live count/bytes globally and per exact descriptor. These are not
  GPU execution timings or physical allocation sizes. Report byte estimates
  as unavailable when a texture format has no single valid copy footprint.
- Migrate the three current source consumers: blur chains, color-matrix
  filters, and box-shadow/clip masks.
- Compile the current linear `SceneFilter` chains directly; do not add a
  semantic effect DAG without a multi-input consumer.
- Spike a scoped encoder facade against one blur callback. Keep it only if it
  makes the closed-pass rule enforceable without distorting current callbacks.
- Retire the public raw-`u64` compatibility API after those consumers migrate.

**Done condition:** a committed CPU-only fixture covers imported input -> mask
-> horizontal blur -> vertical blur -> color matrix plus a disconnected branch.
Its dump, selected-output culling, foreign-handle refusal, resource lifetimes,
and report are deterministic. The four selected transient outputs report four
creations and a peak of two simultaneously live images for one shared
descriptor. One migrated blur path executes through the plan, and existing
pixel receipts stay within their current tolerances.

The first slice landed in `975d9df4f`. It introduces graph-local logical image
handles, imported and transient declarations, named sampled and color-target
accesses, requested-output compilation and culling, stable scheduling, an
inspectable plan dump, logical lifetimes, and CPU-side execution/allocation
reporting. Admission refuses foreign handles, missing producers, duplicate
producers, cycles, invalid access direction, incompatible texture usage, a
load from a fresh transient, mixed legacy/planned modes, and imported texture
bindings whose physical size, format, or usage disagrees with the declaration.

The CPU fixture reports four 2x2 RGBA8 transient creations, 64 logical bytes
created, and a peak of two images / 32 logical bytes. `build_blurred_image` is
the first executable consumer and preserves its prior downstream `COPY_SRC`
contract. The existing backdrop-blur GPU receipt remains green.

The production migration landed in `69b9179c0`. Color-matrix filtering and the
box-shadow mask/downsample/blur/upsample chain now use the same typed image
plan. A new Scene-level receipt proves that `SceneFilter::Invert(1.0)` changes
opaque red layer pixels to cyan through the migrated color-matrix path. The
existing box-shadow, large-blur, and backdrop-blur receipts remain green.

The scoped-encoder facade experiment was rejected for this slice. The present
callbacks already create and drop their render pass within one call. A wrapper
that still exposed `CommandEncoder` would only rename the convention; a facade
that owned `RenderPass` would force lifetime-bound pipeline resources and bind
groups into a new callback shape without yet improving a public or tenant
boundary. Planned callbacks therefore remain crate-private trusted machinery.

RG1's compatibility retirement landed in `0af85a62f`. The useful
`p6_render_graph` and `p9a`/`p9b`/`p9c` assertions now run as crate-local tests
through the planned path. The public `Task`, `TaskId`, `RenderGraphError`,
legacy build/execute methods, graph-module re-exports, and public filter
callback constructors are gone. A crate-local mutex serializes only these
eight GPU tests because moving them into one library test binary exposed
unsafe parallel execution in the default test harness.

This removal is source-breaking relative to the published `netrender 0.1.2`.
It is accepted on unreleased `main`; the first release containing it must be
`0.2.0`, with `netrender_text` and `paint_list_render` dependency constraints
aligned in the same release integration pass. The compatibility patch does
not broaden into that release operation.

RG1 establishes a useful compiled linear plan and the machinery needed to
describe a DAG. It does not by itself earn crate extraction, prepared shapes,
pass merging, or claims of a general renderer graph. The combined-filter
fork/join in RG2b is that promotion gate.

### RG2a: Prove semantic adapter conformance

Keep the existing paint-list corpus and the new rasterizer corpus distinct:

- `paint_list_render/tests/corpus` remains the CPU-only, consumer-derived
  `PaintEnvelope -> Scene` ingress receipt. Its payload-elided fixtures are not
  pixel evidence.
- a small renderable Netrender scene corpus exercises the backend boundary.
  Start with solid geometry, gradients, transforms, clips, and nested layers;
  add images, text, filters, and retained fragments only as their adapters
  become real.

For every renderable fixture and compiled backend:

- admission must either produce a backend-native packet or a
  `BackendAdmissionError` naming the backend, operation index, and reason;
- `BackendCapabilities` must agree with observed admission;
- a supported operation must not disappear, turn transparent, or take an
  unreported fallback path;
- successful renders compare semantic anchors and tolerant pixel regions,
  rather than demanding byte identity across rasterizers;
- the receipt records the Vello family revisions and wgpu row used.

Keep backend-specific retention forms. Conformance is about consumer-visible
scene behavior, not identical lowered data structures.

**Done condition:** one command runs the renderable corpus against Classic,
Hybrid, and CPU; every case produces a checked image or an expected typed
refusal; capability declarations and observed results cannot drift silently.

### RG2b: Prove the three Vello execution shapes

Use one authoritative direct `Scene` whose layer combines backdrop blur and an
element filter chain. This is Netrender's first real fork/join workload even
though the paint-list API cannot yet express it.

- Hybrid records into an encoder batch supplied by the graph executor.
- Classic appears as an explicit submission boundary, followed by a graph
  encoder batch.
- CPU rasterizes outside the graph, then enters through a named ready
  upload/import operation before the same downstream chain. Do not add host
  worker or fence scheduling to the graph for this proof.
- Convert external-texture composition to an encoder-participating operation.
- Keep unsupported Hybrid/CPU scene operations as typed admission errors.

**Done condition:** all three paths produce a visible readback through the
same downstream graph topology; the plan dump names the selected rasterizer
and its execution boundary; Classic remains the default shipping path.

This is the first point at which the all-Vello experiment becomes a real
execution-graph consumer rather than a backend capability probe. RG2a proves
semantic equivalence or honest refusal; RG2b proves scheduling participation.

### RG3: Join one tenant frame

Start with Paredros because it already proves shared-device tenancy. Mesocosm
then checks that the contract supports a different renderer shape.

RG3 first treats each tenant's internal buffer copies, 3D textures, resident
compute, and depth composition as one closed tenant operation. It does not
pull those resources into the shared vocabulary merely because they exist.
The later CubeCL resident-buffer receipt is the gate for exposing that broader
topology to a general execution core.

- Import the tenant-owned color target as a graph resource.
- If the tenant accepts a caller encoder, represent its render as an encoder
  task. Otherwise represent it as an opaque submission boundary.
- Composite the tenant output at its stated scene-op boundary.
- End at Netrender's master texture; native/OS presentation remains the
  compositor host's responsibility.
- Preserve tenant names, the adapter-reported producer path, and fallback
  counters in graph dumps and timing spans. The graph records these diagnostics
  without interpreting producer revisions or deciding fallback policy.
- Make the host the sole owner of uncaptured-error and device-loss callbacks;
  tenants cannot replace them.
- Carry that policy through a host-provided device-health contract rather than
  installing hidden global handlers after Netrender receives `WgpuHandles`.
- Coordinate accountable tenant encoding on the host's frame thread. Work a
  tenant records or submits on another thread falls outside that error scope
  and must declare its own completion/error boundary.
- Attribute validation errors with a tenant-boundary error scope and discard
  the affected encoder/frame. Treat internal, out-of-memory, and device-loss
  failures as shared-device faults rather than tenant-local recovery.
- State the presentation policy honestly: an awaited diagnostic mode can
  suppress the current frame, while optimistic interactive presentation can
  only latch the error and suppress the next frame not yet presented.
- Recreate Netrender, every tenant and resident compute client, their caches,
  pipelines, allocations, leases, and imported targets together after actual
  shared-device loss. Surface-only recovery remains the compositor host's
  concern.

**Done condition:** a Paredros room + Netrender chrome frame is described by
one logical plan on one `WgpuHandles`, pixel-matches the current composed frame,
and reports its tenant, producer path, and submission count. A synthetic tenant
validation failure is attributed and prevents the promised presentation
boundary from committing in awaited diagnostic mode, or latches before the
next unpresented frame in optimistic mode. Mesocosm repeats the contract without
a product-specific graph type.

After both consumers pass, decide whether the neutral graph core belongs in
`netrender_device`. Netrender-specific Vello/filter builders remain in
`netrender` either way. If the intended destination is broader than a render
executor, require one additional cross-stack receipt before extraction:

```text
CubeCL opaque submission
    -> versioned resident buffer
    -> tenant render task reads that exact version
    -> Netrender composite
```

That receipt must use one device, preserve queue order without a CPU wait, and
emit one plan dump. Its source adapter refuses a stale producer stamp; the
graph records and requires the exact imported token. It is evidence for a
general execution core, not a prerequisite for RG3's renderer-tenancy proof.

### RG4: Prepare repeated graph shapes

Borrow the useful idea from `vk-graph::CommandStream`, not its Vulkan API.

- Add typed resource slots to a reusable graph template.
- Split reusable `PlanStructure` from per-frame operations and bindings before
  caching; never retain an RG1 `FnOnce` callback as template structure.
- Bind per-frame imported targets and parameters when instantiating it.
- Cache only plan structure proven stable by profiling.
- Keep dynamic graphs available for irregular filter and tenant workloads.

`TemplateImageSlot` is stable within a `PreparedGraphTemplate`; `ImageNode`
remains local to one graph or prepared-plan instance. Instantiation resolves a
slot binding once on the CPU before encoding. Template identity and graph
identity are separate types, so RG4 does not weaken RG1's foreign-handle check.

**Trigger:** RG2/RG3 measurements show graph construction or repeated
validation is material, or a real consumer repeats the same topology often
enough to simplify its code.

**Done condition:** a repeated filter/compose topology reuses its prepared
plan, accepts a different target each frame, preserves the normal graph dump,
and measures better than rebuilding it.

### RG5: Reuse transient textures

Pool whole wgpu textures by exact descriptor and non-overlapping logical
lifetime. Do not attempt Vulkan-style memory aliasing through raw backend
handles.

- Derive first/last use from the compiled plan.
- Reuse a texture only after its last scheduled access.
- Keep imported, retained tile-cache, master, and externally exported textures
  outside the transient pool.
- Record allocations, reuses, peak live bytes, and retained bytes.

**Trigger:** any RG1-or-later report shows repeated exact descriptors where
created transient count materially exceeds peak-live count, and repeated-frame
measurement shows allocation-path cost or memory pressure worth addressing.
RG5 may activate before RG2 if that evidence appears.

The RG1 fixture satisfies the structural half: four creations versus two
peak-live images for one exact descriptor. The 2026-09-05 repeated production
box-shadow measurement, commit `35cb54ea5`, satisfies the timing half without
activating RG5. On an NVIDIA GeForce RTX 4060 Laptop GPU through Vulkan, 16
completed warmups followed by 64 completed release samples reported:

- 256 x 256, blur radius 16: 33 transient creations, projected exact-descriptor
  peak 2, allocation median/p95 0.067/0.111 ms, host-work p95 1.349 ms;
- 1024 x 1024, blur radius 64: 35 transient creations across full- and
  quarter-resolution descriptors, projected peak 2, allocation median/p95
  0.055/0.102 ms, host-work p95 1.091 ms.

The decision rule was fixed before measurement. Structural pressure must be
present, then allocation p95 must reach either 2% of the configurable frame
budget (0.333 ms at the default 16.667 ms) or both 15% of host-work p95 and a
0.10 ms floor. Neither row reaches a materiality threshold. The reported
peak-live value is a logical lower bound for a future pool, not observed
physical residency: the current executor eagerly creates all selected task
outputs before encoding. RG5 therefore remains deferred unless a later
consumer, backend, or memory-pressure receipt crosses the documented trigger.

**Done condition:** allocation counts fall on a repeated multi-pass workload,
readback remains equivalent, and WebGPU/GL plus one native backend validate
without raw-hal access.

## Deliberate exclusions

- Depending on or wrapping `vk-graph`.
- Depending on AnyRender or introducing a second public scene vocabulary to
  resemble it.
- Exposing Vulkan access flags, queue-family ownership, or raw memory aliasing.
- Replacing wgpu's validation or synchronization.
- Making every CPU job part of the render graph.
- Building host-worker scheduling, CPU fences, or stale-frame policy into RG1.
- Passing execution resources through `Box<dyn Any>` or silently substituting
  transparent output for unsupported work.
- Allowing one render or compute pass to span graph task callbacks.
- Adding texture reuse before the RG1 report shows a repeated descriptor and
  a material reason to reuse it.
- Building a general semantic effect DAG before a multi-input effect requires
  one.
- Treating synthetic branching as evidence of a general consumer graph.
- Treating the execution graph as Mere's general graph runtime.
- Moving the graph into a separate repository before two consumers establish
  a neutral contract.
- Multi-queue scheduling, async compute, pass merging, or automatic parallel
  recording before the single-queue model is measured.
- Changing which Vello realization ships by default.

## Next implementation slice

Implement RG2a's rasterizer-independent scene corpus and typed admission
receipts for Classic, Hybrid, and CPU. Keep the existing paint-list corpus as a
separate ingress receipt. RG2b then gives the three admitted backends explicit
execution boundaries through one combined backdrop-plus-element-filter
fork/join. Retain per-task texture allocation unless a later backend or
consumer crosses RG5's recorded materiality rule. Buffers, prepared templates,
crate movement, tenant callbacks, error-scope coordination, and paint-list
backdrop-filter expansion remain outside RG2a.

## Acceptance summary

The plan succeeds when one authoritative `Scene` is admitted consistently by
each selected rasterizer and Netrender can explain the resulting frame as a
deterministic, validated logical plan over one shared wgpu device. Each
producer states how it participates without gaining scene, graph, or device
authority. It fails if the graph becomes a second device abstraction,
duplicates wgpu barriers, absorbs durable application authority, permits
silent backend degradation, or requires a Vulkan-only path.
