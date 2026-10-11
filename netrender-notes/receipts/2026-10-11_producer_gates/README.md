# Producer resource gates — 2026-10-11

This receipt advances acceptance after the stable Vello source adoption at
provider `5862bba26c8a9f56f6a5fdc8ea6966b360a5d63a`. Classic 0.11,
Sparse CPU/GPU/Common 0.3, Glifo 0.4 and wgpu/Naga 30.0.1 remain unchanged.
Execution used AMD Radeon Pro Vega 56 / Metal with explicit ownership of the
serialized GPU/native lane. Builds used two jobs, disabled incremental/debug
artifacts, and tests used one runtime thread. Each offscreen process had a
120-second outer deadline and five-second submission/readback deadlines.

| Gate | Result | Evidence |
| --- | --- | --- |
| Resource bundle controls | 21 passed | [final controls](checks/final-bundle-controls.log) |
| Existing translator library controls | 14 passed | Same log |
| Actual workshop full composition, CPU vs Sparse GPU | 12 accepted | [metrics](qualified-parity-metrics.json), individual `sparse/*/receipt.json` |
| Actual bound producer scenes, CPU vs Sparse GPU | 24 accepted | Same receipts; sources rendered independently of CPU materialized payloads |
| Synthetic transparent producer, CPU vs both GPU families | Accepted | [synthetic receipt](synthetic-parity-final/receipt.json) |
| Synthetic composed rounded border, CPU vs Sparse GPU | Accepted | Same receipt |
| Classic complete-frame and actual producer pixels | Open: edge thresholds exceeded | [wide-cold receipt](production-wide-cold-parity/receipt.json), [diagnosis](classic-edge-diagnostic.md) |

The twelve real workshop captures cover widths 1280 and 420, each at cold,
repeat, authored RGB edit preserving alpha 73, mode change, resize and source
rebind. Their intact envelopes retain fonts, UI commands, two external draws
and both producer callbacks. This authored workshop has zero shadow requests;
it does not qualify missing production shadow-mask materialization. Captures
and font-bearing complete bundles remain local, outside Git receipts.

Every Sparse GPU composition was accepted with maximum edge channel delta 3
and maximum interior delta 2. Independently rendered producers had maximum
edge delta 2 and interior delta 1. Comparison retains premultiplied encoded-sRGB
RGBA, including alpha. Original thresholds are edge RGBA 32 / alpha 16,
interior RGBA 3 / alpha 2, mean channel delta 1.0; no thresholds were widened.

Classic remains red. The synthetic composition has alpha/RGBA edge delta 42;
the real wide-cold composition has edge RGB delta 45 and the direct producer
scenes 44 and 46, with exact producer interior matches. Offline localization
supports a curved-edge flattening/coverage difference and an independent
glyph-edge difference, without identifying a unique provider root cause. The
canonical cubic geometry, transparent output and captured commands remain
intact. [CPU](production-wide-cold-parity/cpu.png),
[Classic](production-wide-cold-parity/classic.png) and
[Sparse GPU](production-wide-cold-parity/gpu.png) captures preserve the failure.

The final added guard refuses `NormalBorder.do_aa=false`; admitted authored
cases use `true`. It does not change accepted geometry/pixels. The twelve GPU
cases used the formatted executable before that conservative guard; the final
21-control suite and rebuilt synthetic all-renderer rerun qualify the guard,
with the same Classic edge refusal. Final source and pins receive separate
consumer qualification. See the Mere receipt for whole-plan budget atomicity,
production input/accessibility controls, native presentation and exact orderly
fresh-device rehydration. Those scopes do not establish OS assistive-technology
interaction, browser qualification or forced driver loss.

The [adapter contract](../../2026-10-11_complete_producer_resources.md) explains
identity versus execution evidence and conservative typed refusals. Classic
remains the shipping host renderer; Sparse adapters remain opt-in.
