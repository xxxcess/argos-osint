"""Authoring aid, not an app dependency: draws the native Quiet Orbit scene as an SVG.

It ports the geometry, colors and facet hash of apps/fleetworks-desktop/src/orbit.rs, so a pose
can be compared with assets/brand/reference/fleetworks-quiet-orbit.png without building the app.
It does not use the Rune renderer: antialiasing, gradients and blending differ slightly, and a
change to orbit.rs must be mirrored here by hand.

    python3 native_preview.py <seconds> <out.svg> [scale]
"""
import math, sys

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


RIB = [
    (-0.28, 0.22, 1.0, 0.0, 1, 0.16, '#E6D6AE', 0.38),
    (-0.08, 0.26, 1.1, 1.1, -1, 0.18, '#C98A3E', 0.4),
    (0.1, 0.22, 0.9, 2.0, 1, 0.17, '#F2E6C6', 0.34),
    (0.26, 0.2, 1.2, 0.6, -1, 0.14, '#8A5A22', 0.42),
    (-0.4, 0.14, 1.5, 2.8, 2, 0.08, '#FFF3D2', 0.3),
    (0.38, 0.14, 1.0, 1.7, 1, 0.1, '#C98A3E', 0.34),
    (-0.02, 0.3, 0.8, 3.6, 2, 0.07, '#FFF6DC', 0.42),
    (0.16, 0.12, 1.6, 4.4, -2, 0.07, '#3B2814', 0.45),
    (-0.2, 0.18, 1.1, 5.0, -1, 0.1, '#2A1C0E', 0.38),
    (0.0, 0.34, 0.7, 0.4, 1, 0.22, '#F5EAD0', 0.2),
    (-0.34, 0.16, 1.3, 3.1, -2, 0.09, '#B8823A', 0.34),
    (0.3, 0.18, 1.4, 5.6, 2, 0.08, '#EFE0B8', 0.3),
]


def flow(t):
    turn = phase(t)
    out = []
    for lift, sway, waves, off, turns, width, color, alpha in RIB:
        centre = []
        for i in range(25):
            u = -1 + 2 * i / 24
            x = 0.8 * u * R
            y = (lift + sway * math.sin(PI * waves * u + off + turns * turn)) * R
            x, y = x * math.cos(-0.38) - y * math.sin(-0.38), x * math.sin(-0.38) + y * math.cos(-0.38)
            reach = math.hypot(x, y)
            k = 0.74 * R / reach if reach > 0.74 * R else 1.0
            centre.append((CORE[0] + x * k, CORE[1] + y * k))
        left, right = [], []
        for i in range(25):
            b = centre[max(i - 1, 0)]
            a = centre[min(i + 1, 24)]
            dx, dy = a[0] - b[0], a[1] - b[1]
            ln = max(math.hypot(dx, dy), 1e-4)
            u = -1 + 2 * i / 24
            half = max(0.5 * width * R * max(1 - u * u, 0) ** 0.7, 0.15)
            nx, ny = -dy / ln, dx / ln
            left.append((centre[i][0] + nx * half, centre[i][1] + ny * half))
            right.append((centre[i][0] - nx * half, centre[i][1] - ny * half))
        out.append((left + right[::-1], color, alpha))
    return out


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
    out.append('<defs><radialGradient id="g" gradientUnits="userSpaceOnUse" cx="%.2f" cy="%.2f" r="%.2f">'
               '<stop offset="0" stop-color="#C9A66A"/><stop offset=".35" stop-color="#8A6A3E"/>'
               '<stop offset=".7" stop-color="#3A2A18"/><stop offset="1" stop-color="#120E0A"/></radialGradient></defs>'
               % (CORE[0] - R + 0.40 * 2 * R, CORE[1] - R + 0.60 * 2 * R, 2 * R * 0.62))
    out.append('<circle cx="%s" cy="%s" r="%s" fill="#0F0C09"/>' % (CORE[0], CORE[1], R))
    out.append('<circle cx="%s" cy="%s" r="%s" fill="url(#g)"/>' % (CORE[0], CORE[1], R))
    for pts, color, alpha in flow(t):
        out.append(poly(pts, color, alpha))
    out.append('<circle cx="%.2f" cy="%.2f" r="%.2f" fill="#fff" fill-opacity="0.045"/>' % (CORE[0] - 0.3 * R, CORE[1] - 0.34 * R, 0.46 * R))
    out.append('<circle cx="%s" cy="%s" r="%s" fill="none" stroke="#050403" stroke-opacity="0.45" stroke-width="3"/>' % (CORE[0], CORE[1], R - 1.8))
    out.append('<circle cx="%s" cy="%s" r="%s" fill="none" stroke="#CFC5AA" stroke-opacity="0.45" stroke-width="1"/>' % (CORE[0], CORE[1], R - 0.5))
    a0, a1 = PI * 1.12, PI * 1.45
    arc = " ".join("%.2f,%.2f" % (CORE[0] + (R - 4) * math.cos(a0 + (a1 - a0) * i / 12), CORE[1] + (R - 4) * math.sin(a0 + (a1 - a0) * i / 12)) for i in range(13))
    out.append('<polyline points="%s" fill="none" stroke="#F3EEE4" stroke-opacity="0.4" stroke-width="1.2"/>' % arc)
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
