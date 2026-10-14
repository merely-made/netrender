// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Actual shadow-helper resource replacement, cached-scene refresh and retirement.
//! Reused and fresh oracle renderers share one Device/Queue. Scene geometry/key
//! stay constant while only mask parameters change. Optional captures are receipt
//! output, not fixtures used by the assertions.
use netrender::{
    boot, create_netrender_instance,
    scene::{SceneFragment, Transform},
    ColorLoad, NetrenderOptions, Renderer, Scene, WgpuHandles, NO_CLIP,
};

const DIM: u32 = 64;
const KEY: u64 = 0xFFFF_0000_0000_0AA5;

fn renderer(handles: &WgpuHandles) -> Renderer {
    create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            enable_vello: true,
            tile_cache_size: Some(32),
            ..Default::default()
        },
    )
    .unwrap()
}

fn image_scene() -> Scene {
    let mut scene = Scene::new(DIM, DIM);
    scene.push_image_full(
        0.0,
        0.0,
        DIM as f32,
        DIM as f32,
        [0.0, 0.0, 1.0, 1.0],
        [0.2, 0.5, 0.8, 1.0],
        KEY,
        0,
        NO_CLIP,
    );
    scene
}

fn retained_scene(renderer: &Renderer, image: &Scene) -> Scene {
    let id = renderer
        .register_fragment(SceneFragment::from_scene(image.clone()))
        .unwrap();
    let mut scene = Scene::new(DIM, DIM);
    scene.place_fragment(id, Transform::IDENTITY);
    scene
}

