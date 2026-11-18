r"""AgX reference values for tonality's tests (see docs/agx.md).

For linear Rec.2020 (D65) scene colours -> sRGB display code values, two ways:
  ocio:  Blender's shipped config + LUTs (view "AgX", display "sRGB"), through OpenColorIO
         (the LUT files are data; no downloaded code runs).
  exact: AgX Base written out here from its published definition (Eary Chow's AgX_LUT_Gen and
         Troy Sobotka's sigmoid / working-space scripts), evaluated per colour without a LUT.
It also checks the exact version against the values stored at the LUT's grid points.

Usage (Python 3.12; OpenColorIO has no wheels for newer Pythons yet):
  uv venv -p 3.12 /tmp/agx && VIRTUAL_ENV=/tmp/agx uv pip install opencolorio colour-science numpy
  mkdir -p /tmp/blender/luts && B=https://raw.githubusercontent.com/blender/blender/main/release/datafiles/colormanagement
  curl -L -o /tmp/blender/config.ocio $B/config.ocio
  for f in AgX_Base_sRGB.cube AgX_Base_Rec2020.cube AgX_Base_P3.cube luminance_compensation_bt2020.cube \
    Guard_Rail_Shaper_EOTF.spi1d xyz_E_to_D65.spimtx AgX_False_Color.spi1d AgX_Rec2100-HLG_p3_lim.cube; do
    curl -L -o /tmp/blender/luts/$f $B/luts/$f; done
  /tmp/agx/bin/python scripts/agx_reference.py /tmp/blender crates/tonality-core/tests/data/agx_reference.json
"""
import json
import sys

import numpy as np
import colour
import PyOpenColorIO as ocio

colour.utilities.filter_warnings(python_warnings=True)

BLENDER, OUT = sys.argv[1], sys.argv[2]
# Guard-rail luminance weights (Rec.2020, CIE 2015 CMFs). Eary's scripts moved to
# [0.2589235355689848, 0.6104985346066525, 0.13057792982436284] in June 2025; Blender's shipped
# LUTs were made with these earlier ones, and only these reproduce them.
LUMA = np.array([0.2658180370250449, 0.59846986045365, 0.1357121025213052])

REC2020 = colour.RGB_COLOURSPACES["ITU-R BT.2020"]
REC709 = colour.RGB_COLOURSPACES["ITU-R BT.709"]
M2020_TO_709 = REC709.matrix_XYZ_to_RGB @ REC2020.matrix_RGB_to_XYZ
M709_TO_2020 = np.linalg.inv(M2020_TO_709)

# ---- working space (Sobotka's create_workingspace, without shapely) ----


def hull_point(p, w, angle_deg, tri):
    """Where the ray from white `w` through primary `p`, rotated by `angle_deg` (CCW) about w,
    leaves the triangle `tri`."""
    a = np.radians(angle_deg)
    d = p - w
    d = np.array([d[0] * np.cos(a) - d[1] * np.sin(a), d[0] * np.sin(a) + d[1] * np.cos(a)])
    best = None
    for i in range(3):
        e0, e1 = tri[i], tri[(i + 1) % 3]
        e = e1 - e0
        m = np.array([[d[0], -e[0]], [d[1], -e[1]]])
        if abs(np.linalg.det(m)) < 1e-15:
            continue
        t, u = np.linalg.solve(m, e0 - w)
        if t > 0 and -1e-12 <= u <= 1 + 1e-12 and (best is None or t < best):
            best = t
    return w + best * d


def working_matrix(rotate, scale, cs=REC2020):
    """RGB in the inset/rotated space -> RGB in `cs` (Eary: XYZ_to_RGB(cs) @ RGB_to_XYZ(space))."""
    prim = np.asarray(cs.primaries, dtype=float)
    w = np.asarray(cs.whitepoint, dtype=float)
    pts = []
    for p, r, s in zip(prim, rotate, scale):
        h = hull_point(p, w, r, prim)
        pts.append(w + (1.0 - s) * (h - w))
    npm = colour.normalised_primary_matrix(np.array(pts), w)
    return cs.matrix_XYZ_to_RGB @ npm


INSET = working_matrix([2.13976149, -1.22827335, -3.05174246], [0.32965205, 0.28051336, 0.12475368])
OUTSET = np.linalg.inv(working_matrix([0.0, 0.0, 0.0], [0.32317438, 0.28325605, 0.0374326]))

# ---- sigmoid (Jed Smith's tunable sigmoid, as in Sobotka's sigmoid.py; lengths 0) ----
LOG_MIN, LOG_MAX, GREY = -10.0, 6.5, 0.18
PX = -LOG_MIN / (LOG_MAX - LOG_MIN)
PY = 0.18 ** (1 / 2.4)
SLOPE, POWER = 2.4, 1.5


