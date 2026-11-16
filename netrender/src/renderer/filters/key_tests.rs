use super::*;

#[test]
fn generated_slot_avoids_image_and_pattern_sources() {
    let mut scene = Scene::new(8, 8);
    scene.push_image_full(
        0.0,
        0.0,
        4.0,
        4.0,
        [0.0, 0.0, 1.0, 1.0],
        [1.0; 4],
        BACKDROP_FILTER_KEY_BASE,
        0,
        crate::NO_CLIP,
    );
    scene.push_pattern(ELEMENT_FILTER_KEY_BASE, [0.0, 0.0, 4.0, 4.0], [1.0, 1.0]);
    assert!(scene_uses_image_key(&scene, BACKDROP_FILTER_KEY_BASE));
    assert!(scene_uses_image_key(&scene, ELEMENT_FILTER_KEY_BASE));
    assert!(!scene_uses_image_key(&scene, ELEMENT_FILTER_KEY_BASE - 1));
    assert!(BACKDROP_FILTER_KEY_BASE < crate::EXTERNAL_IMAGE_KEY_BASE);
}