fn render(renderer: &Renderer, scene: &Scene, surface: Option<u64>) -> Vec<u8> {
    let texture = renderer
        .wgpu_device
        .core
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow-helper lifecycle output"),
            size: wgpu::Extent3d {
                width: DIM,
                height: DIM,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
    let view = texture.create_view(&Default::default());
    let clear = ColorLoad::Clear(wgpu::Color::TRANSPARENT);
    if let Some(surface) = surface {
        renderer
            .render_vello_scaled_for_checked(surface, scene, &view, clear, 1.0)
            .unwrap();
    } else {
        renderer.render_vello_checked(scene, &view, clear).unwrap();
    }
    let pixels = renderer.wgpu_device.read_rgba8_texture(&texture, DIM, DIM);
    assert_eq!(renderer.poll_vello_curve_quality().unwrap(), Some((0, 0)));
    pixels
}

fn capture(name: &str, pixels: &[u8]) {
    let Ok(path) = std::env::var("NETRENDER_SHADOW_CAPTURE_DIR") else {
        return;
    };
    let path = std::path::Path::new(&path);
    std::fs::create_dir_all(path).unwrap();
    let mut encoder = png::Encoder::new(
        std::fs::File::create(path.join(format!("{name}.png"))).unwrap(),
        DIM,
        DIM,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
}

fn equal_pixels(actual: &[u8], expected: &[u8], context: &str) {
    assert_eq!(actual.len(), expected.len(), "{context}: byte length");
    if let Some(first) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        let pixel = first / 4;
        panic!(
            "{context}: first pixel mismatch at({},{}), got{:?} expected{:?}",
            pixel % DIM as usize,
            pixel / DIM as usize,
            &actual[pixel * 4..pixel * 4 + 4],
            &expected[pixel * 4..pixel * 4 + 4]
        );
    }
}

#[test]
fn repeated_shadow_key_refresh_resize_and_retire_match_same_device_oracle() {
    let handles = boot().unwrap();
    let info = handles.adapter.get_info();
    println!(
        "adapter={} backend={:?} vendor={} device={}",
        info.name, info.backend, info.vendor, info.device
    );
    let reused = renderer(&handles);
    let image = image_scene();
    let retained = retained_scene(&reused, &image);
    let cases = [
        (64, [12.0, 12.0, 52.0, 52.0], 2.0, false),
        (64, [12.0, 12.0, 52.0, 52.0], 2.0, false),
        (64, [12.0, 12.0, 52.0, 52.0], 8.0, false),
        (64, [12.0, 12.0, 52.0, 52.0], 8.0, true),
        (96, [18.0, 18.0, 78.0, 78.0], 8.0, false),
        (64, [8.0, 8.0, 24.0, 24.0], 2.0, false),
    ];
    let mut previous: Option<Vec<u8>> = None;
    let mut leaked_handle = false;
    for (index, (size, bounds, blur, invert)) in cases.into_iter().enumerate() {
        reused.build_box_shadow_mask(KEY, size, bounds, 4.0, blur, invert);
        let live = reused.vello_live_texture_registrations();
        if live != Some(1) {
            leaked_handle = true;
            println!("case{index}: repeated key leaked identity, live{live:?}");
        }
        let fresh = renderer(&handles);
        fresh.build_box_shadow_mask(KEY, size, bounds, 4.0, blur, invert);
        let fresh_retained = retained_scene(&fresh, &image);
        for (lane, surface, retained_path) in [
            ("global", None, false),
            ("surface10", Some(10), false),
            ("surface20", Some(20), false),
            ("retained", None, true),
        ] {
            let actual = render(
                &reused,
                if retained_path { &retained } else { &image },
                surface,
            );
            let expected = render(
                &fresh,
                if retained_path {
                    &fresh_retained
                } else {
                    &image
                },
                surface,
            );
            capture(&format!("case{index}-{lane}-reused"), &actual);
            capture(&format!("case{index}-{lane}-oracle"), &expected);
            equal_pixels(
                &actual,
                &expected,
                &format!("case{index}/{lane}: unchanged Scene mask"),
            );
            if lane == "global" {
                if let Some(old) = &previous {
                    if index == 1 {
                        equal_pixels(&actual, old, "same parameters changed output");
                    } else {
                        assert!(
                            actual != *old,
                            "changed mask parameters did not change pixels"
                        );
                    }
                }
                previous = Some(actual);
            }
        }
        println!("case {index}: size {size}, blur {blur}, invert {invert}, live {live:?}; four lanes match oracle");
        fresh.unregister_image_vello(KEY);
        assert_eq!(fresh.vello_live_texture_registrations(), Some(0));
    }
    reused.unregister_image_vello(KEY);
    assert_eq!(reused.vello_live_texture_registrations(), Some(0));
    reused.unregister_image_vello(KEY);
    assert_eq!(
        reused.vello_live_texture_registrations(),
        Some(0),
        "second retirement changed custody"
    );
    for (lane, surface, retained_path) in [
        ("global", None, false),
        ("surface10", Some(10), false),
        ("surface20", Some(20), false),
        ("retained", None, true),
    ] {
        let pixels = render(
            &reused,
            if retained_path { &retained } else { &image },
            surface,
        );
        assert!(
            pixels.iter().all(|value| *value == 0),
            "{lane}: retired mask still paints"
        );
        capture(&format!("retired-{lane}"), &pixels);
    }
    // Re-admission under the same key after retirement must rebuild the exact
    // unchanged Scene and both keyed stores rather than reuse a missing handle.
    reused.build_box_shadow_mask(KEY, 64, [16.0, 16.0, 48.0, 48.0], 4.0, 4.0, false);
    assert_eq!(reused.vello_live_texture_registrations(), Some(1));
    let fresh = renderer(&handles);
    fresh.build_box_shadow_mask(KEY, 64, [16.0, 16.0, 48.0, 48.0], 4.0, 4.0, false);
    let fresh_retained = retained_scene(&fresh, &image);
    for surface in [None, Some(10), Some(20)] {
        equal_pixels(
            &render(&reused, &image, surface),
            &render(&fresh, &image, surface),
            "re-admission",
        );
    }
    equal_pixels(
        &render(&reused, &retained, None),
        &render(&fresh, &fresh_retained, None),
        "retained re-admission",
    );
    reused.unregister_image_vello(KEY);
    fresh.unregister_image_vello(KEY);
    assert_eq!(reused.vello_live_texture_registrations(), Some(0));
    println!("explicit retire, duplicate retire, unchanged-scene re-admission: live0");
    assert!(
        !leaked_handle,
        "repeated shadow key leaked Vello registrations"
    );
}
