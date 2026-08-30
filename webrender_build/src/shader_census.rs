/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! Deterministic inventory records for WebRender's generated shader variants.
//!
//! This module records raw, Vulkan-prepared, and opt-in normalized candidate
//! lanes. It intentionally stops before GPU device and pipeline creation.

use std::borrow::Cow;
#[cfg(feature = "shader_census")]
use std::io::Write;
#[cfg(feature = "shader_census")]
use std::path::{Path, PathBuf};
#[cfg(feature = "shader_census")]
use std::process::Command;
#[cfg(feature = "shader_census")]
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::shader::{build_shader_strings, ShaderKind, ShaderVersion};
use crate::shader_features::{get_shader_features, ShaderFeatureFlags};

const NAGA_VERSION: &str = "30.0.1";
const SHADERC_VERSION: &str = "0.10.1";
const SPIRV_OPT_ENV: &str = "WEBRENDER_SHADER_CENSUS_SPIRV_OPT";
const SPIRV_OPT_ACTIONS: [&str; 3] = [
    "--target-env=vulkan1.1",
    "--split-combined-image-sampler",
    "--resolve-binding-conflicts",
];

#[cfg(feature = "shader_census")]
static TEMPORARY_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The fixed order in which every variant reports translation routes.
pub const SHADER_TRANSLATION_LANES: [ShaderTranslationLane; 7] = [
    ShaderTranslationLane::AssembledGlsl,
    ShaderTranslationLane::NagaGlslFrontend,
    ShaderTranslationLane::RawShadercSpirvNagaValidated,
    ShaderTranslationLane::ShadercSpirvToNagaWgsl,
    ShaderTranslationLane::VulkanPreparedShadercSpirvNagaValidated,
    ShaderTranslationLane::VulkanPreparedShadercSpirvToNagaWgsl,
    ShaderTranslationLane::NormalizedSpirvThroughNaga,
];

/// A shader-processing lane considered by the experimental WGPU backend work.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
#[cfg_attr(feature = "shader_census", serde(rename_all = "snake_case"))]
pub enum ShaderTranslationLane {
    /// The assembled GLSL emitted by WebRender's existing preprocessing path.
    AssembledGlsl,
    /// Parse the assembled GLSL with Naga's GLSL frontend.
    NagaGlslFrontend,
    /// Compile raw GLSL with shaderc, then parse and validate the SPIR-V with
    /// Naga under `Capabilities::all`. Success is not a WebGPU viability claim.
    RawShadercSpirvNagaValidated,
    /// Compile with shaderc, parse SPIR-V with Naga, and emit WGSL.
    ShadercSpirvToNagaWgsl,
    /// Apply the manifest-declared Vulkan preparation, then compile with
    /// shaderc and validate the resulting SPIR-V with Naga.
    VulkanPreparedShadercSpirvNagaValidated,
    /// Apply the manifest-declared Vulkan preparation, then compile with
    /// shaderc, parse with Naga, and emit WGSL.
    VulkanPreparedShadercSpirvToNagaWgsl,
    /// Run the opt-in `spirv-opt` normalization action on prepared SPIR-V,
    /// then parse it with Naga and emit WGSL.
    NormalizedSpirvThroughNaga,
}

/// The current result of a processing lane for one source record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
#[cfg_attr(feature = "shader_census", serde(rename_all = "snake_case"))]
pub enum ShaderTranslationLaneStatus {
    /// The existing WebRender machinery assembled this source and recorded it.
    Succeeded,
    /// The requested toolchain completed and reported a failure for this lane.
    Failed,
    /// The lane has an explicit non-execution reason, such as a deliberately
    /// absent normalization tool. This is not a successful translation.
    Unavailable,
}

/// A processing lane slot associated with one preprocessed shader stage.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderTranslationLaneRecord {
    pub lane: ShaderTranslationLane,
    pub status: ShaderTranslationLaneStatus,
    /// SHA-256 for an emitted WGSL or SPIR-V artifact, when this lane emitted one.
    pub output_sha256: Option<String>,
    /// SHA-256 for a material intermediate artifact, such as normalized SPIR-V.
    pub intermediate_sha256: Option<String>,
    /// A stable, bounded failure category for aggregation.
    pub failure_class: Option<String>,
    /// A bounded diagnostic suitable for a machine-readable receipt.
    pub diagnostic: Option<String>,
}

/// One deterministic, stage-specific WebRender shader source identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusRecord {
    /// Bump only when this record's serialized meaning changes.
    pub schema_version: u32,
    pub shader_name: String,
    pub shader_version: ShaderVersion,
    pub stage: ShaderKind,
    /// Lexically sorted feature set for comparison independent of input order.
    pub features: Vec<String>,
    /// SHA-256 of the exact preprocessed source passed to a route.
    pub source_sha256: String,
    /// The assembled source plus raw, prepared, and normalized candidates.
    pub lanes: [ShaderTranslationLaneRecord; 7],
}

/// Aggregate counts for a profile, including stable failure categories.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusSummary {
    pub record_count: usize,
    pub lane_count: usize,
    pub succeeded_count: usize,
    pub failed_count: usize,
    pub unavailable_count: usize,
    pub failure_classes: Vec<ShaderCensusFailureClass>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusFailureClass {
    pub class: String,
    pub count: usize,
}

/// One requested WebRender shader profile and its resulting stage records.
///
/// `requested_feature_flags_bits` is retained even when a flag does not affect
/// a particular variant. This makes the corpus selection reproducible instead
/// of inferring it from the variants that happened to be emitted.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusProfile {
    pub shader_version: ShaderVersion,
    pub requested_feature_flags_bits: u32,
    pub records: Vec<ShaderCensusRecord>,
    /// Cross-stage evidence for the normalized vertex/fragment pair of every
    /// selected variant. A stage-local translation success alone does not
    /// imply that the pair can form a render pipeline.
    pub pipeline_audits: Vec<ShaderCensusPipelineAudit>,
    pub summary: ShaderCensusSummary,
}

/// The normalized-SPIR-V evidence retained for one shader stage while a pair
/// is being reconciled.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusNormalizedStageEvidence {
    pub source_sha256: String,
    pub intermediate_sha256: Option<String>,
    pub output_sha256: Option<String>,
    pub translation_status: ShaderTranslationLaneStatus,
    pub failure_class: Option<String>,
    pub diagnostic: Option<String>,
    pub entry_point: Option<String>,
    pub resource_bindings: Vec<ShaderCensusResourceBinding>,
    pub interface_locations: Vec<ShaderCensusStageInterfaceLocation>,
}

/// One reflected resource binding, sorted by group and binding number.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusResourceBinding {
    pub group: u32,
    pub binding: u32,
    pub resource_class: String,
    pub address_space: String,
    pub type_layout: String,
}

/// Whether an entry-point location is read by or written from a stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
#[cfg_attr(feature = "shader_census", serde(rename_all = "snake_case"))]
pub enum ShaderCensusStageInterfaceDirection {
    Input,
    Output,
}

/// The numeric type carried by one stage interface location.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
#[cfg_attr(feature = "shader_census", serde(rename_all = "snake_case"))]
pub enum ShaderCensusInterfaceType {
    Scalar {
        scalar_kind: String,
        width: u8,
    },
    Vector {
        size: u8,
        scalar_kind: String,
        width: u8,
    },
    Matrix {
        columns: u8,
        rows: u8,
        scalar_kind: String,
        width: u8,
    },
    Other {
        type_layout: String,
    },
}

/// One explicit user-defined entry-point input or output location.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusStageInterfaceLocation {
    pub stage: ShaderKind,
    pub direction: ShaderCensusStageInterfaceDirection,
    pub location: u32,
    pub interface_type: ShaderCensusInterfaceType,
    pub interpolation: Option<String>,
    pub sampling: Option<String>,
    pub blend_src: Option<u32>,
    pub per_primitive: bool,
}

/// The status of reconciling two individually normalized shader stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
#[cfg_attr(feature = "shader_census", serde(rename_all = "snake_case"))]
pub enum ShaderCensusPipelineAuditStatus {
    StageTranslationFailed,
    Incompatible,
    PipelineCompatible,
}

/// A bind group location shared by one or both stages.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusBindingPoint {
    pub group: u32,
    pub binding: u32,
}

/// The merged view of a bind group location across the vertex and fragment stages.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusReconciledResourceBinding {
    pub binding_point: ShaderCensusBindingPoint,
    pub vertex: Option<ShaderCensusResourceBinding>,
    pub fragment: Option<ShaderCensusResourceBinding>,
    pub compatible: bool,
}

/// A stable class for a collision or incompatibility found while reconciling a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
#[cfg_attr(feature = "shader_census", serde(rename_all = "snake_case"))]
pub enum ShaderCensusPipelineAuditIssueKind {
    VertexResourceBindingCollision,
    FragmentResourceBindingCollision,
    IncompatibleResourceBindingLayout,
    VertexOutputLocationCollision,
    FragmentInputLocationCollision,
    MissingVertexOutput,
    IncompatibleInterfaceLocation,
}

