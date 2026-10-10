#!/usr/bin/env python3
"""Render ratatui TestBackend cell dumps (JSON) to PNG."""
import argparse, json, re, sys, pathlib
from PIL import Image, ImageDraw, ImageFont
FONT = next((p for p in (
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/System/Library/Fonts/Supplemental/Courier New.ttf",
) if pathlib.Path(p).is_file()), None)
BOLD = next((p for p in (
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
    "/System/Library/Fonts/Supplemental/Courier New Bold.ttf",
) if pathlib.Path(p).is_file()), FONT)
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
    l, t, r, b = font.getbbox("M"); cw = round(font.getlength("M")); ch = SIZE + 4
    pad = 12
    img = Image.new("RGB", (d["width"]*cw + 2*pad, d["height"]*ch + 2*pad), DEF_BG)
    dr = ImageDraw.Draw(img)
    for y, row in enumerate(d["cells"]):
        for x, c in enumerate(row):
            px, py = pad + x*cw, pad + y*ch
            bg = color(c["bg"], DEF_BG); fg = color(c["fg"], DEF_FG)
            dr.rectangle([px, py, px+cw-1, py+ch-1], fill=bg)
            s = c["s"]
            if not s.strip(): continue
            # Box drawing: draw geometrically so lines join across cells.
            if box(dr, s, px, py, cw, ch, fg): continue
            dr.text((px, py+1), s, font=bold if c["b"] else font, fill=fg)
            if c["u"]: dr.line([px, py+ch-2, px+cw-1, py+ch-2], fill=fg)
    img.save(dst)
H = "─━═"; V = "│┃║"
def box(dr, s, px, py, cw, ch, fg):
    cx, cy = px + cw//2, py + ch//2
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