def _scale(lx, ly, tx, ty, p, m):
    return ((m * (lx - tx)) ** -p * ((m * (lx - tx) / (ly - ty)) ** p - 1.0)) ** (-1.0 / p)


S_TOE = -_scale(1.0, 1.0, 1.0 - PX, 1.0 - PY, POWER, SLOPE)
S_SHOULDER = _scale(1.0, 1.0, PX, PY, POWER, SLOPE)


def _curve(x, s, tx, ty, p):
    u = SLOPE * (x - tx) / s
    return s * u / (1.0 + u ** p) ** (1.0 / p) + ty


def sigmoid(x):
    if x < PX:
        return _curve(x, S_TOE, PX, PY, POWER)
    if x <= PX:
        return SLOPE * (x - PX) + PY
    return _curve(x, S_SHOULDER, PX, PY, POWER)


# ---- guard rails (Sakari Kapanen / Eary Chow) ----


def low_side_2020(rgb):
    y = rgb @ LUMA
    inv = rgb.max() - rgb
    y_comp = inv.max() - inv @ LUMA + y
    off = rgb + max(-rgb.min(), 0.0)
    inv_o = off.max() - off
    y_new = inv_o.max() - inv_o @ LUMA + off @ LUMA
    ratio = y_comp / y_new if y_new > y_comp else 1.0
    return ratio * off


def low_side_709(rgb):
    lum = lambda c: (M709_TO_2020 @ c) @ LUMA
    y = lum(rgb)
    inv = rgb.max() - rgb
    y_comp = inv.max() - lum(inv) + y
    y = np.nan if y < 0 else (1 - min(y ** 0.08, 1)) * y_comp + min(y ** 0.08, 1) * y
    off = rgb + max(-rgb.min(), 0.0)
    inv_o = off.max() - off
    y_new = lum(off)
    y_new_comp = inv_o.max() - lum(inv_o) + y_new
    y_new = np.nan if y_new < 0 else (1 - min(y_new ** 0.08, 1)) * y_new_comp + min(y_new ** 0.08, 1) * y_new
    ratio = y / max(y_new, 1e-100) if y_new > y else 1.0  # comparisons with NaN are false
    return ratio * off


# ---- hue: HSV (hexcone), as colour.RGB_to_HSV ----


def hue(c):
    mx, mn = c.max(), c.min()
    d = mx - mn
    if d == 0:
        return 0.0
    r, g, b = c
    if mx == r:
        h = ((g - b) / d) % 6
    elif mx == g:
        h = (b - r) / d + 2
    else:
        h = (r - g) / d + 4
    return h / 6.0


def with_hue(c, h):
    v = c.max()
    s = 0.0 if v == 0 else (v - c.min()) / v
    return np.asarray(colour.HSV_to_RGB(np.array([h, s, v])), dtype=float)


def lerp_hue(h1, h2, t):
    d = h2 - h1
    if d > 0.5:
        h2 -= 1.0
    elif d < -0.5:
        h2 += 1.0
    return (h1 + t * (h2 - h1)) % 1.0


def srgb_oetf(x):
    return np.where(x <= 0.0031308, 12.92 * x, 1.055 * np.power(np.maximum(x, 0), 1 / 2.4) - 0.055)


# Blender's LUT input: E-Gamut, log2 over -10..+15 stops around middle grey (per channel clamp).
E_GAMUT_TO_XYZ = np.array([[0.7053968501, 0.1640413283, 0.08101774865],
                           [0.2801307241, 0.8202066415, -0.1003373656],
                           [-0.1037815116, -0.07290725703, 1.265746519]])
M2020_TO_EG = np.linalg.inv(E_GAMUT_TO_XYZ) @ REC2020.matrix_RGB_to_XYZ
EG_TO_2020 = np.linalg.inv(M2020_TO_EG)


def formation(c):
    """Steps 2-11 of docs/agx.md: Rec.2020 (inside the input domain) -> linear display Rec.709."""
    c = low_side_2020(c)
    c = INSET @ c
    h0 = hue(c)
    with np.errstate(divide="ignore"):
        log = (np.log2(np.maximum(c, 1e-300) / GREY) - LOG_MIN) / (LOG_MAX - LOG_MIN)
    c = np.array([sigmoid(v) for v in log])
    c = np.maximum(c, 0.0) ** 2.4
    c = with_hue(c, lerp_hue(h0, hue(c), 0.4))
    c = OUTSET @ c
    c = M2020_TO_709 @ c
    return low_side_709(c)