/// A typed, deterministic reconciliation finding.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusPipelineAuditIssue {
    pub kind: ShaderCensusPipelineAuditIssueKind,
    pub binding_point: Option<ShaderCensusBindingPoint>,
    pub interface_location: Option<u32>,
    pub vertex_layout: Option<String>,
    pub fragment_layout: Option<String>,
}

/// The result of reflecting and reconciling one normalized vertex/fragment pair.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusPipelineAudit {
    pub schema_version: u32,
    pub shader_name: String,
    pub shader_version: ShaderVersion,
    pub features: Vec<String>,
    pub vertex: ShaderCensusNormalizedStageEvidence,
    pub fragment: ShaderCensusNormalizedStageEvidence,
    pub reconciled_resource_bindings: Vec<ShaderCensusReconciledResourceBinding>,
    pub issues: Vec<ShaderCensusPipelineAuditIssue>,
    pub status: ShaderCensusPipelineAuditStatus,
}

/// The complete, machine-readable result of one or more requested profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusManifest {
    pub schema_version: u32,
    pub toolchain: ShaderCensusToolchain,
    pub profiles: Vec<ShaderCensusProfile>,
}

/// Exact shader tool versions and intentionally absent transformations.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusToolchain {
    pub naga_version: String,
    pub shaderc_version: String,
    pub target_os: String,
    pub target_arch: String,
    pub vulkan_preparation: ShaderCensusVulkanPreparation,
    pub spirv_normalizer: ShaderCensusSpirvNormalizerProvenance,
}

/// Every source and shaderc adjustment applied only to the prepared-input lanes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusVulkanPreparation {
    pub first_version_directive: String,
    pub transform_uniform_replacement: String,
    pub shaderc_auto_bind_uniforms: bool,
    pub shaderc_auto_map_locations: bool,
    /// The historical donor applied this preparation to desktop GL variants.
    pub gl_donor_reproduction: bool,
    /// Applying the same preparation to GLES is a new census experiment.
    pub gles_experimental: bool,
}

/// Provenance for the opt-in `spirv-opt` action used only by the normalized
/// Vulkan-prepared lanes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "shader_census", derive(Serialize))]
pub struct ShaderCensusSpirvNormalizerProvenance {
    pub executable_path: Option<String>,
    pub version: Option<String>,
    pub version_diagnostic: Option<String>,
    pub actions: Vec<String>,
}

impl ShaderCensusManifest {
    /// Create a deterministically ordered manifest from explicitly requested
    /// profile selections.
    pub fn new(mut profiles: Vec<ShaderCensusProfile>) -> Self {
        profiles.sort_unstable_by_key(|profile| {
            (profile.shader_version, profile.requested_feature_flags_bits)
        });
        ShaderCensusManifest {
            schema_version: 4,
            toolchain: ShaderCensusToolchain {
                naga_version: NAGA_VERSION.to_owned(),
                shaderc_version: SHADERC_VERSION.to_owned(),
                target_os: std::env::consts::OS.to_owned(),
                target_arch: std::env::consts::ARCH.to_owned(),
                vulkan_preparation: ShaderCensusVulkanPreparation {
                    first_version_directive: "#version 460".to_owned(),
                    transform_uniform_replacement: "uniform WrLocals { mat4 uTransform; };"
                        .to_owned(),
                    shaderc_auto_bind_uniforms: true,
                    shaderc_auto_map_locations: true,
                    gl_donor_reproduction: true,
                    gles_experimental: true,
                },
                spirv_normalizer: spirv_normalizer_provenance(),
            },
            profiles,
        }
    }
}

impl ShaderCensusRecord {
    fn new(
        shader_name: &str,
        shader_version: ShaderVersion,
        stage: ShaderKind,
        features: Vec<String>,
        source: &str,
        lanes: [ShaderTranslationLaneRecord; 7],
    ) -> Self {
        ShaderCensusRecord {
            schema_version: 3,
            shader_name: shader_name.to_owned(),
            shader_version,
            stage,
            features,
            source_sha256: sha256_hex(source),
            lanes,
        }
    }
}

#[cfg(feature = "shader_census")]
#[derive(Clone, Debug)]
struct ShaderCensusStageReflection {
    entry_point: String,
    resource_bindings: Vec<ShaderCensusResourceBinding>,
    interface_locations: Vec<ShaderCensusStageInterfaceLocation>,
}

#[cfg(feature = "shader_census")]
#[derive(Clone, Debug)]
struct NormalizedStageArtifact {
    lane: ShaderTranslationLaneRecord,
    reflection: Option<ShaderCensusStageReflection>,
    wgsl: Option<String>,
}

#[cfg(feature = "shader_census")]
#[derive(Clone, Debug)]
struct CollectedShaderStage {
    record: ShaderCensusRecord,
    normalized: NormalizedStageArtifact,
}

#[cfg(feature = "shader_census")]
#[derive(Clone, Debug)]
struct CollectedShaderPair {
    vertex: CollectedShaderStage,
    fragment: CollectedShaderStage,
}

/// A reflected normalized pair together with its WGSL, for a narrow external
/// probe. The manifest serializes only the audit and hashes, not the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderCensusNormalizedShaderPair {
    pub audit: ShaderCensusPipelineAudit,
    pub vertex_wgsl: Option<String>,
    pub fragment_wgsl: Option<String>,
}

/// Enumerate every shader variant selected by `flags` and preprocess both
/// stages through WebRender's normal shader source builder.
///
/// The result is sorted by shader name, source feature configuration, and
/// stage, making it suitable for a checked-in receipt or direct JSON output.
pub fn collect_shader_census<G>(
    shader_version: ShaderVersion,
    flags: ShaderFeatureFlags,
    get_source: &G,
) -> Vec<ShaderCensusRecord>
where
    G: Fn(&str) -> Cow<'static, str>,
{
    let pairs = collect_shader_census_pairs(shader_version, flags, get_source);
    let mut records = Vec::with_capacity(pairs.len() * 2);
    for pair in pairs {
        records.push(pair.vertex.record);
        records.push(pair.fragment.record);
    }
    records
}

/// Collect a named profile while retaining the exact feature flags requested
/// from WebRender's feature enumerator.
pub fn collect_shader_census_profile<G>(
    shader_version: ShaderVersion,
    flags: ShaderFeatureFlags,
    get_source: &G,
) -> ShaderCensusProfile
where
    G: Fn(&str) -> Cow<'static, str>,
{
    let pairs = collect_shader_census_pairs(shader_version, flags, get_source);
    let mut records = Vec::with_capacity(pairs.len() * 2);
    let mut pipeline_audits = Vec::with_capacity(pairs.len());
    for pair in pairs {
        pipeline_audits.push(reconcile_normalized_pair(&pair));
        records.push(pair.vertex.record);
        records.push(pair.fragment.record);
    }
    let summary = summarize(&records);
    ShaderCensusProfile {
        shader_version,
        requested_feature_flags_bits: flags.bits(),
        records,
        pipeline_audits,
        summary,
    }
}

/// Collect one explicitly selected pair through the normalized route. This is
/// intentionally limited to a single pair so a headless WGPU probe cannot
/// accidentally grow into a renderer path.
pub fn collect_normalized_shader_pair<G>(
    shader_version: ShaderVersion,
    shader_name: &str,
    features: &[&str],
    get_source: &G,
) -> ShaderCensusNormalizedShaderPair
where
    G: Fn(&str) -> Cow<'static, str>,
{
    let pair = collect_shader_pair(shader_version, shader_name, features, get_source);
    ShaderCensusNormalizedShaderPair {
        audit: reconcile_normalized_pair(&pair),
        vertex_wgsl: pair.vertex.normalized.wgsl,
        fragment_wgsl: pair.fragment.normalized.wgsl,
    }
}

fn collect_shader_census_pairs<G>(
    shader_version: ShaderVersion,
    flags: ShaderFeatureFlags,
    get_source: &G,
) -> Vec<CollectedShaderPair>
where
    G: Fn(&str) -> Cow<'static, str>,
{
    let mut variants = get_shader_features(flags).into_iter().collect::<Vec<_>>();
    variants.sort_unstable_by_key(|(shader_name, _)| *shader_name);

    let mut pairs = Vec::new();
    for (shader_name, mut configs) in variants {
        configs.sort_unstable();
        for config in configs {
            let source_features = config
                .split(',')
                .filter(|feature| !feature.is_empty())
                .collect::<Vec<_>>();
            pairs.push(collect_shader_pair(
                shader_version,
                shader_name,
                &source_features,
                get_source,
            ));
        }
    }
    pairs
}

