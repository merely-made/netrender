# Production flattening qualification — 2026-10-12

Canonical Scene curves, glyph IDs, positions, axes and hint policy are retained.
Provider `2eae6243881d5b24c984d5ee35dde509ca5db782` sets Classic and Sparse
approximation to 1/256 device pixel, with bounded subdivision and typed work
refusals. Conservative estimates saturate and refuse above the Classic global
line budget before preparation. Every final Classic readback is followed by
quality polling and zero outstanding counter frames/bytes.

All eleven geometry cases and the five actual recorded glyph variants pass.
The canonical curved fixture's Classic alpha delta is now at most one, compared
with forty-two in the earlier diagnostic. The original body glyph run retains
all caller inputs and passes with Classic edge alpha delta eleven. Full producer
qualification passes twelve intact panels and twenty-four source scenes.

The unchanged premultiplied encoded-sRGB RGBA8 gates are edge RGBA <=32 / alpha
<=16, interior RGBA <=3 / alpha <=2, mean channel delta <=1.0, with reference
neighbor variation 8 defining edges. Full-panel/source maxima are recorded in
[qualification](production-parity-qualification.json), with all identities and
comparisons in [the twelve-case index](production-parity-summary.json).

- [Geometry receipt](geometry-all/receipt.json).
- [Actual glyph receipt](glyph-canonical/receipt.json).
- [Measured costs and identity-matched methodology](performance-analysis.md).
- [Frozen source and executable hashes](frozen-source-before-gpu.json).
- [Root image review](root-visual-review.json).
- [Focused checks](checks/): 16 new quality controls, 177 passing common controls
  with one existing ignored test, 21 resource-ledger controls, Classic allocation
  and malformed-command regressions, retained placement/master reuse and RG1.

RG1 retains the original allocation-pressure predicate and still reports
MATERIAL for both descriptor sizes; this is an outstanding pooling opportunity.
Peak-live bytes are projected logical bounds rather than physical allocator use.
The unoptimized fresh-session batch has paired CPU median ratio 1.52 and Sparse
host encode/submit ratio 2.01. Completion/readback and initialization remain
separate; no GPU-timestamp or warm-frame throughput claim is made.

Representative wide/narrow intact composition and producer PNGs are included.
Complete font-bearing bundles and the remaining PNGs stay in the local
`output/vello-production-fixes-20261011` execution tree. Their input and output
digests remain in the receipts. No font binaries are committed.

Root ran all builds and serialized runtime gates with two build jobs,
incremental/debug disabled, finite process deadlines and five-second readback
limits. Independently reviewed source lanes ran no adapters. Explicit Moot and
Tabard runtime/CPU leases and disk-floor refusals bounded resource use. Shared
Genet/Cambium actual callback recovery and native presentation have separate Mere
receipts; controlled device destruction is distinct from a physical driver reset.
