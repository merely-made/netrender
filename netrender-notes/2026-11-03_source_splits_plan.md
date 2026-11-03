# Source splits: H5 Netrender batch

**Status (2026-11-03):** landed, 12/12 paths qualified.

## Phases and done-conditions

- **S1. Scene construction.** Group gradient, stroke and inspection builders
  and move command-stream translation into its own implementation module.
- **S2. Sparse resources.** Separate sparse lowering and image-cache state
  from backend admission and session lifecycles.
- **S3. Fixtures and qualification.** Group external-texture, tile, layer,
  resource-bundle and corpus fixtures by responsibility. All twelve original
  paths and resulting modules fit 600 lines; retire their exceptions, pass
  the relevant tests, feature checks and full local gate, commit and push.

This implements Mark's 50 additional oversize-file request under Mere's
house-style H5. It changes source ownership only, preserving public paths,
rendering arithmetic, thresholds, immutable prior receipts and shader bytes.

## Findings

- 2026-11-03: scene builders and sparse image-state maintenance already have
  distinct responsibilities within long implementation files. GPU and CPU
  qualification remain separate claims; no new cross-platform claim follows
  from moving source.
- Compact evidence appends to
  [the receipt](receipts/2026-11-03_source_splits_plan_receipts.md), under the
  shared H5 retained-byte cap of 100 MB across repositories.

## Progress

- 2026-11-03: selected twelve paths in a clean checkout. Baseline started.

- 2026-11-03: S1–S3 landed. Twelve exceptions retired, selected rendering/resource/text tests (156 cases) and full local gate pass. The existing full-suite compose pixel mismatch remains at its original threshold. An explicit native-only WebGl2 fixture arm repairs feature-check exhaustiveness.
