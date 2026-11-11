//! Finding a photo's lens profile: the correction data the camera wrote into the RAW (Sony
//! ARW, DNG opcodes) when there is any, otherwise the lensfun database.
//!
//! Everything is converted to [`LensProfile`]'s radial tables (radius in half-diagonals of the
//! image). Formulas follow darktable's `lens.cc` (embedded data) and lensfun's `mod-coord.cpp`,
//! `mod-subpix.cpp` and `mod-color.cpp` (database models).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::OnceLock;

use rawler::formats::tiff::reader::TiffReader;
use rawler::formats::tiff::GenericTiffReader;
use serde::Deserialize;
use tonality_core::lens::{radial, LensProfile, LensSource, Radial};
use tonality_core::library::ImageMeta;

/// The lens profile for the RAW at `path`, if one is known.
pub fn lens_profile(path: &Path, meta: &ImageMeta) -> Option<LensProfile> {
    let name = || meta.lens.clone().unwrap_or_else(|| "Lens".into());
    embedded(path, name).or_else(|| lensfun(meta))
}

// ---------------------------------------------------------------------------------------------
// Embedded (maker) data
// ---------------------------------------------------------------------------------------------

const SUB_IFDS: u16 = 0x014a;
const SONY_VIGNETTING: u16 = 0x7032;
const SONY_CA: u16 = 0x7035;
const SONY_DISTORTION: u16 = 0x7037;
const OPCODE_LIST_2: u16 = 0xc741;
const OPCODE_LIST_3: u16 = 0xc74e;

fn embedded(path: &Path, name: impl Fn() -> String) -> Option<LensProfile> {
    let mut f = BufReader::new(File::open(path).ok()?);
    let tiff = GenericTiffReader::new(&mut f, 0, 0, None, &[SUB_IFDS]).ok()?;
    let ints = |tag: u16| -> Option<Vec<f32>> {
        let e = tiff.find_first_ifd_with_tag(tag)?.get_entry(tag)?;
        Some((0..e.count() as usize).map(|i| e.value.force_i16(i) as f32).collect())
    };
    if let (Some(d), Some(c), Some(v)) = (ints(SONY_DISTORTION), ints(SONY_CA), ints(SONY_VIGNETTING)) {
        if let Some(p) = sony(&d, &c, &v, name()) {
            return Some(p);
        }
    }
    let data = |tag: u16| -> Option<Vec<u8>> {
        let e = tiff.find_first_ifd_with_tag(tag)?.get_entry(tag)?;
        Some(e.value.get_data().clone())
    };
    let mut ops = Vec::new();
    for tag in [OPCODE_LIST_2, OPCODE_LIST_3] {
        if let Some(d) = data(tag) {
            ops.extend(dng_opcodes(&d));
        }
    }
    dng(&ops, name())
}

/// Sony `DistortionCorrParams`, `ChromaticAberrationCorrParams`, `VignettingCorrParams`:
/// element 0 is the knot count `nc`, then `nc` values (CA: red then blue). Knot `i` sits at
/// radius `(i + 0.5) / (nc - 1)`.
fn sony(d: &[f32], c: &[f32], v: &[f32], name: String) -> Option<LensProfile> {
    let nc = *d.first()? as usize;
    if !(2..=16).contains(&nc) || c.first() != Some(&(2.0 * nc as f32)) || v.first() != Some(&(nc as f32)) {
        return None;
    }
    if d.len() <= nc || c.len() <= 2 * nc || v.len() <= nc {
        return None;
    }
    let knots: Vec<f32> = (0..nc).map(|i| (i as f32 + 0.5) / (nc - 1) as f32).collect();
    let at = |vals: Vec<f32>| -> Radial { radial(|r| interp(&knots, &vals, r)) };
    let g: Vec<f32> = (0..nc).map(|i| d[i + 1] * (-14f32).exp2() + 1.0).collect();
    let red: Vec<f32> = (0..nc).map(|i| g[i] * (c[i + 1] * (-21f32).exp2() + 1.0)).collect();
    let blue: Vec<f32> = (0..nc).map(|i| g[i] * (c[nc + i + 1] * (-21f32).exp2() + 1.0)).collect();
    // The pixel is divided by this factor.
    let vig: Vec<f32> = (0..nc).map(|i| 1.0 / (0.5 - (v[i + 1] * (-13f32).exp2() - 1.0).exp2()).exp2()).collect();
    let chromatic = c[1..=2 * nc].iter().any(|&x| x != 0.0);
    Some(LensProfile {
        source: LensSource::Embedded,
        name,
        distortion: Some([at(red), at(g), at(blue)]),
        chromatic,
        vignetting: Some(at(vig)),
    })
}

