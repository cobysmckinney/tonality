# AgX in tonality

tonality's only display transform is **AgX Base**, as Blender ships it (Blender 4.0 and later,
view "AgX" on an sRGB display). This page writes the transform down, says where every number
comes from, and records how tonality's implementation was checked against Blender's.

The code: `crates/tonality-core/src/agx.rs` (parameters, matrices, a CPU reference) and `agx()` in
`crates/tonality-render/shaders/develop.wgsl` (the same steps on the GPU, for the preview,
export, histogram and mask luminance ranges).

## Sources

- **Troy Sobotka**, original AgX: <https://github.com/sobotka/AgX>, and the generator scripts it
  came from, <https://github.com/sobotka/SB2383-Configuration-Generation> (the tunable sigmoid
  and the inset / outset working-space construction).
- **Eary Chow**, the AgX that Blender adopted: <https://github.com/EaryChow/AgX> (config) and
  <https://github.com/EaryChow/AgX_LUT_Gen> (`AgXBaseRec2020.py`, `AgXBasesRGB.py`,
  `sigmoid.py`, `working_space.py`, `luminance_compenstation_*.py`).
- **Blender's OCIO config**, `release/datafiles/colormanagement/config.ocio` and
  `luts/AgX_Base_sRGB.cube` in <https://github.com/blender/blender>: what users actually see.
- **Jed Smith's tunable sigmoid** (<https://www.desmos.com/calculator/yrysofmx8h>), which
  Sobotka's `sigmoid.py` ports.

Blender bakes AgX into 3D LUTs; the scripts above are what generates them. tonality evaluates the
same math per pixel instead of through a LUT.

## The transform

Input: scene-linear **Rec.2020** (D65), tonality's working space, after every edit. Output:
**sRGB** code values (0–1) for the display. Constants are listed after the steps.

1. **Input domain.** Blender's LUT takes **E-Gamut** log2 from −10 to +15 stops around middle grey
   (0.18), so each E-Gamut channel is clamped to `[0.18·2⁻¹⁰, 0.18·2¹⁵]`. In Rec.2020:
   `c = EG→2020 · clamp(2020→EG · c, lo, hi)`. This only touches values more than 10 stops
   under grey, beyond 15 over, or outside E-Gamut.
2. **Lower guard rail (Rec.2020).** Negative components (colours outside Rec.2020) are lifted by
   adding `max(−min(c), 0)` to every channel, then the result is scaled back so its
   "compensated luminance" matches the original's. Compensated luminance of a colour `x` is
   `max(x) − Y(max(x) − x) + Y(x)`, with `Y` the luminance weights `W` below, and the scale is
   `Yc(original) / Yc(lifted)` when the lifted one is brighter, else 1. (Sakari Kapanen's method,
   as adapted by Eary Chow.)
3. **Inset.** `c = INSET · c`. The inset matrix moves Rec.2020's primaries toward white (so
   bright saturated colours desaturate on their way to white instead of clipping) and rotates
   them slightly (the "flourish": red toward orange, and so on, against the Abney effect).
4. **Record the hue** `h₀` of `c`: HSV hue (the hexcone), in 0–1.
5. **Log encode.** `x = (log₂(c / 0.18) + 10) / 16.5`: −10 stops → 0, middle grey → 10/16.5,
   +6.5 stops → 1. Per channel.
6. **Sigmoid** (per channel). Jed Smith's tunable sigmoid through the pivot
   `(x_p, y_p) = (10/16.5, 0.18^(1/2.4))` with slope `m = 2.4` there, toe and shoulder powers
   `p = 1.5`, passing through (0, 0) and (1, 1):

   ```
   f(u)       = u / (1 + u^p)^(1/p)
   scale(lx, ly, tx, ty) = ((m(lx − tx))^−p · ((m(lx − tx) / (ly − ty))^p − 1))^(−1/p)
   s_toe      = −scale(1, 1, 1 − x_p, 1 − y_p)
   s_shoulder =  scale(1, 1, x_p, y_p)
   y(x) = s · f(m(x − x_p) / s) + y_p,   s = s_toe if x < x_p, else s_shoulder
   ```

