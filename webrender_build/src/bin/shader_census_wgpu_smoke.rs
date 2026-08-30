/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! A deliberately narrow, headless WGPU proof for the normalized `ps_clear`
//! pair. It does not create a renderer device abstraction or submit a draw.

use std::borrow::Cow;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::Serialize;
use webrender_build::shader::{shader_source_from_file, ShaderVersion};
use webrender_build::shader_census::{
    collect_normalized_shader_pair, ShaderCensusInterfaceType, ShaderCensusManifest,
    ShaderCensusPipelineAudit, ShaderCensusPipelineAuditStatus,
    ShaderCensusStageInterfaceDirection, ShaderCensusToolchain,
};

const WGPU_VERSION: &str = "30.0.0";

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum WgpuSmokeStatus {
    AuditNotPipelineCompatible,
    UnsupportedVertexInputLayout,
    AdapterUnavailable,
    DeviceRequestFailed,
    ShaderModuleValidationFailed,
    RenderPipelineValidationFailed,
    PipelineCreated,
}

#[derive(Serialize)]
struct WgpuSmokeFailure {
    class: String,
    diagnostic: String,
}

#[derive(Serialize)]
struct WgpuAdapterEvidence {
    name: String,
    vendor: u32,
    device: u32,
    device_type: String,
    driver: String,
    driver_info: String,
    backend: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum WgpuOperation {
    VertexShaderModule,
    FragmentShaderModule,
    RenderPipeline,
}

#[derive(Serialize)]
struct WgpuCompilationMessage {
    message_type: String,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum WgpuErrorClass {
    OutOfMemory,
    Validation,
    Internal,
}

#[derive(Serialize)]
struct WgpuCapturedError {
    class: WgpuErrorClass,
    diagnostic: String,
}

#[derive(Serialize)]
struct WgpuOperationEvidence {
    operation: WgpuOperation,
    validation_error: Option<WgpuCapturedError>,
    compilation_messages: Vec<WgpuCompilationMessage>,
}

#[derive(Serialize)]
struct WgpuSmokeToolchain {
    wgpu_version: String,
    required_backend: String,
    census_toolchain: ShaderCensusToolchain,
}

#[derive(Serialize)]
struct WgpuSmokeReceipt {
    schema_version: u32,
    shader_name: String,
    shader_version: ShaderVersion,
    requested_features: Vec<String>,
    toolchain: WgpuSmokeToolchain,
    audit: ShaderCensusPipelineAudit,
    adapter: Option<WgpuAdapterEvidence>,
    operations: Vec<WgpuOperationEvidence>,
    status: WgpuSmokeStatus,
    failure: Option<WgpuSmokeFailure>,
}

fn main() {
    let receipt = run_smoke();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &receipt).expect("serializing WGPU smoke receipt");
    output.write_all(b"\n").expect("writing WGPU smoke receipt");
}

fn run_smoke() -> WgpuSmokeReceipt {
    let shader_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../webrender/res");
    let pair = collect_normalized_shader_pair(ShaderVersion::Gl, "ps_clear", &[], &|name| {
        Cow::Owned(shader_source_from_file(
            &shader_dir.join(format!("{}.glsl", name)),
        ))
    });
    let audit = pair.audit;
    let toolchain = WgpuSmokeToolchain {
        wgpu_version: WGPU_VERSION.to_owned(),
        required_backend: "vulkan".to_owned(),
        census_toolchain: ShaderCensusManifest::new(Vec::new()).toolchain,
    };
    let mut receipt = WgpuSmokeReceipt {
        schema_version: 1,
        shader_name: audit.shader_name.clone(),
        shader_version: audit.shader_version,
        requested_features: audit.features.clone(),
        toolchain,
        audit,
        adapter: None,
        operations: Vec::new(),
        status: WgpuSmokeStatus::AuditNotPipelineCompatible,
        failure: None,
    };

    if receipt.audit.status != ShaderCensusPipelineAuditStatus::PipelineCompatible {
        receipt.failure = Some(WgpuSmokeFailure {
            class: "audit_not_pipeline_compatible".to_owned(),
            diagnostic: format!("normalized pair status: {:?}", receipt.audit.status),
        });
        return receipt;
    }

    let (attributes, array_stride) = match reflected_vertex_attributes(&receipt.audit) {
        Ok(layout) => layout,
        Err(diagnostic) => {
            receipt.status = WgpuSmokeStatus::UnsupportedVertexInputLayout;
            receipt.failure = Some(WgpuSmokeFailure {
                class: "unsupported_vertex_input_layout".to_owned(),
                diagnostic,
            });
            return receipt;
        }
    };
    let vertex_wgsl = match pair.vertex_wgsl {
        Some(wgsl) => wgsl,
        None => {
            receipt.failure = Some(WgpuSmokeFailure {
                class: "missing_vertex_wgsl".to_owned(),
                diagnostic: "pipeline-compatible audit did not retain vertex WGSL".to_owned(),
            });
            return receipt;
        }
    };
    let fragment_wgsl = match pair.fragment_wgsl {
        Some(wgsl) => wgsl,
        None => {
            receipt.failure = Some(WgpuSmokeFailure {
                class: "missing_fragment_wgsl".to_owned(),
                diagnostic: "pipeline-compatible audit did not retain fragment WGSL".to_owned(),
            });
            return receipt;
        }
    };

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    })) {
        Ok(adapter) => adapter,
        Err(error) => {
            receipt.status = WgpuSmokeStatus::AdapterUnavailable;
            receipt.failure = Some(WgpuSmokeFailure {
                class: "vulkan_adapter_unavailable".to_owned(),
                diagnostic: error.to_string(),
            });
            return receipt;
        }
    };
    let adapter_info = adapter.get_info();
    receipt.adapter = Some(WgpuAdapterEvidence {
        name: adapter_info.name,
        vendor: adapter_info.vendor,
        device: adapter_info.device,
        device_type: format!("{:?}", adapter_info.device_type),
        driver: adapter_info.driver,
        driver_info: adapter_info.driver_info,
        backend: format!("{:?}", adapter_info.backend),
    });

    let (device, _) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("webrender-census-ps-clear-smoke"),
        ..Default::default()
    })) {
        Ok(device) => device,
        Err(error) => {
            receipt.status = WgpuSmokeStatus::DeviceRequestFailed;
            receipt.failure = Some(WgpuSmokeFailure {
                class: "vulkan_device_request_failed".to_owned(),
                diagnostic: error.to_string(),
            });
            return receipt;
        }
    };

    let vertex_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let vertex_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("webrender-census-ps-clear-vertex"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(&vertex_wgsl)),
    });
    let vertex_validation_error = pollster::block_on(vertex_scope.pop()).map(capture_wgpu_error);
    receipt.operations.push(shader_module_evidence(
        WgpuOperation::VertexShaderModule,
        &vertex_module,
        vertex_validation_error,
    ));

    let fragment_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let fragment_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("webrender-census-ps-clear-fragment"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(&fragment_wgsl)),
    });
    let fragment_validation_error =
        pollster::block_on(fragment_scope.pop()).map(capture_wgpu_error);
    receipt.operations.push(shader_module_evidence(
        WgpuOperation::FragmentShaderModule,
        &fragment_module,
        fragment_validation_error,
    ));
    if receipt
        .operations
        .iter()
        .any(|operation| operation.validation_error.is_some())
    {
        receipt.status = WgpuSmokeStatus::ShaderModuleValidationFailed;
        receipt.failure = Some(WgpuSmokeFailure {
            class: "shader_module_validation".to_owned(),
            diagnostic: "one or more shader modules emitted a WGPU validation error".to_owned(),
        });
        return receipt;
    }

    let vertex_buffers = [Some(wgpu::VertexBufferLayout {
        array_stride,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    })];
    let color_targets = [Some(wgpu::ColorTargetState {
        format: wgpu::TextureFormat::Rgba8Unorm,
        blend: Some(wgpu::BlendState::REPLACE),
        write_mask: wgpu::ColorWrites::ALL,
    })];
    let vertex_entry_point = receipt
        .audit
        .vertex
        .entry_point
        .as_deref()
        .expect("pipeline-compatible vertex audit has an entry point");
    let fragment_entry_point = receipt
        .audit
        .fragment
        .entry_point
        .as_deref()
        .expect("pipeline-compatible fragment audit has an entry point");
    let pipeline_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("webrender-census-ps-clear-pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &vertex_module,
            entry_point: Some(vertex_entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &vertex_buffers,
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fragment_module,
            entry_point: Some(fragment_entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &color_targets,
        }),
        multiview_mask: None,
        cache: None,
    });
    let pipeline_validation_error =
        pollster::block_on(pipeline_scope.pop()).map(capture_wgpu_error);
    receipt.operations.push(WgpuOperationEvidence {
        operation: WgpuOperation::RenderPipeline,
        validation_error: pipeline_validation_error,
        compilation_messages: Vec::new(),
    });
    if receipt
        .operations
        .last()
        .and_then(|operation| operation.validation_error.as_ref())
        .is_some()
    {
        receipt.status = WgpuSmokeStatus::RenderPipelineValidationFailed;
        receipt.failure = Some(WgpuSmokeFailure {
            class: "render_pipeline_validation".to_owned(),
            diagnostic: "render-pipeline creation emitted a WGPU validation error".to_owned(),
        });
        return receipt;
    }

    receipt.status = WgpuSmokeStatus::PipelineCreated;
    receipt
}

