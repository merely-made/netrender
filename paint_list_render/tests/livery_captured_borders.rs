// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Pixel checks for captured Livery borders. The captures' font payloads were
//! elided, so isolate their unchanged border commands for replay.

use netrender::{boot, create_netrender_instance, ColorLoad, NetrenderOptions, Renderer, Scene};
use paint_list_api::specs::{TransformKind, TransformSpec};
use paint_list_api::{
    BorderDetails, BorderStyle, DeviceIntSize, LayoutPoint, LayoutSideOffsets, LayoutSize,
    LayoutTransform, PaintCmd, PaintEnvelope,
};
use paint_list_render::{translate_paint_cmds_to_fragment, translate_paint_list};

const WIDTH: u32 = 336;
const HEIGHT: u32 = 128;

fn captured_border() -> PaintEnvelope {
    let mut envelope: PaintEnvelope =
        postcard::from_bytes(include_bytes!("corpus/livery_article_card.paintlist"))
            .expect("decode captured Livery article card");
    let borders: Vec<_> = envelope
        .commands
        .iter()
        .filter_map(|cmd| match cmd {
            PaintCmd::DrawBorder(border) => Some(border.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        borders.len(),
        2,
        "article card has solid and dashed borders"
    );
    let card = &borders[0];
    assert_eq!(card.placement.bounds.min.x, 8.0);
    assert_eq!(card.placement.bounds.min.y, 8.0);
    assert_eq!(card.placement.bounds.max.x, 328.0);
    assert_eq!(card.placement.bounds.max.y, 113.0);
    assert_eq!(card.widths, LayoutSideOffsets::new(2.0, 2.0, 2.0, 2.0));
    let BorderDetails::Normal(sides) = &card.details else {
        panic!("captured card border is not a normal CSS border");
    };
    assert!(sides.do_aa);
    for side in [sides.top, sides.right, sides.bottom, sides.left] {
        assert_eq!(side.style, BorderStyle::Solid);
        for (actual, expected) in [side.color.r, side.color.g, side.color.b, side.color.a]
            .into_iter()
            .zip([51.0 / 255.0, 85.0 / 255.0, 170.0 / 255.0, 1.0])
        {
            assert!((actual - expected).abs() < 0.001, "captured border color");
        }
    }
    for radius in [
        sides.radius.top_left,
        sides.radius.top_right,
        sides.radius.bottom_right,
        sides.radius.bottom_left,
    ] {
        assert_eq!(radius, LayoutSize::new(8.0, 8.0));
    }
    envelope.commands = vec![PaintCmd::DrawBorder(card.clone())];
    envelope.fonts.clear();
    envelope.images.clear();
    // The captured viewport clips the final eight pixels of this border.
    // Widen only the replay target to inspect the whole captured geometry.
    envelope.viewport = DeviceIntSize::new(WIDTH as i32, HEIGHT as i32);
    envelope
}

fn captured_nested_borders() -> PaintEnvelope {
    let mut envelope: PaintEnvelope =
        postcard::from_bytes(include_bytes!("corpus/livery_nested_rows.paintlist"))
            .expect("decode captured Livery rows");
    let borders: Vec<_> = envelope
        .commands
        .iter()
        .filter_map(|cmd| match cmd {
            PaintCmd::DrawBorder(border) => Some(border.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(borders.len(), 2, "outer panel and wide row borders");
    let outer = &borders[0];
    assert_eq!(outer.placement.bounds.min, LayoutPoint::new(10.0, 10.0));
    assert_eq!(outer.placement.bounds.max, LayoutPoint::new(310.0, 143.0));
    assert_eq!(outer.widths, LayoutSideOffsets::new(0.0, 0.0, 1.0, 4.0));
    let BorderDetails::Normal(outer_sides) = &outer.details else {
        panic!("outer panel border is not a normal CSS border");
    };
    assert_eq!(outer_sides.left.style, BorderStyle::Solid);
    assert_eq!(outer_sides.bottom.style, BorderStyle::Solid);
    assert!(outer_sides.radius.is_zero());
    for (actual, expected) in [
        outer_sides.left.color.r,
        outer_sides.left.color.g,
        outer_sides.left.color.b,
        outer_sides.left.color.a,
    ]
    .into_iter()
    .zip([204.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0, 1.0])
    {
        assert!((actual - expected).abs() < 0.001, "captured left side");
    }
    for (actual, expected) in [
        outer_sides.bottom.color.r,
        outer_sides.bottom.color.g,
        outer_sides.bottom.color.b,
        outer_sides.bottom.color.a,
    ]
    .into_iter()
    .zip([68.0 / 255.0, 68.0 / 255.0, 88.0 / 255.0, 1.0])
    {
        assert!((actual - expected).abs() < 0.001, "captured bottom side");
    }

    let wide = &borders[1];
    assert_eq!(wide.placement.bounds.min, LayoutPoint::new(20.0, 46.0));
    assert_eq!(wide.placement.bounds.max, LayoutPoint::new(304.0, 74.0));
    assert_eq!(wide.widths, LayoutSideOffsets::new(1.0, 1.0, 1.0, 1.0));
    let BorderDetails::Normal(wide_sides) = &wide.details else {
        panic!("wide row border is not a normal CSS border");
    };
    for side in [
        wide_sides.top,
        wide_sides.right,
        wide_sides.bottom,
        wide_sides.left,
    ] {
        assert_eq!(side.style, BorderStyle::Solid);
        for (actual, expected) in [side.color.r, side.color.g, side.color.b, side.color.a]
            .into_iter()
            .zip([90.0 / 255.0, 90.0 / 255.0, 122.0 / 255.0, 1.0])
        {
            assert!((actual - expected).abs() < 0.001, "captured wide row side");
        }
    }
    for radius in [
        wide_sides.radius.top_left,
        wide_sides.radius.top_right,
        wide_sides.radius.bottom_right,
        wide_sides.radius.bottom_left,
    ] {
        assert_eq!(radius, LayoutSize::new(3.0, 3.0));
    }
    envelope.commands = borders.into_iter().map(PaintCmd::DrawBorder).collect();
    envelope.fonts.clear();
    envelope.images.clear();
    envelope
}

fn assert_border_pixels(bytes: &[u8], dx: usize, dy: usize) {
    let at = |x: usize, y: usize| {
        let i = ((y + dy) * WIDTH as usize + x + dx) * 4;
        &bytes[i..i + 4]
    };
    let alpha = |x, y| at(x, y)[3];
    assert_eq!(alpha(7, 60), 0, "outside left edge");
    assert_eq!(alpha(8, 8), 0, "outside the rounded outer corner");
    assert!(alpha(11, 10) > 150, "rounded corner must be painted");
    assert_eq!(alpha(12, 12), 0, "rounded inner hole");
    assert_eq!(alpha(16, 7), 0, "outside top edge");
    assert_eq!(alpha(16, 8), 255, "outer top row");
    assert_eq!(alpha(16, 9), 255, "inner top row");
    assert_eq!(alpha(16, 10), 0, "inside the top border");
    assert_eq!(alpha(8, 60), 255, "outer left column");
    assert_eq!(alpha(9, 60), 255, "inner left column");
    assert_eq!(alpha(10, 60), 0, "inside the left border");
    assert_eq!(alpha(327, 60), 255, "right edge beyond captured viewport");
    assert_eq!(alpha(328, 60), 0, "outside right edge");
    assert_eq!(alpha(325, 60), 0, "inside the right border");
    assert_eq!(alpha(160, 110), 0, "inside the bottom border");
    assert_eq!(alpha(160, 111), 255, "inner bottom row");
    assert_eq!(alpha(160, 112), 255, "outer bottom row");
    assert_eq!(alpha(160, 113), 0, "outside bottom edge");
    let blue = at(16, 8);
    // Opaque #3355aa from the producer CSS; both renderers return RGBA8.
    for (actual, expected) in blue.iter().copied().zip([51u8, 85, 170, 255]) {
        assert!(actual.abs_diff(expected) <= 2, "border color: {blue:?}");
    }
}

#[cfg(feature = "replay-cpu")]
#[test]
fn captured_solid_border_keeps_css_ring_geometry_on_cpu() {
    let scene = translate_paint_list(&captured_border()).expect("captured solid border admitted");
    let mut cpu = netrender::vello_backends::CpuSession::new(
        netrender::vello_backends::SparseResourceLimits::default(),
    );
    let image = cpu.render(&scene).expect("CPU render captured border");
    assert_border_pixels(image.data_as_u8_slice(), 0, 0);
}

#[cfg(feature = "replay-cpu")]
#[test]
fn captured_nested_rows_keep_joined_side_colors_and_rounded_hole_on_cpu() {
    let scene =
        translate_paint_list(&captured_nested_borders()).expect("captured row borders admitted");
    let mut cpu = netrender::vello_backends::CpuSession::new(
        netrender::vello_backends::SparseResourceLimits::default(),
    );
    let image = cpu.render(&scene).expect("CPU render captured row borders");
    let bytes = image.data_as_u8_slice();
    let at = |x: usize, y: usize| {
        let i = (y * 320 + x) * 4;
        &bytes[i..i + 4]
    };
    let near = |actual: &[u8], expected: [u8; 4]| {
        for (a, e) in actual.iter().copied().zip(expected) {
            assert!(
                a.abs_diff(e) <= 2,
                "actual {actual:?}, expected {expected:?}"
            );
        }
    };

    near(at(11, 60), [204, 68, 102, 255]);
    near(at(160, 142), [68, 68, 88, 255]);
    assert_eq!(at(14, 60)[3], 0, "inside left border");
    assert_eq!(at(160, 141)[3], 0, "inside bottom border");
    assert_eq!(at(9, 60)[3], 0, "outside left border");
    assert_eq!(at(160, 143)[3], 0, "outside bottom border");
    for x in 10..14 {
        assert_eq!(at(x, 142)[3], 255, "joined bottom-left corner x={x}");
    }

    near(at(24, 46), [90, 90, 122, 255]);
    assert_eq!(at(24, 47)[3], 0, "inside top of wide row border");
    assert_eq!(at(19, 46)[3], 0, "outside wide row border");
    assert!(at(20, 46)[3] < 255, "rounded outer corner is partial");
    assert!(at(21, 47)[3] > 0, "rounded corner ring");
    assert_eq!(at(22, 48)[3], 0, "rounded inner hole");
    assert_eq!(at(303, 60)[3], 255, "right side of wide row border");
    assert_eq!(at(302, 60)[3], 0, "inside right side of wide row border");
}

fn renderer(handles: &netrender::WgpuHandles) -> Renderer {
    create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            tile_cache_size: Some(64),
            enable_vello: true,
            ..Default::default()
        },
    )
    .expect("create renderer")
}

fn render_bytes_with_clear(
    r: &Renderer,
    handles: &netrender::WgpuHandles,
    scene: &Scene,
    clear: wgpu::Color,
) -> Vec<u8> {
    let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Livery article border target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    r.render_vello(scene, &view, ColorLoad::Clear(clear));
    r.wgpu_device.read_rgba8_texture(&texture, WIDTH, HEIGHT)
}

fn render_bytes(r: &Renderer, handles: &netrender::WgpuHandles, scene: &Scene) -> Vec<u8> {
    render_bytes_with_clear(
        r,
        handles,
        scene,
        wgpu::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        },
    )
}

fn pixel_diff(a: &[u8], b: &[u8]) -> String {
    assert_eq!(a.len(), b.len());
    let mut pixels = 0;
    let mut alpha = 0;
    let mut visible_rgb = 0;
    let mut transparent_rgb = 0;
    let mut max_delta = 0;
    let mut first = Vec::new();
    for (i, (left, right)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        if left == right {
            continue;
        }
        pixels += 1;
        if left[3] != right[3] {
            alpha += 1;
        }
        if left[..3] != right[..3] {
            if left[3] == 0 && right[3] == 0 {
                transparent_rgb += 1;
            } else {
                visible_rgb += 1;
            }
        }
        for (l, r) in left.iter().zip(right.iter()) {
            max_delta = max_delta.max(l.abs_diff(*r));
        }
        if first.len() < 8 {
            first.push((i % WIDTH as usize, i / WIDTH as usize, left, right));
        }
    }
    format!(
        "pixels={pixels}, alpha={alpha}, visible_rgb={visible_rgb}, transparent_rgb={transparent_rgb}, max_delta={max_delta}, first={first:?}"
    )
}

fn same_visible_pixels(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .all(|(left, right)| left[3] == right[3] && (left[3] == 0 || left[..3] == right[..3]))
}

fn shifted_flat_border(mut envelope: PaintEnvelope, dx: f32, dy: f32) -> Scene {
    envelope.commands.insert(
        0,
        PaintCmd::PushTransform(TransformSpec {
            origin: LayoutPoint::new(0.0, 0.0),
            transform: LayoutTransform::new(
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, dx, dy, 0.0, 1.0,
            ),
            kind: TransformKind::Standard,
        }),
    );
    envelope.commands.push(PaintCmd::PopTransform);
    translate_paint_list(&envelope).expect("shifted solid border admitted")
}

#[test]
fn captured_solid_border_retains_the_same_pixels_as_flat_translation() {
    let handles = boot().expect("wgpu boot");
    eprintln!(
        "Livery article border adapter: {:?}",
        handles.adapter.get_info()
    );
    let envelope = captured_border();
    let flat = translate_paint_list(&envelope).expect("captured solid border admitted");
    let flat_pixels = render_bytes(&renderer(&handles), &handles, &flat);
    assert_border_pixels(&flat_pixels, 0, 0);

    let retained_renderer = renderer(&handles);
    let fragment = translate_paint_cmds_to_fragment(&envelope.commands, &[], &[])
        .expect("captured solid border fragment admitted");
    let id = retained_renderer
        .register_fragment(fragment)
        .expect("register captured border fragment");
    let mut placed = Scene::new(WIDTH, HEIGHT);
    placed.place_fragment(id, netrender::scene::Transform::IDENTITY);
    let retained_pixels = render_bytes(&retained_renderer, &handles, &placed);
    assert_border_pixels(&retained_pixels, 0, 0);
    // Vello stores straight RGBA. Its fine.wgsl output divides RGB by
    // max(alpha, 1e-6); tiny coverage can quantize alpha to zero while
    // leaving different RGB under the transparent pixel. The flat tile
    // path and retained master path have different clip composition, so
    // compare every alpha and every RGB byte with nonzero alpha.
    eprintln!(
        "flat/retained: {}",
        pixel_diff(&flat_pixels, &retained_pixels)
    );
    let opaque_flat =
        render_bytes_with_clear(&renderer(&handles), &handles, &flat, wgpu::Color::BLACK);
    let opaque_retained =
        render_bytes_with_clear(&retained_renderer, &handles, &placed, wgpu::Color::BLACK);
    eprintln!(
        "opaque flat/retained: {}",
        pixel_diff(&opaque_flat, &opaque_retained)
    );
    assert_eq!(retained_renderer.fragment_lower_count(), Some(1));

    let (dx, dy) = (3.0, 4.0);
    let shifted_flat = shifted_flat_border(envelope, dx, dy);
    let shifted_flat_pixels = render_bytes(&renderer(&handles), &handles, &shifted_flat);
    let mut shifted_retained = Scene::new(WIDTH, HEIGHT);
    shifted_retained.place_fragment(id, netrender::scene::Transform::translate_2d(dx, dy));
    let shifted_retained_pixels = render_bytes(&retained_renderer, &handles, &shifted_retained);
    assert_border_pixels(&shifted_retained_pixels, dx as usize, dy as usize);
    eprintln!(
        "moved flat/retained: {}",
        pixel_diff(&shifted_flat_pixels, &shifted_retained_pixels)
    );
    let opaque_shifted_flat = render_bytes_with_clear(
        &renderer(&handles),
        &handles,
        &shifted_flat,
        wgpu::Color::BLACK,
    );
    let opaque_shifted_retained = render_bytes_with_clear(
        &retained_renderer,
        &handles,
        &shifted_retained,
        wgpu::Color::BLACK,
    );
    eprintln!(
        "opaque moved flat/retained: {}",
        pixel_diff(&opaque_shifted_flat, &opaque_shifted_retained)
    );
    assert_eq!(
        retained_renderer.fragment_lower_count(),
        Some(1),
        "placement must reuse the cached border lowering"
    );
    assert!(
        same_visible_pixels(&retained_pixels, &flat_pixels),
        "retained border differs from flat replay: {}",
        pixel_diff(&flat_pixels, &retained_pixels)
    );
    assert!(
        same_visible_pixels(&shifted_retained_pixels, &shifted_flat_pixels),
        "moved retained border differs from flat replay: {}",
        pixel_diff(&shifted_flat_pixels, &shifted_retained_pixels)
    );
    assert!(
        opaque_flat == opaque_retained,
        "opaque border differs: {}",
        pixel_diff(&opaque_flat, &opaque_retained)
    );
    assert!(
        opaque_shifted_flat == opaque_shifted_retained,
        "opaque moved border differs: {}",
        pixel_diff(&opaque_shifted_flat, &opaque_shifted_retained)
    );
}
