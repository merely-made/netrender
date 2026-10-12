// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! Diagnostic fixtures only. No backend or provider semantics are changed.
use netrender::{
    PathOp, Scene, SceneClip, SceneGlyphRun, SceneLayer, SceneOp, ScenePath, SceneShape, Transform,
    NO_CLIP,
};
use paint_list_render::bundle::{translate_capture, PortablePaintCapture};
use serde_json::{json, Value};
use skrifa::{
    instance::Size,
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider,
    GlyphId, MetadataProvider,
};

pub struct Case {
    pub name: String,
    pub scene: Scene,
    pub details: Value,
}
pub fn outer_curve() -> ScenePath {
    let k = 0.5522848f32;
    let mut p = ScenePath::new();
    p.move_to(5.0, 0.0)
        .line_to(19.0, 0.0)
        .cubic_to(19.0 + 5.0 * k, 0.0, 24.0, 3.0 - 3.0 * k, 24.0, 3.0)
        .line_to(24.0, 21.0)
        .cubic_to(24.0, 21.0 + 3.0 * k, 19.0 + 5.0 * k, 24.0, 19.0, 24.0)
        .line_to(5.0, 24.0)
        .cubic_to(5.0 - 5.0 * k, 24.0, 0.0, 21.0 + 3.0 * k, 0.0, 21.0)
        .line_to(0.0, 3.0)
        .cubic_to(0.0, 3.0 - 3.0 * k, 5.0 - 5.0 * k, 0.0, 5.0, 0.0)
        .close();
    p
}

pub const POLYLINE_SEGMENTS_PER_QUARTER: usize = 32;
pub const POLYLINE_TOTAL_SEGMENTS: usize = 132;
pub const POLYLINE_SECOND_DERIVATIVE_BOUND: f64 = 13.563;
/// Diagnostic approximation of the unchanged canonical contour. Compute once
/// in f64 from the actual f32 control points, then share exactly these f32 line
/// endpoints with all backends. M/(8*N^2) <=0.001655640 plus endpoint rounding
/// stays below 1/256px. It is never used by production lowering.
pub fn polyline_outer_curve() -> ScenePath {
    let mut output = ScenePath::with_capacity(POLYLINE_TOTAL_SEGMENTS + 2);
    let mut current = [0.0f64; 2];
    for op in outer_curve().ops {
        match op {
            PathOp::MoveTo(x, y) => {
                current = [f64::from(x), f64::from(y)];
                output.move_to(x, y);
            }
            PathOp::LineTo(x, y) => {
                current = [f64::from(x), f64::from(y)];
                output.line_to(x, y);
            }
            PathOp::CubicTo(a, b, c, d, x, y) => {
                let p0 = current;
                let p1 = [f64::from(a), f64::from(b)];
                let p2 = [f64::from(c), f64::from(d)];
                let p3 = [f64::from(x), f64::from(y)];
                for i in 1..=POLYLINE_SEGMENTS_PER_QUARTER {
                    let t = i as f64 / POLYLINE_SEGMENTS_PER_QUARTER as f64;
                    let u = 1.0 - t;
                    let point = std::array::from_fn::<_, 2, _>(|axis| {
                        u * u * u * p0[axis]
                            + 3.0 * u * u * t * p1[axis]
                            + 3.0 * u * t * t * p2[axis]
                            + t * t * t * p3[axis]
                    });
                    output.line_to(point[0] as f32, point[1] as f32);
                }
                current = p3;
            }
            PathOp::Close => {
                output.close();
            }
            PathOp::QuadTo(..) => unreachable!("fixed canonical contour has only cubics"),
        }
    }
    output
}

