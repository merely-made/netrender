// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
#[cfg(feature = "vello-all")]
use crate::scene::ImageData;
#[cfg(any(
    feature = "vello-cpu",
    feature = "vello-hybrid",
    feature = "vello-webgl"
))]
use crate::scene::Scene;

#[test]
fn capabilities_keep_one_contract_and_three_realizations() {
    assert!(VelloBackend::Classic.is_compiled());
    assert_eq!(
        VelloBackend::Hybrid.is_compiled(),
        cfg!(feature = "vello-hybrid")
    );
    assert_eq!(VelloBackend::Cpu.is_compiled(), cfg!(feature = "vello-cpu"));
    assert!(VelloBackend::Classic.capabilities().retained_fragments);
    assert!(VelloBackend::Hybrid.capabilities().native_scene_append);
    assert!(!VelloBackend::Hybrid.capabilities().retained_fragments);
    assert!(VelloBackend::Cpu.capabilities().cpu_output);
    assert!(!VelloBackend::Cpu.capabilities().gpu_output);
    for backend in [
        VelloBackend::Classic,
        VelloBackend::Hybrid,
        VelloBackend::Cpu,
    ] {
        let capabilities = backend.capabilities();
        assert!(capabilities.transforms);
        assert!(capabilities.clips);
        assert!(capabilities.nested_layers);
    }
}

#[cfg(feature = "vello-cpu")]
#[test]
fn sparse_viewport_cpu_refusals_preserve_owned_resources() {
    use crate::scene::ImageData;

    let mut cpu = CpuSession::new(SparseResourceLimits::default());
    let mut warm = Scene::new(4, 4);
    warm.image_sources
        .insert(7, ImageData::from_bytes(1, 1, vec![255, 0, 0, 255]));
    warm.push_image_full(
        0.0,
        0.0,
        4.0,
        4.0,
        [0.0, 0.0, 1.0, 1.0],
        [1.0; 4],
        7,
        0,
        crate::NO_CLIP,
    );
    let original = cpu.render(&warm).unwrap().data_as_u8_slice().to_vec();
    assert_eq!(&original[..4], &[255, 0, 0, 255]);
    let before = cpu.stats();
    let text_before = cpu.text_stats();

    for (width, height) in [(0, 1), (1, 0), (65_533, 1), (1, 65_535), (65_536, 1)]
        .into_iter()
        .chain((65_409..=65_532).map(|width| (width, 1)))
    {
        let mut bad = warm.clone();
        bad.viewport_width = width;
        bad.viewport_height = height;
        // A replacement source would invalidate the warm image cache if
        // dimension refusal happened after resource admission/commit.
        bad.image_sources
            .insert(7, ImageData::from_bytes(1, 1, vec![0, 255, 0, 255]));
        let result = cpu.render(&bad);
        assert!(matches!(
            result,
            Err(SparseSessionError::Admission(
                BackendAdmissionError::EmptyViewport { .. }
                    | BackendAdmissionError::ViewportTooLarge { .. }
            ))
        ));
        assert_eq!(cpu.stats(), before);
        assert_eq!(cpu.text_stats(), text_before);
    }
    assert_eq!(cpu.render(&warm).unwrap().data_as_u8_slice(), original);
}