fn collect_shader_pair<G>(
    shader_version: ShaderVersion,
    shader_name: &str,
    source_features: &[&str],
    get_source: &G,
) -> CollectedShaderPair
where
    G: Fn(&str) -> Cow<'static, str>,
{
    let mut features = source_features
        .iter()
        .map(|feature| (*feature).to_owned())
        .collect::<Vec<_>>();
    features.sort_unstable();
    let (vertex_source, fragment_source) =
        build_shader_strings(shader_version, source_features, shader_name, get_source);
    CollectedShaderPair {
        vertex: collect_shader_stage(
            shader_name,
            shader_version,
            ShaderKind::Vertex,
            features.clone(),
            &vertex_source,
        ),
        fragment: collect_shader_stage(
            shader_name,
            shader_version,
            ShaderKind::Fragment,
            features,
            &fragment_source,
        ),
    }
}

fn collect_shader_stage(
    shader_name: &str,
    shader_version: ShaderVersion,
    stage: ShaderKind,
    features: Vec<String>,
    source: &str,
) -> CollectedShaderStage {
    let (lanes, normalized) = collect_lane_records(shader_name, stage, source);
    CollectedShaderStage {
        record: ShaderCensusRecord::new(
            shader_name,
            shader_version,
            stage,
            features,
            source,
            lanes,
        ),
        normalized,
    }
}

/// Write the typed census records as a stable JSON array.
#[cfg(feature = "shader_census")]
pub fn write_shader_census_json<W: Write>(
    manifest: &ShaderCensusManifest,
    output: W,
) -> Result<(), serde_json::Error> {
    serde_json::to_writer_pretty(output, manifest)
}

fn sha256_hex(source: &str) -> String {
    sha256_bytes(source.as_bytes())
}

fn sha256_bytes(source: &[u8]) -> String {
    let digest = Sha256::digest(source);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{:02x}", byte).expect("writing to String cannot fail");
    }
    encoded
}

fn evidence_lane(source: &str) -> ShaderTranslationLaneRecord {
    ShaderTranslationLaneRecord {
        lane: ShaderTranslationLane::AssembledGlsl,
        status: ShaderTranslationLaneStatus::Succeeded,
        output_sha256: Some(sha256_hex(source)),
        intermediate_sha256: None,
        failure_class: None,
        diagnostic: None,
    }
}

fn unavailable_lane(
    lane: ShaderTranslationLane,
    class: &str,
    diagnostic: &str,
) -> ShaderTranslationLaneRecord {
    ShaderTranslationLaneRecord {
        lane,
        status: ShaderTranslationLaneStatus::Unavailable,
        output_sha256: None,
        intermediate_sha256: None,
        failure_class: Some(class.to_owned()),
        diagnostic: Some(bounded_diagnostic(diagnostic)),
    }
}

fn collect_lane_records(
    shader_name: &str,
    stage: ShaderKind,
    source: &str,
) -> ([ShaderTranslationLaneRecord; 7], NormalizedStageArtifact) {
    let direct_naga = run_naga_glsl_to_wgsl(stage, source);
    let compiled_spirv = compile_shaderc(stage, shader_name, source);
    let raw_spirv = run_shaderc_spirv_naga_validated(
        ShaderTranslationLane::RawShadercSpirvNagaValidated,
        &compiled_spirv,
    );
    let spirv_to_wgsl = run_shaderc_spirv_to_naga_wgsl(
        ShaderTranslationLane::ShadercSpirvToNagaWgsl,
        &compiled_spirv,
    );
    let prepared_source = prepare_vulkan_glsl(source);
    let prepared_spirv = compile_vulkan_prepared_shaderc(stage, shader_name, &prepared_source);
    let prepared_validation = run_shaderc_spirv_naga_validated(
        ShaderTranslationLane::VulkanPreparedShadercSpirvNagaValidated,
        &prepared_spirv,
    );
    let prepared_wgsl = run_shaderc_spirv_to_naga_wgsl(
        ShaderTranslationLane::VulkanPreparedShadercSpirvToNagaWgsl,
        &prepared_spirv,
    );
    let normalized = run_normalized_prepared_spirv(stage, &prepared_spirv);
    let lanes = [
        evidence_lane(source),
        direct_naga,
        raw_spirv,
        spirv_to_wgsl,
        prepared_validation,
        prepared_wgsl,
        normalized.lane.clone(),
    ];
    (lanes, normalized)
}

#[cfg(feature = "shader_census")]
fn run_naga_glsl_to_wgsl(stage: ShaderKind, source: &str) -> ShaderTranslationLaneRecord {
    let lane = ShaderTranslationLane::NagaGlslFrontend;
    let module = match parse_naga_glsl(stage, source) {
        Ok(module) => module,
        Err(diagnostic) => return failed_lane(lane, "naga_glsl_parse", diagnostic),
    };
    match validate_and_write_wgsl(&module) {
        Ok(wgsl) => succeeded_lane(lane, &wgsl),
        Err((class, diagnostic)) => failed_lane(lane, &class, diagnostic),
    }
}

#[cfg(feature = "shader_census")]
fn run_shaderc_spirv_naga_validated(
    lane: ShaderTranslationLane,
    compiled_spirv: &Result<Vec<u8>, String>,
) -> ShaderTranslationLaneRecord {
    let spirv = match compiled_spirv {
        Ok(spirv) => spirv,
        Err(diagnostic) => return failed_lane(lane, "shaderc_compile", diagnostic.clone()),
    };
    match parse_and_validate_spirv(&spirv) {
        Ok(()) => succeeded_lane_bytes(lane, &spirv),
        Err((class, diagnostic)) => failed_lane(lane, &class, diagnostic),
    }
}

#[cfg(feature = "shader_census")]
fn run_shaderc_spirv_to_naga_wgsl(
    lane: ShaderTranslationLane,
    compiled_spirv: &Result<Vec<u8>, String>,
) -> ShaderTranslationLaneRecord {
    let spirv = match compiled_spirv {
        Ok(spirv) => spirv,
        Err(diagnostic) => return failed_lane(lane, "shaderc_compile", diagnostic.clone()),
    };
    let module = match parse_naga_spirv(&spirv) {
        Ok(module) => module,
        Err(diagnostic) => return failed_lane(lane, "naga_spirv_parse", diagnostic),
    };
    match validate_and_write_wgsl(&module) {
        Ok(wgsl) => succeeded_lane(lane, &wgsl),
        Err((class, diagnostic)) => failed_lane(lane, &class, diagnostic),
    }
}

#[cfg(feature = "shader_census")]
fn run_normalized_prepared_spirv(
    stage: ShaderKind,
    prepared_spirv: &Result<Vec<u8>, String>,
) -> NormalizedStageArtifact {
    let lane = ShaderTranslationLane::NormalizedSpirvThroughNaga;
    let normalizer = match std::env::var_os(SPIRV_OPT_ENV) {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => {
            return NormalizedStageArtifact {
                lane: unavailable_lane(
                    lane,
                    "spirv_normalizer_not_configured",
                    "set WEBRENDER_SHADER_CENSUS_SPIRV_OPT to an executable spirv-opt path",
                ),
                reflection: None,
                wgsl: None,
            };
        }
    };
    let prepared_spirv = match prepared_spirv {
        Ok(spirv) => spirv,
        Err(diagnostic) => {
            return NormalizedStageArtifact {
                lane: failed_lane(lane, "shaderc_compile", diagnostic.clone()),
                reflection: None,
                wgsl: None,
            };
        }
    };
    let normalized_spirv = match normalize_prepared_spirv(&normalizer, prepared_spirv) {
        Ok(spirv) => spirv,
        Err(diagnostic) => {
            return NormalizedStageArtifact {
                lane: failed_lane(lane, "spirv_normalizer_execute", diagnostic),
                reflection: None,
                wgsl: None,
            };
        }
    };
    let intermediate_sha256 = sha256_bytes(&normalized_spirv);
    let module = match parse_naga_spirv(&normalized_spirv) {
        Ok(module) => module,
        Err(diagnostic) => {
            let mut lane = failed_lane(lane, "naga_spirv_parse", diagnostic);
            lane.intermediate_sha256 = Some(intermediate_sha256);
            return NormalizedStageArtifact {
                lane,
                reflection: None,
                wgsl: None,
            };
        }
    };
    let info = match validate_module(&module) {
        Ok(info) => info,
        Err((class, diagnostic)) => {
            let mut lane = failed_lane(lane, &class, diagnostic);
            lane.intermediate_sha256 = Some(intermediate_sha256);
            return NormalizedStageArtifact {
                lane,
                reflection: None,
                wgsl: None,
            };
        }
    };
    let reflection = match reflect_normalized_stage(&module, &info, stage) {
        Ok(reflection) => reflection,
        Err((class, diagnostic)) => {
            let mut lane = failed_lane(lane, &class, diagnostic);
            lane.intermediate_sha256 = Some(intermediate_sha256);
            return NormalizedStageArtifact {
                lane,
                reflection: None,
                wgsl: None,
            };
        }
    };
    let (mut lane, wgsl) = match write_wgsl(&module, &info) {
        Ok(wgsl) => (succeeded_lane(lane, &wgsl), Some(wgsl)),
        Err((class, diagnostic)) => (failed_lane(lane, &class, diagnostic), None),
    };
    lane.intermediate_sha256 = Some(intermediate_sha256);
    NormalizedStageArtifact {
        lane,
        reflection: Some(reflection),
        wgsl,
    }
}