/// Piecewise-linear `y(x)` through the knots, flat beyond them.
fn interp(xs: &[f32], ys: &[f32], x: f32) -> f32 {
    if x <= xs[0] {
        return ys[0];
    }
    for i in 1..xs.len() {
        if x <= xs[i] {
            let t = (x - xs[i - 1]) / (xs[i] - xs[i - 1]).max(1e-9);
            return ys[i - 1] + t * (ys[i] - ys[i - 1]);
        }
    }
    *ys.last().unwrap()
}

#[derive(Debug, Clone, PartialEq)]
enum Opcode {
    /// Per plane (1 or 3): radial coefficients kr0..kr3.
    WarpRectilinear(Vec<[f64; 4]>),
    /// k0..k4.
    FixVignetteRadial([f64; 5]),
}

/// Parse a DNG opcode list (big-endian): count, then (id, version, flags, size, parameters).
fn dng_opcodes(d: &[u8]) -> Vec<Opcode> {
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
    let f64_at = |o: usize| d.get(o..o + 8).map(|b| f64::from_be_bytes(b.try_into().unwrap()));
    let mut out = Vec::new();
    let Some(n) = u32_at(0) else { return out };
    let mut o = 4;
    for _ in 0..n.min(64) {
        let (Some(id), Some(size)) = (u32_at(o), u32_at(o + 12)) else { break };
        let p = o + 16;
        match id {
            1 => {
                if let Some(planes) = u32_at(p).filter(|n| (1..=4).contains(n)) {
                    let coeffs: Option<Vec<[f64; 4]>> = (0..planes as usize)
                        .map(|i| {
                            let b = p + 4 + i * 48;
                            Some([f64_at(b)?, f64_at(b + 8)?, f64_at(b + 16)?, f64_at(b + 24)?])
                        })
                        .collect();
                    if let Some(c) = coeffs {
                        out.push(Opcode::WarpRectilinear(c));
                    }
                }
            }
            3 => {
                let k: Option<Vec<f64>> = (0..5).map(|i| f64_at(p + i * 8)).collect();
                if let Some(k) = k {
                    out.push(Opcode::FixVignetteRadial(k.try_into().unwrap()));
                }
            }
            _ => {}
        }
        o = p + size as usize;
    }
    out
}

/// DNG `WarpRectilinear` (source radius ratio kr0 + kr1 r² + kr2 r⁴ + kr3 r⁶) and
/// `FixVignetteRadial` (gain 1 + k0 r² + … + k4 r¹⁰). Both normalise r to the farthest corner,
/// i.e. half-diagonals for a centred lens (the optical centre offset is ignored).
fn dng(ops: &[Opcode], name: String) -> Option<LensProfile> {
    let mut p = LensProfile { source: LensSource::Embedded, name, distortion: None, chromatic: false, vignetting: None };
    for op in ops {
        match op {
            Opcode::WarpRectilinear(planes) => {
                let f = |k: [f64; 4]| -> Radial {
                    radial(|r| {
                        let r2 = (r * r) as f64;
                        (k[0] + r2 * (k[1] + r2 * (k[2] + r2 * k[3]))) as f32
                    })
                };
                let ch: Vec<Radial> = planes.iter().map(|&k| f(k)).collect();
                p.distortion = Some(if ch.len() >= 3 { [ch[0], ch[1], ch[2]] } else { [ch[0]; 3] });
                p.chromatic = ch.len() >= 3 && (ch[0] != ch[1] || ch[2] != ch[1]);
            }
            Opcode::FixVignetteRadial(k) => {
                p.vignetting = Some(radial(|r| {
                    let r2 = (r * r) as f64;
                    let mut g = 0.0;
                    for c in k.iter().rev() {
                        g = (g + c) * r2;
                    }
                    (1.0 + g) as f32
                }));
            }
        }
    }
    (p.distortion.is_some() || p.vignetting.is_some()).then_some(p)
}

