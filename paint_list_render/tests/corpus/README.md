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

## The seeds are placeholders

The three fixtures here were hand-built, because no consumer capture
existed when the corpus was created. Two imitate shapes that real code
emits (`genet-wpt` reftests, cambium host chrome); the third is
deliberately synthetic and should stay. Their `.provenance` files say so
and name what should replace them.

## The obvious source

`genet/ports/genet-wpt` already builds one `PaintEnvelope` per WPT test,
complete with the reftest backdrop and the device-scale root transform.
A single `fs::write` in that harness turns a WPT run into thousands of
real web-page paint lists.

That is the version of this corpus worth having. Sample it rather than
committing all of it: pick the fixtures that cover distinct command
shapes, keep them small, and let the rest stay in the WPT run.
