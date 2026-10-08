# Vello upstream adoption lane

**Status:** scoped on 2026-10-08; implementation and consumer repins remain open.
Mark directed that Vello have its own lane in the workspace upstream refresh.
Other maintained forks target stable releases and compatible upstream changes.

## Source and ownership

The baseline is `mark-ik/vello`, branch `mark-ik/all-vellos`, at
`6a174d2777ab555286a851865384df20a313ce3c`. Stable upstream `v0.11.0` is
`f3000c8d9e7a9c7e6abb09b9587238bf8b4860ff`. The baseline has seven commits
outside upstream and lacks 69 commits in that release. A merge preview conflicts
in workspace/lock manifests, CI/examples, moved append code, GPU rendering, and
the test harness. This is an adoption project, not a version substitution.

| Owner | Current source | Adoption obligation |
| --- | --- | --- |
| Vello fork, Classic | Published `netrender-vello` 0.10.1; maintenance consolidated in `6a174d27` | Preserve package identity or provide an explicit replacement and release transition. |
| Vello fork, sparse CPU/Hybrid | Append prototype at `ca3f40ea182216883cd543c7b9deae991268917c` | Port bounded append and its refusal tests through the GPU rename. |
| Netrender | Classic default; opt-in CPU/Hybrid/common pinned together to `ca3f40ea` | Migrate adapters with one sparse source identity and the host's existing wgpu device. |
| Genet | Netrender consumers and hosted renderer | Adopt a qualified Netrender revision, then verify composition and host lifecycle. |
| Mere and products | Renderer patches, glyph types, device sharing, site payload | Align the family after provider gates; qualify actual consumers separately. |

Upstream moves Classic to `research/vello_research` with companion encoding and
shader crates. `vello_hybrid` becomes `vello_gpu`; CPU/GPU/common declare 0.3.0.
The release declares wgpu 30.0.0, Peniko 0.6.1, Skrifa 0.44.0, and Rust 1.89.
Do not independently bump glyph-facing Skrifa versions across Classic's
boundary: variation-coordinate types are part of the consumer interface.

The seven fork commits cover wgpu 30, the Netrender package rename, consumed
scene append, allocation-count buffer growth, and maintenance consolidation.
Retain their behavior or account for its tested upstream replacement. Append
remains constrained to consumed scenes and tile-aligned translations; this
lane does not expand it into arbitrary retained fragment composition.

## Text-family coordination

Mere's `2026-10-08_dependency_currency_plan.md`, currently in its active
`dep-currency` lane, records this refresh as the preceding stage (D8). It assigns
the Linebender text migration to this Vello lane: Parley/Parley data, Skrifa,
read-fonts, and Harfrust across Netrender and Genet. The provider's glyph-facing
types, shaping results, pixels, and measured footprint must qualify together.
The other dependency stage keeps its provider-first order for accessibility,
security/runtime, storage, parsers, and compatible registry updates. Its plan
lands with its Rapier work; this scope does not publish those active branches.

## Execution gates

1. **Provider map.** Inventory moved crates/APIs and compare stable against main
   only where a concrete fix is needed. Record source and feature identities.
   Resolve the merge retaining append and allocation recovery; preserve defaults.
2. **Provider qualification.** Run Classic, CPU, and GPU tests plus append's
   positive/refusal controls. Exercise buffer overflow and bounded recovery,
   repeated frames, device/resource teardown, image readback, and shared-device
   execution. Record backend, driver, source hashes, and feature set.
3. **Netrender adoption.** Update sparse crates together. Run default and alternate
   suites, image/pattern/text admission tests, and intact paused Isocosm replay
   with Classic as the control. Compare pixels, allocation bounds, and timings.
   Preserve typed refusals, including Hybrid bilinear repeat seams unless an
   explicit repair earns new evidence.
4. **Host and product adoption.** Qualify Genet composition and teardown, then
   align Genet/Netrender/Mere pins. Verify affected native and browser products;
   regenerate the site runtime and measure compressed payload before promotion.
   Headless provider receipts do not close headed host acceptance or live
   external producer composition.
5. **Publication.** Publish the tested provider identity and coordinated consumer
   pins. Retain prior immutable releases and receipts for rollback. Record the
   package/version transition before publishing.

**Done condition:** stable source incorporated; owned extensions preserved or
replaced by tested equivalents; Classic/CPU/GPU and consumer gates passing
against coherent immutable sources; publication and remaining host limitations
documented. A clean merge or compile alone does not close the lane.

The [sparse backend plan](2026-09-27_sparse_backends_plan.md) remains the
capability and consumer checklist. The
[upstream ask](2026-08-10_vello_hybrid_upstream_ask.md) remains a draft; this lane
does not authorize contacting upstream maintainers.
