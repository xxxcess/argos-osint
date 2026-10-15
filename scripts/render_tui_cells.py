#!/usr/bin/env python3
"""Render ratatui TestBackend cell dumps (JSON) to PNG."""
import argparse, json, re, sys, pathlib, math, unicodedata
from PIL import Image, ImageDraw, ImageFont
FONT = next((p for p in (
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/System/Library/Fonts/Supplemental/Courier New.ttf",
) if pathlib.Path(p).is_file()), None)
BOLD = next((p for p in (
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
    "/System/Library/Fonts/Supplemental/Courier New Bold.ttf",
) if pathlib.Path(p).is_file()), FONT)
WIDE_FONT = next((p for p in (
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
) if pathlib.Path(p).is_file()), None)
SIZE = 16
NAMED = {"Reset": None, "Black": (0,0,0), "Red": (205,49,49), "Green": (13,188,121), "Yellow": (229,229,16),
         "Blue": (36,114,200), "Magenta": (188,63,188), "Cyan": (17,168,205), "Gray": (204,204,204),
         "DarkGray": (118,118,118), "LightRed": (241,76,76), "LightGreen": (35,209,139), "LightYellow": (245,245,67),
         "LightBlue": (59,142,234), "LightMagenta": (214,112,214), "LightCyan": (41,184,219), "White": (242,242,242)}
DEF_FG, DEF_BG = (232,232,232), (20,20,20)
def color(raw, default):
    m = re.match(r"Rgb\((\d+), (\d+), (\d+)\)", raw)
    if m: return tuple(int(v) for v in m.groups())
    c = NAMED.get(raw)
    return default if c is None else c