def exact(rgb2020):
    c = np.asarray(rgb2020, dtype=float)
    c = EG_TO_2020 @ np.clip(M2020_TO_EG @ c, GREY * 2.0 ** -10, GREY * 2.0 ** 15)
    return srgb_oetf(np.clip(np.nan_to_num(formation(c)), 0.0, 1.0))


def check_nodes():
    """Exact vs the values stored at AgX_Base_sRGB.cube's grid points (E-Gamut log2 input)."""
    lines = [l for l in open(BLENDER + "/luts/AgX_Base_sRGB.cube") if l[:1] in "-0123456789."]
    lut = np.array([[float(v) for v in l.split()] for l in lines])
    n = round(len(lut) ** (1 / 3))
    errs = []
    for k in np.random.default_rng(1).choice(len(lut), 4000, replace=False):
        idx = np.array([k % n, (k // n) % n, k // (n * n)])  # .cube: red changes fastest
        eg = GREY * 2.0 ** (idx / (n - 1) * 25.0 - 10.0)
        c = formation(EG_TO_2020 @ eg)
        c = np.clip(np.nan_to_num(np.maximum(c, 0) ** (1 / 2.4)), 0, 1)
        errs.append(np.abs(c - lut[k]).max())
    errs = np.array(errs)
    print(f"LUT grid points ({len(errs)} of {n}^3): |table - exact| mean {errs.mean():.6f}, max {errs.max():.5f}")


def samples():
    out = [[0.0, 0.0, 0.0]]
    for ev in range(-12, 13):
        g = 0.18 * 2.0 ** ev
        out.append([g, g, g])
    to2020 = REC2020.matrix_XYZ_to_RGB @ REC709.matrix_RGB_to_XYZ
    base = [
        [1, 0, 0], [0, 1, 0], [0, 0, 1], [1, 1, 0], [0, 1, 1], [1, 0, 1],
        list(to2020 @ [1, 0, 0]), list(to2020 @ [0, 1, 0]), list(to2020 @ [0, 0, 1]),
        [0.8, 0.5, 0.35], [0.3, 0.5, 0.9], [0.9, 0.6, 0.1], [0.2, 0.7, 0.3], [0.6, 0.2, 0.5],
    ]
    for c in base:
        c = np.array(c, dtype=float)
        c = c / max(np.dot(c, [0.2627, 0.6780, 0.0593]), 1e-6) * 0.18
        for ev in (-6, -3, -1, 0, 1, 3, 5, 8):
            out.append(list(c * 2.0 ** ev))
    for c in ([0.30, 0.10, -0.02], [-0.03, 0.25, 0.10], [0.05, -0.01, 0.40], [0.9, -0.05, 0.02]):
        for ev in (-2, 0, 2):
            out.append(list(np.array(c) * 2.0 ** ev))
    rng = np.random.default_rng(2383)
    for _ in range(200):
        ev = rng.uniform(-9, 7)
        c = rng.uniform(0, 1, 3)
        c = c / max(np.dot(c, [0.2627, 0.6780, 0.0593]), 1e-6) * 0.18 * 2.0 ** ev
        out.append(list(c))
    return out


def main():
    print("inset\n", INSET, "\noutset\n", OUTSET)
    check_nodes()
    cfg = ocio.Config.CreateFromFile(BLENDER + "/config.ocio")
    proc = cfg.getProcessor(ocio.DisplayViewTransform(src="Linear Rec.2020", display="sRGB", view="AgX")).getDefaultCPUProcessor()
    rows = []
    for c in samples():
        px = np.array([c], dtype=np.float32)
        proc.applyRGB(px)
        rows.append({"in": [float(v) for v in c], "ocio": [float(v) for v in px[0]], "exact": [float(v) for v in exact(c)]})
    d = np.array([[abs(a - b) for a, b in zip(r["ocio"], r["exact"])] for r in rows])
    print(f"{len(rows)} samples; |ocio - exact| max {d.max():.5f}, mean {d.mean():.5f}, 99th pct {np.percentile(d, 99):.5f}")
    for i in np.argsort(d.max(axis=1))[-8:]:
        r = rows[i]
        print("  worst:", [round(v, 5) for v in r["in"]], "ocio", [round(v, 4) for v in r["ocio"]], "exact", [round(v, 4) for v in r["exact"]])
    r = lambda v: [round(x, 9) for x in v]
    rows = [{"in": r(x["in"]), "blender": r(x["ocio"]), "exact": r(x["exact"])} for x in rows]
    with open(OUT, "w") as f:
        f.write('{"source": "scripts/agx_reference.py", "inset": %s, "outset": %s, "rows": [\n' % (
            json.dumps([r(v) for v in INSET.tolist()]), json.dumps([r(v) for v in OUTSET.tolist()])))
        f.write(",\n".join(json.dumps(x) for x in rows))
        f.write("\n]}\n")


main()
