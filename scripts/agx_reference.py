r"""AgX reference values for tonality's tests (see docs/agx.md).

For linear Rec.2020 (D65) scene colours, three display targets, each two ways:
  sRGB        AgX Base, sRGB code values          (Blender: display "sRGB", view "AgX")
  Display P3  AgX Base, Display P3 code values    (display "Display P3", view "AgX")
  HDR         AgX HDR, 1000 nits, Rec.2100 HLG    (display "Rec.2100-HLG", view "AgX - HDR 1000 nits")
  ocio:  Blender's shipped config + LUTs, through OpenColorIO (the LUT files are data; no
         downloaded code runs).
  exact: AgX written out here from its published definition (Eary Chow's AgX_LUT_Gen, including
         AgXHLG.py for HDR, and Troy Sobotka's sigmoid / working-space scripts), per colour,
         without a LUT.
It also checks the exact versions against the values stored at each LUT's grid points.

Usage (Python 3.12; OpenColorIO has no wheels for newer Pythons yet):
  uv venv -p 3.12 /tmp/agx && VIRTUAL_ENV=/tmp/agx uv pip install opencolorio colour-science numpy
  mkdir -p /tmp/blender/luts && B=https://raw.githubusercontent.com/blender/blender/main/release/datafiles/colormanagement
  curl -L -o /tmp/blender/config.ocio $B/config.ocio
  for f in AgX_Base_sRGB.cube AgX_Base_Rec2020.cube AgX_Base_P3.cube luminance_compensation_bt2020.cube \
    Guard_Rail_Shaper_EOTF.spi1d xyz_E_to_D65.spimtx AgX_False_Color.spi1d AgX_Rec2100-HLG_p3_lim.cube; do
    curl -L -o /tmp/blender/luts/$f $B/luts/$f; done
  /tmp/agx/bin/python scripts/agx_reference.py /tmp/blender crates/tonality-core/tests/data/agx_reference.json
Add ALL_NODES=1 to check every grid point instead of a random 4000 per LUT (slow).
"""
import json
import math
import os
import sys

import numpy as np
import colour
import PyOpenColorIO as ocio

colour.utilities.filter_warnings(python_warnings=True)

BLENDER, OUT = sys.argv[1], sys.argv[2]
# Guard-rail luminance weights (Rec.2020, CIE 2015 CMFs). Eary's scripts moved from the first to
# the second in June 2025. Blender's AgX Base LUTs (sRGB, P3) were made before that, its HDR LUT
# (August 2025) after; each set reproduces only its own LUTs.
LUMA_BASE = np.array([0.2658180370250449, 0.59846986045365, 0.1357121025213052])
LUMA_HDR = np.array([0.2589235355689848, 0.6104985346066525, 0.13057792982436284])

REC2020 = colour.RGB_COLOURSPACES["ITU-R BT.2020"]
REC709 = colour.RGB_COLOURSPACES["ITU-R BT.709"]
P3 = colour.RGB_COLOURSPACES["P3-D65"]
M2020_TO_709 = REC709.matrix_XYZ_to_RGB @ REC2020.matrix_RGB_to_XYZ
M709_TO_2020 = np.linalg.inv(M2020_TO_709)
M2020_TO_P3 = P3.matrix_XYZ_to_RGB @ REC2020.matrix_RGB_to_XYZ
MP3_TO_2020 = np.linalg.inv(M2020_TO_P3)

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

# ---- sigmoid (Jed Smith's tunable sigmoid, as in Sobotka's sigmoid.py; lengths 0, limits
# (0, 0) and (1, 1)) ----
LOG_MIN, LOG_MAX, GREY = -10.0, 6.5, 0.18
PX = -LOG_MIN / (LOG_MAX - LOG_MIN)
PY = 0.18 ** (1 / 2.4)


def _scale(lx, ly, tx, ty, p, m):
    return ((m * (lx - tx)) ** -p * ((m * (lx - tx) / (ly - ty)) ** p - 1.0)) ** (-1.0 / p)


class Sigmoid:
    """Through pivot (px, py) with slope m there, toe and shoulder powers pt and ps."""

    def __init__(self, px, py, m, pt, ps):
        self.px, self.py, self.m, self.pt, self.ps = px, py, m, pt, ps
        self.s_toe = -_scale(1.0, 1.0, 1.0 - px, 1.0 - py, pt, m)
        self.s_shoulder = _scale(1.0, 1.0, px, py, ps, m)

    def __call__(self, x):
        if x < self.px:
            s, p = self.s_toe, self.pt
        elif x <= self.px:
            return self.m * (x - self.px) + self.py
        else:
            s, p = self.s_shoulder, self.ps
        u = self.m * (x - self.px) / s
        if math.isinf(u):  # log2(0): the toe's limit (numpy.ma keeps the infinity there)
            return -math.inf
        return s * u / (1.0 + u ** p) ** (1.0 / p) + self.py


