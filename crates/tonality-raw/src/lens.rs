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
        Some(
            (0..e.count() as usize)
                .map(|i| e.value.force_i16(i) as f32)
                .collect(),
        )
    };
    if let (Some(d), Some(c), Some(v)) =
        (ints(SONY_DISTORTION), ints(SONY_CA), ints(SONY_VIGNETTING))
    {
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
    if !(2..=16).contains(&nc)
        || c.first() != Some(&(2.0 * nc as f32))
        || v.first() != Some(&(nc as f32))
    {
        return None;
    }
    if d.len() <= nc || c.len() <= 2 * nc || v.len() <= nc {
        return None;
    }
    let knots: Vec<f32> = (0..nc)
        .map(|i| (i as f32 + 0.5) / (nc - 1) as f32)
        .collect();
    let at = |vals: Vec<f32>| -> Radial { radial(|r| interp(&knots, &vals, r)) };
    let g: Vec<f32> = (0..nc).map(|i| d[i + 1] * (-14f32).exp2() + 1.0).collect();
    let red: Vec<f32> = (0..nc)
        .map(|i| g[i] * (c[i + 1] * (-21f32).exp2() + 1.0))
        .collect();
    let blue: Vec<f32> = (0..nc)
        .map(|i| g[i] * (c[nc + i + 1] * (-21f32).exp2() + 1.0))
        .collect();
    // The pixel is divided by this factor.
    let vig: Vec<f32> = (0..nc)
        .map(|i| 1.0 / (0.5 - (v[i + 1] * (-13f32).exp2() - 1.0).exp2()).exp2())
        .collect();
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
    let u32_at = |o: usize| {
        d.get(o..o + 4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    };
    let f64_at = |o: usize| {
        d.get(o..o + 8)
            .map(|b| f64::from_be_bytes(b.try_into().unwrap()))
    };
    let mut out = Vec::new();
    let Some(n) = u32_at(0) else { return out };
    let mut o = 4;
    for _ in 0..n.min(64) {
        let (Some(id), Some(size)) = (u32_at(o), u32_at(o + 12)) else {
            break;
        };
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
    let mut p = LensProfile {
        source: LensSource::Embedded,
        name,
        distortion: None,
        chromatic: false,
        vignetting: None,
    };
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
                p.distortion = Some(if ch.len() >= 3 {
                    [ch[0], ch[1], ch[2]]
                } else {
                    [ch[0]; 3]
                });
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
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/lensfun.json")))
            .expect("lensfun.json is generated at build time")
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
    out.into_iter()
        .map(|t| t.trim_end_matches(['-', '.']).to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// The version markers in a tokenised lens name ("II", "Mark III", "Mk2"), as numbers.
fn versions<S: AsRef<str>>(t: &[S]) -> Vec<u8> {
    let roman = |s: &str| match s {
        "ii" => Some(2),
        "iii" => Some(3),
        "iv" => Some(4),
        _ => None,
    };
    let mut out = Vec::new();
    for (i, w) in t.iter().map(AsRef::as_ref).enumerate() {
        if let Some(v) = roman(w) {
            out.push(v);
        } else if matches!(w, "mark" | "mk") {
            // "Mark 2" / "Mk2": the number belongs to the marker ("Mark II" is caught above).
            if let Some(v) = t.get(i + 1).and_then(|n| n.as_ref().parse::<u8>().ok()) {
                out.push(v);
            }
        }
    }
    out.sort_unstable();
    out
}

/// A lens name as words: `core` are the ones outside parentheses and brackets, `all` adds the
/// ones inside. What's inside is a product code or a note ("(A012)", "[IF]", "(SEL35F14GM)"),
/// which one of two names for the same lens often has and the other doesn't.
struct Words {
    core: Vec<String>,
    all: Vec<String>,
}

/// Words that are in every lens name, or only decorate one ("Zoom-Nikkor"): no evidence of
/// which lens it is.
const FILLER: [&str; 3] = ["mm", "f", "zoom"];

fn words(name: &str, maker: &str) -> Words {
    let skip = tokens(maker);
    let clean = |text: &str| -> Vec<String> {
        tokens(text)
            .into_iter()
            .filter(|t| !skip.contains(t) && !FILLER.contains(&t.as_str()))
            .map(|t| match t.as_str() {
                // Sigma's lines, which a camera abbreviates and lensfun sometimes spells out.
                "art" => "a".into(),
                "contemporary" => "c".into(),
                "sport" | "sports" => "s".into(),
                _ => t,
            })
            .collect()
    };
    let (mut outside, mut inside, mut depth) = (String::new(), String::new(), 0u32);
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => {
                outside.push(c);
                continue;
            }
            _ => {
                inside.push(c);
                continue;
            }
        }
        // A bracket separates words on both sides of it.
        outside.push(' ');
        inside.push(' ');
    }
    // A maker's product code written without parentheses ("| Contemporary 017", "VC USD
    // A009"): a number with a leading zero is never a focal length or an aperture. It, and a
    // single letter in front of it, count like the words in parentheses.
    let mut core = clean(&outside);
    let mut codes = Vec::new();
    while let Some(i) = core
        .iter()
        .position(|t| t.len() >= 3 && t.starts_with('0') && t.chars().all(|c| c.is_ascii_digit()))
    {
        codes.push(core.remove(i));
        if i > 0 && core[i - 1].len() == 1 && core[i - 1].chars().all(char::is_alphabetic) {
            codes.push(core.remove(i - 1));
        }
    }
    let mut all = core.clone();
    all.extend(codes);
    all.extend(clean(&inside));
    Words { core, all }
}

/// How well a database lens name matches the camera's lens string, or `None` if it isn't the
/// same lens.
///
/// A lens that isn't in the database must match nothing: a profile for another lens bends and
/// brightens the photo wrongly, which is worse than offering none. Two lenses of the same
/// focal length and aperture share every number and "mm" and "f", so those alone say nothing.
/// What tells lenses apart is the other words (the mount letters, "GM", "Art", "VCM"), and so:
///
/// - every number in the database name must be in the camera's string;
/// - both must name the same version ("II", "Mark III");
/// - the camera's string may not have a word the database name lacks: a newer design is often
///   the old name plus one letter ("RF70-200mm F2.8 L IS USM Z");
/// - of the database name's words, most must be in the camera's string, or (for the short
///   names some cameras write, "M.12-40mm F2.8") all of the camera's words must be in it.
///
/// Words in parentheses count for the other name's benefit only (see [`Words`]). The score is
/// the share of both names' words, those included, that the other has: of two names that both
/// pass, the one that also agrees on the product code wins ("E 20mm F2.8 F050" is Tamron's,
/// "E 20mm F2.8" is Sony's).
fn name_score(db_name: &str, exif_name: &str, maker: &str) -> Option<f32> {
    let (want, exif) = (words(db_name, maker), words(exif_name, maker));
    if want.core.is_empty() || exif.core.is_empty() {
        return None;
    }
    let number = |t: &String| t.starts_with(|c: char| c.is_ascii_digit());
    if want.core.iter().any(|t| number(t) && !exif.all.contains(t)) {
        return None;
    }
    if versions(&want.all) != versions(&exif.all) {
        return None;
    }
    if exif
        .core
        .iter()
        .any(|t| !number(t) && !want.all.contains(t))
    {
        return None;
    }
    let db_hits = want.core.iter().filter(|t| exif.all.contains(t)).count();
    let exif_hits = exif.core.iter().filter(|t| want.all.contains(t)).count();
    let most_of_db = db_hits as f32 / want.core.len() as f32 >= 0.75;
    let all_of_exif = exif_hits == exif.core.len() && exif.core.iter().any(number);
    let shared = |a: &Words, b: &Words| a.all.iter().filter(|t| b.all.contains(t)).count();
    (most_of_db || all_of_exif).then_some(
        (shared(&want, &exif) + shared(&exif, &want)) as f32
            / (want.all.len() + exif.all.len()) as f32,
    )
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The lensfun camera for a decoder's make and model. Those are rawler's cleaned names ("Pentax",
/// "K-1"), and lensfun often lists only what the camera writes in EXIF ("RICOH IMAGING COMPANY,
/// LTD.", "PENTAX K-1"), so the EXIF names rawler knows for that camera are tried too.
fn camera(make: &str, model: &str) -> Option<&'static DbCamera> {
    let mut names = vec![(make.to_string(), model.to_string())];
    names.extend(
        rawler::global_loader()
            .get_cameras()
            .values()
            .filter(|c| same(&c.clean_make, make) && same(&c.clean_model, model))
            .map(|c| (c.make.clone(), c.model.clone())),
    );
    let maker_matches = |m: &str, make: &str| {
        let (a, b) = (m.trim().to_lowercase(), make.trim().to_lowercase());
        // An empty make would "contain" in every maker: no match rather than any camera.
        !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
    };
    names.iter().find_map(|(make, model)| {
        db().cameras
            .iter()
            .find(|c| maker_matches(&c.maker, make) && c.models.iter().any(|m| same(m, model)))
    })
}

fn lensfun(meta: &ImageMeta) -> Option<LensProfile> {
    let lens_name = meta.lens.as_deref()?;
    let focal = meta.focal_length?;
    let db = db();
    let cam = camera(&meta.make, &meta.model)?;
    let mut mounts = vec![cam.mount.as_str()];
    if let Some((_, compat)) = db.mounts.iter().find(|(n, _)| *n == cam.mount) {
        mounts.extend(compat.iter().map(String::as_str));
    }
    let lens = db
        .lenses
        .iter()
        .filter(|l| {
            l.mounts.iter().any(|m| mounts.contains(&m.as_str())) && l.crop <= cam.crop * 1.02
        })
        .filter_map(|l| {
            let s = l
                .models
                .iter()
                .filter_map(|m| name_score(m, lens_name, &l.maker))
                .fold(f32::MIN, f32::max);
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
            let terms = a[2..]
                .iter()
                .filter_map(|v| v.as_f64().map(|v| v as f32))
                .collect();
            Some((f, model, terms))
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let below = rows.iter().rev().find(|r| r.0 <= focal);
    let above = rows.iter().find(|r| r.0 >= focal);
    match (below, above) {
        (Some(a), Some(b)) if a.1 == b.1 && b.0 > a.0 => {
            let t = (focal - a.0) / (b.0 - a.0);
            Some((
                a.1.clone(),
                a.2.iter().zip(&b.2).map(|(x, y)| x + t * (y - x)).collect(),
            ))
        }
        (Some(a), Some(b)) => Some(if focal - a.0 <= b.0 - focal {
            (a.1.clone(), a.2.clone())
        } else {
            (b.1.clone(), b.2.clone())
        }),
        (Some(a), None) | (None, Some(a)) => Some((a.1.clone(), a.2.clone())),
        (None, None) => None,
    }
}

/// Vignetting k1..k3 at `focal` and `aperture`, for the farthest focus distance calibrated
/// (the subject distance isn't known): bilinear in focal length and stops.
///
/// "Farthest" is per focal length. Calibrations don't use one set of distances throughout
/// (a zoom measured at 300 m at one end and 1000 m at the other, or one stray pair at 1000 m),
/// and keeping only the rows at the lens's single farthest distance would throw every other
/// focal length away.
fn vignetting_at(vig: &[[f32; 6]], focal: f32, aperture: f32) -> Option<[f32; 3]> {
    let mut focals: Vec<f32> = vig.iter().map(|v| v[0]).collect();
    focals.sort_by(f32::total_cmp);
    focals.dedup();
    let at_focal = |f: f32| -> Option<[f32; 3]> {
        let here = || vig.iter().filter(move |v| v[0] == f);
        let far = here().map(|v| v[2]).fold(f32::MIN, f32::max);
        let mut ap: Vec<&[f32; 6]> = here().filter(|v| v[2] == far).collect();
        ap.sort_by(|a, b| a[1].total_cmp(&b[1]));
        let s = |a: f32| a.max(0.5).log2();
        let lo = ap.iter().rev().find(|v| v[1] <= aperture).or(ap.first())?;
        let hi = ap.iter().find(|v| v[1] >= aperture).or(ap.last())?;
        let t = if hi[1] > lo[1] {
            ((s(aperture) - s(lo[1])) / (s(hi[1]) - s(lo[1]))).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(std::array::from_fn(|i| {
            lo[3 + i] + t * (hi[3 + i] - lo[3 + i])
        }))
    };
    let lo = focals
        .iter()
        .rev()
        .find(|&&f| f <= focal)
        .or(focals.first())?;
    let hi = focals.iter().find(|&&f| f >= focal).or(focals.last())?;
    let (a, b) = (at_focal(*lo)?, at_focal(*hi)?);
    let t = if hi > lo {
        ((focal - lo) / (hi - lo)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some(std::array::from_fn(|i| a[i] + t * (b[i] - a[i])))
}

fn profile_from_lensfun(
    lens: &DbLens,
    cam_crop: f32,
    focal: f32,
    aperture: f32,
) -> Option<LensProfile> {
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
            Some((m, t)) if m == "poly5" && t.len() >= 2 => {
                1.0 + t[0] * h * h + t[1] * h * h * h * h
            }
            _ => 1.0,
        }
    };
    let tca = at_focal(&lens.tca, focal);
    // Red / blue radius multipliers, at a (distorted) Hugin radius.
    let tca_at = |c: usize, h: f32| -> f32 {
        match &tca {
            Some((m, t)) if m == "linear" && t.len() >= 2 => t[if c == 0 { 0 } else { 1 }],
            Some((m, t)) if m == "poly3" && t.len() >= 6 => {
                let (v, cc, b) = if c == 0 {
                    (t[0], t[2], t[4])
                } else {
                    (t[1], t[3], t[5])
                };
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
    (0..=20)
        .map(|i| (tonality_core::lens::sample(f, i as f32 / 20.0) - 1.0).abs())
        .fold(0.0, f32::max)
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
        assert_eq!(
            tokens("E 18-135mm F3.5-5.6 OSS"),
            ["e", "18-135", "mm", "f", "3.5-5.6", "oss"]
        );
        assert_eq!(
            tokens("E 18-135mm f/3.5-5.6 OSS"),
            tokens("E 18-135mm F3.5-5.6 OSS")
        );
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
        let p = lensfun(&meta(
            "Sony",
            "ILCE-6600",
            "E 18-135mm F3.5-5.6 OSS",
            42.0,
            5.6,
        ))
        .expect("in the database");
        assert_eq!(p.name, "Sony E 18-135mm f/3.5-5.6 OSS");
        assert!(p.distortion.is_some() && p.vignetting.is_some());
        let p = lensfun(&meta(
            "Canon",
            "EOS 5D Mark III",
            "EF24-105mm f/4L IS USM",
            24.0,
            4.0,
        ))
        .expect("Canon");
        assert!(p.name.contains("24-105"), "{}", p.name);
        assert!(lensfun(&meta(
            "Sony",
            "ILCE-6600",
            "Some 12-34mm F9 Lens",
            20.0,
            9.0
        ))
        .is_none());
        assert!(lensfun(&meta(
            "Nobody",
            "Camera 1",
            "E 18-135mm F3.5-5.6 OSS",
            42.0,
            5.6
        ))
        .is_none());
    }

    #[test]
    fn finds_cameras_by_the_decoders_names() {
        // rawler's cleaned make and model, as `read_meta` reports them, and a model name lensfun
        // has for that camera.
        for (make, model, lensfun_model) in [
            ("Pentax", "K-1", "Pentax K-1"),
            ("Pentax", "K-3 Mark III", "Pentax K-3 Mark III"),
            ("Nikon", "Z 50 II", "Nikon Z50_2"),
            ("OM Digital Solutions", "OM-1 Mark II", "OM-1MarkII"),
            ("Fujifilm", "GFX 100 II", "GFX100 II"),
            ("Leica", "M9", "M9 Digital Camera"),
            ("Canon", "PowerShot G7 X", "Canon PowerShot G7 X"),
            ("Canon", "EOS 5D Mark III", "Canon EOS 5D Mark III"),
            ("Sony", "ILCE-6600", "ILCE-6600"),
        ] {
            let cam = camera(make, model).unwrap_or_else(|| panic!("{make} {model}"));
            assert!(
                cam.models.iter().any(|m| m == lensfun_model),
                "{make} {model}: {:?}",
                cam.models
            );
        }
        assert!(camera("Pentax", "K-9000").is_none());
    }

    #[test]
    fn needs_the_same_lens_version() {
        let name =
            |make, model, lens| lensfun(&meta(make, model, lens, 100.0, 4.0)).map(|p| p.name);
        // Only the Mark I is in the database.
        assert_eq!(
            name("Sony", "ILCE-6600", "FE 70-200mm F2.8 GM OSS").as_deref(),
            Some("Sony FE 70-200mm f/2.8 GM OSS")
        );
        assert_eq!(
            name("Sony", "ILCE-6600", "FE 70-200mm F2.8 GM OSS II"),
            None
        );
        // The IS and IS II are; the IS III isn't.
        let canon = |lens| name("Canon", "EOS 5D Mark IV", lens);
        assert_eq!(
            canon("EF70-200mm f/2.8L IS USM").as_deref(),
            Some("Canon EF 70-200mm f/2.8L IS USM")
        );
        assert_eq!(
            canon("EF70-200mm f/2.8L IS II USM").as_deref(),
            Some("Canon EF 70-200mm f/2.8L IS II USM")
        );
        assert_eq!(canon("EF70-200mm f/2.8L IS III USM"), None);
        assert_eq!(versions(&tokens("Mark II")), [2]);
        assert_eq!(versions(&tokens("OM-1 Mk3")), [3]);
        assert!(versions(&tokens("E 18-135mm F3.5-5.6 OSS")).is_empty());
    }

    #[test]
    fn a_lens_that_is_not_in_the_database_gets_no_other_lens_profile() {
        let name = |make, model, lens| lensfun(&meta(make, model, lens, 85.0, 4.0)).map(|p| p.name);
        // Same focal length and aperture as another lens is not the same lens: each of these
        // used to get the profile named beside it.
        for (make, model, lens) in [
            ("Canon", "EOS R5", "RF 85mm F1.4L VCM"), // Zeiss Otus 85mm f/1.4
            ("Canon", "EOS R5", "RF70-200mm F2.8 L IS USM Z"), // the older RF 70-200mm F2.8L
            ("Sony", "ILCE-7M3", "FE 100mm F2.8 Macro GM OSS"), // an M42 Orestor 100mm f/2.8
            (
                "Sony",
                "ILCE-7M3",
                "FE 70-200mm F2.8 GM OSS + 1.4X Teleconverter",
            ), // the bare lens
            ("Nikon", "Z 6", "NIKKOR Z 35mm f/1.8 S II"), // (a version that doesn't exist)
        ] {
            assert_eq!(name(make, model, lens), None, "{make} {model} + {lens}");
        }
        // A product code in the database's name isn't part of the lens's: this one used to
        // fall through to the older ZA.
        assert_eq!(
            name("Sony", "ILCE-7M3", "FE 35mm F1.4 GM").as_deref(),
            Some("Sony FE 35mm f/1.4 GM (SEL35F14GM)")
        );
        // Sigma's lines, abbreviated by the camera and spelled out (with a code) by lensfun.
        let sigma = name("Sony", "ILCE-7M3", "85mm F1.4 DG DN | A").expect("the DG DN Art");
        assert!(sigma.contains("DG DN") && sigma.contains("Art"), "{sigma}");
        // Two lenses with one name but for the code: the code decides, either way round.
        let tamron = name("Sony", "ILCE-6600", "E 20mm F2.8 F050").expect("Tamron 20mm");
        assert!(tamron.starts_with("Tamron"), "{tamron}");
        assert_eq!(
            name("Sony", "ILCE-6600", "E 20mm F2.8").as_deref(),
            Some("Sony E 20mm f/2.8")
        );
    }

    #[test]
    fn every_lens_in_the_database_matches_its_own_name() {
        // The matcher may be strict about other lenses, never about the lens itself.
        for l in &db().lenses {
            for m in &l.models {
                assert!(
                    name_score(m, m, &l.maker).is_some() || words(m, &l.maker).core.is_empty(),
                    "{} | {m}",
                    l.maker
                );
            }
        }
    }

    #[test]
    fn vignetting_uses_each_focal_lengths_own_farthest_distance() {
        // 100 mm calibrated out to 80 m, 135 mm with one extra pair at 1000 m.
        let vig = [
            [100.0, 5.0, 1.6, -0.9, 0.0, 0.0],
            [100.0, 5.0, 80.0, -0.5, -0.2, 0.1],
            [135.0, 5.6, 80.0, -0.4, 0.0, 0.0],
            [135.0, 25.0, 1000.0, -0.1, 0.05, 0.01],
        ];
        // At 100 mm: its own row at 80 m, not the 135 mm row that happens to be farthest.
        assert_eq!(vignetting_at(&vig, 100.0, 5.0), Some([-0.5, -0.2, 0.1]));
        assert_eq!(vignetting_at(&vig, 135.0, 25.0), Some([-0.1, 0.05, 0.01]));
        let between = vignetting_at(&vig, 117.5, 5.0).unwrap();
        assert!((between[0] - (-0.3)).abs() < 1e-5, "{between:?}");

        // The lens this was found on: every focal length but 135 mm was being ignored.
        let sigma = db()
            .lenses
            .iter()
            .find(|l| l.models.iter().any(|m| m.contains("100-400mm F5-6.3 DG OS HSM")))
            .expect("Sigma 100-400mm Contemporary");
        let k = vignetting_at(&sigma.vig, 100.0, 5.0).unwrap();
        let corner_gain = 1.0 / (1.0 + k[0] + k[1] + k[2]);
        assert!(corner_gain > 2.0, "100 mm f/5 corners need {corner_gain}×, not 1.07×");
    }

    #[test]
    fn matches_short_micro_four_thirds_names() {
        let p = lensfun(&meta(
            "Olympus",
            "E-M1",
            "OLYMPUS M.12-40mm F2.8",
            25.0,
            2.8,
        ))
        .expect("Olympus 12-40");
        assert_eq!(p.name, "Olympus M.Zuiko Digital ED 12-40mm f/2.8 Pro");
        let p = lensfun(&meta(
            "Panasonic",
            "DC-GX9",
            "LUMIX G VARIO 12-32/F3.5-5.6",
            12.0,
            3.5,
        ))
        .expect("Lumix 12-32");
        assert_eq!(
            p.name,
            "Panasonic Lumix G Vario 12-32mm f/3.5-5.6 Asph. Mega OIS"
        );
    }

    #[test]
    fn parses_sony_and_dng_data() {
        // DSC00084.ARW (E 18-135mm at 42 mm).
        let d = [
            11., 3., 1., -2., -6., -11., -18., -24., -31., -37., -40., -37., 0., 0., 0., 0., 0.,
        ];
        let c = [
            22., 128., 128., 0., 0., 0., 0., 128., 128., 256., 384., 640., 1408., 1152., 1024.,
            896., 896., 896., 768., 640., 512., 256., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0.,
        ];
        let v = [
            11., 0., 96., 384., 816., 1328., 1824., 2320., 2816., 3312., 3840., 4416., 0., 0., 0.,
            0., 0.,
        ];
        let p = sony(&d, &c, &v, "x".into()).unwrap();
        let [_, g, _] = p.distortion.unwrap();
        assert!((sample(&g, 1.0) - (1.0 - 37.0 / 16384.0)).abs() < 2e-4);
        let vig = p.vignetting.unwrap();
        assert!((sample(&vig, 0.05) - 1.0).abs() < 1e-3);
        assert!(
            sample(&vig, 1.0) > 1.15 && sample(&vig, 1.0) < 1.2,
            "{}",
            sample(&vig, 1.0)
        );
        assert!(sony(&d[..5], &c, &v, "x".into()).is_none());
        assert!(max_deviation(&g) < 0.01, "almost no distortion at 42 mm");

        let mut list = Vec::new();
        list.extend(2u32.to_be_bytes());
        // WarpRectilinear, one plane: kr = [1, -0.02, 0, 0], kt = 0, centre 0.5.
        list.extend(
            [1u32, 0x0103_0000, 0, 4 + 48 + 16]
                .iter()
                .flat_map(|v| v.to_be_bytes()),
        );
        list.extend(1u32.to_be_bytes());
        for v in [1.0f64, -0.02, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5] {
            list.extend(v.to_be_bytes());
        }
        // FixVignetteRadial: k0 = 0.5.
        list.extend(
            [3u32, 0x0103_0000, 0, 56]
                .iter()
                .flat_map(|v| v.to_be_bytes()),
        );
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
