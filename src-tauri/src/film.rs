//! Turning a scanned film negative into a positive.
//!
//! A negative records light as density: the more light reached the film,
//! the darker it is. Each colour layer has its own response, and the clear
//! film carries a tint of its own (the orange mask of colour film). So the
//! inversion works in density, channel by channel:
//!
//! 1. Divide by the film base (the clear film), which takes its tint off.
//! 2. Density is `-log10` of what is left: zero for clear film.
//! 3. Each channel's density range in the frame, from its thinnest point
//!    (the deepest shadow) to its densest (the brightest highlight), is
//!    stretched onto one common range. The densest points balance the
//!    highlights; the clear film, black in every channel, keeps the shadows
//!    neutral (`measure_range`).
//! 4. Density is proportional to the scene's light in stops, so that range
//!    becomes light again: the positive is scene-referred, like a RAW, and
//!    the rest of the editor works on it unchanged.
//! 5. How bright: the frame's average lands where an ordinary RAW's does,
//!    as a camera's meter would put it, and highlights that would land near
//!    white are rolled off below it, as a print does (`Look::reach`, `Look::shoulder`).
//!
//! The develop pipeline does the arithmetic on the GPU (`film_positive` in
//! shaders/prepare.wgsl); `Look::invert` here is its CPU twin, for the tests.
//! Measuring the film (the base and the range) happens here, on a small copy
//! of the scan (`Sample`).

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::develop::LinearImage;
use crate::edit::{Adjustments, Crop};
use crate::geometry::{self, Affine};

/// What kind of film a scan is of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Not a negative: a digital photo, or a slide, which is already a positive.
    #[default]
    None,
    Colour,
    BlackAndWhite,
}

/// Each channel's density range: thinnest (shadows) and densest (highlights).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub low: [f32; 3],
    pub high: [f32; 3],
    /// The frame's average, as a share of the way from the thin end to the
    /// dense end (in stops of light). None in ranges kept before it was
    /// measured: it is measured from the frame then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<f32>,
}

/// A photo's film settings, as the recipe stores them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Film {
    pub kind: Kind,
    /// The clear film's colour, as the scan holds it (linear light). None:
    /// guessed from the scan.
    pub base: Option<[f32; 3]>,
    /// The density range the frame is stretched over. None: measured from
    /// the frame inside the crop. Set when a roll is balanced together, so
    /// every frame gets the same numbers.
    pub range: Option<Range>,
}

impl Film {
    pub fn is_negative(&self) -> bool {
        self.kind != Kind::None
    }
}

/// The frame's average (the mean of its light in stops) lands here, in scene
/// light (1.0 is where a RAW clips): where an ordinary RAW's average sits,
/// so a negative opens about as bright as one.
pub const KEY: f32 = 0.08;
/// The densest point of the frame lands at most here: a stop under clipping,
/// where the starting look still draws detail (about 234 of 255). Anything
/// that would land brighter is rolled off from `KNEE` up, as a print's paper
/// does, so a bright sky or a lit wall isn't pressed against white.
pub const TOP: f32 = 0.5;
const KNEE: f32 = 0.18;
/// Where the densest point would land before that, in scene light, is kept
/// within these: a frame that is nearly all sky isn't made grey, and a
/// night scene, mostly shadow, isn't brightened into a day.
const REACH: (f32, f32) = (0.4, 2.0);
/// How steeply typical negative film builds density with light, per decade.
/// The frame's density range divided by this is the scene's range.
const GAMMA: f32 = 0.6;
/// The scene's range is kept within these many stops, so a nearly flat frame
/// isn't stretched into noise and a very contrasty one isn't left grey.
const STOPS: (f32, f32) = (4.5, 10.0);
/// The share of the frame left out at each end when measuring its range,
/// so dust, scratches and a stray bright pixel don't set it.
const ENDS: (f32, f32) = (0.002, 0.998);
/// The light around the film is looked for in steps of this much
/// brightness (in decades, about a sixth of a stop)...
const VALLEY_STEP: f32 = 0.05;
/// ...and taken to be at most this share of the scan.
const LIGHT_SHARE: f64 = 0.06;
/// How much of each side of an uncropped scan the balance is measured in.
const UNCROPPED: f32 = 0.6;
/// The thinnest a channel's range can be, in density.
const NARROWEST: f32 = 0.05;
/// The positive is capped here, far past white, where the black of a film
/// holder would otherwise overflow. The same in prepare.wgsl.
const BRIGHTEST: f32 = 64.0;
/// Transmittance never goes below this, so black areas have a density.
const DARKEST: f32 = 1e-5;