// ---------------------------------------------------------------------------------------------
// lensfun
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct Db {
    mounts: Vec<(String, Vec<String>)>,
    cameras: Vec<DbCamera>,
    lenses: Vec<DbLens>,
}

#[derive(Deserialize)]
struct DbCamera {
    maker: String,
    models: Vec<String>,
    mount: String,
    crop: f32,
}

#[derive(Deserialize)]
struct DbLens {
    maker: String,
    models: Vec<String>,
    mounts: Vec<String>,
    crop: f32,
    aspect: f32,
    /// `[focal, model, terms…]`
    dist: Vec<serde_json::Value>,
    tca: Vec<serde_json::Value>,
    /// `[focal, aperture, distance, k1, k2, k3]`
    vig: Vec<[f32; 6]>,
}

fn db() -> &'static Db {
    static DB: OnceLock<Db> = OnceLock::new();
    DB.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/lensfun.json"))).expect("lensfun.json is generated at build time")
    })
}

/// Lower-case words and numbers, without "/" (so "f/3.5-5.6" and "F3.5-5.6" agree) and with
/// "mm" split off ("18-135mm" = "18-135 mm").
fn tokens(s: &str) -> Vec<String> {
    let s = s.to_lowercase().replace('/', "");
    let (mut out, mut cur, mut kind) = (Vec::new(), String::new(), 0u8);
    for c in s.chars() {
        // 1: letters, 2: a number (digits, with '.' and '-' inside it), 0: separator.
        let k = if c.is_alphabetic() {
            1
        } else if c.is_ascii_digit() || ((c == '.' || c == '-') && kind == 2) {
            2
        } else {
            0
        };
        if k != kind && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if k != 0 {
            cur.push(c);
        }
        kind = k;
    }
    out.push(cur);
    out.into_iter().map(|t| t.trim_end_matches(['-', '.']).to_string()).filter(|t| !t.is_empty()).collect()
}

