# Auto

Two buttons in Edit give a starting point from the photo itself: **Auto** in the Light section
header (exposure, contrast, highlights, shadows, whites, blacks) and **Auto** in the White
Balance list. Each is one undoable step. Both read the preview inside the crop
(`tonality_core::auto::sample`, a 192×192 grid), so cropping changes what they see. Code:
`crates/tonality-core/src/auto.rs`; commands `auto_tone`, `auto_white_balance` in
`src-tauri/src/lib.rs`.

## Tone

Auto predicts the screen: the shader's `tone()` ported to Rust (`tone_ev`) and AgX Base on grey
(`display_of_ev`, a table of the exact transform; luminance is what Light changes). A GPU test
(`auto_tone_model_matches_the_shader`) renders greys through the shader with several Light
settings and checks the prediction within 0.002, so the two can't drift apart.

It reads the photo's luminance (white-balanced as the edit has it, through the camera matrix) as
percentiles, leaving out sensor-clipped pixels, which no setting brings back. Then, in two
stages:

1. **Exposure** (−5…+5, 0.05 steps): the median on screen near 0.42 (just under AgX's middle
   grey, 0.46), with the targets below as they stand at zero. Beyond ±2 stops exposure costs
   extra, so a night scene is brightened but stays a night scene.
2. **The other five sliders**, by coordinate descent (steps 20 → 1), hold the median where
   exposure put it (call it _m_) and shape the range. Each has a target in the region it moves,
   placed from _m_, so a night scene held dark keeps dark surroundings:
   - **Whites:** the brightest 0.5% near _m_ + 0.48 (at most 0.92). Clipping costs a lot,
     being dimmer a little.
   - **Highlights:** the 90th percentile near _m_ + 0.3 (at most 0.76), pulled in when it crowds
     the top.
   - **Shadows:** the 10th percentile near _m_ − 0.26 (at least 0.03), lifted when it has sunk.
   - **Blacks:** the darkest 0.5% near 0.02, and never under 0.004 (crushing costs a lot).
   - **Contrast:** 0.28 between the 25th and 75th percentiles.

   Each slider costs a little, so a photo that's already close gets gentle values. Auto keeps
   contrast within ±40, whites and blacks within ±60, highlights within −80…0 and shadows within
   0…60. Highlights only recover and shadows only lift: the other way they would only add
   contrast, which is the contrast slider's job.

Same photo, same crop and white balance: same result.

## White balance

Grey pixels (after Huo et al., 2006): start from the camera's own estimate (As Shot), take the
pixels that look neutral under it (HSV saturation in Rec.2020 under a tolerance going from 0.4 to
0.08, widened up to 0.4 if too few qualify), weight each by how grey it looks, average their
chromaticity, and repeat. Large coloured areas (a blue car, the sky, a lawn) don't vote, which an
average over the whole photo can't avoid. The result is named with temperature and tint the way
As Shot is. Its tint stays within ±15 of the camera's: what fools this method mostly shifts tint,
and cameras seldom get tint far wrong. With no neutral-looking pixels, Auto gives As Shot.

Known limit: when the camera's estimate is far off (hundreds of kelvin) **and** much of the frame
is one colour, some of that colour can look greyer than the real greys. The tests cover guesses
about 10% off.

## White balance picker

The eyedropper beside the White Balance list (W) is the manual counterpart: click something that
should be neutral and its colour is taken as the light's. `auto::sample_around` maps the click
(a position in the framed photo) through the crop, turns, flip, straighten and lens correction to
the preview, as `sample` does, and reads a 5 × 5 pixel patch of camera RGB there (the preview's
pixels, about 2–4 source pixels each). `auto::white_balance_at` averages the patch, leaving out
pixels clipped in any channel or black, and names the mean with temperature and tint the way As
Shot is named. No tint limit: the user chose the spot. Command: `pick_white_balance`.

## Checked

`auto::tests` (synthetic scenes: dark, bright, well exposed, flat, ten stops of range, a blown
window; white balance under 2850–6500 K with half the frame one strong blue; crop sampling), the
GPU test above, and real photos through `export::tests::auto_on_real_photos` (ignored; renders
before/after).