/// Everything the inversion needs, worked out: what the develop pipeline applies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub kind: Kind,
    pub base: [f32; 3],
    pub range: Range,
}

impl Look {
    /// The scene's range in stops between the two ends of the density range.
    pub fn stops(&self) -> f32 {
        let spans = self.spans();
        let span = match self.kind {
            Kind::BlackAndWhite => spans[0],
            _ => (spans[0] + spans[1] + spans[2]) / 3.0,
        };
        (span / (GAMMA * std::f32::consts::LOG10_2)).clamp(STOPS.0, STOPS.1)
    }

    fn spans(&self) -> [f32; 3] {
        std::array::from_fn(|c| (self.range.high[c] - self.range.low[c]).max(NARROWEST))
    }

    /// Where the densest point of the frame lands, in scene light, before
    /// the highlights are rolled off: wherever puts the frame's average on
    /// `KEY`, within `REACH`.
    pub fn reach(&self) -> f32 {
        let average = self.range.key.unwrap_or(0.5).clamp(0.0, 1.0);
        (KEY * (self.stops() * (1.0 - average)).exp2()).clamp(REACH.0, REACH.1)
    }

    /// How much the highlights are rolled off above `KNEE`, so that the
    /// densest point lands on `TOP` rather than at `reach`: 0 when it lands
    /// there or lower anyway. Above the knee, a pixel `x` stops over it ends
    /// up `x / (1 + shoulder * x)` stops over.
    pub fn shoulder(&self) -> f32 {
        let (over, wanted) = ((self.reach() / KNEE).log2(), (TOP / KNEE).log2());
        if over <= wanted { 0.0 } else { (over / wanted - 1.0) / over }
    }

    /// The positive of one scan pixel, in scene light.
    pub fn invert(&self, pixel: [f32; 3]) -> [f32; 3] {
        let (stops, reach, shoulder) = (self.stops(), self.reach(), self.shoulder());
        let spans = self.spans();
        let light = |density: f32, c: usize| {
            let position = (density - self.range.low[c]) / spans[c];
            let light = reach * (stops * (position - 1.0)).exp2();
            let over = (light.max(DARKEST) / KNEE).log2();
            let light = if over > 0.0 { KNEE * (over / (1.0 + shoulder * over)).exp2() } else { light };
            light.min(BRIGHTEST)
        };
        match self.kind {
            Kind::BlackAndWhite => {
                let v = light(density_bw(pixel, self.base), 0);
                [v; 3]
            }
            _ => {
                let d = densities(pixel, self.base);
                std::array::from_fn(|c| light(d[c], c))
            }
        }
    }

    /// The look as the film pass in prepare.wgsl reads it: base and kind,
    /// low ends, spans, then the reach, the range in stops, the knee and the shoulder.
    pub fn uniform(&self) -> [[f32; 4]; 4] {
        let [r, g, b] = self.base;
        let kind = if self.kind == Kind::BlackAndWhite { 2.0 } else { 1.0 };
        let spans = self.spans();
        let [l0, l1, l2] = self.range.low;
        [[r, g, b, kind], [l0, l1, l2, 0.0], [spans[0], spans[1], spans[2], 0.0], [self.reach(), self.stops(), KNEE, self.shoulder()]]
    }

    /// The settings that change what the models see: the kind of film and
    /// its base, written into found mattes' names, with `MATTE_TAG`. The
    /// range is left out: it moves with the crop, and the subject's outline doesn't.
    pub fn matte_name(&self) -> String {
        let base = self.base.map(|v| format!("{:.4}", v)).join(",");
        let hash = blake3::hash(format!("{:?}-{base}-{MATTE_TAG}", self.kind).as_bytes()).to_hex();
        format!("-film-{}", &hash[..10])
    }
}

/// The film base as it looks in the scan, in sRGB: for a swatch of it.
pub fn swatch(base: [f32; 3]) -> [u8; 3] {
    crate::segment::base_look(base, false).map(crate::segment::encode)
}