fn polyline_cases() -> Vec<Case> {
    let shared = polyline_outer_curve();
    let mut cases = Vec::new();
    for (name, as_clip) in [("polyline-fill", false), ("polyline-clip-only", true)] {
        let mut scene = Scene::new(40, 40);
        if as_clip {
            scene.push_layer_clip(SceneClip::Path(shared.clone()));
            scene.push_rect(
                -8.0,
                -8.0,
                32.0,
                32.0,
                [0.41960785, 0.5529412, 0.6666667, 1.0],
            );
            scene.pop_layer();
        } else {
            scene.push_shape_filled(shared.clone(), [0.41960785, 0.5529412, 0.6666667, 1.0]);
        }
        cases.push(Case{name:name.into(),scene,details:json!({"approximation_purpose":"diagnostic only: isolate fine area coverage from provider curve flattening; canonical curve controls unchanged","segments_per_quarter":POLYLINE_SEGMENTS_PER_QUARTER,"total_line_segments":POLYLINE_TOTAL_SEGMENTS,"second_derivative_bound":POLYLINE_SECOND_DERIVATIVE_BOUND,"f64_chord_error_bound_pixels":POLYLINE_SECOND_DERIVATIVE_BOUND/(8.0*(POLYLINE_SEGMENTS_PER_QUARTER*POLYLINE_SEGMENTS_PER_QUARTER) as f64),"total_error_bound_pixels":1.0/256.0,"endpoint_rounding_bound_pixels":2.0f64.sqrt()/1048576.0,"clip_count":usize::from(as_clip),"transform":Transform::IDENTITY.m,"endpoint_sharing":"same precomputed f32 line endpoint vector and same admitted Scene for every backend; no backend-specific preparation"})});
    }
    cases
}
fn curve_case(name: &str, clips: usize, fill_curve: bool, transform: Transform) -> Case {
    let mut scene = Scene::new(40, 40);
    let id = scene.push_transform(transform);
    for _ in 0..clips {
        let mut layer = SceneLayer::clip(SceneClip::Path(outer_curve()));
        layer.transform_id = id;
        scene.push_layer(layer);
    }
    if fill_curve {
        scene.push_shape(SceneShape {
            path: outer_curve(),
            fill_color: Some([0.41960785, 0.5529412, 0.6666667, 1.0]),
            stroke: None,
            transform_id: id,
            clip_rect: NO_CLIP,
            clip_corner_radii: [0.0; 4],
        });
    } else {
        let mut p = ScenePath::new();
        p.move_to(-8.0, -8.0)
            .line_to(32.0, -8.0)
            .line_to(32.0, 32.0)
            .line_to(-8.0, 32.0)
            .close();
        scene.push_shape(SceneShape {
            path: p,
            fill_color: Some([0.41960785, 0.5529412, 0.6666667, 1.0]),
            stroke: None,
            transform_id: id,
            clip_rect: NO_CLIP,
            clip_corner_radii: [0.0; 4],
        });
    }
    for _ in 0..clips {
        scene.pop_layer();
    }
    Case {
        name: name.into(),
        scene,
        details: json!({"curve":"canonical four ellipse cubics, 24x24, radii5x3", "clip_count":clips,"fill_is_same_curve":fill_curve,"transform":transform.m,"semantic_control":"one clip over covering paint should equal direct curve coverage; coincident fill and clip diagnose coverage multiplication rather than change its geometry"}),
    }
}
pub fn geometry_cases() -> Vec<Case> {
    let mut cases = vec![
        curve_case("curve-fill", 0, true, Transform::IDENTITY),
        curve_case("curve-clip-only", 1, false, Transform::IDENTITY),
        curve_case(
            "curve-fill-and-coincident-clip",
            1,
            true,
            Transform::IDENTITY,
        ),
        curve_case(
            "curve-double-coincident-clip",
            2,
            false,
            Transform::IDENTITY,
        ),
        curve_case(
            "curve-fill-fractional-translation",
            0,
            true,
            Transform::translate_2d(0.25, 0.375),
        ),
        curve_case(
            "curve-fill-uniform-scale",
            0,
            true,
            Transform::scale_2d(1.25, 1.25),
        ),
    ];
    for (name, multicolor, radius) in [
        ("border-ellipse-uniform", false, [5.0, 3.0]),
        ("border-ellipse-accent-plus", true, [5.0, 3.0]),
        ("border-square-accent-plus", true, [0.0, 0.0]),
    ] {
        use paint_list_api::*;
        let solid = BorderSide {
            style: BorderStyle::Solid,
            color: ColorF::new(0.41960785, 0.5529412, 0.6666667, 1.0),
        };
        let r = LayoutSize::new(radius[0], radius[1]);
        let capture = PortablePaintCapture {
            envelope: PaintEnvelope {
                engine: EngineId::GENET,
                viewport: DeviceIntSize::new(40, 40),
                generation: 1,
                fonts: vec![],
                images: vec![],
                commands: vec![PaintCmd::DrawBorder(BorderItem {
                    placement: CommonPlacement::new(LayoutRect::new(
                        LayoutPoint::new(0.0, 0.0),
                        LayoutPoint::new(24.0, 24.0),
                    )),
                    widths: LayoutSideOffsets::new(1.0, 1.0, 1.0, 7.0),
                    details: BorderDetails::Normal(NormalBorder {
                        left: if multicolor {
                            BorderSide {
                                style: BorderStyle::Solid,
                                color: ColorF::new(0.2, 0.4, 0.78431374, 0.2863),
                            }
                        } else {
                            solid
                        },
                        top: solid,
                        right: solid,
                        bottom: solid,
                        radius: BorderRadius {
                            top_left: r,
                            top_right: r,
                            bottom_left: r,
                            bottom_right: r,
                        },
                        do_aa: true,
                    }),
                })],
            },
            producers: vec![],
        };
        cases.push(Case{name:name.into(),scene:translate_capture(&capture).expect("fixed admitted border").scene,details:json!({"widths":[1,1,1,7],"radii":radius,"multicolor":multicolor,"source":"actual accent border geometry isolated without external previews"})});
    }
    cases.extend(polyline_cases());
    cases
}

