/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use std::borrow::Cow;
use std::io::{self, Write};
use std::path::PathBuf;

use webrender_build::shader::{shader_source_from_file, ShaderVersion};
use webrender_build::shader_census::{
    collect_shader_census_profile, write_shader_census_json, ShaderCensusManifest,
};
use webrender_build::shader_features::ShaderFeatureFlags;

fn main() {
    let shader_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../webrender/res");
    let mut profiles = Vec::new();

    // This is the complete feature matrix defined by WebRender 0.70. A GLES
    // target cannot select rectangle textures, so remove that GL-only bit for
    // its enumeration while retaining the optional external-image variants.
    let gl_flags = ShaderFeatureFlags::all();
    profiles.push(collect_shader_census_profile(
        ShaderVersion::Gl,
        gl_flags,
        &|name| load_shader(&shader_dir, name),
    ));
    let mut gles_flags = ShaderFeatureFlags::all();
    gles_flags.remove(ShaderFeatureFlags::GL);
    profiles.push(collect_shader_census_profile(
        ShaderVersion::Gles,
        gles_flags,
        &|name| load_shader(&shader_dir, name),
    ));
    let manifest = ShaderCensusManifest::new(profiles);

    let stdout = io::stdout();
    let mut output = stdout.lock();
    write_shader_census_json(&manifest, &mut output).expect("serializing shader census");
    output.write_all(b"\n").expect("writing shader census");
}

fn load_shader(shader_dir: &PathBuf, name: &str) -> Cow<'static, str> {
    Cow::Owned(shader_source_from_file(
        &shader_dir.join(format!("{}.glsl", name)),
    ))
}