/// Each channel's density, after taking off the film base.
pub fn densities(pixel: [f32; 3], base: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|c| -(pixel[c] / base[c].max(DARKEST)).max(DARKEST).log10())
}

/// The density of black and white film: the channels, each relative to the
/// base, combined into one, so a tint in the base or the light leaves no cast.
pub fn density_bw(pixel: [f32; 3], base: [f32; 3]) -> f32 {
    let t: f32 = (0..3).map(|c| (pixel[c] / base[c].max(DARKEST)).max(DARKEST)).sum::<f32>() / 3.0;
    -t.max(DARKEST).log10()
}

/// The long edge of `Sample`.
pub const SAMPLE_EDGE: u32 = 1024;

/// A small copy of the scan, averaged down, to measure the film in.
#[derive(Debug, Clone)]
pub struct Sample {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>,
    /// The film base guessed from it, once worked out.
    guessed: std::sync::OnceLock<[f32; 3]>,
}

impl Sample {
    pub fn of(image: &LinearImage) -> Self {
        let (width, height) = crate::gpu::fit_within(image.width, image.height, SAMPLE_EDGE.min(image.width.max(image.height)));
        let (sx, sy) = (image.width as f32 / width as f32, image.height as f32 / height as f32);
        let span = |i: u32, scale: f32, limit: u32| {
            let start = ((i as f32 * scale) as usize).min(limit as usize - 1);
            start..(((i + 1) as f32 * scale) as usize).clamp(start + 1, limit as usize)
        };
        let stride = image.width as usize;
        let pixels = (0..width * height)
            .into_par_iter()
            .map(|i| {
                let (lines, columns) = (span(i / width, sy, image.height), span(i % width, sx, image.width));
                let mut sum = [0f32; 3];
                for line in lines.clone() {
                    for pixel in &image.pixels[line * stride + columns.start..line * stride + columns.end] {
                        sum = std::array::from_fn(|c| sum[c] + pixel[c]);
                    }
                }
                let count = (lines.len() * columns.len()) as f32;
                sum.map(|v| v / count)
            })
            .collect();
        Self { width, height, pixels, guessed: Default::default() }
    }

    /// The film base, guessed from the clearest film in the whole scan (`guess_base`).
    pub fn guessed_base(&self) -> [f32; 3] {
        *self.guessed.get_or_init(|| guess_base(&self.pixels))
    }

    /// The film base at a point on the photo file (0..1 across and down): the
    /// scan averaged over a small patch there, so grain and noise don't count.
    pub fn base_at(&self, x: f32, y: f32) -> [f32; 3] {
        let (cx, cy) = ((x.clamp(0.0, 1.0) * self.width as f32) as i64, (y.clamp(0.0, 1.0) * self.height as f32) as i64);
        let mut sum = [0f32; 3];
        let mut count = 0.0f32;
        for yy in (cy - 2).max(0)..(cy + 3).min(self.height as i64) {
            for xx in (cx - 2).max(0)..(cx + 3).min(self.width as i64) {
                let p = self.pixels[(yy * self.width as i64 + xx) as usize];
                sum = std::array::from_fn(|c| sum[c] + p[c]);
                count += 1.0;
            }
        }
        sum.map(|v| (v / count.max(1.0)).max(DARKEST))
    }

    /// The pixels inside the crop `adjustments` make. Until the photo is
    /// cropped, the middle of it: a scan usually has the film holder and the
    /// light around the frame at its edges.
    fn inside(&self, adjustments: &Adjustments) -> Vec<[f32; 3]> {
        let mut framing = adjustments.clone();
        if framing.crop == Crop::default() {
            framing.crop = Crop { x: 0.5, y: 0.5, width: UNCROPPED, height: UNCROPPED };
        }
        let to_frame = crop_from_source(self.width, self.height, &framing);
        let (w, h) = (self.width as f64, self.height as f64);
        self.pixels
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let source = [((i % self.width as usize) as f64 + 0.5) / w, ((i / self.width as usize) as f64 + 0.5) / h];
                let [x, y] = to_frame.apply(source);
                (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)
            })
            .map(|(_, p)| *p)
            .collect()
    }
}

