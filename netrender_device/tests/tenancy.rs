// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The tenancy boot seam: one device serving netrender and a tenant
//! renderer.
//!
//! What these assert is the property the seam exists for. A tenant that
//! boots its own device produces textures netrender cannot sample, and
//! the failure shows up as an empty composite rather than an error, so
//! the receipts here are about *sameness of device* and about a missing
//! feature being named at boot instead of at pipeline creation.

use netrender_device::{REQUIRED_INTER_STAGE_VARIABLES, TenantNeeds, boot_shared};

fn needs() -> TenantNeeds {
    TenantNeeds {
        // A mesh tenant's usual ask. Optional on purpose: a thin
        // adapter should still boot, without them.
        optional_features: wgpu::Features::INDIRECT_FIRST_INSTANCE
            | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT
            | wgpu::Features::VERTEX_WRITABLE_STORAGE
            | wgpu::Features::CLEAR_TEXTURE,
        label: Some("tenancy test device"),
        ..Default::default()
    }
}

#[test]
fn one_device_serves_both_and_the_tenants_texture_is_sampleable() {
    let handles = boot_shared(wgpu::Backends::all(), None, &needs()).expect("wgpu boot");

    // The tenant creates its target on the shared device.
    let target = handles.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tenant target"),
        size: wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());

    // And netrender's side can bind it: creating a bind group against
    // the same device is exactly what compositing does, and it is a
    // validation error if the texture came from a different device.
    let layout = handles
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
    let _bound = handles
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
    handles
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
}

#[test]
fn netrenders_own_minimum_survives_a_tenants_limits() {
    // A tenant asking for wgpu's defaults must not quietly lower the one
    // limit netrender raises: the host used to copy that number, and a
    // stale copy is a shader that fails to link much later.
    let handles = boot_shared(
        wgpu::Backends::all(),
        None,
        &TenantNeeds {
            limits: Some(wgpu::Limits::default()),
            ..needs()
        },
    )
    .expect("wgpu boot");

    assert!(
        handles.device.limits().max_inter_stage_shader_variables >= REQUIRED_INTER_STAGE_VARIABLES,
        "the tenant's limits overrode netrender's minimum"
    );
}

#[test]
fn an_optional_feature_the_adapter_lacks_does_not_refuse_the_boot() {
    // The distinction the two feature fields exist for. Requesting a
    // feature the adapter does not have is a hard error in wgpu, so
    // "optional" has to mean dropped rather than asked for.
    let handles = boot_shared(
        wgpu::Backends::all(),
        None,
        &TenantNeeds {
            optional_features: wgpu::Features::all(),
            label: Some("greedy tenant"),
            ..Default::default()
        },
    )
    .expect("an unsatisfiable optional ask still boots");

    let granted = handles.device.features();
    assert!(
        granted.contains(netrender_device::REQUIRED_FEATURES),
        "netrender's own requirement went missing"
    );
    assert!(
        handles.adapter.features().contains(granted),
        "the device was granted a feature its adapter never had"
    );
    // And no experimental feature rode in on the opportunistic ask.
    // wgpu advertises those on the adapter but refuses the device
    // unless they were requested deliberately, so an unfiltered
    // intersection is a boot failure waiting for the right machine.
    assert!(
        (granted & wgpu::Features::all_experimental_mask()).is_empty(),
        "an experimental feature was granted opportunistically: {:?}",
        granted & wgpu::Features::all_experimental_mask()
    );
}

#[test]
fn a_required_feature_the_adapter_lacks_is_named_at_boot() {
    // Every adapter is missing something out of Features::all(), so this
    // asks for the whole set as *required* and expects the boot to fail
    // with the gap named rather than to fail later in a pipeline.
    let refused = boot_shared(
        wgpu::Backends::all(),
        None,
        &TenantNeeds {
            required_features: wgpu::Features::all(),
            ..Default::default()
        },
    );

    match refused {
        Err(netrender_device::BootError::MissingFeatures(missing)) => {
            assert!(!missing.is_empty(), "a refusal that names nothing");
        }
        Err(other) => panic!("wrong error: {other}"),
        Ok(_) => panic!("an adapter with every wgpu feature is not a thing we support"),
    }
}

#[test]
fn a_greedy_tenant_gets_the_adapters_features_without_the_traps() {
    // The JIT-compute-runtime shape: CubeCL compiles against adapter
    // capability, so an adopted device holding less than the adapter
    // fails shader validation at launch. Greedy grants the adapter's
    // set, still minus the mappable-primary trap and experimentals.
    let handles = boot_shared(
        wgpu::Backends::all(),
        None,
        &TenantNeeds {
            greedy: true,
            label: Some("greedy jit tenant"),
            ..Default::default()
        },
    )
    .expect("greedy boot");

    let granted = handles.device.features();
    let adapter = handles.adapter.features();
    let expected = (adapter - wgpu::Features::MAPPABLE_PRIMARY_BUFFERS)
        & !wgpu::Features::all_experimental_mask();
    assert!(
        granted.contains(expected),
        "greedy missed adapter features: {:?}",
        expected - granted
    );
    assert!(!granted.contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS));
    assert!((granted & wgpu::Features::all_experimental_mask()).is_empty());

    // And the adapter's limits came along (spot-check the two that JIT
    // kernels actually hit), with netrender's own minimum still raised.
    let limits = handles.device.limits();
    let adapter_limits = handles.adapter.limits();
    assert!(limits.max_buffer_size >= adapter_limits.max_buffer_size);
    assert!(
        limits.max_storage_buffer_binding_size >= adapter_limits.max_storage_buffer_binding_size
    );
    assert!(limits.max_inter_stage_shader_variables >= REQUIRED_INTER_STAGE_VARIABLES);
}
