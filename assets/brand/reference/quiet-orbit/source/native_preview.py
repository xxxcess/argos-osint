"""Authoring aid, not an app dependency: draws the native Quiet Orbit scene as an SVG.

It ports the geometry, colors and facet hash of apps/fleetworks-desktop/src/orbit.rs, so a pose
can be compared with assets/brand/reference/fleetworks-quiet-orbit.png without building the app.
It does not use the Rune renderer: antialiasing, gradients and blending differ slightly, and a
change to orbit.rs must be mirrored here by hand.

    python3 native_preview.py <seconds> <out.svg> [scale]
"""
import base64
import math
import sys
from pathlib import Path

M = 0xFFFFFFFF
SEED = 0xA4605A11


def mix(x):
    x &= M
    x ^= x >> 16
    x = (x * 0x7FEB352D) & M
    x ^= x >> 15
    x = (x * 0x846CA68B) & M
    return (x ^ (x >> 16)) & M


def noise(i, j, salt):
    return mix(mix(SEED ^ ((salt * 0x9E3779B1) & M)) ^ (j * 16 + i))


PI, TAU = math.pi, math.tau
ENV = (144.0, 176.0)
CORE = (72.0, 61.0)
R = 40.0
CC = (72.0, 105.0)
CR = 62.0
CUT = (72.0, 69.0)
CUTR = 50.0
RATIO = 1.3031175
LIFT = 38.0 / 99.760765 * R
INCL = 66 * PI / 180
PS = 0.70
PW, PH = 22.7, 22.0
PD = 7.5
STILL = 1.5
LOOP = 8.0
FLECKS = [(72, 142), (44, 149), (96, 135), (110, 143), (62, 159)]


def inside(p, m):
    return math.hypot(p[0] - CC[0], p[1] - CC[1]) + m < CR and math.hypot(p[0] - CUT[0], p[1] - CUT[1]) - m > CUTR


def facets():
    def vertex(i, j):
        jit = lambda s: (noise(i, j, s) % 1000) / 1000 - 0.5
        return (8 + i * 10 + jit(21) * 6, 40 + j * 10 + jit(22) * 6)
    out = []
    for j in range(13):
        for i in range(14):
            a, b, c, d = vertex(i, j), vertex(i + 1, j), vertex(i + 1, j + 1), vertex(i, j + 1)
            halves = [[a, b, c], [a, c, d]] if noise(i, j, 23) & 1 == 0 else [[a, b, d], [b, c, d]]
            for n, pts in enumerate(halves):
                if all(inside(p, 1.5) for p in pts):
                    out.append((pts, (noise(i, j, 24 + n) % 1000) / 1000, noise(i, j, 26) % 3 == 0))
    return out


def phase(t):
    return TAU * ((t / LOOP) % 1.0)


def satellites(t):
    ph = phase(t)
    ring = R * RATIO
    res = []
    for idx in range(5):
        th = PI / 2 + ph + TAU * idx / 5
        x3 = ring * math.cos(th)
        y3 = ring * math.sin(th) * math.cos(INCL)
        z = ring * math.sin(th) * math.sin(INCL)
        per = 1.0 / (1.0 - z / (R * PD))
        sx, sy = x3 * per, -y3 * per
        cx, cy = CORE[0] + sx, CORE[1] - LIFT + sy
        sc = per * PS
        yaw = 0.82 + 0.18 * abs(math.sin(th))
        turn = math.atan2(sy, sx) + PI / 2 + 0.055 * math.sin(2 * ph + idx)
        s, c = math.sin(turn), math.cos(turn)
        local = [(0, -PH * 2 / 3), (-PW / 2 * yaw, PH / 3), (PW / 2 * yaw, PH / 3)]
        res.append(([(cx + sc * (x * c - y * s), cy + sc * (x * s + y * c)) for x, y in local], z))
    return res


CORE_TEXTURE = Path(__file__).resolve().parents[4] / "brand/orbit/fleetworks-core-texture.png"
MARGIN, DRIFT = 1.12, 0.035


def core_drift(t):
    turn = phase(t)
    return [(DRIFT * math.sin(turn), DRIFT * math.cos(turn)),
            (-0.8 * DRIFT * math.sin(2 * turn), 0.8 * DRIFT * math.cos(2 * turn + 1.0))]



def crescent():
    d = CC[1] - CUT[1]
    a = (d * d + CR * CR - CUTR * CUTR) / (2 * d)
    h = math.sqrt(CR * CR - a * a)
    tip_y = CC[1] - a

    def arc(c, r, f, t, n):
        return [(c[0] + r * math.cos(f + (t - f) * i / n), c[1] + r * math.sin(f + (t - f) * i / n)) for i in range(n + 1)]
    pts = arc(CC, CR, math.atan2(-a, h), math.atan2(-a, -h) + TAU, 40)
    inner = arc(CUT, CUTR, math.atan2(tip_y - CUT[1], -h) + TAU, math.atan2(tip_y - CUT[1], h), 40)
    return pts + inner[1:-1]


