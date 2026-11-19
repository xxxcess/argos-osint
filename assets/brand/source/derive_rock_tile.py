#!/usr/bin/env python3
"""Derive the action-button rock tile from the approved crescent source.

Authoring tool, not an app dependency; Python 3 standard library only. It follows
docs/design/matte-canvas/theme.md, "Deterministic asset and tint recipe":

1. Crop the coarse rock interior of the crescent from the approved source image. The box sits
   inside the crescent's dark rock, clear of the polished rim.
2. Take luminance (Rec. 709 weights on the sRGB bytes) and cap glowing inclusions and
   highlights at a high percentile, so only the dark mineral relief remains.
3. Make one seamless 512 x 512 tile by mirror quilting: the crop is stacked with its mirror to
   fill a 256 x 256 block, then the block is mirrored on both axes. The tile repeats without a
   seam and is shown at a fixed 256 x 256 logical pixels (2 source pixels per logical pixel).
4. Normalize to T in [-1, 1] with zero mean (scale by the 99th percentile of |Y - mean|, clip,
   re-center), quantized to 1/16 steps; the relief at rest opacity is under 7 channel levels,
   so the steps are invisible.
5. Export one RGBA overlay at opacity 1: T > 0 is TEXT_PRIMARY at alpha 0.22 T, T < 0 is
   CAST_SHADOW at alpha min(1, 1.25 |T|). At opacity o a button changes each channel by about
   round(50 * o * T) levels. Native draws the raw RGBA, web draws the PNG; both scale it by
   opacity (rest 0.12, hover 0.14, pressed 0.10, disabled 0.04).

    python3 assets/brand/source/derive_rock_tile.py            # write the outputs
    python3 assets/brand/source/derive_rock_tile.py --check    # verify the committed outputs
"""
import base64
import hashlib
import json
import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "assets/brand/source/fleetworks-liquid-metal-glass.png"
SOURCE_SHA256 = "1ae553782431da7649a1d88accdacc9076c3a039f1c2b5c0018f5998d7d70112"
OUT = ROOT / "assets/brand/rock"
CROP = (655, 715, 256, 96)  # x, y, width, height in source pixels
HIGHLIGHT_PERCENTILE = 97
SCALE_PERCENTILE = 99
STEPS = 16  # T is quantized to multiples of 1/STEPS
LIGHT = (213, 220, 229)  # TEXT_PRIMARY
DARK = (2, 3, 4)  # CAST_SHADOW
SIZE = 512


def read_png(path):
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos, idat, header = 8, [], None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat.append(body)
        pos += 12 + length
    width, height, depth, color, _, _, interlace = header
    assert depth == 8 and interlace == 0 and color in (2, 6), header
    bpp = 3 if color == 2 else 4
    raw = zlib.decompress(b"".join(idat))
    stride = width * bpp
    rows, prev = [], bytearray(stride)
    for y in range(height):
        base = y * (stride + 1)
        kind, line = raw[base], bytearray(raw[base + 1 : base + 1 + stride])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if kind == 1:
                line[i] = (line[i] + a) & 255
            elif kind == 2:
                line[i] = (line[i] + b) & 255
            elif kind == 3:
                line[i] = (line[i] + ((a + b) >> 1)) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 255
        rows.append(line)
        prev = line
    return width, height, bpp, rows


def write_png(path, width, height, rgba):
    def chunk(kind, body):
        crc = zlib.crc32(kind + body) & 0xFFFFFFFF
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", crc)

    lines = b"".join(b"\x00" + rgba[y * width * 4 : (y + 1) * width * 4] for y in range(height))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(lines, 9)) + chunk(b"IEND", b"")
    path.write_bytes(png)
    return png


def percentile(values, p):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(p / 100 * (len(ordered) - 1))))]


INCLUSION_LUMA = 150  # glints and sparkles are far brighter than the rock around them
INCLUSION_RADIUS = 10