/// Maps a position on the photo file to one in the crop (0..1 inside it).
fn crop_from_source(width: u32, height: u32, adjustments: &Adjustments) -> Affine {
    let frame = geometry::frame(width, height, adjustments, false);
    geometry::frame_to_source(width, height, adjustments, &frame).inverse()
}

/// The clearest film in `pixels`, rebate included: the light that comes
/// through the film where nothing was exposed. Light that reached the camera
/// around the film (sprocket holes, gaps in the holder) is brighter still,
/// so it is left out: anything near clipping, and a bright group standing
/// apart above the rest. Clear film always holds back some light, so there
/// is a gap between the two that the film itself never fills.
pub fn guess_base<'a>(pixels: impl IntoIterator<Item = &'a [f32; 3]>) -> [f32; 3] {
    let brightness = |p: &[f32; 3]| (p[0] + p[1] + p[2]) / 3.0;
    let mut candidates: Vec<[f32; 3]> = pixels
        .into_iter()
        .filter(|p| p.iter().all(|v| v.is_finite() && *v > 0.0) && p[0].max(p[1]).max(p[2]) < 0.95)
        .copied()
        .collect();
    if candidates.is_empty() {
        return [1.0; 3];
    }
    candidates.par_sort_unstable_by(|a, b| brightness(a).total_cmp(&brightness(b)));
    let n = candidates.len();
    let at = |share: f64| ((n - 1) as f64 * share).round() as usize;
    let mut top = at(0.998);

    // How many of the brightest tenth fall in each step of brightness, from the top down.
    let floor = at(0.9);
    let level = |i: usize| (brightness(&candidates[i]).max(1e-6)).log10();
    let peak = level(at(1.0));
    let steps = ((peak - level(floor)) / VALLEY_STEP).ceil().max(1.0) as usize;
    let mut counts = vec![0usize; steps + 1];
    for i in floor..n {
        counts[(((peak - level(i)) / VALLEY_STEP) as usize).min(steps)] += 1;
    }
    // A step nearly empty next to full ones on both sides, with only a
    // little above it: the light, standing apart from the film.
    for step in 1..steps {
        let above: usize = counts[..step].iter().sum();
        if above as f64 > n as f64 * LIGHT_SHARE {
            break;
        }
        let fuller_above = counts[..step].iter().copied().max().unwrap_or(0);
        let below = step + 1..(step + 4).min(steps + 1);
        let fuller_below = counts[below.clone()].iter().copied().max().unwrap_or(0);
        if above > 0 && counts[step] * 2 < fuller_above && counts[step] * 4 < fuller_below {
            // Everything brighter than this step is the light, and the
            // film starts where the steps fill up again.
            let film = below.into_iter().find(|&s| counts[s] * 4 >= fuller_below).unwrap_or(step + 1);
            let edge = peak - VALLEY_STEP * film as f32;
            top = candidates.partition_point(|p| brightness(p).max(1e-6).log10() <= edge).saturating_sub(1);
            break;
        }
    }
    // Average a band just below that, so a few hot pixels don't set it.
    let band = (n / 200).max(1);
    let chosen = &candidates[top.saturating_sub(band)..=top];
    let mut sum = [0f32; 3];
    for p in chosen {
        sum = std::array::from_fn(|c| sum[c] + p[c]);
    }
    sum.map(|v| (v / chosen.len() as f32).max(DARKEST))
}

/// Each channel's density range across `pixels`, its two ends taken a
/// little way in so dust and stray pixels don't set them.
///
/// Each channel's densest point is its own: stretching them all to the same
/// height balances the highlights. The thin end is measured as a share of
/// that, the same share in every channel, so the clear film stays the
/// colour of black and the shadows aren't tinted: a channel that builds
/// density faster has its thin end further from clear film in proportion.
pub fn measure_range(kind: Kind, base: [f32; 3], pixels: &[[f32; 3]]) -> Range {
    if pixels.is_empty() {
        return Range { low: [0.0; 3], high: [1.0; 3], key: Some(0.5) };
    }
    let channels: Vec<Vec<f32>> = match kind {
        Kind::BlackAndWhite => vec![pixels.par_iter().map(|&p| density_bw(p, base)).collect()],
        _ => (0..3).map(|c| pixels.par_iter().map(|&p| densities(p, base)[c]).collect()).collect(),
    };
    let ends: Vec<(f32, f32)> = channels
        .into_iter()
        .map(|mut values| {
            let n = values.len() - 1;
            let low = *values.select_nth_unstable_by((n as f64 * ENDS.0 as f64) as usize, f32::total_cmp).1;
            let high = *values.select_nth_unstable_by((n as f64 * ENDS.1 as f64).round() as usize, f32::total_cmp).1;
            (low, high.max(low + NARROWEST))
        })
        .collect();
    let pick = |c: usize| ends[c.min(ends.len() - 1)];
    let high: [f32; 3] = std::array::from_fn(|c| pick(c).1);
    let share = ends.iter().map(|(low, high)| low / high).sum::<f32>() / ends.len() as f32;
    let mut range = Range { low: high.map(|h| share * h), high, key: None };
    range.key = Some(average(kind, base, &range, pixels));
    range
}

