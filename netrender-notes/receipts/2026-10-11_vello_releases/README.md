# Vello release source adoption — 2026-10-11

Mark requested the latest releases before further producer-resource work. The
stable upstream release tags Classic 0.11.0, Sparse Strips 0.3.0 and Glifo 0.4.0
all resolve to `f3000c8d9e7a9c7e6abb09b9587238bf8b4860ff` (2026-10-02).
The maintained provider is `5862bba26c8a9f56f6a5fdc8ea6966b360a5d63a` from
`https://github.com/mark-ik/vello`, published on both `mark-ik/all-vellos`
and the isolated adoption branch, incorporating that release into
`10f01d6d88e94eac087daf033b24895cf97b8e82`.

## Result and boundaries

Netrender consumes Classic `netrender-vello` 0.11.0, CPU/GPU/Common 0.3.0 and
Glifo 0.4.0 from one immutable Git revision. `vello-gpu` selects the renamed
upstream GPU package; `vello-hybrid` remains a feature compatibility alias.
Classic remains the default. Wgpu/Naga use 30.0.1. Skrifa stays on the shared
0.44 glyph ABI; the separate Parley adapter uses 0.12 and `font_metrics()`.
Genet's existing named Parley 0.10 fork is a separate shaping source that emits
neutral font bytes/glyph instances; no Parley Layout crosses that boundary.

The maintained Classic coarse and fine shaders, dynamic allocation recovery,
image-atlas residency, and resource resolver files are byte-identical to the
prior maintained baseline. Bounded consumed sparse scene append was moved to
upstream's GPU package, retaining tile-aligned translations and typed refusals.
Additional checked table/span/paint/bounding-box remaps run before mutation.
Glifo's new fallible draw API returns an attributed `GlyphRender` error before
CPU output or GPU encoding is returned. Resource admission remains atomic;
runtime glyph errors do not promise to roll back already admitted resource caches.

Sparse viewport admission refuses empty dimensions before provider construction.
GPU dimensions must fit four-pixel tile rounding. CPU additionally needs its
128-pixel horizontal depth-bucket rounding to fit; extreme widths are refused
before hydration/target allocation. These constraints are adapter admission,
not an upstream provider repair. The boundary tests exercise real thin CPU
outputs and verify that refused replacement sources do not change warm resources.

## Executed provider checks

Builds were serialized with `-j 2`, debug information and incremental compilation
disabled. Runtime tests use `--test-threads=1`. The GPU lane was explicitly handed
off by Moot, then released to Tabard after this finite batch; no headed app or
browser was launched. Logs preserve actual commands' output and identities.

- `cargo test --locked -p vello_gpu append -j 2`: 9 append controls passed;
  no GPU initialized.
- `cargo test --locked -p glifo -p vello_common -p vello_cpu --lib -j 2`:
  26 + 173 + 92 passed, with one existing Common test ignored. Together with
  append: **300 distinct CPU tests passed, one ignored**.
- Focused Classic targets `coarse_bitmap`, `compare_gpu_cpu`, `dynamic_memory`,
  `fine_command_guards`, `image_atlas`, `retained_preview_scene`: 13 normal cases
  passed. The large-bin stress and two deliberate diagnostics were ignored in
  this first invocation.
- Exact opt-in `fine_command_guards` and `retained_preview_scene` targets with
  `--ignored`: both passed. Thus **15 distinct Classic cases passed**; large-bin
  stress remains unexecuted. Command guards use a 4×4 bounded dispatch, and
  renderer readbacks wait at most five seconds for their own submission.
- Nine owned Netrender GPU image/pattern/text cases passed on the existing host
  device, covering semantic pixels, resource limits, lifecycle, text variations,
  and explicit unsupported-pattern refusals.

Classic's CPU/GPU comparison uses its CPU **coarse fallback**, with the same fine
shader, and a FLIP mean threshold 0.01. It is not a three-renderer application
comparison against Sparse Strips CPU. Other retained regressions check exact
pixels. Allocation recovery verifies actual first-frame overflow and third-frame
recovery on one renderer; it is not driver-reset recovery.

Provider library CPU checks ran at `8eec8b182cbe326e0ed4fc64a13311eed4377601`.
The final `5862bba2` follow-up adds only the previously omitted dynamic-memory
regression; library code is unchanged. Classic GPU checks and final Netrender
checks use `5862bba2`. Earlier failed Wasm-binding resolution, a cancelled
intermediate build and the missed-test build are preserved in the local output
folder rather than counted as successful checks.

The runtime reports AMD Radeon Pro Vega 56 / Metal. Wgpu's driver fields are
empty, so no driver version is inferred. See `source-and-environment.json`.
The 22 PNG captures are raw generated controls in `classic-captures/`, with
artifact hashes. Representative coarse, layered blend and fill-rule captures
were inspected visually; assertions supply the actual acceptance criteria.

## Netrender and consumer checks

Final CPU capture, sparse resource, dimension, text and compilation logs are
listed alongside this receipt. The final sparse-resource/fragment/capture run
passed 24 cases; three dimension controls and 15 CPU text-adapter controls
passed, plus the exact typed corpus-refusal control. Both the default workspace
check and the all-feature/all-target check passed. Accepted limits are CPU 65,408×65,532 and GPU 65,532×65,532 (each
subject to the smaller configured pixel/resource budget). A first capture invocation without `serde` ran
zero capture tests; only the final invocation with `serde` counts as capture
qualification. Dimension testing also exposed the CPU depth-bucket constraint
and was repaired before acceptance; failed boundary logs remain local evidence.

The source fingerprint records modified code and manifests on parent `dfa2bd001`.
The immutable resulting Netrender commit is the consumer pin; it also includes
the prior bounded source registry and historical 2026-10-10 CPU proof. Historical
receipts remain historical rather than being relabeled as new-provider proof.

This source adoption does **not** establish the full external-producer Workshop
bundle, cross-renderer application parity, native input/accessibility acceptance,
headed host presentation, browser/WebGPU qualification or driver-reset recovery.
Those remain separate gates in the adoption and sparse-backend plans. No registry
publication was performed. The previously removed public legacy graph API still
requires the separately planned Netrender 0.2 registry release.