def lerp(c1, c2, t):
    a = [int(c1[i:i + 2], 16) for i in (1, 3, 5)]
    b = [int(c2[i:i + 2], 16) for i in (1, 3, 5)]
    return "#%02x%02x%02x" % tuple(round(a[k] + (b[k] - a[k]) * t) for k in range(3))


def poly(pts, fill, op=1.0, stroke=None, sw=0, sop=1.0):
    p = " ".join("%.2f,%.2f" % q for q in pts)
    s = ' stroke="%s" stroke-width="%s" stroke-opacity="%s"' % (stroke, sw, sop) if stroke else ""
    f = 'fill="%s" fill-opacity="%s"' % (fill, op) if fill else 'fill="none"'
    return '<polygon points="%s" %s%s/>' % (p, f, s)


def prism(sat):
    (ap, l, r), _ = sat
    mid = ((l[0] + r[0]) / 2, (l[1] + r[1]) / 2)
    ce = ((ap[0] + l[0] + r[0]) / 3, (ap[1] + l[1] + r[1]) / 3)
    tw = lambda p, t: (p[0] + (ce[0] - p[0]) * t, p[1] + (ce[1] - p[1]) * t)
    return "".join([
        poly([ap, l, r], "#07090C"),
        poly([ap, l, mid], "#232C38"),
        poly([ap, mid, r], "#0C1015"),
        poly([tw(ap, .45), tw(l, .45), tw(r, .45)], "#10151C", 0.85),
        poly([l, mid, tw(mid, .5)], "#D9B77A", 0.16),
        '<line x1="%.2f" y1="%.2f" x2="%.2f" y2="%.2f" stroke="#D5DEEA" stroke-opacity="0.75" stroke-width="0.6"/>' % (ap[0], ap[1], l[0], l[1]),
        poly([ap, l, r], None, 1, "#9AA6B6", 0.6, 0.40),
    ])


def scene(t, scale):
    sats = satellites(t)
    out = ['<rect width="%d" height="%d" fill="#060709"/>' % (ENV[0], ENV[1])]
    for s in sats:
        if s[1] < 0:
            out.append(prism(s))
    cp = crescent()
    out.append(poly(cp, "#0A0C10"))
    for pts, tone, lit in facets():
        out.append(poly(pts, lerp("#07090C", "#323A46", tone ** 2.0), 1, "#04060A", 0.35, 0.7))
        if lit:
            out.append('<line x1="%.2f" y1="%.2f" x2="%.2f" y2="%.2f" stroke="#AEB8C8" stroke-opacity="0.38" stroke-width="0.5"/>' % (pts[0][0], pts[0][1], pts[1][0], pts[1][1]))
    out.append(poly(cp, None, 1, "#A8B2C0", 1.0, 0.62))
    for n, (x, y) in enumerate(FLECKS):
        out.append('<circle cx="%d" cy="%d" r="1" fill="%s" fill-opacity="%s"/>' % (x, y, "#F0DDA8" if n < 3 else "#C9D2E0", 0.6 if n < 3 else 0.35))
    d = 2 * R
    image = d * MARGIN
    uri = "data:image/png;base64," + base64.b64encode(CORE_TEXTURE.read_bytes()).decode()
    out.append('<defs><clipPath id="c"><circle cx="%s" cy="%s" r="%s"/></clipPath></defs>' % (CORE[0], CORE[1], R))
    out.append('<circle cx="%s" cy="%s" r="%s" fill="#0B0907"/>' % (CORE[0], CORE[1], R))
    drift = core_drift(t)
    for (dx, dy), opacity in ((drift[1], 1.0), (drift[0], 0.55)):
        out.append('<image href="%s" x="%.2f" y="%.2f" width="%.2f" height="%.2f" opacity="%s" clip-path="url(#c)"/>' % (uri, CORE[0] - image / 2 + dx * d, CORE[1] - image / 2 + dy * d, image, image, opacity))
    out.append('<circle cx="%s" cy="%s" r="%s" fill="none" stroke="#CFC5AA" stroke-opacity="0.3" stroke-width="0.8"/>' % (CORE[0], CORE[1], R - 0.5))
    for s in sats:
        if s[1] >= 0:
            out.append(prism(s))
    w, h = ENV[0] * scale, ENV[1] * scale
    return '<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" viewBox="0 0 %d %d">%s</svg>' % (w, h, ENV[0], ENV[1], "".join(out))


if __name__ == "__main__":
    t = float(sys.argv[1])
    path = sys.argv[2]
    scale = float(sys.argv[3]) if len(sys.argv) > 3 else 4
    open(path, "w").write(scene(t, scale))
    fs = facets()
    print("facets", len(fs), "tone range", min(f[1] for f in fs), max(f[1] for f in fs), "lit", sum(f[2] for f in fs))
