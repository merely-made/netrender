# Vello upstream adoption lane

**Status:** activated on 2026-10-11; provider releases incorporated and focused
CPU/GPU qualification passed. Netrender and consumer qualification is in progress;
complete producer bundles and headed host acceptance remain separate gates.
Mark directed that Vello have its own lane in the workspace upstream refresh.
Other maintained forks target stable releases and compatible upstream changes.

## 2026-10-11 execution scope

Mark explicitly requested the most recent Vello releases before further
producer-resource work. Live Git tags, GitHub release metadata and crates.io
agree on Classic 0.11.0, CPU/GPU/common 0.3.0 and Glifo 0.4.0, all released
2026-10-02 at `f3000c8d9e7a9c7e6abb09b9587238bf8b4860ff`.
Use those stable releases rather than unreleased main.

The actual maintained consumer baseline is now
`10f01d6d88e94eac087daf033b24895cf97b8e82`, which includes the original seven
fork commits plus Radeon coarse traversal, native fine command progress guards
and resident image-atlas preservation. Port from that baseline; the earlier
`6a174d27` inventory alone omits fixes already used by Mere. Preserve the
`netrender-vello` package identity, coherent encoding/shader sources, bounded
buffer recovery and consumed/tile-aligned sparse append with its refusals.
Classic's upstream package is still named `vello`; `research/vello_research`
is its directory, not a new package name. Hybrid's package becomes `vello_gpu`.

Isolated provider checkout: `Code/worktrees/vello-release-adoption`, branch
`codex/vello-release-adoption`. Netrender adoption stays on
`codex/vello-cambium-resource-bridge` after `dfa2bd001`, retaining the tested CPU
source registry. Keep public compatibility aliases where useful; qualify the
actual upstream packages and APIs. Scope text upgrades against the new release's
glyph/Blob type identities before changing Skrifa independently.

Builds are serialized with two Cargo jobs, reusing existing target artifacts
where possible. No feature-wide tests that implicitly boot GPUs before explicit
handoff. GPU/native/browser ownership is being coordinated with the Moot and
Tabard tasks, separately from read-only subagent audits. Existing 0.10 CPU/native
receipts remain historical controls and are not relabeled as 0.11 evidence.

After provider and Netrender gates, resume complete producer-resource capture:
the real Rootstock emitter plus the bound SceneProducer callbacks, preserving
external key and content stamp, source identity/revision, logical/physical size,
scale, straight-alpha sRGB declaration, payload identity and shadow side channels.
Current windowless Harness suspends producers before full emission, so a portable
capture seam is needed; reconstructing preview DOMs or stripping external commands
does not close this gate. CPU capture, live GPU import, host presentation,
input/accessibility and device recovery remain separate acceptance receipts.

## Source and ownership

The initial inventory used `mark-ik/vello`, branch `mark-ik/all-vellos`, at
`6a174d2777ab555286a851865384df20a313ce3c`. The actual merge starts from its
maintained descendant `10f01d6d88e94eac087daf033b24895cf97b8e82`. Stable upstream `v0.11.0` is
`f3000c8d9e7a9c7e6abb09b9587238bf8b4860ff`. The initial baseline had seven commits
outside upstream and lacked 69 commits in that release. The merge preview conflicted
in workspace/lock manifests, CI/examples, moved append code, GPU rendering, and
the test harness. Those conflicts are resolved in candidate `5862bba26c8a9f56f6a5fdc8ea6966b360a5d63a`,
published on the isolated adoption branch and the maintained `mark-ik/all-vellos`
branch after its focused checks passed. The provider receipt records both
executed checks and the remaining consumer/host scope.

| Owner | Current source | Adoption obligation |
| --- | --- | --- |
| Vello fork, Classic | Adopted `netrender-vello` 0.11.0 at `5862bba2`; prior registry release remains 0.10.1 | Preserve package identity or provide an explicit replacement and release transition. |
| Vello fork, sparse CPU/GPU | CPU/GPU/common 0.3.0 and Glifo 0.4.0 at `5862bba2`, with bounded append preserved | Port bounded append and its refusal tests through the GPU rename. |
| Netrender | Classic default; all providers pinned together to `5862bba2`, with `vello-gpu` and compatibility `vello-hybrid` aliases | Migrate adapters with one sparse source identity and the host's existing wgpu device. |
| Genet | Netrender consumers and hosted renderer | Adopt a qualified Netrender revision, then verify composition and host lifecycle. |
| Mere and products | Renderer patches, glyph types, device sharing, site payload | Align the family after provider gates; qualify actual consumers separately. |

Upstream moves Classic to `research/vello_research` with companion encoding and
shader crates. `vello_hybrid` becomes `vello_gpu`; CPU/GPU/common declare 0.3.0.
The release declares wgpu 30.0.0, Peniko 0.6.1, Skrifa 0.44.0, and Rust 1.89.
Do not independently bump glyph-facing Skrifa versions across Classic's
boundary: variation-coordinate types are part of the consumer interface.

The original seven fork commits cover wgpu 30, the Netrender package rename, consumed
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

The [2026-10-11 release receipt](receipts/2026-10-11_vello_releases/README.md)
records immutable sources, serial CPU/GPU checks and captures. No crates.io
publication is part of this source adoption. The public legacy API removal
already on main still requires the separately planned Netrender 0.2 release.

The [sparse backend plan](2026-09-27_sparse_backends_plan.md) remains the
capability and consumer checklist. The
[upstream ask](2026-08-10_vello_hybrid_upstream_ask.md) remains a draft; this lane
does not authorize contacting upstream maintainers.