7. **Linearise:** `c = max(y, 0)^2.4`. The sigmoid's output is display-encoded with a pure
   2.4 power, so middle grey lands on exactly 0.18 of display white.
8. **Restore 40% of the hue.** The per-channel curve skews hues (a little red becomes orange,
   orange becomes yellow, as brightness rises). Eary's AgX keeps the curve's saturation and
   value but moves its HSV hue 60% of the way back to `h₀` (shortest way round the circle):
   `h = h₀ + 0.4 · (h − h₀)`.
9. **Outset:** `c = OUTSET · c`, restoring some of the purity the inset took away (no rotation).
10. **To the display's primaries:** `c = 2020→709 · c`.
11. **Lower guard rail (Rec.709).** Step 2 again for the display gamut, with two differences from
    Eary's sRGB script: luminance is measured after converting back to Rec.2020
    (`W709 = (709→2020)ᵀ · W`), and the compensated luminance is blended with the plain one by
    `t = min(Y^0.08, 1)`: `Y' = (1 − t)·Yc + t·Y` (so Rec.2020 green isn't lifted too far). A
    negative `Y` leaves the colour as it is (in the script it becomes NaN and the comparison fails).
12. **Clip** to 0–1 and **encode for the display** with the sRGB curve (IEC 61966-2-1, the
    piece-wise one). Blender does the same: its AgX LUT writes a 2.4 power, which its "sRGB"
    display decodes and re-encodes with the sRGB curve.

### Constants

| | Value | Source |
|---|---|---|
| Middle grey | 0.18 | `AgXBaseRec2020.py` |
| Log range | −10 to +6.5 stops (16.5) | `AgXBaseRec2020.py` |
| Pivot | x = 10/16.5, y = 0.18^(1/2.4) | `AgXBaseRec2020.py` |
| Slope, toe and shoulder powers | 2.4, 1.5, 1.5 | `AgXBaseRec2020.py` |
| Inset | rotate [2.13976149, −1.22827335, −3.05174246]°, scale [0.32965205, 0.28051336, 0.12475368] | `AgXBaseRec2020.py` |
| Outset | rotate [0, 0, 0], scale [0.32317438, 0.28325605, 0.0374326], inverted | `AgXBaseRec2020.py` |
| Hue restore | 40% of the curve's shift kept | `AgXBaseRec2020.py` (`mix_percent`) |
| Luminance weights `W` | [0.2658180370250449, 0.59846986045365, 0.1357121025213052] | the guard-rail scripts before June 2025 (see below) |
| E-Gamut → XYZ | FilmLight E-Gamut | `luminance_compenstation_bt2020.py` |
| Rec.2020, Rec.709 primaries | ITU-R BT.2020, BT.709, D65 | standards |

**Inset and outset matrices** are built from those parameters by Sobotka's
`create_workingspace`: in CIE xy, rotate each primary's direction from the white point by its
angle, find where that ray leaves the Rec.2020 triangle, pull that point toward white by
`(1 − scale)`, and take the RGB→XYZ matrix of the resulting primaries (same white). Then
`INSET = XYZ→2020 · RGB→XYZ(inset primaries)` and `OUTSET = (XYZ→2020 · RGB→XYZ(outset primaries))⁻¹`.
tonality computes them in Rust at startup, not as pasted constants. They agree with the inset
matrix written in Blender's config (the "AgX Log" colourspace) to 8 digits:

```
INSET  = [0.856627 0.095121 0.048252]     OUTSET = [ 1.127101 -0.110607 -0.016494]
         [0.137319 0.761242 0.101439]              [-0.141330  1.157824 -0.016494]
         [0.111898 0.076799 0.811302]              [-0.141330 -0.110607  1.251936]
```

(Eary notes that the parameters printed in Blender's LUT headers, `rotate = [3.0, −1, −2.0]` /
`inset = [0.4, 0.22, 0.13]`, were made for Rec.709 primaries but applied in Rec.2020; the
parameters above reproduce the same matrices starting from Rec.2020.)

**Luminance weights.** In June 2025 Eary's scripts switched the guard rails' weights to
[0.2589, 0.6105, 0.1306]. Blender's shipped LUTs were made with the earlier ones, and only the
earlier weights reproduce them (see below), so tonality uses those. The difference is visible only
in colours outside the display gamut.

## How it was checked

`scripts/agx_reference.py` evaluates both of these for 350 colours (greys from −12 to +12 stops,
Rec.2020 and Rec.709 primaries and secondaries across exposures, everyday colours, colours with a
negative component, and 200 random ones):

- **Blender**: its `config.ocio` and LUT files through OpenColorIO 2.6
  (`Linear Rec.2020` → display `sRGB`, view `AgX`), and
- **exact**: the steps above, in numpy, written from the sources (no downloaded code runs).

The results are in `crates/tonality-core/tests/data/agx_reference.json`.

- **At the LUT's own grid points** (the 57³ nodes of `AgX_Base_sRGB.cube`, decoded from E-Gamut
  log2 and pushed through the steps above) the exact values match Blender's table with a mean
  difference of 0.000008 and 0.00026 at the 99.9th percentile. The largest, 0.013 (in the table's
  2.4-power encoding), are on nearly black channels, where the 1/2.4 power magnifies tiny
  differences: 0.013 there is about 0.00003 linear, 0.0004 in sRGB. (The script checks a random
  4000 of the nodes; all 185,193 were checked once, November 2026.)