#[cfg(any(
    feature = "vello-cpu",
    feature = "vello-hybrid",
    feature = "vello-webgl"
))]
#[test]
fn sparse_viewport_preflight_and_free_lowerers_agree() {
    for (width, height) in [
        (0, 1),
        (1, 0),
        (0, 0),
        (65_533, 1),
        (1, 65_533),
        (65_534, 1),
        (1, 65_534),
        (65_535, 1),
        (1, 65_535),
        (65_536, 1),
        (1, u32::MAX),
    ] {
        let scene = Scene::new(width, height);
        for backend in [VelloBackend::Cpu, VelloBackend::Hybrid] {
            let error = if width == 0 || height == 0 {
                BackendAdmissionError::EmptyViewport {
                    backend,
                    width,
                    height,
                }
            } else {
                BackendAdmissionError::ViewportTooLarge {
                    backend,
                    width,
                    height,
                }
            };
            assert_eq!(
                validate_scene_for_backend(backend, &scene),
                Err(error.clone())
            );
            assert_eq!(sparse::dimensions(&scene, backend), Err(error.clone()));
            #[cfg(feature = "vello-cpu")]
            if backend == VelloBackend::Cpu {
                assert_eq!(scene_to_vello_cpu(&scene).map(|_| ()), Err(error.clone()));
            }
            #[cfg(feature = "vello-hybrid")]
            if backend == VelloBackend::Hybrid {
                assert_eq!(scene_to_vello_hybrid(&scene).map(|_| ()), Err(error));
            }
        }
    }
    // The CPU depth-bucket limit is stricter than the GPU tile limit.
    // Check the entire extra refusal band without constructing providers.
    for width in 65_409..=65_532 {
        let scene = Scene::new(width, 1);
        let error = BackendAdmissionError::ViewportTooLarge {
            backend: VelloBackend::Cpu,
            width,
            height: 1,
        };
        assert_eq!(
            validate_scene_for_backend(VelloBackend::Cpu, &scene),
            Err(error.clone())
        );
        assert_eq!(
            sparse::dimensions(&scene, VelloBackend::Cpu),
            Err(error.clone())
        );
        #[cfg(feature = "vello-cpu")]
        assert_eq!(scene_to_vello_cpu(&scene).map(|_| ()), Err(error));
        assert_eq!(
            validate_scene_for_backend(VelloBackend::Hybrid, &scene),
            Ok(())
        );
        assert_eq!(
            sparse::dimensions(&scene, VelloBackend::Hybrid),
            Ok((width as u16, 1))
        );
    }
    // Classic does not use the sparse provider's four-pixel/u16 grid.
    assert!(validate_scene_for_backend(VelloBackend::Classic, &Scene::new(65_535, 1)).is_ok());
}

#[cfg(any(
    feature = "vello-cpu",
    feature = "vello-hybrid",
    feature = "vello-webgl"
))]
#[test]
fn sparse_viewport_largest_backend_dimensions_are_accepted() {
    for (backend, largest_width) in [(VelloBackend::Cpu, 65_408), (VelloBackend::Hybrid, 65_532)] {
        for (width, height) in [(largest_width, 1), (1, 65_532)] {
            let scene = Scene::new(width, height);
            assert_eq!(validate_scene_for_backend(backend, &scene), Ok(()));
            assert_eq!(
                sparse::dimensions(&scene, backend),
                Ok((width as u16, height as u16))
            );
            #[cfg(feature = "vello-cpu")]
            if backend == VelloBackend::Cpu {
                // A thin viewport exercises real CPU output at both
                // backend boundaries without a large raster allocation.
                assert!(scene_to_vello_cpu(&scene).is_ok());
                let pixels = CpuSession::new(SparseResourceLimits::default())
                    .render(&scene)
                    .unwrap();
                assert_eq!(
                    pixels.data_as_u8_slice().len(),
                    (width * height * 4) as usize
                );
            }
            #[cfg(feature = "vello-hybrid")]
            if backend == VelloBackend::Hybrid {
                assert!(scene_to_vello_hybrid(&scene).is_ok());
            }
        }
    }
}

#[cfg(feature = "vello-cpu")]
#[test]
fn cpu_lowerer_rasterizes_a_netrender_scene() {
    let mut scene = Scene::new(8, 6);
    scene.push_rect(2.0, 1.0, 6.0, 5.0, [1.0, 0.0, 1.0, 1.0]);
    let context = scene_to_vello_cpu(&scene).unwrap();
    let mut target = vello_cpu::Pixmap::new(8, 6);
    let mut resources = vello_cpu::Resources::new();
    context.render(&mut target, &mut resources);

    let center = target.data()[3 * 8 + 3];
    assert_eq!((center.r, center.g, center.b, center.a), (255, 0, 255, 255));
    let corner = target.data()[0];
    assert_eq!((corner.r, corner.g, corner.b, corner.a), (0, 0, 0, 0));
}