#[derive(Default)]
struct Pen(ScenePath);
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.quad_to(cx, cy, x, y);
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.0.cubic_to(a, b, c, d, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}
/// Preserve one real caller-shaped run; no text reshaping, glyph substitutions,
/// fallback font, or font copies are introduced. Output files contain hashes,
/// glyph positions and PNGs only; font-bearing scenes are never written.
pub fn glyph_cases(
    source: &Scene,
    op_index: usize,
) -> Result<Vec<Case>, Box<dyn std::error::Error>> {
    let Some(SceneOp::GlyphRun(run)) = source.ops.get(op_index) else {
        return Err("selected op is not a glyph run".into());
    };
    if run.glyphs.is_empty() || run.glyphs.len() > 256 {
        return Err("glyph run must contain 1..256 glyphs".into());
    }
    let blob = source
        .fonts
        .get(run.font_id as usize)
        .ok_or("missing font")?;
    let font = skrifa::FontRef::from_index(blob.data.data(), blob.index)?;
    let settings: Vec<(&str, f32)> = run
        .font_axis_values
        .iter()
        .map(|(t, v)| Ok((std::str::from_utf8(t.as_slice())?, *v)))
        .collect::<Result<_, std::str::Utf8Error>>()?;
    let axes = font.axes();
    let location = axes.location(settings);
    let outlines = font.outline_glyphs();
    let upem = f32::from(font.head()?.units_per_em());
    if upem == 0.0 {
        return Err("font has zero UPEM".into());
    }
    let details = json!({"op_index":op_index,"font_id":run.font_id,"font_index":blob.index,"font_size":run.font_size,"font_upem":upem,"axis_settings":run.font_axis_values,"normalized_coords":location.coords().iter().map(|v|v.to_bits()).collect::<Vec<_>>(),"glyphs":run.glyphs.iter().map(|g|json!({"id":g.id,"x":g.x,"y":g.y})).collect::<Vec<_>>(),"transform":source.transforms[run.transform_id as usize].m,"clip_rect":if run.clip_rect==NO_CLIP {Value::Null}else{json!(run.clip_rect)},"clip_radii":run.clip_corner_radii,"hint":false,"atlas_cache":false});
    let mut original = Scene::new(source.viewport_width, source.viewport_height);
    original.fonts = source.fonts.clone();
    original.transforms = source.transforms.clone();
    original.ops.push(SceneOp::GlyphRun(run.clone()));
    let mut cases = vec![Case {
        name: "glyph-original-run".into(),
        scene: original,
        details: details.clone(),
    }];
    for (name, size, ratio) in [
        ("glyph-skrifa-draw-size", run.font_size, 1.0),
        ("glyph-skrifa-upem-scaled", upem, run.font_size / upem),
    ] {
        let mut scene = Scene::new(source.viewport_width, source.viewport_height);
        let world = &source.transforms[run.transform_id as usize];
        for glyph in &run.glyphs {
            let outline = outlines
                .get(GlyphId::new(glyph.id))
                .ok_or("glyph has no outline")?;
            let mut pen = Pen::default();
            outline.draw(
                DrawSettings::unhinted(Size::new(size), location.coords()),
                &mut pen,
            )?;
            let transform = Transform::scale_2d(ratio, -ratio)
                .then(&Transform::translate_2d(glyph.x, glyph.y))
                .then(world);
            let id = scene.push_transform(transform);
            scene.push_shape(SceneShape {
                path: pen.0,
                fill_color: Some(run.color),
                stroke: None,
                transform_id: id,
                clip_rect: run.clip_rect,
                clip_corner_radii: run.clip_corner_radii,
            });
        }
        let mut details = details.clone();
        details["outline_size"] = json!(size);
        details["outline_ratio"] = json!(ratio);
        cases.push(Case {
            name: name.into(),
            scene,
            details,
        });
    }
    // Transform variants preserve the caller's run and compare provider scale
    // absorption separately from glyph outline representation.
    for (name, post) in [
        (
            "glyph-run-fractional-translation",
            Transform::translate_2d(0.25, 0.375),
        ),
        ("glyph-run-uniform-scale", Transform::scale_2d(1.25, 1.25)),
    ] {
        let mut scene = cases[0].scene.clone();
        let mut transformed: SceneGlyphRun = run.clone();
        transformed.transform_id =
            scene.push_transform(source.transforms[run.transform_id as usize].then(&post));
        // A device clip stays in device space. Record this controlled change;
        // baseline representation comparisons above preserve the original clip.
        transformed.clip_rect = NO_CLIP;
        transformed.clip_corner_radii = [0.0; 4];
        scene.ops = vec![SceneOp::GlyphRun(transformed)];
        scene.viewport_width = ((source.viewport_width as f64) * 1.25).ceil() as u32 + 2;
        scene.viewport_height = ((source.viewport_height as f64) * 1.25).ceil() as u32 + 2;
        let mut details = details.clone();
        details["post_transform"] = json!(post.m);
        details["controlled_device_clip"] = json!("disabled only in transform variant");
        cases.push(Case {
            name: name.into(),
            scene,
            details,
        });
    }
    Ok(cases)
}
