# Colour grading

A tint and a brightness change for each of the **shadows**, **midtones** and **highlights**, in
the **Color Grading** section. One wheel holds three dots, dark grey for shadows, mid grey for
midtones and white for highlights. A dot's direction from the centre is the tint's hue, and its
distance is the tint's strength; at the centre there is no tint.

Code: `crates/tonality-core/src/grading.rs` (the settings), `grade()` in
`crates/tonality-render/shaders/develop.wgsl` (the picture) and `ui/lib/edit/grading.ts` (how
the dots move).

## In the picture

Grading runs after the colour mixer and before AgX, on scene-linear Rec.2020.

**Which range a pixel is in** goes by how bright it will look: its luminance through AgX Base
and the sRGB curve, 0–1. So "shadows" means what is dark in the finished picture, whatever
exposure it took to get there.

- **Balance** bends that scale first: `x = seen^(2^(−2·balance))`, balance −1…1. Positive counts
  more of the picture as highlights, negative as shadows.
- **Blending** sets how far the ends reach, `reach = 0.25 + 0.5·blending` (blending 0…1):
  shadows are `1 − smoothstep(0, reach, x)`, highlights `smoothstep(1 − reach, 1, x)`, and the
  midtones are what is left. The three always sum to one.

**The tint** is added in OkLab: `(a, b) += L · 0.2 · saturation · (cos hue, sin hue)`, weighted
by the three ranges. Scaling by lightness `L` makes it a change of colour, not of brightness, and
keeps black black. Hue is an OkLCh hue in degrees, the same as the colour mixer's and a mask's
tint. AgX then handles the result like any other colour, so a strong tint near white fades out
the way a bright colour does.

**Luminance** multiplies the colour by `2^luminance` (−1…1 stop), weighted the same way.

With every tint and luminance at zero the stage is skipped, and the settings are left out of the
saved edit, so photos edited before this existed keep their history hashes.

## Linking the dots

**Link** holds the three dots in a shape. The shape is rigid and pinned at the wheel's centre:
drag any dot round the wheel and all three turn; drag it in or out and all three move in or out.

| Shape               | Shadows | Midtones                | Highlights |
| ------------------- | ------- | ----------------------- | ---------- |
| Analogous           | 0°      | 30°                     | 60°        |
| Complementary       | 0°      | at the centre (no tint) | 180°       |
| Triadic             | 0°      | 120°                    | 240°       |
| Split-complementary | 0°      | 150°                    | 210°       |

- **Turning the link on** builds the shape around the selected dot, or around one that has a
  tint if the selected one doesn't, so linking never throws a grade away. The other dots go to
  whichever mirror image of the shape is nearer to where they were.
- **Flip** swaps to the mirror image. Switching shape keeps the way round.
- **Turning it off** moves nothing. Each dot is then free, for fine adjustments.

The link is saved with the edit, but only the panel reads it: the picture is made from the three
dots alone, so a linked grade and the same dots unlinked export identically. Shapes with four
points (square, tetradic) are left out, as there are three dots.