fn shader_module_evidence(
    operation: WgpuOperation,
    module: &wgpu::ShaderModule,
    validation_error: Option<WgpuCapturedError>,
) -> WgpuOperationEvidence {
    let compilation_messages = pollster::block_on(module.get_compilation_info())
        .messages
        .into_iter()
        .map(|message| WgpuCompilationMessage {
            message_type: format!("{:?}", message.message_type),
            message: message.message,
        })
        .collect();
    WgpuOperationEvidence {
        operation,
        validation_error,
        compilation_messages,
    }
}

fn capture_wgpu_error(error: wgpu::Error) -> WgpuCapturedError {
    let class = match &error {
        wgpu::Error::OutOfMemory { .. } => WgpuErrorClass::OutOfMemory,
        wgpu::Error::Validation { .. } => WgpuErrorClass::Validation,
        wgpu::Error::Internal { .. } => WgpuErrorClass::Internal,
    };
    WgpuCapturedError {
        class,
        diagnostic: error.to_string(),
    }
}

fn reflected_vertex_attributes(
    audit: &ShaderCensusPipelineAudit,
) -> Result<(Vec<wgpu::VertexAttribute>, wgpu::BufferAddress), String> {
    let mut inputs = audit
        .vertex
        .interface_locations
        .iter()
        .filter(|location| location.direction == ShaderCensusStageInterfaceDirection::Input)
        .collect::<Vec<_>>();
    inputs.sort_unstable_by_key(|location| location.location);
    if inputs.is_empty() {
        return Err("the vertex stage exposes no location-bound inputs".to_owned());
    }

    let mut attributes = Vec::with_capacity(inputs.len());
    let mut offset = 0;
    let mut previous_location = None;
    for input in inputs {
        if previous_location == Some(input.location) {
            return Err(format!(
                "duplicate vertex input location {}",
                input.location
            ));
        }
        let (format, size) = wgpu_vertex_format(&input.interface_type)?;
        attributes.push(wgpu::VertexAttribute {
            format,
            offset,
            shader_location: input.location,
        });
        offset += size;
        previous_location = Some(input.location);
    }
    Ok((attributes, offset))
}

