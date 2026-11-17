#!/usr/bin/env python3
"""Derive the Quiet Orbit core texture from the approved source image.

Authoring tool, not an app dependency; Python 3 standard library only. The approved still shows
the smoky glass sphere with its molten ivory and amber flow. This crops that sphere (with a small
margin) and resamples it to a square texture that native draws, clipped to a circle, as two
slowly drifting layers, so the core flows like the reference without playing the baked GIF.

    python3 assets/brand/source/derive_core_texture.py            # write the texture and manifest
    python3 assets/brand/source/derive_core_texture.py --check    # verify the committed texture
"""
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from derive_rock_tile import SOURCE, SOURCE_SHA256, fnv1a32, read_png, write_png  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "assets/brand/orbit"
# The sphere in the source, measured once from its bright extent: centre and radius in pixels.
CENTRE = (835.5, 434.5)
RADIUS = 209.0
MARGIN = 1.12  # the texture covers this multiple of the sphere diameter
SIZE = 384


def sample(rows, bpp, x, y):
    """Bilinear sample of the source at float coordinates, as (r, g, b)."""
    x0, y0 = int(x), int(y)
    fx, fy = x - x0, y - y0
    out = []
    for c in range(3):
        a = rows[y0][x0 * bpp + c]
        b = rows[y0][(x0 + 1) * bpp + c]
        d = rows[y0 + 1][x0 * bpp + c]
        e = rows[y0 + 1][(x0 + 1) * bpp + c]
        out.append((a * (1 - fx) + b * fx) * (1 - fy) + (d * (1 - fx) + e * fx) * fy)
    return out


def derive():
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == SOURCE_SHA256, "source changed"
    _, _, bpp, rows = read_png(SOURCE)
    half = RADIUS * MARGIN
    rgba = bytearray()
    for j in range(SIZE):
        y = CENTRE[1] - half + 2 * half * (j + 0.5) / SIZE
        for i in range(SIZE):
            x = CENTRE[0] - half + 2 * half * (i + 0.5) / SIZE
            r, g, b = sample(rows, bpp, x, y)
            rgba += bytes((round(r), round(g), round(b), 255))
    return bytes(rgba)


def main():
    rgba = derive()
    if "--check" in sys.argv:
        committed = (OUT / "fleetworks-core-texture.rgba").read_bytes()
        assert committed == rgba, "the committed texture differs from the derivation"
        print("fleetworks-core-texture.rgba matches the derivation", fnv1a32(rgba))
        return
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "fleetworks-core-texture.rgba").write_bytes(rgba)
    png = write_png(OUT / "fleetworks-core-texture.png", SIZE, SIZE, rgba)
    manifest = {
        "schema": 1,
        "role": "Quiet Orbit core: the approved smoky glass sphere with molten flow, drawn clipped to a circle",
        "source": {"path": "assets/brand/source/fleetworks-liquid-metal-glass.png", "sha256": SOURCE_SHA256},
        "sphere_in_source_px": {"centre": list(CENTRE), "radius": RADIUS},
        "texture": {"size_px": SIZE, "covers_sphere_diameters": MARGIN, "resampling": "bilinear", "alpha": "opaque"},
        "outputs": {
            "fleetworks-core-texture.png": {"bytes": len(png), "sha256": hashlib.sha256(png).hexdigest()},
            "fleetworks-core-texture.rgba": {
                "bytes": len(rgba),
                "sha256": hashlib.sha256(rgba).hexdigest(),
                "fnv1a32": fnv1a32(rgba),
            },
        },
    }
    (OUT / "fleetworks-core-texture.manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print("wrote", len(rgba), "byte RGBA, fnv1a32", fnv1a32(rgba))


if __name__ == "__main__":
    main()
