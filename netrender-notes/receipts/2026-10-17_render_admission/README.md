# Rendering admission verification

Classic and Sparse reject nested retained placements before changing retained content. Checked Classic methods expose the refusal. Paint translation now returns a command-indexed error for unsupported border styles or geometry; normal-border admission has one owner shared with portable bundles. Corpus replay aggregates mismatches and missing expectations. The article fixture intentionally expects refusal at command 8 rather than a misleading solid border.

Verified on 2026-10-17 with serialized jobs, 5 GiB memory limit, zero swap allowance, 150% CPU quota, nice 10, one Cargo job and test thread:

- paint_list_render, replay-cpu + resource-bundle, all targets: 63 passed, zero failed or ignored. Includes captured GPU/CPU border pixels, retained leaf draws, admission regressions and corpus aggregation.
- netrender, vello-cpu, pe4_fragment_retention + vb5_sparse_fragments: 15 passed, zero failed or ignored. Includes Classic GPU draw before/after refused mutation and Sparse cache preservation.
- The first paint gate exposed the article fixture's newly explicit refusal. Its expectation was corrected; paint commands were not changed.

Raw logs: /home/markik/Code/testing/audits/genet-architecture-20261017/render-admission/{supplier-first,supplier-final,fragment-verified}.log. Final paint gate peak RSS 342512 KiB; retained-fragment gate 669392 KiB; both zero swaps.

This verifies admission and retained supported paths, not complete CSS border conformance. Double/groove and other unsupported borders refuse the frame. Other translator fallback categories remain outside this change. Consumers must handle the newly fallible translation entrypoints.
