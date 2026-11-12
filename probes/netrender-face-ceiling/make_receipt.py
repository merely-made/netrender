"""Turn the probe's raw JSON into the L0a receipt (JSON + markdown)."""

import json
import subprocess

RAW = "l0a_raw.json"
OUT_JSON = "l0a_netrender_face_ceiling.json"
OUT_MD = "l0a_netrender_face_ceiling.md"

REFUSALS = [
    {
        "kind": "not-a-refusal",
        "what": "Retained fragments DO carry a per-placement transform.",
        "detail": "Scene::place_fragment(FragmentId, Transform) takes an arbitrary "
                  "column-major 4x4. Across 60 frames with every body's transform "
                  "changing, Renderer::fragment_lower_count() stayed equal to the body "
                  "count (one lower per fragment, ever), so placement-only change costs "
                  "an append, not a re-lower. Fragment mode was measured in full.",
    },
    {
        "kind": "cache-scope",
        "what": "The fragment path bypasses the retained tile cache entirely.",
        "detail": "vello_cached_tile_count() == 0 in fragment mode; == 40 (8x5 tiles of "
                  "256 px over 1920x1080) in static and live flat modes. So 'tile cache "
                  "warm' is true for static/live and inapplicable to fragment.",
    },
    {
        "kind": "no-free-static-frame",
        "what": "A byte-identical scene is not free on the tile path.",
        "detail": "With the tile cache fully warm, dirty_tile_rebuild falls to ~0.003 ms, "
                  "but master_compose plus vello_render are still paid every frame "
                  "(24.5 ms netrender-reported total at 3000 bodies x 50 rects). Only the "
                  "fragment path has a whole-master cache (fragment_master_hits), and it "
                  "hits only when placement is unchanged too.",
    },
    {
        "kind": "probe-environment",
        "what": "mere/.cargo/config.toml breaks any cargo invocation run from inside mere.",
        "detail": "Its [patch] table for the genet git source points servo-paint at "
                  "../genet/components/paint, a directory that no longer exists, so cargo "
                  "fails at resolution ('unable to update .../genet/components/paint'). "
                  "Worked around by running cargo with --manifest-path from a cwd outside "
                  "mere; the probe itself needs no [patch] of its own. Nothing was edited.",
    },
    {
        "kind": "api-shape",
        "what": "FrameTimings.total is a struct field, not a named span.",
        "detail": "Named spans on the tile path are master_append, refresh_image_data, "
                  "tile_invalidate, dirty_tile_rebuild, master_compose, vello_render; the "
                  "fragment path adds fragment_signature and omits the tile spans.",
    },
]


def fmt(x, n=2):
    return "-" if x is None else f"{x:.{n}f}"


def ceiling(cells, budget=16.0):
    """Largest element count whose median frame time is under `budget` ms."""
    ok = [c for c in cells if c["frame_ms"] is not None and c["frame_ms"] < budget]
    if not ok:
        return None
    best = max(ok, key=lambda c: c["elements"])
    return {
        "elements": best["elements"],
        "bodies": best["bodies"],
        "rects_per_body": best["rects_per_body"],
        "frame_ms": best["frame_ms"],
    }


def main():
    raw = json.load(open(RAW, encoding="utf-8"))
    commit = subprocess.run(
        ["git", "-C", ".", "rev-parse", "--short", "HEAD"],
        capture_output=True, text=True,
    ).stdout.strip()

    by_mode = {}
    for m in ("static", "live", "fragment"):
        by_mode[m] = ceiling([c for c in raw["cells"] if c["mode"] == m])

    raw["date"] = "2026-09-11"
    raw["netrender_commit"] = commit
    raw["probe_source"] = (
        "probes/netrender-face-ceiling"
    )
    raw["tile_cache_warm"] = {
        "static": True, "live": True, "fragment": "bypassed (fragment path skips tiles)",
    }
    raw["ceilings_under_16ms"] = by_mode
    raw["refusals"] = REFUSALS
    json.dump(raw, open(OUT_JSON, "w", encoding="utf-8"), indent=2)

    rows = []
    for c in raw["cells"]:
        rows.append(
            f"| {c['mode']} | {c['bodies']} | {c['rects_per_body']} | {c['elements']} | "
            f"{fmt(c['build_ms'])} | {fmt(c['encode_ms'])} | {fmt(c['frame_ms'])} | "
            f"{fmt(c['span_master_compose_ms'])} | {fmt(c['span_dirty_tile_rebuild_ms'], 3)} | "
            f"{fmt(c['span_vello_render_ms'])} | {c['frames_measured']} |"
        )

    a = raw["adapter"]
    md = [
        "# L0a — netrender face ceiling",
        "",
        "**Date:** 2026-09-11  ",
        f"**Device:** {a['name']} ({a['device_type']}, {a['backend']})  ",
        f"**Driver:** {a['driver']} {a['driver_info']}  ",
        f"**netrender commit:** `{commit}`  ",
        "**Viewport:** 1920x1080 Rgba8Unorm; tile cache 256 px "
        "(warm: 40 tiles in static/live, bypassed in fragment mode)  ",
        "**Method:** 5 warmup frames, then 60 measured (10 where a warmup frame "
        "exceeded 150 ms); medians. `frame ms` = build + render_vello + "
        "`device.poll(Wait)`.  ",
        "",
        "| mode | bodies | rects/body | elements | build ms | encode ms | frame ms | "
        "compose ms | dirty-tile ms | vello ms | frames |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        *rows,
        "",
        "## Conclusion",
        "",
        f"1. Static (identical scene every frame, flat rects): "
        f"{by_mode['static']['elements']:,} rectangles "
        f"({by_mode['static']['bodies']} bodies x {by_mode['static']['rects_per_body']}) "
        f"at {by_mode['static']['frame_ms']:.1f} ms is the largest cell under 16 ms; "
        f"50k is the honest static ceiling.",
        f"2. Live via the flat path (every body's transform changes, scene rebuilt): "
        f"{by_mode['live']['elements']:,} rectangles at "
        f"{by_mode['live']['frame_ms']:.1f} ms — the ceiling sits near 10-15k, "
        "because 20k costs 18.4 ms.",
        f"3. Live via retained fragments: {by_mode['fragment']['elements']:,} rectangles "
        f"at {by_mode['fragment']['frame_ms']:.1f} ms, and interpolation to 16 ms puts "
        "the ceiling near 290k — a 20x lift over the flat live path, with "
        "`fragment_lower_count` flat at one lower per body across all 60 frames.",
        "4. CPU path overhead dominates, not GPU. On the live flat path at 600k rects, "
        "`dirty_tile_rebuild` is 346 ms of a 449 ms netrender total (77%) while "
        "`vello_render` is 19 ms; the tile cache re-lowers every tile a moving body "
        "touches.",
        "5. On the fragment path the two halves are comparable (14.3 ms `master_compose` "
        "vs 11.4 ms `vello_render` at 600k rects), so a face-batch scene op (L3) would "
        "attack the compose half; below ~200k rects neither half is the constraint.",
        "",
    ]
    open(OUT_MD, "w", encoding="utf-8").write("\n".join(md))
    print("wrote", OUT_JSON, "and", OUT_MD)


if __name__ == "__main__":
    main()