def remove_inclusions(luma):
    """Replace glowing inclusions with the median of the dark pixels around them."""
    height, width = len(luma), len(luma[0])
    out = [line[:] for line in luma]
    for y in range(height):
        for x in range(width):
            if luma[y][x] <= INCLUSION_LUMA:
                continue
            near = [
                luma[j][i]
                for j in range(max(0, y - INCLUSION_RADIUS), min(height, y + INCLUSION_RADIUS + 1))
                for i in range(max(0, x - INCLUSION_RADIUS), min(width, x + INCLUSION_RADIUS + 1))
                if luma[j][i] <= INCLUSION_LUMA
            ]
            out[y][x] = sorted(near)[len(near) // 2] if near else 25.0
    return out


def fnv1a32(data):
    h = 0x811C9DC5
    for byte in data:
        h = ((h ^ byte) * 0x01000193) & 0xFFFFFFFF
    return h


def derive():
    digest = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    assert digest == SOURCE_SHA256, f"source changed: {digest}"
    width, _, bpp, rows = read_png(SOURCE)
    x0, y0, w, h = CROP
    luma = []
    for y in range(y0, y0 + h):
        line = []
        for x in range(x0, x0 + w):
            r, g, b = rows[y][x * bpp : x * bpp + 3]
            line.append(0.2126 * r + 0.7152 * g + 0.0722 * b)
        luma.append(line)
    cap = percentile([v for line in luma for v in line], HIGHLIGHT_PERCENTILE)
    luma = remove_inclusions(luma)
    cap = min(cap, percentile([v for line in luma for v in line], HIGHLIGHT_PERCENTILE))
    luma = [[min(v, cap) for v in line] for line in luma]
    half = SIZE // 2
    block = []
    for r in range(half):
        k = r % (2 * h)
        block.append(luma[k] if k < h else luma[2 * h - 1 - k])
    block = [line[:half] if len(line) >= half else line + line[::-1] for line in block]
    top = [line + line[::-1] for line in block]
    tile = top + top[::-1]
    flat = [v for line in tile for v in line]
    mean = sum(flat) / len(flat)
    scale = percentile([abs(v - mean) for v in flat], SCALE_PERCENTILE) or 1.0
    t = [[max(-1.0, min(1.0, (v - mean) / scale)) for v in line] for line in tile]
    mean_t = sum(v for line in t for v in line) / (SIZE * SIZE)
    t = [[round((v - mean_t) * STEPS) / STEPS for v in line] for line in t]
    t = [[max(-1.0, min(1.0, v)) for v in line] for line in t]
    rgba = bytearray()
    for line in t:
        for v in line:
            if v > 0:
                rgba += bytes((*LIGHT, round(255 * min(1.0, 0.22 * v))))
            elif v < 0:
                rgba += bytes((*DARK, round(255 * min(1.0, 1.25 * -v))))
            else:
                rgba += bytes((0, 0, 0, 0))
    final_mean = sum(v for line in t for v in line) / (SIZE * SIZE)
    return bytes(rgba), cap, mean, scale, final_mean


def outputs(rgba):
    png_path = OUT / "fleetworks-rock-tile.png"
    OUT.mkdir(parents=True, exist_ok=True)
    png = write_png(png_path, SIZE, SIZE, rgba)
    (OUT / "fleetworks-rock-tile.rgba").write_bytes(rgba)
    return png


def manifest(rgba, png, cap, mean, scale, final_mean):
    return {
        "schema": 1,
        "role": "Coarse crescent rock overlay for Submit, Confirm, Create, Delete and Connect",
        "recipe": "docs/design/matte-canvas/theme.md#deterministic-asset-and-tint-recipe",
        "source": {"path": "assets/brand/source/fleetworks-liquid-metal-glass.png", "sha256": SOURCE_SHA256},
        "crop_box_source_px": {"x": CROP[0], "y": CROP[1], "width": CROP[2], "height": CROP[3]},
        "excluded": "polished rim, glowing inclusions, molten sphere and highlights (luminance capped at the 97th percentile of the crop)",
        "luminance": "0.2126 R + 0.7152 G + 0.0722 B on sRGB bytes",
        "highlight_cap_luma": round(cap, 3),
        "tile": "mirror quilt: crop stacked with its mirror to 256 x 256, then mirrored on both axes to 512 x 512; seamless when repeated",
        "tile_size_px": SIZE,
        "displayed_logical_px": 256,
        "normalization": {
            "mean_luma": round(mean, 3),
            "scale_luma_p99_abs_deviation": round(scale, 3),
            "quantization_steps": STEPS,
            "mean_T_after_quantization": round(final_mean, 5),
        },
        "export": {
            "overlay_at_opacity_1": "T > 0: TEXT_PRIMARY #D5DCE5 alpha 0.22 T; T < 0: CAST_SHADOW #020304 alpha min(1, 1.25 |T|)",
            "opacity_by_state": {"rest": 0.12, "hover": 0.14, "pressed": 0.10, "disabled": 0.04},
        },
        "outputs": {
            "fleetworks-rock-tile.png": {"bytes": len(png), "sha256": hashlib.sha256(png).hexdigest()},
            "fleetworks-rock-tile.rgba": {
                "bytes": len(rgba),
                "sha256": hashlib.sha256(rgba).hexdigest(),
                "fnv1a32": fnv1a32(rgba),
            },
        },
    }


def main():
    rgba, cap, mean, scale, final_mean = derive()
    if "--check" in sys.argv:
        committed = (OUT / "fleetworks-rock-tile.rgba").read_bytes()
        assert committed == rgba, "the committed .rgba differs from the derivation"
        print("fleetworks-rock-tile.rgba matches the derivation", fnv1a32(rgba))
        return
    png = outputs(rgba)
    data = manifest(rgba, png, cap, mean, scale, final_mean)
    (OUT / "fleetworks-rock-tile.manifest.json").write_text(json.dumps(data, indent=2) + "\n")
    # The web build embeds the PNG, as FleetworksMark embeds its vector, so it needs no asset URL.
    uri = "data:image/png;base64," + base64.b64encode(png).decode()
    web = ROOT / "packages/ui/src/rockTileData.ts"
    web.write_text(
        "// Generated by assets/brand/source/derive_rock_tile.py from the approved crescent crop.\n"
        "// Do not edit. Its sha256 is in assets/brand/rock/fleetworks-rock-tile.manifest.json.\n"
        f'export const ROCK_TILE_DATA_URI =\n  "{uri}";\n'
    )
    print("wrote", len(png), "byte PNG,", len(rgba), "byte RGBA, fnv1a32", fnv1a32(rgba))


if __name__ == "__main__":
    main()
