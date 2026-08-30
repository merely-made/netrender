/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! Deterministic inventory records for WebRender's generated shader variants.
//!
//! This module intentionally stops after preprocessing. It records four
//! candidate translation lanes so a later tool can fill them in without
//! changing the inventory schema or treating an unimplemented route as a pass.

use std::borrow::Cow;
#[cfg(feature = "shader_census")]
use std::io::Write;

use sha2::{Digest, Sha256};

use crate::shader::{build_shader_strings, ShaderKind, ShaderVersion};
use crate::shader_features::{get_shader_features, ShaderFeatureFlags};

const NAGA_VERSION: &str = "30.0.1";
const SHADERC_VERSION: &str = "0.10.1";

/// The fixed order in which every variant reports translation routes.
pub const SHADER_TRANSLATION_LANES: [ShaderTranslationLane; 5] = [
    ShaderTranslationLane::AssembledGlsl,
    ShaderTranslationLane::NagaGlslFrontend,
    ShaderTranslationLane::RawShadercSpirvNagaValidated,
    ShaderTranslationLane::ShadercSpirvToNagaWgsl,
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
    /// Apply a declared SPIR-V normalization pass before passing it into Naga.
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
    /// The assembled source plus the three candidates and normalization diagnostic.
    pub lanes: [ShaderTranslationLaneRecord; 5],
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
    pub summary: ShaderCensusSummary,
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
    pub normalization_action: String,
    pub target_os: String,
    pub target_arch: String,
}

impl ShaderCensusManifest {
    /// Create a deterministically ordered manifest from explicitly requested
    /// profile selections.
    pub fn new(mut profiles: Vec<ShaderCensusProfile>) -> Self {
        profiles.sort_unstable_by_key(|profile| {
            (profile.shader_version, profile.requested_feature_flags_bits)
        });
        ShaderCensusManifest {
            schema_version: 1,
            toolchain: ShaderCensusToolchain {
                naga_version: NAGA_VERSION.to_owned(),
                shaderc_version: SHADERC_VERSION.to_owned(),
                normalization_action: "not_configured".to_owned(),
                target_os: std::env::consts::OS.to_owned(),
                target_arch: std::env::consts::ARCH.to_owned(),
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
    ) -> Self {
        ShaderCensusRecord {
            schema_version: 1,
            shader_name: shader_name.to_owned(),
            shader_version,
            stage,
            features,
            source_sha256: sha256_hex(source),
            lanes: collect_lane_records(shader_name, stage, source),
        }
    }
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
    let mut variants = get_shader_features(flags).into_iter().collect::<Vec<_>>();
    variants.sort_unstable_by_key(|(shader_name, _)| *shader_name);

    let mut records = Vec::new();
    for (shader_name, mut configs) in variants {
        configs.sort_unstable();
        for config in configs {
            let source_features = config
                .split(',')
                .filter(|feature| !feature.is_empty())
                .collect::<Vec<_>>();
            let mut features = source_features
                .iter()
                .map(|feature| (*feature).to_owned())
                .collect::<Vec<_>>();
            features.sort_unstable();

            let (vertex, fragment) =
                build_shader_strings(shader_version, &source_features, shader_name, get_source);
            records.push(ShaderCensusRecord::new(
                shader_name,
                shader_version,
                ShaderKind::Vertex,
                features.clone(),
                &vertex,
            ));
            records.push(ShaderCensusRecord::new(
                shader_name,
                shader_version,
                ShaderKind::Fragment,
                features,
                &fragment,
            ));
        }
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
    let records = collect_shader_census(shader_version, flags, get_source);
    let summary = summarize(&records);
    ShaderCensusProfile {
        shader_version,
        requested_feature_flags_bits: flags.bits(),
        records,
        summary,
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
        failure_class: Some(class.to_owned()),
        diagnostic: Some(bounded_diagnostic(diagnostic)),
    }
}

fn collect_lane_records(
    shader_name: &str,
    stage: ShaderKind,
    source: &str,
) -> [ShaderTranslationLaneRecord; 5] {
    let direct_naga = run_naga_glsl_to_wgsl(stage, source);
    let compiled_spirv = compile_shaderc(stage, shader_name, source);
    let raw_spirv = run_raw_shaderc_spirv(&compiled_spirv);
    let spirv_to_wgsl = run_shaderc_spirv_to_wgsl(&compiled_spirv);
    [
        evidence_lane(source),
        direct_naga,
        raw_spirv,
        spirv_to_wgsl,
        unavailable_lane(
            ShaderTranslationLane::NormalizedSpirvThroughNaga,
            "normalization_not_configured",
            "no SPIR-V normalization action is configured; the raw lane remains unmodified",
        ),
    ]
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
fn run_raw_shaderc_spirv(compiled_spirv: &Result<Vec<u8>, String>) -> ShaderTranslationLaneRecord {
    let lane = ShaderTranslationLane::RawShadercSpirvNagaValidated;
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
fn run_shaderc_spirv_to_wgsl(
    compiled_spirv: &Result<Vec<u8>, String>,
) -> ShaderTranslationLaneRecord {
    let lane = ShaderTranslationLane::ShadercSpirvToNagaWgsl;
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
fn succeeded_lane(lane: ShaderTranslationLane, output: &str) -> ShaderTranslationLaneRecord {
    ShaderTranslationLaneRecord {
        lane,
        status: ShaderTranslationLaneStatus::Succeeded,
        output_sha256: Some(sha256_hex(output)),
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
    naga::back::wgsl::write_string(module, &info, naga::back::wgsl::WriterFlags::empty())
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
    let compiler = shaderc::Compiler::new().map_err(|error| error.to_string())?;
    let mut options = shaderc::CompileOptions::new().map_err(|error| error.to_string())?;
    options.set_target_env(
        shaderc::TargetEnv::Vulkan,
        shaderc::EnvVersion::Vulkan1_1 as u32,
    );
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
                && record.lanes.iter().any(|lane| {
                    lane.lane == ShaderTranslationLane::NormalizedSpirvThroughNaga
                        && lane.status == ShaderTranslationLaneStatus::Unavailable
                })
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
        assert!(json.contains("requested_feature_flags_bits"));
        assert!(json.contains("\"naga_version\": \"30.0.1\""));
        assert!(json.contains("\"shaderc_version\": \"0.10.1\""));
        assert!(json.contains(&format!("\"target_os\": \"{}\"", std::env::consts::OS)));
        assert!(json.contains(&format!("\"target_arch\": \"{}\"", std::env::consts::ARCH)));
    }
}
