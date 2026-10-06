#!/usr/bin/env python3
"""Render ratatui TestBackend cell dumps (JSON) to PNG."""
import json, re, sys, pathlib
from PIL import Image, ImageDraw, ImageFont
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
SIZE = 16
NAMED = {"Reset": None, "Black": (0,0,0), "Red": (205,49,49), "Green": (13,188,121), "Yellow": (229,229,16),
         "Blue": (36,114,200), "Magenta": (188,63,188), "Cyan": (17,168,205), "Gray": (204,204,204),
         "DarkGray": (118,118,118), "LightRed": (241,76,76), "LightGreen": (35,209,139), "LightYellow": (245,245,67),
         "LightBlue": (59,142,234), "LightMagenta": (214,112,214), "LightCyan": (41,184,219), "White": (242,242,242)}
DEF_FG, DEF_BG = (214,232,236), (6,14,22)
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
    src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    for f in sorted(src.glob("*.json")):
        render(f, out / (f.stem + ".png")); print(out / (f.stem + ".png"))