/// How far up `range` `pixels` are on average, from 0 at its thin end to 1
/// at its dense end: the mean of their light in stops, as the positive has it.
fn average(kind: Kind, base: [f32; 3], range: &Range, pixels: &[[f32; 3]]) -> f32 {
    let spans: [f32; 3] = std::array::from_fn(|c| (range.high[c] - range.low[c]).max(NARROWEST));
    let position = |density: f32, c: usize| ((density - range.low[c]) / spans[c]).clamp(0.0, 1.0) as f64;
    let sum: f64 = pixels
        .par_iter()
        .map(|&p| match kind {
            Kind::BlackAndWhite => position(density_bw(p, base), 0),
            _ => {
                let d = densities(p, base);
                (0..3).map(|c| position(d[c], c)).sum::<f64>() / 3.0
            }
        })
        .sum();
    (sum / pixels.len().max(1) as f64) as f32
}

/// Names the way a positive is drawn for the models to look at. Part of
/// each found matte's name, so changing how bright or contrasty a positive
/// comes out (the constants above) finds every matte of a negative again.
const MATTE_TAG: &str = "key-shoulder-1";

/// What a frame's film settings come to, measured where they are left to
/// be: the base from the whole scan, the range from inside the crop.
pub fn resolve(sample: &Sample, adjustments: &Adjustments) -> Option<Look> {
    let film = &adjustments.film;
    if !film.is_negative() {
        return None;
    }
    let base = film.base.unwrap_or_else(|| sample.guessed_base());
    let range = match film.range {
        Some(range @ Range { key: Some(_), .. }) => range,
        // Balanced with a roll before the average was kept: the frame's own.
        Some(range) => Range { key: Some(average(film.kind, base, &range, &sample.inside(adjustments))), ..range },
        None => measure_range(film.kind, base, &sample.inside(adjustments)),
    };
    Some(Look { kind: film.kind, base, range })
}

