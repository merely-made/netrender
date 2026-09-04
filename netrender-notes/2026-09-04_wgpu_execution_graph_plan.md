# Netrender wgpu execution graph plan

**Date:** 2026-09-04

**Status:** RG0 delivered 2026-09-04; RG1 not started

**Prior art:**

- [`vk-graph`](https://github.com/attackgoat/vk-graph) for compiled GPU work;
- [`AnyRender`](https://github.com/DioxusLabs/anyrender) for the semantic
  paint/backend seam above it.

**Owns:** frame-local GPU dependency planning in Netrender

**Does not own:** scene meaning, product state, GPU synchronization, or device creation

## Decision

Evolve Netrender's existing `RenderGraph` into a validated, inspectable wgpu
execution graph. Keep one `WgpuHandles` instance as the physical device and
queue authority. Let wgpu own barriers, resource transitions, backend choice,
and command submission validity.

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
into a sibling crate, requires a second renderer to insert a real operation
through the same public graph contract. Sharing a device alone is not that
proof.

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

The 2026-09-04 baseline receipt remains green:

```text
cargo test -p netrender --test p6_render_graph -j 1
2 passed; 0 failed
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
| CPU | produces host pixels | host work before the GPU graph, followed by an upload/import step |

Netrender's filter callbacks and a future Mesocosm march pass can participate
in a caller-owned encoder. `compose_external_texture` currently creates and
submits another encoder; it is a good first function to split into
"encode into this encoder" and convenience "encode + submit" forms.

One logical graph therefore does not imply one command encoder or one queue
submission. Finalization partitions ordered work into encoder batches around
opaque submission boundaries. Queue order supplies the physical sequencing;
the graph makes that order deliberate and visible.

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
| wgpu | device lifetime | resource validity, barriers, backend synchronization, and queue submission |
| Mesocosm simulation | saved/replayed world | ecology, matter, causality, and deterministic outcomes |

The execution graph may project durable state into work. It never becomes the
record of that state. A completed GPU pass cannot author a Mesocosm ecological
fact or mutate a Mere content graph by implication.

## Working vocabulary

Names are provisional until RG1 proves them in code.

```rust
ImageNode
BufferNode
TaskNode
ImportedImage
TransientImageDesc
ImageAccess
BufferAccess
ExecutionPlan
ExecutionStep
EncoderBatch
SubmissionBoundary
GraphBuildError
GraphExecutionError
```

RG2 may additionally need an internal `RasterExecution` with three explicit
forms: encoder participation, opaque submission, and CPU pixels. That name and
shape remain provisional until the conformance work proves what data crosses
the boundary. `BackendCapabilities` and `BackendAdmissionError` already name
the semantic side and should be extended instead of duplicated.

Resource handles are graph-local, typed indices. In checked builds they also
carry a graph generation so a handle from one graph cannot address a resource
in another.

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

Access order to the same resource remains stable. The compiler may reorder
independent work, but it cannot use graph optimization to change two reads or
writes whose relative order was declared by the builder. Scene painter order
has already been compiled inside the raster task and is not reconstructed here.

`ExecutionPlan` is pure scheduling metadata plus opaque encode operations. It
has a deterministic text dump containing names, resource descriptions,
accesses, edges, selected outputs, encoder batches, and submission boundaries.
The dump excludes raw handles, callback contents, and backend-specific state.

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

### RG1: Separate build, compile, and execute

- Introduce graph-local `ImageNode` handles and explicit imported/transient
  image registration.
- Introduce named task access declarations.
- Compile a graph for one or more requested output nodes into an
  `ExecutionPlan` before allocating or encoding.
- Cull disconnected tasks that cannot affect a requested output.
- Keep one encoder-batch task kind as the first executable form.
- Migrate the three current source consumers: blur chains, color-matrix
  filters, and box-shadow/clip masks.
- Compile the current linear `SceneFilter` chains directly; do not add a
  semantic effect DAG without a multi-input consumer.
- Retire the public raw-`u64` compatibility API after those consumers migrate.

**Done condition:** a committed logical-plan fixture covers
mask -> horizontal blur -> vertical blur -> color matrix; its order and
resource lifetimes are deterministic; existing pixel receipts stay within
their current tolerances.

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

Use one small authoritative Netrender scene and the same downstream blur and
color-matrix chain.

- Hybrid records into an encoder batch supplied by the graph executor.
- Classic appears as an explicit submission boundary, followed by a graph
  encoder batch.
- CPU rasterizes to host pixels, then enters through a named upload/import
  operation before the same downstream chain.
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

- Import the tenant-owned color target as a graph resource.
- If the tenant accepts a caller encoder, represent its render as an encoder
  task. Otherwise represent it as an opaque submission boundary.
- Composite the tenant output at its stated scene-op boundary.
- End at Netrender's master texture; native/OS presentation remains the
  compositor host's responsibility.
- Preserve tenant names in graph dumps and timing spans.

**Done condition:** a Paredros room + Netrender chrome frame is described by
one logical plan on one `WgpuHandles`, pixel-matches the current composed frame,
and reports its tenant and submission count. Mesocosm repeats the contract
without a product-specific graph type.

After both consumers pass, decide whether the neutral graph core belongs in
`netrender_device`. Netrender-specific Vello/filter builders remain in
`netrender` either way.

### RG4: Prepare repeated graph shapes

Borrow the useful idea from `vk-graph::CommandStream`, not its Vulkan API.

- Add typed resource slots to a reusable graph template.
- Bind per-frame imported targets and parameters when instantiating it.
- Cache only plan structure proven stable by profiling.
- Keep dynamic graphs available for irregular filter and tenant workloads.

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

**Trigger:** a measured RG2/RG3 frame shows transient allocation churn or
memory pressure worth addressing.

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
- Passing execution resources through `Box<dyn Any>` or silently substituting
  transparent output for unsupported work.
- Building a general semantic effect DAG before a multi-input effect requires
  one.
- Treating the execution graph as Mere's general graph runtime.
- Moving the graph into a separate repository before two consumers establish
  a neutral contract.
- Multi-queue scheduling, async compute, pass merging, or automatic parallel
  recording before the single-queue model is measured.
- Changing which Vello realization ships by default.

## First implementation slice

RG0 is intentionally small and independently valuable. It changes failure
behavior and scheduling internals while preserving the current work model.
The patch should touch only:

- `netrender/src/render_graph.rs`;
- focused render-graph unit/integration tests;
- callers for the new `Result` return;
- this plan and the verification record after the receipt lands.

Do not combine RG0 with typed resources, pooling, Vello integration, crate
movement, or tenant changes. Those need their own evidence.

## Acceptance summary

The plan succeeds when one authoritative `Scene` is admitted consistently by
each selected rasterizer and Netrender can explain the resulting frame as a
deterministic, validated logical plan over one shared wgpu device. Each
producer states how it participates without gaining scene, graph, or device
authority. It fails if the graph becomes a second device abstraction,
duplicates wgpu barriers, absorbs durable application authority, permits
silent backend degradation, or requires a Vulkan-only path.
