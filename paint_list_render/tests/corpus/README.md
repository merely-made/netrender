# Paint-list corpus

Real consumer paint lists, replayed as a regression suite.

netrender's other tests are scenes it wrote for itself. They prove the
API does what the API says. They cannot catch a consumer driving the
vocabulary in a shape nobody anticipated, because no consumer wrote
them. Eight repos depend on this vocabulary. This directory is where
their actual output lives.

The harness is [`../pcorpus_replay.rs`](../pcorpus_replay.rs).

## Contributing a fixture

Wherever your app already has something implementing `PaintList`:

```rust
let envelope = paint_list_api::PaintEnvelope::from_list(&list);
std::fs::write("my_scene.paintlist", postcard::to_allocvec(&envelope)?)?;
```

That is the whole capture. `paint_list_api` has serde as a hard
dependency, so no feature flag is needed on your side, and
`PaintEnvelope` is documented as the wire shape for exactly this.

Then:

1. Drop `my_scene.paintlist` in this directory.
2. Write `my_scene.provenance` next to it (see below).
3. Record the golden once:
   ```bash
   NETRENDER_CORPUS_BLESS=1 cargo test -p paint_list_render --test pcorpus_replay
   ```
   That run fails on purpose. A blessed run asserts nothing, so it
   refuses to look like a pass.
4. Re-run without the variable. Commit the `.paintlist`, `.provenance`
   and `.ops` together.

## Three files per fixture

| File | What it is |
| --- | --- |
| `<name>.paintlist` | postcard-encoded `PaintEnvelope`. The capture. |
| `<name>.provenance` | Where it came from. Required. |
| `<name>.ops` | Recorded `Scene::dump_ops()` output. The golden. |

`.provenance` is plain `key: value` lines. Three keys are required and
enforced by the harness:

- `source:` — who produced it, and how
- `captured:` — whether it came off a running consumer, stated plainly
- `describes:` — what the scene actually is

Add whatever else is useful. The existing seeds use `models:`,
`replace-with:` and `rationale:`.

The `captured:` key is not bureaucracy. A corpus that cannot tell you
whether a scene is real or invented is not evidence, and the current
seeds are all invented.

## What the harness asserts

- **Wire stability.** Decode then re-encode reproduces the bytes. Catches
  a serde representation change that would invalidate every stored
  fixture and every IPC peer on a different build.
- **Provenance.** Every fixture states its origin.
- **Translation stability.** `translate_paint_list` produces exactly the
  recorded op stream. This is the consumer-visible contract.

It is CPU-only. No GPU, no device boot, runs anywhere. The rasterizer
already has GPU receipts; what was missing was coverage of the ingress.

A GPU tier (replay each fixture through `render_vello` and compare
against a golden image) is a reasonable later addition. It needs `wgpu`
as a dev-dependency here and it inherits the cross-platform tolerance
problem the rest of the suite already deals with, so it was left out
rather than half-done.

## Eliding font and image payloads

`FontResource` carries raw TTF/OTF bytes inline, and its own doc notes
fonts run 100 KB to 20 MB. A faithful capture of any page with text is
therefore multi-megabyte: the article-card fixture below was 2,037,474
bytes before elision and 2,012 after.

So captures replace font and image payloads with a short stand-in. Every
command field stays untouched — keys, glyph ids, positions, colours —
because the command stream is what the corpus tests. The payload each key
resolves to is opaque to the translator.

This is verified, not assumed: capturing with `--keep-payloads` and
blessing produces a byte-identical `.ops` golden. Re-check that if the
translator ever starts reading font bytes.

The cost: an elided fixture can be translated but not rasterized, since
the font bytes are not a real face. Fine for the CPU tier. **Any future
GPU tier needs either unelided fixtures or a substitute face**, and that
is the main reason the GPU tier was left out rather than half-done.

Say `payloads-elided:` in the `.provenance` either way.

## What is here

| fixture | captured | covers |
| --- | --- | --- |
| `livery_article_card` | yes | `DrawText`, `DrawBorder`, rounded-clip layers |
| `livery_nested_rows` | yes | overflow clip scopes, per-side borders |
| `wpt_reftest_page` | no, hand-built | backdrop + device-scale transform shape |
| `cambium_panel` | no, hand-built | `DrawPath`, filter chain on a layer |
| `nested_stacks` | no, deliberately synthetic | deep clip/transform/layer nesting |

The two `livery_*` fixtures came off the real pipeline (Livery cascade,
Taffy layout, Parley shaping) via
`genet/components/genet-livery/examples/capture_paint_corpus.rs`. They
are worth studying: the article card lowers `border-radius` into a
rounded-clip layer plus an inset `Stroke` with shrunk radii, which is a
path no hand-written seed exercised.

The three hand-built fixtures stay. `nested_stacks` should stay
permanently — real scenes nest too shallowly to reach the case where
stack-handling bugs live. The other two are still worth replacing with
captures of the shapes they imitate.

## Scaling up

`genet/ports/genet-wpt` already builds one `PaintEnvelope` per WPT test,
complete with the reftest backdrop and the device-scale root transform.
A single `fs::write` there turns a WPT run into thousands of real
web-page paint lists.

Sample rather than committing all of it: pick fixtures that cover
distinct command shapes, keep them small, and let the rest stay in the
run. Still uncovered by anything here: gradients, images, external
textures, box shadows, and nine-patch borders.