/// One balance for a whole roll: the base and density range measured across
/// every frame together, each frame inside its own crop. A base picked by
/// hand on any of them is kept for all.
pub fn balance_roll(kind: Kind, frames: &[(Sample, Adjustments)]) -> ([f32; 3], Range) {
    let picked = frames.iter().find_map(|(_, adjustments)| adjustments.film.base.filter(|_| adjustments.film.range.is_none()));
    let base = picked.unwrap_or_else(|| guess_base(frames.iter().flat_map(|(sample, _)| &sample.pixels)));
    let inside: Vec<[f32; 3]> = frames.iter().flat_map(|(sample, adjustments)| sample.inside(adjustments)).collect();
    (base, measure_range(kind, base, &inside))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of `scene` (positive linear light) as a scan of a negative:
    /// each channel's density grows with log light at its own gamma, on top
    /// of a tinted base, with `border` pixels of clear film around it.
    pub(crate) fn negative(scene: &[[f32; 3]], width: u32, border: u32, base: [f32; 3], gammas: [f32; 3]) -> LinearImage {
        let height = scene.len() as u32 / width;
        let (w, h) = (width + 2 * border, height + 2 * border);
        let pixels = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if x < border || y < border || x >= width + border || y >= height + border {
                    return base;
                }
                let p = scene[((y - border) * width + x - border) as usize];
                std::array::from_fn(|c| base[c] * 10f32.powf(-gammas[c] * (p[c].max(1e-4) / 0.0005).log10()))
            })
            .collect();
        LinearImage { width: w, height: h, pixels, scene_referred: true }
    }

    #[test]
    fn density_is_zero_on_clear_film_and_grows_as_it_darkens() {
        let base = [0.8, 0.5, 0.3];
        assert_eq!(densities(base, base), [0.0, 0.0, 0.0]);
        let d = densities([0.08, 0.05, 0.003], base);
        assert!((d[0] - 1.0).abs() < 1e-5 && (d[1] - 1.0).abs() < 1e-5 && (d[2] - 2.0).abs() < 1e-5);
        assert!(density_bw([0.0; 3], base).is_finite());
        assert!(densities([2.0, 2.0, 2.0], base).iter().all(|d| *d < 0.0), "light around the film is thinner than clear film");
    }

    #[test]
    fn the_ends_of_the_range_land_on_black_and_the_top() {
        let look = Look { kind: Kind::Colour, base: [0.8, 0.5, 0.3], range: Range { low: [0.1, 0.2, 0.3], high: [1.1, 1.4, 1.5], key: Some(0.5) } };
        let at = |d: [f32; 3]| look.invert(std::array::from_fn(|c| look.base[c] * 10f32.powf(-d[c])));
        for v in at(look.range.high) {
            assert!((v - TOP).abs() < 1e-4, "{v}");
        }
        let bottom = look.reach() * (-look.stops()).exp2();
        for v in at(look.range.low) {
            assert!((v - bottom).abs() < 1e-5, "{v}");
        }
        // Halfway in density is halfway in stops, in every channel alike:
        // the frame's average, which lands where an ordinary RAW's does.
        let middle = at(std::array::from_fn(|c| (look.range.low[c] + look.range.high[c]) / 2.0));
        assert!(middle.iter().all(|v| (v - middle[0]).abs() < 1e-5), "{middle:?}");
        assert!((middle[0] - KEY).abs() < 1e-4, "{middle:?}");
    }

    #[test]
    fn highlights_are_rolled_off_below_white_and_keep_their_order() {
        let range = Range { low: [0.1; 3], high: [1.3; 3], key: Some(0.3) };
        let look = Look { kind: Kind::BlackAndWhite, base: [0.6; 3], range };
        // A dark frame: its average on KEY would put its densest point past white...
        assert!(look.reach() > 1.0 && look.shoulder() > 0.0);
        let at = |d: f32| look.invert([0.6 * 10f32.powf(-d); 3])[0];
        // ...so the highlights are rolled off to land on TOP, in order.
        assert!((at(1.3) - TOP).abs() < 1e-4, "{}", at(1.3));
        let mut last = 0.0;
        for step in 0..=40 {
            let v = at(0.1 + 1.2 * step as f32 / 40.0);
            assert!(v > last, "step {step}: {v} after {last}");
            last = v;
        }
        // The rest of the frame is left as it was.
        let below = look.reach() * (look.stops() * -0.6).exp2();
        assert!(below < KNEE && (at(0.1 + 1.2 * 0.4) - below).abs() < 1e-5);

        // A bright frame (nearly all sky) isn't made grey.
        let sky = Look { range: Range { key: Some(0.9), ..range }, ..look };
        assert_eq!((sky.reach(), sky.shoulder()), (REACH.0, 0.0));
    }

    #[test]
    fn black_and_white_comes_out_grey() {
        let look = Look { kind: Kind::BlackAndWhite, base: [0.7, 0.6, 0.75], range: Range { low: [0.1; 3], high: [1.2; 3], key: Some(0.5) } };
        for pixel in [[0.3, 0.2, 0.35], [0.05, 0.04, 0.06], [0.7, 0.6, 0.75]] {
            let [r, g, b] = look.invert(pixel);
            assert!(r == g && g == b);
        }
        // Thinner film is a darker positive.
        assert!(look.invert([0.6, 0.5, 0.6])[0] < look.invert([0.1, 0.1, 0.1])[0]);
    }

    #[test]
    fn the_base_is_the_clear_film_not_the_light_around_it() {
        let film = [0.6, 0.35, 0.2];
        let mut pixels = Vec::new();
        // Mostly picture, denser than the base.
        for i in 0..9000 {
            let d = 0.2 + (i % 100) as f32 / 100.0;
            pixels.push(film.map(|v| v * 10f32.powf(-d)));
        }
        // A rebate of clear film, with grain.
        for i in 0..800 {
            let grain = 1.0 + ((i % 7) as f32 - 3.0) * 0.004;
            pixels.push(film.map(|v| v * grain));
        }
        // Sprocket holes: the light source, unclipped, and a little clipped.
        pixels.extend(std::iter::repeat_n([0.85, 0.83, 0.8], 150));
        pixels.extend(std::iter::repeat_n([1.0, 1.0, 1.0], 100));
        // A few hot pixels.
        pixels.extend(std::iter::repeat_n([0.9, 0.0, 0.9], 3));
        let base = guess_base(&pixels);
        for c in 0..3 {
            assert!((base[c] / film[c] - 1.0).abs() < 0.02, "{base:?} is not {film:?}");
        }
    }

    #[test]
    fn a_negative_comes_back_neutral_inside_the_crop() {
        // Grey steps from deep shadow to highlight, each channel built up at
        // its own gamma on an orange base, inside a clear border.
        let width = 64;
        let scene: Vec<[f32; 3]> = (0..width * 48).map(|i| [0.002 * 2f32.powf((i % width) as f32 / 8.0); 3]).collect();
        let base = [0.75, 0.42, 0.22];
        let image = negative(&scene, width, 8, base, [0.55, 0.62, 0.7]);
        let sample = Sample::of(&image);
        let guessed = guess_base(&sample.pixels);
        for c in 0..3 {
            assert!((guessed[c] / base[c] - 1.0).abs() < 0.01, "{guessed:?}");
        }
        // Crop to the picture: 64 x 48 inside 80 x 64.
        let crop = Crop { x: 0.5, y: 0.5, width: 64.0 / 80.0, height: 48.0 / 64.0 };
        let film = Film { kind: Kind::Colour, ..Default::default() };
        let adjustments = Adjustments { film, crop, ..Default::default() };
        let look = resolve(&sample, &adjustments).unwrap();
        let mut last = 0.0;
        for x in (0..width).step_by(8) {
            let p = look.invert(image.pixels[(24 + 8) * 80 + 8 + x as usize]);
            let mean = (p[0] + p[1] + p[2]) / 3.0;
            assert!(p.iter().all(|v| (v / mean - 1.0).abs() < 0.03), "grey step {x} came out {p:?}");
            assert!(mean > last, "tones keep their order");
            last = mean;
        }
    }

    #[test]
    fn a_roll_gets_one_balance() {
        let width = 32;
        let base = [0.75, 0.42, 0.22];
        let frame = |brightness: f32| {
            let scene: Vec<[f32; 3]> = (0..width * 24).map(|i| [brightness * 2f32.powf((i % width) as f32 / 6.0); 3]).collect();
            Sample::of(&negative(&scene, width, 4, base, [0.6; 3]))
        };
        let film = Film { kind: Kind::Colour, ..Default::default() };
        let crop = Crop { x: 0.5, y: 0.5, width: 32.0 / 40.0, height: 24.0 / 32.0 };
        let adjustments = Adjustments { film, crop, ..Default::default() };
        let frames = vec![(frame(0.002), adjustments.clone()), (frame(0.02), adjustments.clone())];
        let (roll_base, range) = balance_roll(Kind::Colour, &frames);
        let alone = resolve(&frames[0].0, &adjustments).unwrap();
        // The roll spans both frames: wider than either alone.
        assert!(range.high[1] > alone.range.high[1] + 0.3, "{range:?} vs {:?}", alone.range);
        assert!((roll_base[0] / base[0] - 1.0).abs() < 0.01);
        // So does its average, so every frame is drawn as bright as the roll is.
        assert!(range.key.is_some());
        let reach = |sample: &Sample| {
            let balanced = Adjustments { film: Film { range: Some(range), ..film }, ..adjustments.clone() };
            resolve(sample, &balanced).unwrap().reach()
        };
        assert_eq!(reach(&frames[0].0), reach(&frames[1].0));

        // A base picked by hand on one frame goes to the whole roll.
        let mut picked = adjustments.clone();
        picked.film.base = Some([0.7, 0.4, 0.2]);
        let (roll_base, _) = balance_roll(Kind::Colour, &[(frames[0].0.clone(), adjustments), (frames[1].0.clone(), picked)]);
        assert_eq!(roll_base, [0.7, 0.4, 0.2]);
    }
}
