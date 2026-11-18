# Texture, Clarity, Dehaze

Three sliders (−100…+100) in **Effects**, and the same three on masks (added to the global
amounts). All three look at the photo **as shot** (camera RGB, as-shot white balance, no edits),
so what counts as detail or haze doesn't shift as other sliders move. The result is applied to
the developed colour after exposure, white balance and the masks' settings, and before vignette,
tone and AgX.

Code: `crates/tonality-render/src/presence.rs` (per-photo maps) and `presence()` in
`crates/tonality-render/shaders/develop.wgsl`.

## Texture: fine detail

A band-pass of as-shot log₂ luminance: the pixel with four neighbours 2 px away, minus eight
neighbours 6 px away (full-resolution pixels). It's computed per pixel in the shader, like
sharpening. On the fitted preview the taps fall inside one texel and the effect shrinks, as it
would in a downscaled export. At 100% it is exact.

- **Positive** multiplies the colour by `2^(k·b)`. `b` is the band, cored first: detail under
  `0.03·√(0.18/L)` stops, roughly the shot-noise level (more in the shadows), isn't boosted.
  Then it's limited by `b / (1 + 2|b|)`, so edges don't overshoot. `k = 1.6 × amount`.
- **Negative** removes the band (`k = amount`, no coring): skin and surfaces smooth while edges
  stay.

## Clarity: mid-scale local contrast

The base is an edge-preserving blur of as-shot log₂ luminance: a **guided filter** (He, Sun &
Tang 2010) of the luminance by itself, radius 2% of the long edge, ε = 0.25 stops². Edges stronger
than about half a stop stay sharp in the base, so boosting `L − base` gives no halos. It uses the
**fast guided filter** (He & Sun 2015): `a`, `b` are fitted on a copy with a 512 px long edge.
The shader rebuilds `base = a·L + b` from full-resolution `L`, with `a`, `b` sampled bilinearly.
The maps cover the whole photo, so the preview, crops and export tiles all agree.

The colour is multiplied by `2^(k·d·m)`:
- `d` is the detail `L − base`, limited by `d / (1 + |d|/2)`.
- `m = exp(−e²/12.5)` weights the mid-tones, where `e` is the current pixel's stops from middle
  grey. Shadows and highlights move less.
- `k = 0.8 × amount` when positive, `0.9 × amount` when negative.

## Dehaze: the dark channel prior

He, Sun & Tang (2009): in haze-free outdoor photos most patches have some pixel that is dark in
some channel, so brightness in the **dark channel** (`min` over the patch and the channels) is
haze. On the 512 px copy, in as-shot Rec.2020:

- **Airlight** `A`: the mean colour of the 0.1% of pixels with the brightest dark channel.
  It's kept in camera RGB and developed like a pixel, so it follows white balance and exposure.
- **Transmission**: `t = 1 − 0.95 · dark(I / A)`, with a patch radius of 0.2% of the long edge.
  The patch is kept small because larger ones leave bright halos around thin dark objects
  (poles, branches). It's refined by a guided filter (radius 1.5%, ε = 10⁻⁴) with luminance ÷ the
  airlight's as the guide, and rebuilt at full resolution like Clarity's base.

Positive amounts recover the scene: `J = (I − A) / max(1 − amount·(1 − t), 0.1) + A`. Negative
amounts add haze: `I` mixes toward `A` by `|amount| · (0.08 + 0.4·(1 − t))`, more where there's
already haze.

## Notes

- **Making the maps:** clarity and haze coefficients (RGBA16F, 512 px) come from the preview, made
  on first use and cached for the last four photos (`Images::presence`). Export makes them from
  the full-resolution decode, once per photo, only if some version uses Clarity or Dehaze.
- **Before the maps arrive:** the preview switches the maps off in its parameters until they've
  loaded, so a photo never flashes with a placeholder map.
- **Saved edits:** the settings are left out of the saved edit at 0, so edits made before they
  existed keep their hash and history.
