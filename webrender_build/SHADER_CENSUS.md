# WebRender shader translation census

This is an instrumentation slice for WebRender's generated GLSL shader
corpus. It records whether selected translation routes can process each
preprocessed shader stage and whether the normalized WGSL stages have a
compatible pipeline interface. It is not a renderer backend and does not
replace WebRender's GL renderer.

In particular, a successful translation, Naga validation, or interface audit
does not establish visual correctness, device validation, draw submission, or
pixel parity. The optional smoke binary is intentionally narrower still: it
attempts only one `ps_clear` pipeline creation after a successful audit; it
does not submit a draw.

## Reproducible inputs

The branch pins the toolchain used by the census in
`webrender_build/Cargo.toml`:

- `naga = 30.0.1` with `glsl-in`, `spv-in`, and `wgsl-out`
- `shaderc = 0.10.1`
- `wgpu = 30.0.0` for the optional smoke binary

The corpus comes from `get_shader_features` and is assembled through
`build_shader_strings`, the same preprocessing path used by WebRender. The
full profiles enumerate every enabled variant for desktop GL and GLES. The
test corpus count is 236 GL stages plus 198 GLES stages, or 434 stages total.
Each record includes the shader name, selected feature configuration, stage,
and SHA-256 of the assembled source, so a receipt identifies its source input.

## Lanes

Every stage receipt has these lanes, in a fixed order:

1. `assembled_glsl`: evidence that WebRender assembled the GLSL source.
2. `naga_glsl_frontend`: assembled GLSL parsed by Naga and written as WGSL.
3. `raw_shaderc_spirv_naga_validated`: assembled GLSL compiled by shaderc to
   SPIR-V, then parsed and validated by Naga. This is not a WGPU runtime claim.
4. `shaderc_spirv_to_naga_wgsl`: the same raw SPIR-V parsed, validated, and
   written as WGSL by Naga.
5. `vulkan_prepared_shaderc_spirv_naga_validated`: shaderc output after the
   declared Vulkan preparation, parsed and validated by Naga.
6. `vulkan_prepared_shaderc_spirv_to_naga_wgsl`: the prepared SPIR-V written
   as WGSL by Naga.
7. `normalized_spirv_through_naga`: prepared SPIR-V normalized by `spirv-opt`,
   then parsed, validated, reflected, and written as WGSL. Only this lane feeds
   the pair-level pipeline audit.

Vulkan preparation deliberately has a small, visible contract: it changes the
first GLSL version directive to `#version 460`, replaces the first
`uniform mat4 uTransform;` with `uniform WrLocals { mat4 uTransform; };`,
targets Vulkan 1.1, and enables shaderc auto binding and location mapping.

## Required normalization

The normalized lane is deliberately opt-in. Set
`WEBRENDER_SHADER_CENSUS_SPIRV_OPT` to the exact `spirv-opt` executable you
want recorded in the receipt. It runs these actions:

```text
--target-env=vulkan1.1
--split-combined-image-sampler
--resolve-binding-conflicts
```

Without the variable, the lane is reported as `unavailable` with
`spirv_normalizer_not_configured`; that is not a translation success. The
manifest records the executable path, its `--version` output when available,
and the requested actions.

From the workspace root, run the complete receipt with a known normalizer:

```sh
WEBRENDER_SHADER_CENSUS_SPIRV_OPT=/absolute/path/to/spirv-opt \
  cargo run -p webrender_build --features shader_census --bin shader-census
```

Save its standard output as a JSON receipt if desired. Do not configure the
variable to an empty string or a missing executable: that only records an
unavailable or execution-failed normalization lane.

The focused Vulkan WGPU smoke receipt is:

```sh
WEBRENDER_SHADER_CENSUS_SPIRV_OPT=/absolute/path/to/spirv-opt \
  cargo run -p webrender_build --features shader_census_wgpu_smoke \
  --bin shader-census-wgpu-smoke
```

It requires a Vulkan adapter and may report `adapter_unavailable` or
`device_request_failed` on a host that cannot provide one. It should be read
as a pipeline-creation probe for the normalized `ps_clear` pair, never as a
rendering test.

## Reading a receipt

The top-level `toolchain` gives the dependency versions, target platform,
preparation contract, and normalizer provenance. Each profile has:

- `records`: one JSON record per shader stage. Each lane has `succeeded`,
  `failed`, or `unavailable`, optional source/intermediate/output digests, a
  stable `failure_class`, and a bounded diagnostic.
- `summary`: aggregate stage-lane counts and failure classes. Use this for
  comparison; use the corresponding record for a fixable diagnostic.
- `pipeline_audits`: one result for each vertex/fragment pair, derived only
  from the normalized lane. `pipeline_compatible` means the reflected resource
  bindings and inter-stage locations agree under the audit's checks.
  `stage_translation_failed` means a normalized stage did not complete.
  `incompatible` names collisions, incompatible bindings, missing vertex
  outputs, or incompatible inter-stage locations in `issues`.

`pipeline_compatible` still does not prove WGPU accepts a concrete vertex
layout, target format, feature set, or runtime command stream. Conversely, a
WGPU smoke failure should be treated as device/pipeline evidence for its one
pair, not as a failure of the whole renderer.

## Tests

The census module has deterministic JSON, corpus-size, preparation-contract,
normalizer-provenance, and pair-reconciliation unit tests. Run the focused
gate with:

```sh
cargo test -p webrender_build --features shader_census shader_census
```

The command is a harness check, not a measured corpus receipt. Produce and
retain the JSON receipt separately whenever comparing toolchain versions or
translation behavior.