#[cfg(feature = "shader_census")]
fn normalize_prepared_spirv(normalizer: &Path, input: &[u8]) -> Result<Vec<u8>, String> {
    let files = TemporarySpirvFiles::new()?;
    std::fs::write(&files.input, input).map_err(|error| error.to_string())?;
    let output = Command::new(normalizer)
        .args(SPIRV_OPT_ACTIONS)
        .arg("-o")
        .arg(&files.output)
        .arg(&files.input)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "spirv-opt exited {}: {}",
            output.status,
            command_diagnostic(&output.stdout, &output.stderr),
        ));
    }
    std::fs::read(&files.output).map_err(|error| error.to_string())
}

#[cfg(feature = "shader_census")]
fn reflect_normalized_stage(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: ShaderKind,
) -> Result<ShaderCensusStageReflection, (String, String)> {
    let (entry_point_index, entry_point) = module
        .entry_points
        .iter()
        .enumerate()
        .find(|(_, entry_point)| entry_point.stage == naga_stage(stage))
        .ok_or_else(|| {
            (
                "naga_entry_point_reflection".to_owned(),
                format!("no {:?} entry point in normalized SPIR-V module", stage),
            )
        })?;
    let entry_point_info = info.get_entry_point(entry_point_index);

    let mut resource_bindings = module
        .global_variables
        .iter()
        .filter_map(|(handle, variable)| {
            if entry_point_info[handle].is_empty() {
                return None;
            }
            variable.binding.as_ref().map(|binding| {
                let sampled_by_sampler = entry_point_info
                    .sampling_set
                    .iter()
                    .any(|key| key.image == handle);
                ShaderCensusResourceBinding {
                    group: binding.group,
                    binding: binding.binding,
                    resource_class: reflected_resource_class(module, variable, sampled_by_sampler),
                    address_space: format!("{:?}", variable.space),
                    type_layout: reflected_type_layout(module, variable.ty),
                }
            })
        })
        .collect::<Vec<_>>();
    resource_bindings.sort_unstable();

    let mut interface_locations = Vec::new();
    for argument in &entry_point.function.arguments {
        reflect_interface_locations(
            module,
            stage,
            ShaderCensusStageInterfaceDirection::Input,
            argument.ty,
            argument.binding.as_ref(),
            &mut interface_locations,
        );
    }
    if let Some(result) = &entry_point.function.result {
        reflect_interface_locations(
            module,
            stage,
            ShaderCensusStageInterfaceDirection::Output,
            result.ty,
            result.binding.as_ref(),
            &mut interface_locations,
        );
    }
    interface_locations.sort_unstable();

    Ok(ShaderCensusStageReflection {
        entry_point: entry_point.name.clone(),
        resource_bindings,
        interface_locations,
    })
}

#[cfg(feature = "shader_census")]
fn reflected_resource_class(
    module: &naga::Module,
    variable: &naga::GlobalVariable,
    sampled_by_sampler: bool,
) -> String {
    match (&variable.space, &module.types[variable.ty].inner) {
        (naga::AddressSpace::Uniform, _) => "uniform_buffer".to_owned(),
        (naga::AddressSpace::Storage { access }, _) => format!("storage_buffer:{:?}", access),
        (naga::AddressSpace::Handle, naga::TypeInner::Sampler { comparison }) => {
            format!("sampler:comparison={}", comparison)
        }
        (
            naga::AddressSpace::Handle,
            naga::TypeInner::Image {
                dim,
                arrayed,
                class,
            },
        ) => match class {
            naga::ImageClass::Sampled {
                kind: naga::ScalarKind::Float,
                ..
            } => format!(
                "texture:{:?}:arrayed={}:class={:?}:filterable={}",
                dim, arrayed, class, sampled_by_sampler
            ),
            _ => format!("texture:{:?}:arrayed={}:class={:?}", dim, arrayed, class),
        },
        (naga::AddressSpace::Handle, naga::TypeInner::BindingArray { base, .. }) => {
            format!(
                "binding_array:{}",
                reflected_resource_class_for_type(module, *base, sampled_by_sampler)
            )
        }
        (space, type_inner) => format!("other:{:?}:{:?}", space, type_inner),
    }
}

#[cfg(feature = "shader_census")]
fn reflected_resource_class_for_type(
    module: &naga::Module,
    ty: naga::Handle<naga::Type>,
    sampled_by_sampler: bool,
) -> String {
    match &module.types[ty].inner {
        naga::TypeInner::Sampler { comparison } => format!("sampler:comparison={}", comparison),
        naga::TypeInner::Image {
            dim,
            arrayed,
            class,
        } => match class {
            naga::ImageClass::Sampled {
                kind: naga::ScalarKind::Float,
                ..
            } => format!(
                "texture:{:?}:arrayed={}:class={:?}:filterable={}",
                dim, arrayed, class, sampled_by_sampler
            ),
            _ => format!("texture:{:?}:arrayed={}:class={:?}", dim, arrayed, class),
        },
        naga::TypeInner::BindingArray { base, .. } => format!(
            "binding_array:{}",
            reflected_resource_class_for_type(module, *base, sampled_by_sampler)
        ),
        other => format!("other:{:?}", other),
    }
}

