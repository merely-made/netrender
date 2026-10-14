# Direct WebGL2 sparse session

The wasm-only `vello-webgl` feature uses the pinned Vello GPU/common provider
2eae6243881d5b24c984d5ee35dde509ca5db782. `WebGlSession::start` owns a renderer
created on the caller's canvas; `WebGlSessionInit::try_finish` polls compilation
without blocking the browser. The host chooses scheduling, sizing and graphics
generation. CPU-owned image sources are accepted; foreign GPU textures cannot
be registered through this session. Ordinary provider errors stay typed as
session failures. This does not establish WebGL API conformance in Genet.

Complete sparse operation/resource admission precedes image hydration and draw.
Late filters refuse without changing framebuffer pixels or image counters.
Outline text uses the existing sparse text cache; shaping belongs to the caller.
Bilinear repeat remains explicitly refused by GPU routes. Padded-edge image
uploads compensate for the pinned shader's Pad clamp before half-pixel filtering.
Only bilinear image source regions extend; original paint mapping and nearest
repeat domains remain intact. GPU stats include the extra row and column,
admission bounds padded bytes and reserves one texel of texture dimensions.

The standalone fixture passes in Chrome on ANGLE AMD/radeonsi renoir and in
Firefox with its privacy-reduced renderer label. It asserts ordered shapes,
clipped opacity, transparent bilinear corners, outline glyph pixels, odd-width
resize and last-pixel alpha, unchanged target/resources after late refusal,
image replacement/removal without re-uploading another image, context loss,
recreation and rejection of old generations. Restoration is requested on the
next animation turn after loss dispatch. This qualifies the session; the actual
Graphshell viewer and human assistive-technology checks remain consumer gates.

Native CPU/Hybrid image, pattern, text and fragment regressions pass 28 tests.
Native admission and CPU reference tests are in `netrender/tests/webgl_scene.rs`.
The browser Rust fixture is `netrender/examples/webgl_scene.rs`; its page is
`netrender/examples/webgl_scene.html`. Build the cdylib for
`wasm32-unknown-unknown` with `--no-default-features --features vello-webgl`,
run matching wasm-bindgen with `--target web --out-dir pkg`, and serve the page
and pkg directory together. Terminal results appear in `#result` and POST to
`/result`; an automated receiver may collect that POST. Use real event turns
for context loss, rather than exiting Chrome on a virtual-time deadline.

Compact test/resource/source evidence is in `qualification.json`. All gates
used the serialized ThinkPad lock, kernel memory caps, disabled swapping,
unchanged source checks and browser cgroup membership assertions. The Chrome
fixture peak was 250757120 bytes and Firefox 790085632 bytes, with no pressure
or OOM events. These are standalone fixture costs, not viewer RAM budgets.