#[cfg(feature = "vello-hybrid")]
#[test]
fn hybrid_lowerer_produces_an_appendable_scene() {
    let mut source = Scene::new(32, 32);
    source.push_rect(0.0, 0.0, 8.0, 8.0, [0.0, 1.0, 0.0, 1.0]);
    let donor = scene_to_vello_hybrid(&source).unwrap();
    let mut master = vello_hybrid::Scene::new(32, 32);
    master.append_scene(donor, Some((16, 16))).unwrap();
}

#[cfg(all(feature = "vello-hybrid", not(target_arch = "wasm32")))]
#[test]
fn hybrid_gpu_renders_on_netrenders_wgpu_device() {
    let handles = netrender_device::boot().expect("wgpu device");
    let mut source = Scene::new(8, 8);
    source.push_rect(0.0, 0.0, 8.0, 8.0, [1.0, 0.0, 1.0, 1.0]);
    let scene = scene_to_vello_hybrid(&source).unwrap();

    let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Netrender Vello Hybrid proof target"),
        size: wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let render_size = vello_hybrid::RenderSize {
        width: 8,
        height: 8,
    };
    let (mut renderer, mut resources) = vello_hybrid::Renderer::new(
        &handles.device,
        &vello_hybrid::RenderTargetConfig {
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: 8,
            height: 8,
        },
    );
    let depth = vello_hybrid::Renderer::create_depth_texture_view(&handles.device, &render_size);
    let mut encoder = handles
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Netrender Vello Hybrid proof encoder"),
        });
    renderer
        .render(
            &scene,
            &mut resources,
            &handles.device,
            &handles.queue,
            &mut encoder,
            &render_size,
            &view,
            Some(&depth),
            &vello_hybrid::TextureBindings::new(),
            vello_hybrid::TargetInit::Clear(vello_hybrid::ClearSettings::default()),
        )
        .unwrap();

    let bytes_per_row = 256;
    let buffer = handles.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Netrender Vello Hybrid proof readback"),
        size: bytes_per_row * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row as u32),
                rows_per_image: Some(8),
            },
        },
        wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
    );
    let submission = handles.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    handles
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(5)),
        })
        .unwrap();
    receiver.try_recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let center = 4 * bytes_per_row as usize + 4 * 4;
    assert_eq!(&mapped[center..center + 4], &[255, 0, 255, 255]);
}

#[cfg(feature = "vello-all")]
#[test]
fn sparse_backends_refuse_unwired_images_identically() {
    let mut scene = Scene::new(2, 2);
    scene.push_image(
        0.0,
        0.0,
        2.0,
        2.0,
        7,
        ImageData::from_bytes(1, 1, vec![255, 255, 255, 255]),
    );
    let cpu = scene_to_vello_cpu(&scene).unwrap_err();
    let hybrid = scene_to_vello_hybrid(&scene).unwrap_err();
    assert!(matches!(
        cpu,
        BackendAdmissionError::UnsupportedOperation {
            backend: VelloBackend::Cpu,
            operation: "Image",
            ..
        }
    ));
    assert!(matches!(
        hybrid,
        BackendAdmissionError::UnsupportedOperation {
            backend: VelloBackend::Hybrid,
            operation: "Image",
            ..
        }
    ));
}

#[cfg(feature = "vello-all")]
#[test]
fn sparse_backends_admit_the_same_gradient_layer_subset() {
    let mut scene = Scene::new(32, 32);
    scene.push_layer_alpha(0.75);
    scene.push_linear_gradient(
        0.0,
        0.0,
        32.0,
        32.0,
        [0.0, 0.0],
        [32.0, 0.0],
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
    );
    scene.pop_layer();

    scene_to_vello_cpu(&scene).unwrap();
    scene_to_vello_hybrid(&scene).unwrap();
}
