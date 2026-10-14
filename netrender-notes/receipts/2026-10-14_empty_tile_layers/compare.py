# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0
"""Compare intact Nodes/Links baseline and fixed cold/warm replay directories.

Usage: compare.py BASELINE_NODES FIXED_NODES BASELINE_LINKS FIXED_LINKS OUT_JSON
Opaque regions are established by the captured Scene's header/roster rectangles,
not inferred from the damaged image. Resource and Scene hashes are recorded by
the qualification receipt separately.
"""
import hashlib
import json
import pathlib
import sys

from PIL import Image, ImageChops


def load(directory, frame):
    path = pathlib.Path(directory) / f"{frame}.png"
    image = Image.open(path).convert("RGBA")
    assert image.size == (2360, 1600), image.size
    return image, {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def alpha_region(image, box):
    alpha = image.crop(box).getchannel("A")
    histogram = alpha.histogram()
    return {"bounds": box, "alpha_extrema": alpha.getextrema(),
            "transparent_pixels": histogram[0], "not_opaque_pixels": sum(histogram[:255])}


def difference_metrics(image):
    return {"rgba_extrema": image.getextrema(),
            "bounds": image.getbbox(alpha_only=False),
            "changed_pixels": sum(pixel != (0, 0, 0, 0) for pixel in image.get_flattened_data())}


def within_existing_interior_gate(image):
    extrema = image.getextrema()
    return all(hi <= 3 for lo, hi in extrema[:3]) and extrema[3][1] <= 2


def main():
    baseline_nodes, fixed_nodes, baseline_links, fixed_links, output = sys.argv[1:]
    result = {"frames": {}, "scope": "captured native scene rasterization; headed final composition qualified separately"}
    for frame in ("cold", "warm"):
        bn, bn_meta = load(baseline_nodes, frame)
        fn, fn_meta = load(fixed_nodes, frame)
        bl, bl_meta = load(baseline_links, frame)
        fl, fl_meta = load(fixed_links, frame)
        regions = ((0, 0, 1175, 62), (1185, 62, 2360, 1600))
        baseline = [alpha_region(bn, region) for region in regions]
        fixed = [alpha_region(fn, region) for region in regions]
        assert all(region["transparent_pixels"] > 0 for region in baseline), baseline
        assert all(region["not_opaque_pixels"] == 0 for region in fixed), fixed
        links_difference = ImageChops.difference(bl, fl)
        assert not any(hi for lo, hi in links_difference.getextrema()), links_difference.getextrema()
        opaque_overlap = ImageChops.multiply(
            bn.getchannel("A").point(lambda alpha: 255 if alpha == 255 else 0),
            fn.getchannel("A").point(lambda alpha: 255 if alpha == 255 else 0),
        )
        opaque_delta = Image.composite(ImageChops.difference(bn, fn),
                                       Image.new("RGBA", bn.size, (0, 0, 0, 0)), opaque_overlap)
        assert within_existing_interior_gate(opaque_delta), opaque_delta.getextrema()
        result["frames"][frame] = {
            "baseline_nodes": bn_meta, "fixed_nodes": fn_meta,
            "baseline_links": bl_meta, "fixed_links": fl_meta,
            "nodes_baseline_regions": baseline, "nodes_fixed_regions": fixed,
            "links_rgba_identical": True,
            "nodes_opaque_overlap": {"pixels": opaque_overlap.histogram()[255],
                                      **difference_metrics(opaque_delta)},
        }
    cold, _ = load(fixed_nodes, "cold")
    warm, _ = load(fixed_nodes, "warm")
    original_cold, _ = load(baseline_nodes, "cold")
    original_warm, _ = load(baseline_nodes, "warm")
    fixed_delta = ImageChops.difference(cold, warm)
    original_delta = ImageChops.difference(original_cold, original_warm)
    assert within_existing_interior_gate(fixed_delta), fixed_delta.getextrema()
    assert within_existing_interior_gate(original_delta), original_delta.getextrema()
    result["nodes_cold_warm_residual"] = {"unpruned": difference_metrics(original_delta),
                                          "fixed": difference_metrics(fixed_delta)}
    result["existing_interior_rgba_gate"] = {"rgb": 3, "alpha": 2,
        "source": "netrender-notes/receipts/2026-10-12_production_flattening/README.md:16-17"}
    result["passed"] = True
    pathlib.Path(output).write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print("Nodes baseline holes reproduced, fixed authored opaque regions restored; Links unchanged; small Nodes deltas recorded within existing gate")


if __name__ == "__main__":
    main()