/// How well a database lens name matches the camera's lens string: every number in the
/// database name must appear, and most of its words. `None` if it doesn't match.
fn name_score(db_name: &str, exif: &[String], maker: &str) -> Option<f32> {
    let skip = tokens(maker);
    let want: Vec<String> = tokens(db_name).into_iter().filter(|t| !skip.contains(t)).collect();
    if want.is_empty() {
        return None;
    }
    let has = |t: &String| exif.contains(t);
    if want.iter().any(|t| t.starts_with(|c: char| c.is_ascii_digit()) && !has(t)) {
        return None;
    }
    let hit = want.iter().filter(|t| has(t)).count() as f32;
    let score = hit / want.len() as f32;
    let extra = exif.iter().filter(|t| !want.contains(t) && !skip.contains(t)).count() as f32;
    (score >= 0.75).then_some(score - 0.01 * extra)
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn lensfun(meta: &ImageMeta) -> Option<LensProfile> {
    let lens_name = meta.lens.as_deref()?;
    let focal = meta.focal_length?;
    let db = db();
    let maker_matches = |m: &str| {
        let (a, b) = (m.trim().to_lowercase(), meta.make.trim().to_lowercase());
        // An empty make would "contain" in every maker: no match rather than any camera.
        !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
    };
    let cam = db.cameras.iter().find(|c| maker_matches(&c.maker) && c.models.iter().any(|m| same(m, &meta.model)))?;
    let mut mounts = vec![cam.mount.as_str()];
    if let Some((_, compat)) = db.mounts.iter().find(|(n, _)| *n == cam.mount) {
        mounts.extend(compat.iter().map(String::as_str));
    }
    let exif = tokens(lens_name);
    let lens = db
        .lenses
        .iter()
        .filter(|l| l.mounts.iter().any(|m| mounts.contains(&m.as_str())) && l.crop <= cam.crop * 1.02)
        .filter_map(|l| {
            let s = l.models.iter().filter_map(|m| name_score(m, &exif, &l.maker)).fold(f32::MIN, f32::max);
            (s > f32::MIN).then_some((s, l))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))?
        .1;
    profile_from_lensfun(lens, cam.crop, focal, meta.f_number.unwrap_or(8.0))
}

/// Entries' terms at `focal`: linear between the nearest calibrated focal lengths with the same
/// model, otherwise the nearest one.
fn at_focal(entries: &[serde_json::Value], focal: f32) -> Option<(String, Vec<f32>)> {
    let mut rows: Vec<(f32, String, Vec<f32>)> = entries
        .iter()
        .filter_map(|e| {
            let a = e.as_array()?;
            let f = a.first()?.as_f64()? as f32;
            let model = a.get(1)?.as_str()?.to_string();
            let terms = a[2..].iter().filter_map(|v| v.as_f64().map(|v| v as f32)).collect();
            Some((f, model, terms))
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let below = rows.iter().rev().find(|r| r.0 <= focal);
    let above = rows.iter().find(|r| r.0 >= focal);
    match (below, above) {
        (Some(a), Some(b)) if a.1 == b.1 && b.0 > a.0 => {
            let t = (focal - a.0) / (b.0 - a.0);
            Some((a.1.clone(), a.2.iter().zip(&b.2).map(|(x, y)| x + t * (y - x)).collect()))
        }
        (Some(a), Some(b)) => Some(if focal - a.0 <= b.0 - focal { (a.1.clone(), a.2.clone()) } else { (b.1.clone(), b.2.clone()) }),
        (Some(a), None) | (None, Some(a)) => Some((a.1.clone(), a.2.clone())),
        (None, None) => None,
    }
}

/// Vignetting k1..k3 at `focal` and `aperture`, for the farthest focus distance calibrated
/// (the subject distance isn't known): bilinear in focal length and stops.
fn vignetting_at(vig: &[[f32; 6]], focal: f32, aperture: f32) -> Option<[f32; 3]> {
    let far = vig.iter().map(|v| v[2]).fold(f32::MIN, f32::max);
    let rows: Vec<&[f32; 6]> = vig.iter().filter(|v| v[2] == far).collect();
    let mut focals: Vec<f32> = rows.iter().map(|v| v[0]).collect();
    focals.sort_by(f32::total_cmp);
    focals.dedup();
    let at_focal = |f: f32| -> Option<[f32; 3]> {
        let mut ap: Vec<&&[f32; 6]> = rows.iter().filter(|v| v[0] == f).collect();
        ap.sort_by(|a, b| a[1].total_cmp(&b[1]));
        let s = |a: f32| a.max(0.5).log2();
        let lo = ap.iter().rev().find(|v| v[1] <= aperture).or(ap.first())?;
        let hi = ap.iter().find(|v| v[1] >= aperture).or(ap.last())?;
        let t = if hi[1] > lo[1] { ((s(aperture) - s(lo[1])) / (s(hi[1]) - s(lo[1]))).clamp(0.0, 1.0) } else { 0.0 };
        Some(std::array::from_fn(|i| lo[3 + i] + t * (hi[3 + i] - lo[3 + i])))
    };
    let lo = focals.iter().rev().find(|&&f| f <= focal).or(focals.first())?;
    let hi = focals.iter().find(|&&f| f >= focal).or(focals.last())?;
    let (a, b) = (at_focal(*lo)?, at_focal(*hi)?);
    let t = if hi > lo { ((focal - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    Some(std::array::from_fn(|i| a[i] + t * (b[i] - a[i])))
}

fn profile_from_lensfun(lens: &DbLens, cam_crop: f32, focal: f32, aperture: f32) -> Option<LensProfile> {
    // lensfun's distortion and TCA models use Hugin units (r = 1 at half the short side of the
    // calibration frame); vignetting uses r = 1 at its corner. Our r is half-diagonals of this
    // image, whose sensor may be smaller than the calibration one.
    let crop = lens.crop / cam_crop;
    let hugin = |r: f32| r * lens.aspect.max(1.0).hypot(1.0) * crop;

    let dist = at_focal(&lens.dist, focal);
    let ratio = |r: f32| -> f32 {
        let h = hugin(r);
        match &dist {
            Some((m, t)) if m == "ptlens" && t.len() >= 3 => {
                let d = 1.0 - t[0] - t[1] - t[2];
                let x = h / d;
                (t[0] * x * x * x + t[1] * x * x + t[2] * x + d) / d
            }
            Some((m, t)) if m == "poly3" && !t.is_empty() => {
                let d = 1.0 - t[0];
                let x = h / d;
                1.0 + t[0] * x * x / d
            }
            Some((m, t)) if m == "poly5" && t.len() >= 2 => 1.0 + t[0] * h * h + t[1] * h * h * h * h,
            _ => 1.0,
        }
    };
    let tca = at_focal(&lens.tca, focal);
    // Red / blue radius multipliers, at a (distorted) Hugin radius.
    let tca_at = |c: usize, h: f32| -> f32 {
        match &tca {
            Some((m, t)) if m == "linear" && t.len() >= 2 => t[if c == 0 { 0 } else { 1 }],
            Some((m, t)) if m == "poly3" && t.len() >= 6 => {
                let (v, cc, b) = if c == 0 { (t[0], t[2], t[4]) } else { (t[1], t[3], t[5]) };
                b * h * h + cc * h + v
            }
            _ => 1.0,
        }
    };
    let green = radial(ratio);
    let red = radial(|r| ratio(r) * tca_at(0, hugin(r) * ratio(r)));
    let blue = radial(|r| ratio(r) * tca_at(2, hugin(r) * ratio(r)));

    let vignetting = vignetting_at(&lens.vig, focal, aperture).map(|[k1, k2, k3]| {
        radial(|r| {
            let v2 = (r * crop).powi(2);
            1.0 / (1.0 + v2 * (k1 + v2 * (k2 + v2 * k3))).max(0.05)
        })
    });
    let name = match lens.models.first() {
        Some(m) if m.to_lowercase().starts_with(&lens.maker.to_lowercase()) => m.clone(),
        Some(m) => format!("{} {m}", lens.maker),
        None => lens.maker.clone(),
    };
    Some(LensProfile {
        source: LensSource::Lensfun,
        name,
        distortion: (dist.is_some() || tca.is_some()).then_some([red, green, blue]),
        chromatic: tca.is_some(),
        vignetting,
    })
}

/// Largest |ratio − 1| of a radial function out to the corners.
#[cfg(test)]
fn max_deviation(f: &Radial) -> f32 {
    (0..=20).map(|i| (tonality_core::lens::sample(f, i as f32 / 20.0) - 1.0).abs()).fold(0.0, f32::max)
}

/// Both sources for one photo (diagnostics): `(embedded, lensfun)`.
pub fn lens_profiles(path: &Path, meta: &ImageMeta) -> (Option<LensProfile>, Option<LensProfile>) {
    let name = || meta.lens.clone().unwrap_or_else(|| "Lens".into());
    (embedded(path, name), lensfun(meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonality_core::lens::sample;

    #[test]
    fn an_empty_make_matches_no_camera() {
        assert!(lensfun(&meta("", "ILCE-7M3", "FE 28-70mm F3.5-5.6 OSS", 35.0, 5.6)).is_none());
    }

    #[test]
    fn tokenises_lens_names_alike() {
        assert_eq!(tokens("E 18-135mm F3.5-5.6 OSS"), ["e", "18-135", "mm", "f", "3.5-5.6", "oss"]);
        assert_eq!(tokens("E 18-135mm f/3.5-5.6 OSS"), tokens("E 18-135mm F3.5-5.6 OSS"));
    }

    fn meta(make: &str, model: &str, lens: &str, focal: f32, f: f32) -> ImageMeta {
        ImageMeta {
            make: make.into(),
            model: model.into(),
            lens: Some(lens.into()),
            focal_length: Some(focal),
            f_number: Some(f),
            ..Default::default()
        }
    }

    #[test]
    fn finds_lenses_in_lensfun() {
        let p = lensfun(&meta("Sony", "ILCE-6600", "E 18-135mm F3.5-5.6 OSS", 42.0, 5.6)).expect("in the database");
        assert_eq!(p.name, "Sony E 18-135mm f/3.5-5.6 OSS");
        assert!(p.distortion.is_some() && p.vignetting.is_some());
        let p = lensfun(&meta("Canon", "Canon EOS 5D Mark III", "EF24-105mm f/4L IS USM", 24.0, 4.0)).expect("Canon");
        assert!(p.name.contains("24-105"), "{}", p.name);
        assert!(lensfun(&meta("Sony", "ILCE-6600", "Some 12-34mm F9 Lens", 20.0, 9.0)).is_none());
        assert!(lensfun(&meta("Nobody", "Camera 1", "E 18-135mm F3.5-5.6 OSS", 42.0, 5.6)).is_none());
    }

    #[test]
    fn parses_sony_and_dng_data() {
        // DSC00084.ARW (E 18-135mm at 42 mm).
        let d = [11., 3., 1., -2., -6., -11., -18., -24., -31., -37., -40., -37., 0., 0., 0., 0., 0.];
        let c = [22., 128., 128., 0., 0., 0., 0., 128., 128., 256., 384., 640., 1408., 1152., 1024., 896., 896., 896., 768., 640., 512., 256., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0.];
        let v = [11., 0., 96., 384., 816., 1328., 1824., 2320., 2816., 3312., 3840., 4416., 0., 0., 0., 0., 0.];
        let p = sony(&d, &c, &v, "x".into()).unwrap();
        let [_, g, _] = p.distortion.unwrap();
        assert!((sample(&g, 1.0) - (1.0 - 37.0 / 16384.0)).abs() < 2e-4);
        let vig = p.vignetting.unwrap();
        assert!((sample(&vig, 0.05) - 1.0).abs() < 1e-3);
        assert!(sample(&vig, 1.0) > 1.15 && sample(&vig, 1.0) < 1.2, "{}", sample(&vig, 1.0));
        assert!(sony(&d[..5], &c, &v, "x".into()).is_none());
        assert!(max_deviation(&g) < 0.01, "almost no distortion at 42 mm");

        let mut list = Vec::new();
        list.extend(2u32.to_be_bytes());
        // WarpRectilinear, one plane: kr = [1, -0.02, 0, 0], kt = 0, centre 0.5.
        list.extend([1u32, 0x0103_0000, 0, 4 + 48 + 16].iter().flat_map(|v| v.to_be_bytes()));
        list.extend(1u32.to_be_bytes());
        for v in [1.0f64, -0.02, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5] {
            list.extend(v.to_be_bytes());
        }
        // FixVignetteRadial: k0 = 0.5.
        list.extend([3u32, 0x0103_0000, 0, 56].iter().flat_map(|v| v.to_be_bytes()));
        for v in [0.5f64, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5] {
            list.extend(v.to_be_bytes());
        }
        let ops = dng_opcodes(&list);
        assert_eq!(ops.len(), 2);
        let p = dng(&ops, "x".into()).unwrap();
        assert!((sample(&p.distortion.unwrap()[1], 1.0) - 0.98).abs() < 1e-5);
        assert!((sample(&p.vignetting.unwrap(), 1.0) - 1.5).abs() < 1e-3);
    }
}