fn wgpu_vertex_format(
    interface_type: &ShaderCensusInterfaceType,
) -> Result<(wgpu::VertexFormat, u64), String> {
    let (size, scalar_kind, width) = match interface_type {
        ShaderCensusInterfaceType::Scalar { scalar_kind, width } => {
            (1, scalar_kind.as_str(), *width)
        }
        ShaderCensusInterfaceType::Vector {
            size,
            scalar_kind,
            width,
        } => (*size, scalar_kind.as_str(), *width),
        ShaderCensusInterfaceType::Matrix { .. } => {
            return Err("matrix vertex inputs are outside this narrow probe".to_owned());
        }
        ShaderCensusInterfaceType::Other { type_layout } => {
            return Err(format!("non-vertex-format input {type_layout}"));
        }
    };
    let format = match (scalar_kind, width, size) {
        ("Float", 4, 1) => wgpu::VertexFormat::Float32,
        ("Float", 4, 2) => wgpu::VertexFormat::Float32x2,
        ("Float", 4, 3) => wgpu::VertexFormat::Float32x3,
        ("Float", 4, 4) => wgpu::VertexFormat::Float32x4,
        ("Sint", 4, 1) => wgpu::VertexFormat::Sint32,
        ("Sint", 4, 2) => wgpu::VertexFormat::Sint32x2,
        ("Sint", 4, 3) => wgpu::VertexFormat::Sint32x3,
        ("Sint", 4, 4) => wgpu::VertexFormat::Sint32x4,
        ("Uint", 4, 1) => wgpu::VertexFormat::Uint32,
        ("Uint", 4, 2) => wgpu::VertexFormat::Uint32x2,
        ("Uint", 4, 3) => wgpu::VertexFormat::Uint32x3,
        ("Uint", 4, 4) => wgpu::VertexFormat::Uint32x4,
        _ => {
            return Err(format!(
                "unsupported vertex input scalar={scalar_kind} width={width} size={size}"
            ));
        }
    };
    Ok((format, u64::from(width) * u64::from(size)))
}