SIGMOID = Sigmoid(PX, PY, 2.4, 1.5, 1.5)
# HDR (AgXHLG.py): 1000 nits peak over a 100 nit reference white. The shoulder's power is
# multiplied by (1000 / 100) ^ log10(2) = 2, and middle grey is then darkened by that ratio.
HDR_RATIO = 1000.0 / 100.0
SIGMOID_HDR = Sigmoid(PX, PY, 2.4, 1.5, 1.5 * HDR_RATIO ** math.log10(2.0))
HDR_PURITY = 0.5
# Darkening runs on log2 from -20 stops to linear 1.0, around 0.18.
DARK_MAX = math.log2(1 / 0.18)


def dark_encode(v):
    return (math.log2(v / 0.18) + 20.0) / (20.0 + DARK_MAX) if v > 0 else -math.inf


def dark_decode(x):
    return 0.18 * 2.0 ** (x * (20.0 + DARK_MAX) - 20.0)


DARKEN = Sigmoid(dark_encode(0.18), dark_encode(0.18 / HDR_RATIO), 1.000001, 3.0, 1.0)

# ---- guard rails (Sakari Kapanen / Eary Chow) ----


def low_side_2020(rgb, luma):
    y = rgb @ luma
    inv = rgb.max() - rgb
    y_comp = inv.max() - inv @ luma + y
    off = rgb + max(-rgb.min(), 0.0)
    inv_o = off.max() - off
    y_new = inv_o.max() - inv_o @ luma + off @ luma
    ratio = y_comp / y_new if y_new > y_comp else 1.0
    return ratio * off


def low_side_display(rgb, to_2020, luma):
    """The display gamut's lower guard rail; luminance measured back in Rec.2020."""
    lum = lambda c: (to_2020 @ c) @ luma
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


def saturation(c):
    v = c.max()
    return 0.0 if v == 0 else (v - c.min()) / v


def hsv_to_rgb(h, s, v):
    return np.asarray(colour.HSV_to_RGB(np.array([h, s, v])), dtype=float)


def with_hue(c, h):
    return hsv_to_rgb(h, saturation(c), c.max())


def lerp_hue(h1, h2, t):
    d = h2 - h1
    if d > 0.5:
        h2 -= 1.0
    elif d < -0.5:
        h2 += 1.0
    return (h1 + t * (h2 - h1)) % 1.0


def srgb_oetf(x):
    return np.where(x <= 0.0031308, 12.92 * x, 1.055 * np.power(np.maximum(x, 0), 1 / 2.4) - 0.055)


def hlg(c):
    """Display light (Rec.2020, 1 = 1000 nits) -> Rec.2100 HLG signal, as Blender's LUT."""
    return np.clip(colour.models.eotf_inverse_BT2100_HLG(np.clip(c, 0, 1) * 1000.0), 0.0, 1.0)


# Blender's LUT input: E-Gamut, log2 over -10..+15 stops around middle grey (per channel clamp).
E_GAMUT_TO_XYZ = np.array([[0.7053968501, 0.1640413283, 0.08101774865],
                           [0.2801307241, 0.8202066415, -0.1003373656],
                           [-0.1037815116, -0.07290725703, 1.265746519]])
M2020_TO_EG = np.linalg.inv(E_GAMUT_TO_XYZ) @ REC2020.matrix_RGB_to_XYZ
EG_TO_2020 = np.linalg.inv(M2020_TO_EG)


def log_encode(c):
    with np.errstate(divide="ignore"):
        return (np.log2(np.maximum(c, 1e-300) / GREY) - LOG_MIN) / (LOG_MAX - LOG_MIN)


def formation(c, to_display, display_to_2020):
    """AgX Base, steps 2-11 of docs/agx.md: Rec.2020 (inside the input domain) -> linear
    display RGB."""
    c = low_side_2020(c, LUMA_BASE)
    c = INSET @ c
    h0 = hue(c)
    c = np.array([SIGMOID(v) for v in log_encode(c)])
    c = np.maximum(c, 0.0) ** 2.4
    c = with_hue(c, lerp_hue(h0, hue(c), 0.4))
    c = OUTSET @ c
    c = to_display @ c
    return low_side_display(c, display_to_2020, LUMA_BASE)


def darken(c):
    """AgXHLG.py's darken_middle_grey: grey down by the HDR ratio (per channel, on log2), then
    hue and saturation halfway back to where they were."""
    d = np.array([dark_decode(DARKEN(dark_encode(v))) if v > 0 else 0.0 for v in c])
    h = lerp_hue(hue(c), hue(d), HDR_PURITY)
    s = saturation(c) + HDR_PURITY * (saturation(d) - saturation(c))
    return hsv_to_rgb(h, s, d.max())