- **Between grid points** Blender interpolates (tetrahedral, 57 points over 25 stops), so for
  arbitrary colours the two differ by 0.0013 on average (sRGB code value, 0–1), 0.018 at the 99th
  percentile, and up to 0.063 for very saturated colours where one channel heads to zero and the
  table can't follow. That is Blender's approximation of AgX, not a difference in the transform.

Rust tests:

- `agx::tests` compare the Rust CPU version with the exact values (within 1e-5) and with
  Blender's (mean below 0.002, 99th percentile below 0.02), and the matrices with Blender's.
- `tonality-render/tests/agx_gpu.rs` renders the reference colours through the export shader
  (every other adjustment off) and compares them with the CPU version: within 1e-3, measured
  0.00025 on an Intel UHD 630. The colours outside Rec.2020 come in through a camera matrix with
  negative entries (camera = XYZ), as they would from a real camera, so the whole develop pass
  carries them to AgX. The preview runs the same shader, transpiled to GLSL.

Middle grey (0.18) displays as sRGB 0.4614, display-linear 0.18.

## What changed from tonality's earlier AgX

The earlier `agx()` was the widely copied "minimal AgX" shader approximation: a 6th-order
polynomial fitted to the sigmoid, applied with the outset *before* linearising, a 2.2 power
instead of 2.4, no hue restore and no guard rails, and a 4-digit Rec.2020→sRGB matrix. Its
matrices were in fact right. Against the exact transform it was off by 0.027 on average and up to
0.36 for saturated colours; middle grey came out at 0.50 instead of 0.46, and 6 stops under grey
at 0.0075 instead of 0.016. **Existing edits look different after this change**: slightly darker
mid-tones, more open shadows, and saturated highlights keep their hue better.

## Notes

- Colours outside Rec.2020 reach AgX as they are. Camera RGB clips at black (after noise
  reduction), but the camera matrix can still give a saturated colour a negative Rec.2020
  component, and Saturation can push one out; nothing clamps them before AgX, whose first guard
  rail (step 2) brings them in. On the way, Tone scales colours by a luminance floored at
  0.0593 × the largest component (`scaling_luma`; a colour inside Rec.2020 never goes below it),
  because a colour with a negative component can have a luminance near or below zero while
  another component is large, and the ratio would blow it up. Dehaze floors its result at black
  per component, except one already below black, which scales with the rest of the signal.
  `agx_gpu.rs` checks a dark blue with negative luminance against its in-gamut neighbour at
  Contrast −100.
- Masks' luminance ranges and the histogram see the same AgX output as the photo.

## Later

- **Display P3** and HDR: swap step 10's matrix and step 11's weights for the target display
  (Blender's P3 view is the same chain with P3 in place of Rec.709).
- **Looks** (Punchy, contrast variants) are CDL grades in AgX log space in Blender; not planned.