#[cfg(feature = "shader_census")]
fn reflected_type_layout(module: &naga::Module, ty: naga::Handle<naga::Type>) -> String {
    match &module.types[ty].inner {
        naga::TypeInner::Scalar(scalar) => {
            format!("scalar:{:?}:{}", scalar.kind, scalar.width)
        }
        naga::TypeInner::Vector { size, scalar } => {
            format!("vector:{:?}:{:?}:{}", size, scalar.kind, scalar.width)
        }
        naga::TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => format!(
            "matrix:{:?}:{:?}:{:?}:{}",
            columns, rows, scalar.kind, scalar.width
        ),
        naga::TypeInner::Atomic(scalar) => {
            format!("atomic:{:?}:{}", scalar.kind, scalar.width)
        }
        naga::TypeInner::Pointer { base, space } => format!(
            "pointer:{:?}:{}",
            space,
            reflected_type_layout(module, *base)
        ),
        naga::TypeInner::ValuePointer {
            size,
            scalar,
            space,
        } => format!(
            "value_pointer:{:?}:{:?}:{:?}:{}",
            space, size, scalar.kind, scalar.width
        ),
        naga::TypeInner::Array { base, size, stride } => format!(
            "array:{}:{}:{}",
            reflected_array_size(*size),
            stride,
            reflected_type_layout(module, *base)
        ),
        naga::TypeInner::Struct { members, span } => format!(
            "struct:{}:[{}]",
            span,
            members
                .iter()
                .map(|member| format!(
                    "{}:{}",
                    member.offset,
                    reflected_type_layout(module, member.ty)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        naga::TypeInner::Image {
            dim,
            arrayed,
            class,
        } => format!("image:{:?}:{}:{:?}", dim, arrayed, class),
        naga::TypeInner::Sampler { comparison } => format!("sampler:{}", comparison),
        naga::TypeInner::BindingArray { base, size } => format!(
            "binding_array:{}:{}",
            reflected_array_size(*size),
            reflected_type_layout(module, *base)
        ),
        other => format!("other:{:?}", other),
    }
}

#[cfg(feature = "shader_census")]
fn reflected_array_size(size: naga::ArraySize) -> String {
    match size {
        naga::ArraySize::Constant(count) => count.get().to_string(),
        naga::ArraySize::Dynamic => "dynamic".to_owned(),
        naga::ArraySize::Pending(_) => "pending".to_owned(),
    }
}

#[cfg(feature = "shader_census")]
fn reflect_interface_locations(
    module: &naga::Module,
    stage: ShaderKind,
    direction: ShaderCensusStageInterfaceDirection,
    ty: naga::Handle<naga::Type>,
    binding: Option<&naga::Binding>,
    locations: &mut Vec<ShaderCensusStageInterfaceLocation>,
) {
    if let Some(naga::Binding::Location {
        location,
        interpolation,
        sampling,
        blend_src,
        per_primitive,
    }) = binding
    {
        locations.push(ShaderCensusStageInterfaceLocation {
            stage,
            direction,
            location: *location,
            interface_type: reflected_interface_type(module, ty),
            interpolation: interpolation.map(|value| format!("{:?}", value)),
            sampling: sampling.map(|value| format!("{:?}", value)),
            blend_src: *blend_src,
            per_primitive: *per_primitive,
        });
        return;
    }

    if binding.is_none() {
        if let naga::TypeInner::Struct { members, .. } = &module.types[ty].inner {
            for member in members {
                reflect_interface_locations(
                    module,
                    stage,
                    direction,
                    member.ty,
                    member.binding.as_ref(),
                    locations,
                );
            }
        }
    }
}

#[cfg(feature = "shader_census")]
fn reflected_interface_type(
    module: &naga::Module,
    ty: naga::Handle<naga::Type>,
) -> ShaderCensusInterfaceType {
    match &module.types[ty].inner {
        naga::TypeInner::Scalar(scalar) => ShaderCensusInterfaceType::Scalar {
            scalar_kind: format!("{:?}", scalar.kind),
            width: scalar.width,
        },
        naga::TypeInner::Vector { size, scalar } => ShaderCensusInterfaceType::Vector {
            size: reflected_vector_size(*size),
            scalar_kind: format!("{:?}", scalar.kind),
            width: scalar.width,
        },
        naga::TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => ShaderCensusInterfaceType::Matrix {
            columns: reflected_vector_size(*columns),
            rows: reflected_vector_size(*rows),
            scalar_kind: format!("{:?}", scalar.kind),
            width: scalar.width,
        },
        _ => ShaderCensusInterfaceType::Other {
            type_layout: reflected_type_layout(module, ty),
        },
    }
}

#[cfg(feature = "shader_census")]
fn reflected_vector_size(size: naga::VectorSize) -> u8 {
    match size {
        naga::VectorSize::Bi => 2,
        naga::VectorSize::Tri => 3,
        naga::VectorSize::Quad => 4,
    }
}

#[cfg(feature = "shader_census")]
fn normalized_stage_evidence(stage: &CollectedShaderStage) -> ShaderCensusNormalizedStageEvidence {
    let lane = &stage.normalized.lane;
    let reflection = stage.normalized.reflection.as_ref();
    ShaderCensusNormalizedStageEvidence {
        source_sha256: stage.record.source_sha256.clone(),
        intermediate_sha256: lane.intermediate_sha256.clone(),
        output_sha256: lane.output_sha256.clone(),
        translation_status: lane.status,
        failure_class: lane.failure_class.clone(),
        diagnostic: lane.diagnostic.clone(),
        entry_point: reflection.map(|reflection| reflection.entry_point.clone()),
        resource_bindings: reflection
            .map(|reflection| reflection.resource_bindings.clone())
            .unwrap_or_default(),
        interface_locations: reflection
            .map(|reflection| reflection.interface_locations.clone())
            .unwrap_or_default(),
    }
}

#[cfg(feature = "shader_census")]
fn reconcile_normalized_pair(pair: &CollectedShaderPair) -> ShaderCensusPipelineAudit {
    use std::collections::BTreeSet;

    let vertex = normalized_stage_evidence(&pair.vertex);
    let fragment = normalized_stage_evidence(&pair.fragment);
    let mut issues = Vec::new();
    let vertex_bindings = unique_resource_bindings(
        &vertex.resource_bindings,
        ShaderCensusPipelineAuditIssueKind::VertexResourceBindingCollision,
        &mut issues,
    );
    let fragment_bindings = unique_resource_bindings(
        &fragment.resource_bindings,
        ShaderCensusPipelineAuditIssueKind::FragmentResourceBindingCollision,
        &mut issues,
    );
    let binding_points = vertex_bindings
        .keys()
        .chain(fragment_bindings.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut reconciled_resource_bindings = Vec::with_capacity(binding_points.len());
    for binding_point in binding_points {
        let vertex_binding = vertex_bindings.get(&binding_point).cloned();
        let fragment_binding = fragment_bindings.get(&binding_point).cloned();
        let compatible = match (&vertex_binding, &fragment_binding) {
            (Some(vertex), Some(fragment)) => resource_bindings_match(vertex, fragment),
            _ => true,
        };
        if !compatible {
            issues.push(ShaderCensusPipelineAuditIssue {
                kind: ShaderCensusPipelineAuditIssueKind::IncompatibleResourceBindingLayout,
                binding_point: Some(binding_point.clone()),
                interface_location: None,
                vertex_layout: vertex_binding.as_ref().map(resource_binding_layout),
                fragment_layout: fragment_binding.as_ref().map(resource_binding_layout),
            });
        }
        reconciled_resource_bindings.push(ShaderCensusReconciledResourceBinding {
            binding_point,
            vertex: vertex_binding,
            fragment: fragment_binding,
            compatible,
        });
    }

    let vertex_outputs = unique_interface_locations(
        &vertex.interface_locations,
        ShaderCensusStageInterfaceDirection::Output,
        ShaderCensusPipelineAuditIssueKind::VertexOutputLocationCollision,
        &mut issues,
    );
    let fragment_inputs = unique_interface_locations(
        &fragment.interface_locations,
        ShaderCensusStageInterfaceDirection::Input,
        ShaderCensusPipelineAuditIssueKind::FragmentInputLocationCollision,
        &mut issues,
    );
    for (location, fragment_input) in fragment_inputs {
        match vertex_outputs.get(&location) {
            None => issues.push(ShaderCensusPipelineAuditIssue {
                kind: ShaderCensusPipelineAuditIssueKind::MissingVertexOutput,
                binding_point: None,
                interface_location: Some(location),
                vertex_layout: None,
                fragment_layout: Some(interface_location_layout(&fragment_input)),
            }),
            Some(vertex_output) if !interface_locations_match(vertex_output, &fragment_input) => {
                issues.push(ShaderCensusPipelineAuditIssue {
                    kind: ShaderCensusPipelineAuditIssueKind::IncompatibleInterfaceLocation,
                    binding_point: None,
                    interface_location: Some(location),
                    vertex_layout: Some(interface_location_layout(vertex_output)),
                    fragment_layout: Some(interface_location_layout(&fragment_input)),
                });
            }
            Some(_) => {}
        }
    }

    issues.sort_unstable();
    let status = if vertex.translation_status != ShaderTranslationLaneStatus::Succeeded
        || fragment.translation_status != ShaderTranslationLaneStatus::Succeeded
    {
        ShaderCensusPipelineAuditStatus::StageTranslationFailed
    } else if issues.is_empty() {
        ShaderCensusPipelineAuditStatus::PipelineCompatible
    } else {
        ShaderCensusPipelineAuditStatus::Incompatible
    };

    ShaderCensusPipelineAudit {
        schema_version: 1,
        shader_name: pair.vertex.record.shader_name.clone(),
        shader_version: pair.vertex.record.shader_version,
        features: pair.vertex.record.features.clone(),
        vertex,
        fragment,
        reconciled_resource_bindings,
        issues,
        status,
    }
}

#[cfg(feature = "shader_census")]
fn unique_resource_bindings(
    bindings: &[ShaderCensusResourceBinding],
    collision_kind: ShaderCensusPipelineAuditIssueKind,
    issues: &mut Vec<ShaderCensusPipelineAuditIssue>,
) -> std::collections::BTreeMap<ShaderCensusBindingPoint, ShaderCensusResourceBinding> {
    use std::collections::btree_map::Entry;

    let mut unique = std::collections::BTreeMap::new();
    for binding in bindings {
        let binding_point = ShaderCensusBindingPoint {
            group: binding.group,
            binding: binding.binding,
        };
        match unique.entry(binding_point.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(binding.clone());
            }
            Entry::Occupied(_) => issues.push(ShaderCensusPipelineAuditIssue {
                kind: collision_kind,
                binding_point: Some(binding_point),
                interface_location: None,
                vertex_layout: None,
                fragment_layout: None,
            }),
        }
    }
    unique
}

#[cfg(feature = "shader_census")]
fn unique_interface_locations(
    locations: &[ShaderCensusStageInterfaceLocation],
    direction: ShaderCensusStageInterfaceDirection,
    collision_kind: ShaderCensusPipelineAuditIssueKind,
    issues: &mut Vec<ShaderCensusPipelineAuditIssue>,
) -> std::collections::BTreeMap<u32, ShaderCensusStageInterfaceLocation> {
    use std::collections::btree_map::Entry;

    let mut unique = std::collections::BTreeMap::new();
    for interface_location in locations
        .iter()
        .filter(|location| location.direction == direction)
    {
        match unique.entry(interface_location.location) {
            Entry::Vacant(entry) => {
                entry.insert(interface_location.clone());
            }
            Entry::Occupied(_) => issues.push(ShaderCensusPipelineAuditIssue {
                kind: collision_kind,
                binding_point: None,
                interface_location: Some(interface_location.location),
                vertex_layout: None,
                fragment_layout: None,
            }),
        }
    }
    unique
}

#[cfg(feature = "shader_census")]
fn resource_bindings_match(
    vertex: &ShaderCensusResourceBinding,
    fragment: &ShaderCensusResourceBinding,
) -> bool {
    vertex.resource_class == fragment.resource_class
        && vertex.address_space == fragment.address_space
        && vertex.type_layout == fragment.type_layout
}

#[cfg(feature = "shader_census")]
fn resource_binding_layout(binding: &ShaderCensusResourceBinding) -> String {
    format!(
        "{}:{}:{}",
        binding.resource_class, binding.address_space, binding.type_layout
    )
}

#[cfg(feature = "shader_census")]
fn interface_locations_match(
    vertex: &ShaderCensusStageInterfaceLocation,
    fragment: &ShaderCensusStageInterfaceLocation,
) -> bool {
    interface_type_is_subtype_of(&fragment.interface_type, &vertex.interface_type)
        && vertex.interpolation == fragment.interpolation
        && vertex.sampling == fragment.sampling
        && vertex.blend_src == fragment.blend_src
        && vertex.per_primitive == fragment.per_primitive
}

/// The WGPU inter-stage rule permits a fragment input to consume a numeric
/// scalar, vector, or matrix no wider than the vertex output at that location.
#[cfg(feature = "shader_census")]
fn interface_type_is_subtype_of(
    fragment: &ShaderCensusInterfaceType,
    vertex: &ShaderCensusInterfaceType,
) -> bool {
    match (fragment, vertex) {
        (
            ShaderCensusInterfaceType::Scalar {
                scalar_kind: fragment_kind,
                width: fragment_width,
            },
            ShaderCensusInterfaceType::Scalar {
                scalar_kind: vertex_kind,
                width: vertex_width,
            },
        ) => fragment_kind == vertex_kind && fragment_width <= vertex_width,
        (
            ShaderCensusInterfaceType::Scalar {
                scalar_kind: fragment_kind,
                width: fragment_width,
            },
            ShaderCensusInterfaceType::Vector {
                scalar_kind: vertex_kind,
                width: vertex_width,
                ..
            },
        ) => fragment_kind == vertex_kind && fragment_width <= vertex_width,
        (
            ShaderCensusInterfaceType::Vector {
                size: fragment_size,
                scalar_kind: fragment_kind,
                width: fragment_width,
            },
            ShaderCensusInterfaceType::Vector {
                size: vertex_size,
                scalar_kind: vertex_kind,
                width: vertex_width,
            },
        ) => {
            fragment_kind == vertex_kind
                && fragment_width <= vertex_width
                && fragment_size <= vertex_size
        }
        (
            ShaderCensusInterfaceType::Matrix {
                columns: fragment_columns,
                rows: fragment_rows,
                scalar_kind: fragment_kind,
                width: fragment_width,
            },
            ShaderCensusInterfaceType::Matrix {
                columns: vertex_columns,
                rows: vertex_rows,
                scalar_kind: vertex_kind,
                width: vertex_width,
            },
        ) => {
            fragment_kind == vertex_kind
                && fragment_width <= vertex_width
                && fragment_columns == vertex_columns
                && fragment_rows == vertex_rows
        }
        (ShaderCensusInterfaceType::Other { .. }, ShaderCensusInterfaceType::Other { .. }) => {
            fragment == vertex
        }
        _ => false,
    }
}

#[cfg(feature = "shader_census")]
fn interface_location_layout(location: &ShaderCensusStageInterfaceLocation) -> String {
    format!(
        "{:?}:{:?}:{:?}:{:?}:{}",
        location.interface_type,
        location.interpolation,
        location.sampling,
        location.blend_src,
        location.per_primitive
    )
}

#[cfg(feature = "shader_census")]
struct TemporarySpirvFiles {
    directory: PathBuf,
    input: PathBuf,
    output: PathBuf,
}

#[cfg(feature = "shader_census")]
impl TemporarySpirvFiles {
    fn new() -> Result<Self, String> {
        for _ in 0..32 {
            let counter = TEMPORARY_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "webrender-shader-census-{}-{}",
                std::process::id(),
                counter,
            ));
            match std::fs::create_dir(&directory) {
                Ok(()) => {
                    return Ok(TemporarySpirvFiles {
                        input: directory.join("input.spv"),
                        output: directory.join("output.spv"),
                        directory,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("could not allocate a temporary SPIR-V directory".to_owned())
    }
}

#[cfg(feature = "shader_census")]
impl Drop for TemporarySpirvFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.input);
        let _ = std::fs::remove_file(&self.output);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[cfg(feature = "shader_census")]
fn succeeded_lane(lane: ShaderTranslationLane, output: &str) -> ShaderTranslationLaneRecord {
    ShaderTranslationLaneRecord {
        lane,
        status: ShaderTranslationLaneStatus::Succeeded,
        output_sha256: Some(sha256_hex(output)),
        intermediate_sha256: None,
        failure_class: None,
        diagnostic: None,
    }
}

#[cfg(feature = "shader_census")]
fn succeeded_lane_bytes(lane: ShaderTranslationLane, output: &[u8]) -> ShaderTranslationLaneRecord {
    ShaderTranslationLaneRecord {
        lane,
        status: ShaderTranslationLaneStatus::Succeeded,
        output_sha256: Some(sha256_bytes(output)),
        intermediate_sha256: None,
        failure_class: None,
        diagnostic: None,
    }
}

#[cfg(feature = "shader_census")]
fn failed_lane(
    lane: ShaderTranslationLane,
    class: &str,
    diagnostic: String,
) -> ShaderTranslationLaneRecord {
    ShaderTranslationLaneRecord {
        lane,
        status: ShaderTranslationLaneStatus::Failed,
        output_sha256: None,
        intermediate_sha256: None,
        failure_class: Some(class.to_owned()),
        diagnostic: Some(bounded_diagnostic(&diagnostic)),
    }
}

#[cfg(feature = "shader_census")]
fn parse_naga_glsl(stage: ShaderKind, source: &str) -> Result<naga::Module, String> {
    let mut frontend = naga::front::glsl::Frontend::default();
    frontend
        .parse(&naga::front::glsl::Options::from(naga_stage(stage)), source)
        .map_err(|error| error.to_string())
}

#[cfg(feature = "shader_census")]
fn parse_naga_spirv(source: &[u8]) -> Result<naga::Module, String> {
    naga::front::spv::parse_u8_slice(source, &naga::front::spv::Options::default())
        .map_err(|error| error.to_string())
}

#[cfg(feature = "shader_census")]
fn parse_and_validate_spirv(source: &[u8]) -> Result<(), (String, String)> {
    let module = parse_naga_spirv(source)
        .map_err(|diagnostic| ("naga_spirv_parse".to_owned(), diagnostic))?;
    validate_module(&module).map(|_| ())
}

#[cfg(feature = "shader_census")]
fn validate_and_write_wgsl(module: &naga::Module) -> Result<String, (String, String)> {
    let info = validate_module(module)?;
    write_wgsl(module, &info)
}

#[cfg(feature = "shader_census")]
fn write_wgsl(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
) -> Result<String, (String, String)> {
    naga::back::wgsl::write_string(module, info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|error| ("naga_wgsl_write".to_owned(), error.to_string()))
}

#[cfg(feature = "shader_census")]
fn validate_module(module: &naga::Module) -> Result<naga::valid::ModuleInfo, (String, String)> {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    .map_err(|error| ("naga_validate".to_owned(), error.to_string()))
}

#[cfg(feature = "shader_census")]
fn compile_shaderc(stage: ShaderKind, shader_name: &str, source: &str) -> Result<Vec<u8>, String> {
    compile_shaderc_with_options(stage, shader_name, source, false)
}

#[cfg(feature = "shader_census")]
fn compile_vulkan_prepared_shaderc(
    stage: ShaderKind,
    shader_name: &str,
    source: &str,
) -> Result<Vec<u8>, String> {
    compile_shaderc_with_options(stage, shader_name, source, true)
}

#[cfg(feature = "shader_census")]
fn compile_shaderc_with_options(
    stage: ShaderKind,
    shader_name: &str,
    source: &str,
    use_vulkan_preparation: bool,
) -> Result<Vec<u8>, String> {
    let compiler = shaderc::Compiler::new().map_err(|error| error.to_string())?;
    let mut options = shaderc::CompileOptions::new().map_err(|error| error.to_string())?;
    options.set_target_env(
        shaderc::TargetEnv::Vulkan,
        shaderc::EnvVersion::Vulkan1_1 as u32,
    );
    if use_vulkan_preparation {
        options.set_auto_bind_uniforms(true);
        options.set_auto_map_locations(true);
    }
    let artifact = compiler
        .compile_into_spirv(
            source,
            shaderc_kind(stage),
            shader_name,
            "main",
            Some(&options),
        )
        .map_err(|error| error.to_string())?;
    Ok(artifact.as_binary_u8().to_vec())
}

/// Apply only the declared source edits for the prepared Vulkan lanes.
fn prepare_vulkan_glsl(source: &str) -> String {
    let mut prepared = String::with_capacity(source.len());
    let mut replaced_version = false;
    for line in source.split_inclusive('\n') {
        if !replaced_version && line.trim_start().starts_with("#version") {
            prepared.push_str("#version 460");
            if line.ends_with('\n') {
                prepared.push('\n');
            }
            replaced_version = true;
        } else {
            prepared.push_str(line);
        }
    }
    prepared.replacen(
        "uniform mat4 uTransform;",
        "uniform WrLocals { mat4 uTransform; };",
        1,
    )
}

#[cfg(feature = "shader_census")]
fn shaderc_kind(stage: ShaderKind) -> shaderc::ShaderKind {
    match stage {
        ShaderKind::Vertex => shaderc::ShaderKind::Vertex,
        ShaderKind::Fragment => shaderc::ShaderKind::Fragment,
    }
}

#[cfg(feature = "shader_census")]
fn naga_stage(stage: ShaderKind) -> naga::ShaderStage {
    match stage {
        ShaderKind::Vertex => naga::ShaderStage::Vertex,
        ShaderKind::Fragment => naga::ShaderStage::Fragment,
    }
}

fn summarize(records: &[ShaderCensusRecord]) -> ShaderCensusSummary {
    use std::collections::BTreeMap;

    let mut succeeded_count = 0;
    let mut failed_count = 0;
    let mut unavailable_count = 0;
    let mut failure_classes = BTreeMap::new();
    for lane in records.iter().flat_map(|record| &record.lanes) {
        match lane.status {
            ShaderTranslationLaneStatus::Succeeded => succeeded_count += 1,
            ShaderTranslationLaneStatus::Failed => failed_count += 1,
            ShaderTranslationLaneStatus::Unavailable => unavailable_count += 1,
        }
        if let Some(class) = &lane.failure_class {
            *failure_classes.entry(class.clone()).or_insert(0) += 1;
        }
    }
    ShaderCensusSummary {
        record_count: records.len(),
        lane_count: records.len() * SHADER_TRANSLATION_LANES.len(),
        succeeded_count,
        failed_count,
        unavailable_count,
        failure_classes: failure_classes
            .into_iter()
            .map(|(class, count)| ShaderCensusFailureClass { class, count })
            .collect(),
    }
}

fn bounded_diagnostic(diagnostic: &str) -> String {
    diagnostic.chars().take(240).collect()
}

#[cfg(feature = "shader_census")]
fn spirv_normalizer_provenance() -> ShaderCensusSpirvNormalizerProvenance {
    let executable_path = std::env::var_os(SPIRV_OPT_ENV).map(PathBuf::from);
    let (version, version_diagnostic) = match executable_path.as_deref() {
        None => (None, None),
        Some(path) => match Command::new(path).arg("--version").output() {
            Ok(output) if output.status.success() => (
                Some(command_diagnostic(&output.stdout, &output.stderr)),
                None,
            ),
            Ok(output) => (
                None,
                Some(bounded_diagnostic(&format!(
                    "spirv-opt --version exited {}: {}",
                    output.status,
                    command_diagnostic(&output.stdout, &output.stderr),
                ))),
            ),
            Err(error) => (None, Some(bounded_diagnostic(&error.to_string()))),
        },
    };
    ShaderCensusSpirvNormalizerProvenance {
        executable_path: executable_path.map(|path| path.display().to_string()),
        version,
        version_diagnostic,
        actions: SPIRV_OPT_ACTIONS
            .iter()
            .map(|action| (*action).to_owned())
            .collect(),
    }
}

#[cfg(feature = "shader_census")]
fn command_diagnostic(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    bounded_diagnostic(&format!("{}{}", stdout.trim(), stderr.trim()))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    fn test_source(_: &str) -> Cow<'static, str> {
        Cow::Borrowed("void main() {}\n")
    }

    #[test]
    fn census_enumerates_existing_variants_in_a_stable_order() {
        let records =
            collect_shader_census(ShaderVersion::Gl, ShaderFeatureFlags::empty(), &test_source);
        let expected_variant_count = get_shader_features(ShaderFeatureFlags::empty())
            .values()
            .map(Vec::len)
            .sum::<usize>();

        assert_eq!(records.len(), expected_variant_count * 2);
        assert!(records.windows(2).all(|pair| {
            (
                &pair[0].shader_name,
                pair[0].shader_version,
                &pair[0].features,
                pair[0].stage,
            ) <= (
                &pair[1].shader_name,
                pair[1].shader_version,
                &pair[1].features,
                pair[1].stage,
            )
        }));
        assert!(records
            .iter()
            .all(|record| record.source_sha256.len() == 64));
        assert!(records.iter().all(|record| {
            record
                .lanes
                .iter()
                .any(|lane| lane.status == ShaderTranslationLaneStatus::Succeeded)
        }));
        assert!(records.iter().all(|record| {
            record.lanes.len() == SHADER_TRANSLATION_LANES.len()
                && record
                    .lanes
                    .iter()
                    .any(|lane| lane.lane == ShaderTranslationLane::NormalizedSpirvThroughNaga)
        }));
    }

    #[test]
    fn census_digest_tracks_the_preprocessed_stage_source() {
        let vertex =
            collect_shader_census(ShaderVersion::Gl, ShaderFeatureFlags::empty(), &test_source)
                .into_iter()
                .find(|record| record.stage == ShaderKind::Vertex)
                .unwrap();
        let fragment =
            collect_shader_census(ShaderVersion::Gl, ShaderFeatureFlags::empty(), &test_source)
                .into_iter()
                .find(|record| record.stage == ShaderKind::Fragment)
                .unwrap();

        assert_ne!(vertex.source_sha256, fragment.source_sha256);
        assert!(vertex
            .source_sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn vulkan_preparation_applies_only_the_declared_source_edits() {
        let source = "#version 150\nlayout(location = 0) in vec2 aPosition;\nuniform mat4 uTransform;\nvoid main() {}\n";

        assert_eq!(
            prepare_vulkan_glsl(source),
            "#version 460\nlayout(location = 0) in vec2 aPosition;\nuniform WrLocals { mat4 uTransform; };\nvoid main() {}\n"
        );
    }

    #[test]
    fn vulkan_preparation_leaves_other_source_text_unchanged() {
        let source =
            "#version 300 es\nprecision mediump float;\nuniform vec4 untouched;\nvoid main() {}";

        assert_eq!(
            prepare_vulkan_glsl(source),
            "#version 460\nprecision mediump float;\nuniform vec4 untouched;\nvoid main() {}"
        );
    }

    #[test]
    fn lane_order_includes_raw_and_prepared_candidates() {
        let records =
            collect_shader_census(ShaderVersion::Gl, ShaderFeatureFlags::empty(), &test_source);
        let actual = records[0]
            .lanes
            .iter()
            .map(|lane| lane.lane.clone())
            .collect::<Vec<_>>();

        assert_eq!(actual, SHADER_TRANSLATION_LANES.to_vec());
    }

    #[test]
    fn normalizer_provenance_declares_the_exact_opt_in_actions() {
        let provenance = spirv_normalizer_provenance();

        assert_eq!(
            provenance.actions,
            vec![
                "--target-env=vulkan1.1".to_owned(),
                "--split-combined-image-sampler".to_owned(),
                "--resolve-binding-conflicts".to_owned(),
            ]
        );
    }

    #[test]
    fn temporary_spirv_files_are_removed_on_drop() {
        let directory;
        {
            let files = TemporarySpirvFiles::new().unwrap();
            directory = files.directory.clone();
            std::fs::write(&files.input, b"input").unwrap();
            std::fs::write(&files.output, b"output").unwrap();
            assert!(directory.exists());
        }
        assert!(!directory.exists());
    }

    #[test]
    fn profile_preserves_the_exact_requested_flags() {
        let flags = ShaderFeatureFlags::GL | ShaderFeatureFlags::DEBUG;
        let profile = collect_shader_census_profile(ShaderVersion::Gl, flags, &test_source);

        assert_eq!(profile.requested_feature_flags_bits, flags.bits());
        assert!(profile
            .records
            .iter()
            .any(|record| record.shader_name == "debug_color"));
        assert_eq!(
            profile.summary.lane_count,
            profile.records.len() * SHADER_TRANSLATION_LANES.len()
        );
        assert_eq!(
            profile.summary.succeeded_count
                + profile.summary.failed_count
                + profile.summary.unavailable_count,
            profile.summary.lane_count
        );
    }

    #[test]
    fn profile_has_one_sorted_pipeline_audit_for_each_stage_pair() {
        let profile = collect_shader_census_profile(
            ShaderVersion::Gl,
            ShaderFeatureFlags::empty(),
            &test_source,
        );

        assert_eq!(profile.pipeline_audits.len(), profile.records.len() / 2);
        assert!(profile.pipeline_audits.windows(2).all(|pair| {
            (
                &pair[0].shader_name,
                pair[0].shader_version,
                &pair[0].features,
            ) <= (
                &pair[1].shader_name,
                pair[1].shader_version,
                &pair[1].features,
            )
        }));
        assert!(profile.pipeline_audits.iter().all(|audit| {
            audit.status == ShaderCensusPipelineAuditStatus::StageTranslationFailed
                && audit.vertex.source_sha256.len() == 64
                && audit.fragment.source_sha256.len() == 64
        }));
    }

    #[test]
    fn reconciliation_matches_wgpu_resources_and_stage_locations() {
        let vertex = test_collected_stage(
            ShaderKind::Vertex,
            vec![test_resource(0, 0, "uniform_buffer", "layout_a")],
            vec![test_location(
                ShaderKind::Vertex,
                ShaderCensusStageInterfaceDirection::Output,
                0,
                4,
            )],
        );
        let fragment = test_collected_stage(
            ShaderKind::Fragment,
            vec![test_resource(0, 0, "uniform_buffer", "layout_a")],
            vec![test_location(
                ShaderKind::Fragment,
                ShaderCensusStageInterfaceDirection::Input,
                0,
                3,
            )],
        );
        let compatible = reconcile_normalized_pair(&CollectedShaderPair { vertex, fragment });

        assert_eq!(
            compatible.status,
            ShaderCensusPipelineAuditStatus::PipelineCompatible
        );
        assert!(compatible.issues.is_empty());
        assert_eq!(compatible.reconciled_resource_bindings.len(), 1);

        let vertex = test_collected_stage(
            ShaderKind::Vertex,
            vec![
                test_resource(0, 0, "uniform_buffer", "layout_a"),
                test_resource(0, 0, "storage_buffer", "layout_b"),
            ],
            vec![
                test_location(
                    ShaderKind::Vertex,
                    ShaderCensusStageInterfaceDirection::Output,
                    0,
                    3,
                ),
                test_location(
                    ShaderKind::Vertex,
                    ShaderCensusStageInterfaceDirection::Output,
                    0,
                    3,
                ),
            ],
        );
        let fragment = test_collected_stage(
            ShaderKind::Fragment,
            vec![test_resource(0, 0, "uniform_buffer", "layout_c")],
            vec![
                test_location(
                    ShaderKind::Fragment,
                    ShaderCensusStageInterfaceDirection::Input,
                    0,
                    4,
                ),
                test_location(
                    ShaderKind::Fragment,
                    ShaderCensusStageInterfaceDirection::Input,
                    1,
                    4,
                ),
            ],
        );
        let incompatible = reconcile_normalized_pair(&CollectedShaderPair { vertex, fragment });
        let issue_kinds = incompatible
            .issues
            .iter()
            .map(|issue| issue.kind)
            .collect::<Vec<_>>();

        assert_eq!(
            incompatible.status,
            ShaderCensusPipelineAuditStatus::Incompatible
        );
        assert!(issue_kinds
            .contains(&ShaderCensusPipelineAuditIssueKind::VertexResourceBindingCollision));
        assert!(issue_kinds
            .contains(&ShaderCensusPipelineAuditIssueKind::IncompatibleResourceBindingLayout));
        assert!(issue_kinds
            .contains(&ShaderCensusPipelineAuditIssueKind::VertexOutputLocationCollision));
        assert!(issue_kinds
            .contains(&ShaderCensusPipelineAuditIssueKind::IncompatibleInterfaceLocation));
        assert!(issue_kinds.contains(&ShaderCensusPipelineAuditIssueKind::MissingVertexOutput));
    }

    #[test]
    fn interstage_numeric_subtyping_matches_wgpu() {
        let fragment_vector = ShaderCensusInterfaceType::Vector {
            size: 3,
            scalar_kind: "Float".to_owned(),
            width: 4,
        };
        let vertex_vector = ShaderCensusInterfaceType::Vector {
            size: 4,
            scalar_kind: "Float".to_owned(),
            width: 4,
        };
        assert!(interface_type_is_subtype_of(
            &fragment_vector,
            &vertex_vector
        ));
        assert!(!interface_type_is_subtype_of(
            &vertex_vector,
            &fragment_vector
        ));

        let fragment_matrix = ShaderCensusInterfaceType::Matrix {
            columns: 3,
            rows: 2,
            scalar_kind: "Float".to_owned(),
            width: 4,
        };
        let vertex_matrix = ShaderCensusInterfaceType::Matrix {
            columns: 3,
            rows: 2,
            scalar_kind: "Float".to_owned(),
            width: 4,
        };
        assert!(interface_type_is_subtype_of(
            &fragment_matrix,
            &vertex_matrix
        ));
    }

    fn test_collected_stage(
        stage: ShaderKind,
        resource_bindings: Vec<ShaderCensusResourceBinding>,
        interface_locations: Vec<ShaderCensusStageInterfaceLocation>,
    ) -> CollectedShaderStage {
        let lane = ShaderTranslationLaneRecord {
            lane: ShaderTranslationLane::NormalizedSpirvThroughNaga,
            status: ShaderTranslationLaneStatus::Succeeded,
            output_sha256: Some("c".repeat(64)),
            intermediate_sha256: Some("b".repeat(64)),
            failure_class: None,
            diagnostic: None,
        };
        CollectedShaderStage {
            record: ShaderCensusRecord {
                schema_version: 3,
                shader_name: "test_pair".to_owned(),
                shader_version: ShaderVersion::Gl,
                stage,
                features: Vec::new(),
                source_sha256: "a".repeat(64),
                lanes: [
                    lane.clone(),
                    lane.clone(),
                    lane.clone(),
                    lane.clone(),
                    lane.clone(),
                    lane.clone(),
                    lane.clone(),
                ],
            },
            normalized: NormalizedStageArtifact {
                lane,
                reflection: Some(ShaderCensusStageReflection {
                    entry_point: "main".to_owned(),
                    resource_bindings,
                    interface_locations,
                }),
                wgsl: Some("@vertex fn main() {}".to_owned()),
            },
        }
    }

    fn test_resource(
        group: u32,
        binding: u32,
        resource_class: &str,
        type_layout: &str,
    ) -> ShaderCensusResourceBinding {
        ShaderCensusResourceBinding {
            group,
            binding,
            resource_class: resource_class.to_owned(),
            address_space: "Uniform".to_owned(),
            type_layout: type_layout.to_owned(),
        }
    }

    fn test_location(
        stage: ShaderKind,
        direction: ShaderCensusStageInterfaceDirection,
        location: u32,
        size: u8,
    ) -> ShaderCensusStageInterfaceLocation {
        ShaderCensusStageInterfaceLocation {
            stage,
            direction,
            location,
            interface_type: ShaderCensusInterfaceType::Vector {
                size,
                scalar_kind: "Float".to_owned(),
                width: 4,
            },
            interpolation: Some("Perspective".to_owned()),
            sampling: Some("Center".to_owned()),
            blend_src: None,
            per_primitive: false,
        }
    }

    #[test]
    fn complete_gl_and_gles_corpus_has_434_stage_records() {
        let gl_records =
            collect_shader_census(ShaderVersion::Gl, ShaderFeatureFlags::all(), &test_source);
        let mut gles_flags = ShaderFeatureFlags::all();
        gles_flags.remove(ShaderFeatureFlags::GL);
        let gles_records = collect_shader_census(ShaderVersion::Gles, gles_flags, &test_source);

        assert_eq!(gl_records.len(), 236);
        assert_eq!(gles_records.len(), 198);
        assert_eq!(gl_records.len() + gles_records.len(), 434);
    }

    #[cfg(feature = "shader_census")]
    #[test]
    fn json_output_is_deterministic() {
        let manifest = ShaderCensusManifest::new(vec![collect_shader_census_profile(
            ShaderVersion::Gles,
            ShaderFeatureFlags::empty(),
            &test_source,
        )]);
        let mut first = Vec::new();
        let mut second = Vec::new();
        write_shader_census_json(&manifest, &mut first).unwrap();
        write_shader_census_json(&manifest, &mut second).unwrap();

        assert_eq!(first, second);
        let json = String::from_utf8(first).unwrap();
        assert!(json.contains("raw_shaderc_spirv_naga_validated"));
        assert!(json.contains("vulkan_prepared_shaderc_spirv_naga_validated"));
        assert!(json.contains("vulkan_prepared_shaderc_spirv_to_naga_wgsl"));
        assert!(json.contains("requested_feature_flags_bits"));
        assert!(json.contains("\"naga_version\": \"30.0.1\""));
        assert!(json.contains("\"shaderc_version\": \"0.10.1\""));
        assert!(json.contains(&format!("\"target_os\": \"{}\"", std::env::consts::OS)));
        assert!(json.contains(&format!("\"target_arch\": \"{}\"", std::env::consts::ARCH)));
        assert!(json.contains("\"first_version_directive\": \"#version 460\""));
        assert!(json.contains("\"shaderc_auto_bind_uniforms\": true"));
        assert!(json.contains("\"shaderc_auto_map_locations\": true"));
        assert!(json.contains("\"gl_donor_reproduction\": true"));
        assert!(json.contains("\"gles_experimental\": true"));
        assert!(json.contains("\"spirv_normalizer\""));
        assert!(json.contains("--target-env=vulkan1.1"));
        assert!(json.contains("--split-combined-image-sampler"));
        assert!(json.contains("--resolve-binding-conflicts"));
    }
}