def formation_hdr(c):
    """AgX HDR (AgXHLG.py): Rec.2020 -> display light in Rec.2020, limited to P3, 1 = 1000 nits."""
    c = low_side_2020(c, LUMA_HDR)
    c = INSET @ c
    h0 = hue(c)
    c = np.array([SIGMOID_HDR(v) for v in np.clip(log_encode(c), 0.0, 1.0)])
    c = np.maximum(c, 0.0) ** 2.4
    c = darken(c)
    c = with_hue(c, lerp_hue(h0, hue(c), 0.4))
    c = OUTSET @ c
    c = MP3_TO_2020 @ low_side_display(M2020_TO_P3 @ c, MP3_TO_2020, LUMA_HDR)
    return np.clip(c, 0.0, 1.0)


def input_domain(rgb2020):
    c = np.asarray(rgb2020, dtype=float)
    return EG_TO_2020 @ np.clip(M2020_TO_EG @ c, GREY * 2.0 ** -10, GREY * 2.0 ** 15)


def exact(rgb2020, target):
    c = input_domain(rgb2020)
    if target == "srgb":
        return srgb_oetf(np.clip(np.nan_to_num(formation(c, M2020_TO_709, M709_TO_2020)), 0.0, 1.0))
    if target == "p3":
        return srgb_oetf(np.clip(np.nan_to_num(formation(c, M2020_TO_P3, MP3_TO_2020)), 0.0, 1.0))
    return hlg(np.nan_to_num(formation_hdr(c)))


def check_nodes(cube, f, encode):
    """Exact vs the values stored at a LUT's grid points (E-Gamut log2 input)."""
    lines = [l for l in open(BLENDER + "/luts/" + cube) if l[:1] in "-0123456789."]
    lut = np.array([[float(v) for v in l.split()] for l in lines])
    n = round(len(lut) ** (1 / 3))
    every = os.environ.get("ALL_NODES")
    keys = range(len(lut)) if every else np.random.default_rng(1).choice(len(lut), 4000, replace=False)
    errs = []
    for k in keys:
        idx = np.array([k % n, (k // n) % n, k // (n * n)])  # .cube: red changes fastest
        eg = GREY * 2.0 ** (idx / (n - 1) * 25.0 - 10.0)
        c = encode(np.nan_to_num(f(EG_TO_2020 @ eg)))
        errs.append(np.abs(c - lut[k]).max())
    errs = np.array(errs)
    print(f"{cube} grid points ({len(errs)} of {n}^3): |table - exact| mean {errs.mean():.6f}, "
          f"99.9th pct {np.percentile(errs, 99.9):.5f}, max {errs.max():.5f}")


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


TARGETS = {
    # key in the JSON: (OCIO display, OCIO view)
    "srgb": ("sRGB", "AgX"),
    "p3": ("Display P3", "AgX"),
    "hlg": ("Rec.2100-HLG", "AgX - HDR 1000 nits"),
}


def main():
    print("inset\n", INSET, "\noutset\n", OUTSET)
    gamma = lambda c: np.clip(np.maximum(c, 0) ** (1 / 2.4), 0, 1)
    check_nodes("AgX_Base_sRGB.cube", lambda c: formation(c, M2020_TO_709, M709_TO_2020), gamma)
    check_nodes("AgX_Base_P3.cube", lambda c: formation(c, M2020_TO_P3, MP3_TO_2020), gamma)
    check_nodes("AgX_Rec2100-HLG_p3_lim.cube", formation_hdr, hlg)
    cfg = ocio.Config.CreateFromFile(BLENDER + "/config.ocio")
    rows = [{"in": c} for c in samples()]
    for key, (display, view) in TARGETS.items():
        proc = cfg.getProcessor(ocio.DisplayViewTransform(src="Linear Rec.2020", display=display, view=view)).getDefaultCPUProcessor()
        d = []
        for r in rows:
            px = np.array([r["in"]], dtype=np.float32)
            proc.applyRGB(px)
            r["blender_" + key] = [float(v) for v in px[0]]
            r["exact_" + key] = [float(v) for v in exact(r["in"], key)]
            d.append([abs(a - b) for a, b in zip(r["blender_" + key], r["exact_" + key])])
        d = np.array(d)
        print(f"{key}: {len(rows)} samples; |ocio - exact| max {d.max():.5f}, mean {d.mean():.5f}, 99th pct {np.percentile(d, 99):.5f}")
        for i in np.argsort(d.max(axis=1))[-4:]:
            r = rows[i]
            print("  worst:", [round(v, 5) for v in r["in"]], "ocio", [round(v, 4) for v in r["blender_" + key]], "exact", [round(v, 4) for v in r["exact_" + key]])
    r9 = lambda v: [round(x, 9) for x in v]
    keys = ["in"] + [p + k for k in TARGETS for p in ("blender_", "exact_")]
    with open(OUT, "w") as f:
        f.write('{"source": "scripts/agx_reference.py", "inset": %s, "outset": %s, "rows": [\n' % (
            json.dumps([r9(v) for v in INSET.tolist()]), json.dumps([r9(v) for v in OUTSET.tolist()])))
        f.write(",\n".join(json.dumps({k: r9(r[k]) for k in keys}) for r in rows))
        f.write("\n]}\n")


main()