def render(src, dst):
    d = json.load(open(src))
    font, bold = ImageFont.truetype(FONT, SIZE), ImageFont.truetype(BOLD, SIZE)
    wide = ImageFont.truetype(WIDE_FONT, SIZE) if WIDE_FONT else None
    l, t, r, b = font.getbbox("M"); cw = round(font.getlength("M")); ch = SIZE + 4
    pad = 12
    img = Image.new("RGB", (d["width"]*cw + 2*pad, d["height"]*ch + 2*pad), DEF_BG)
    dr = ImageDraw.Draw(img)
    # Paint backgrounds first: a wide glyph spans the following continuation cell.
    for y, row in enumerate(d["cells"]):
        for x, c in enumerate(row):
            px, py = pad + x*cw, pad + y*ch
            dr.rectangle([px, py, px+cw-1, py+ch-1], fill=color(c["bg"], DEF_BG))
    for y, row in enumerate(d["cells"]):
        for x, c in enumerate(row):
            px, py = pad + x*cw, pad + y*ch
            fg = color(c["fg"], DEF_FG)
            s = c["s"]
            if not s.strip(): continue
            # Box drawing: draw geometrically so lines join across cells.
            if box(dr, s, px, py, cw, ch, fg): continue
            if wide and any(unicodedata.east_asian_width(char) in ("W", "F") for char in s):
                _, top, _, bottom = wide.getbbox(s)
                dr.text((px, py + (ch - (bottom - top)) // 2 - top), s, font=wide, fill=fg)
            else:
                dr.text((px, py+1), s, font=bold if c["b"] else font, fill=fg)
            if c["u"]: dr.line([px, py+ch-2, px+cw-1, py+ch-2], fill=fg)
    img.save(dst)
H = "─━═"; V = "│┃║"
def box(dr, s, px, py, cw, ch, fg):
    cx, cy = px + cw//2, py + ch//2
    # Braille (spinners, maps): draw the 2x4 dot grid; most monospace fonts lack it.
    if len(s) == 1 and 0x2800 <= ord(s) <= 0x28FF:
        bits = ord(s) - 0x2800
        dots = [(0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08),
                (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80)]
        r = max(1, cw // 6)
        for col, row, bit in dots:
            if bits & bit:
                dx = px + cw * (1 + 2 * col) // 4
                dy = py + ch * (1 + 2 * row) // 8
                dr.ellipse([dx - r, dy - r, dx + r, dy + r], fill=fg)
        return True
    # Brain Topic/Inference glyphs are absent from some platform monospace fonts.
    if s in ("◆", "◇"):
        points = [(cx, py + 4), (px + cw - 2, cy), (cx, py + ch - 4), (px + 2, cy)]
        dr.polygon(points, fill=fg if s == "◆" else None, outline=fg)
        return True
    # Topology direction and tentative paths must remain legible in cell captures.
    if s in ("▶", "◀"):
        points = ([(px + 2, py + 4), (px + 2, py + ch - 4), (px + cw - 2, cy)]
                  if s == "▶" else
                  [(px + cw - 2, py + 4), (px + cw - 2, py + ch - 4), (px + 2, cy)])
        dr.polygon(points, fill=fg)
        return True
    if s in ("┄", "┆", "┊"):
        extent = cw if s == "┄" else ch
        for start in range(0, extent, 4):
            end = min(start + 1, extent - 1)
            segment = ([px + start, cy, px + end, cy] if s == "┄" else
                       [cx, py + start, cx, py + end])
            dr.line(segment, fill=fg)
        return True
    if s == "★":
        points = []
        for i in range(10):
            angle = -math.pi / 2 + i * math.pi / 5
            radius = 1.0 if i % 2 == 0 else 0.45
            points.append((cx + math.cos(angle) * (cw / 2 - 1) * radius,
                           cy + math.sin(angle) * (ch / 2 - 3) * radius))
        dr.polygon(points, fill=fg)
        return True
    # Terminal block/diagonal glyphs may be absent from platform fonts.
    if s in "█▉▊▋▌▍▎▏":
        fraction = (8 - "█▉▊▋▌▍▎▏".index(s)) / 8
        dr.rectangle([px, py, px + max(1, round(cw*fraction)) - 1, py+ch-1], fill=fg)
        return True
    if s in "▁▂▃▄▅▆▇":
        fraction = ("▁▂▃▄▅▆▇".index(s) + 1) / 8
        dr.rectangle([px, py + ch - max(1, round(ch*fraction)), px+cw-1, py+ch-1], fill=fg)
        return True
    if s in "╱╲":
        dr.line([px, py+ch-1 if s == "╱" else py, px+cw-1, py if s == "╱" else py+ch-1], fill=fg)
        return True
    if s in "▾▸":
        points = [(px+1, cy-3), (px+cw-2, cy-3), (cx, cy+3)] if s == "▾" else [(cx-3, cy-4), (cx-3, cy+4), (cx+3, cy)]
        dr.polygon(points, fill=fg)
        return True
    segs = {"─":"lr","│":"ud","┌":"rd","┐":"ld","└":"ru","┘":"lu","├":"udr","┤":"udl","┬":"lrd","┴":"lru","┼":"lrud",
            "╭":"rd","╮":"ld","╰":"ru","╯":"lu"}.get(s)
    if segs is None: return False
    for k in segs:
        if k=="l": dr.line([px, cy, cx, cy], fill=fg)
        if k=="r": dr.line([cx, cy, px+cw, cy], fill=fg)
        if k=="u": dr.line([cx, py, cx, cy], fill=fg)
        if k=="d": dr.line([cx, cy, cx, py+ch], fill=fg)
    return True
if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cells", type=pathlib.Path)
    parser.add_argument("screenshots", type=pathlib.Path)
    args = parser.parse_args()
    src, out = args.cells, args.screenshots
    if FONT is None:
        parser.error("Install DejaVu Sans Mono or Courier New for rendering")
    if not list(src.glob("*.json")):
        parser.error(f"No JSON cell dumps in {src}")
    out.mkdir(parents=True, exist_ok=True)
    for f in sorted(src.glob("*.json")):
        render(f, out / (f.stem + ".png")); print(out / (f.stem + ".png"))
