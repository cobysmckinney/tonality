//! The DNG `GainMap` opcode (ID 9) from `OpcodeList2`: a grid of gains, bilinearly
//! interpolated, that multiplies the linearised raw data before demosaicing. Phone DNGs
//! (iPhone, Pixel) use it for vignetting and colour shading, typically as four maps, one per
//! Bayer phase (`RowPitch` = `ColPitch` = 2, `Top`/`Left` = 0 or 1).
//!
//! Follows the DNG 1.4 specification and the DNG SDK (`dng_gain_map.cpp`). OpcodeList2 works
//! on the stage-2 image, which is the active area, so rectangles and map coordinates are
//! relative to the active area's origin and size.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use rawler::formats::tiff::reader::TiffReader;
use rawler::formats::tiff::GenericTiffReader;
use rayon::prelude::*;

const SUB_IFDS: u16 = 0x014a;
const OPCODE_LIST_2: u16 = 0xc741;
const GAIN_MAP: u32 = 9;

/// One `GainMap` opcode.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GainMap {
    /// Affected rectangle `[top, left, bottom, right)` in stage-2 pixels.
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
    /// First plane, number of planes.
    pub plane: u32,
    pub planes: u32,
    /// Only every `row_pitch`-th row / `col_pitch`-th column (from top / left) is affected.
    pub row_pitch: u32,
    pub col_pitch: u32,
    /// Map size in points (vertical, horizontal).
    pub points_v: u32,
    pub points_h: u32,
    /// Distance between map points and position of point (0, 0), as fractions of the image's
    /// height / width.
    pub spacing_v: f64,
    pub spacing_h: f64,
    pub origin_v: f64,
    pub origin_h: f64,
    pub map_planes: u32,
    /// `gains[(v * points_h + h) * map_planes + p]`.
    pub gains: Vec<f32>,
}

/// The `GainMap` opcodes in the `OpcodeList2` of the RAW at `path` (empty for non-DNGs).
pub(crate) fn read(path: &Path) -> Vec<GainMap> {
    let read = || -> Option<Vec<GainMap>> {
        let mut f = BufReader::new(File::open(path).ok()?);
        let tiff = GenericTiffReader::new(&mut f, 0, 0, None, &[SUB_IFDS]).ok()?;
        let e = tiff
            .find_first_ifd_with_tag(OPCODE_LIST_2)?
            .get_entry(OPCODE_LIST_2)?;
        Some(parse(e.value.get_data()))
    };
    read().unwrap_or_default()
}

/// Parse the `GainMap`s out of a DNG opcode list (big-endian: count, then per opcode its ID,
/// DNG version, flags, parameter size and parameters). Other opcodes are skipped; a truncated
/// or inconsistent `GainMap` is dropped.
pub(crate) fn parse(d: &[u8]) -> Vec<GainMap> {
    let u32_at = |o: usize| {
        d.get(o..o.checked_add(4)?)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    };
    let mut out = Vec::new();
    let Some(n) = u32_at(0) else { return out };
    let mut o = 4usize;
    for _ in 0..n.min(256) {
        let (Some(id), Some(size)) = (u32_at(o), o.checked_add(12).and_then(u32_at)) else {
            break;
        };
        let p = o + 16;
        let Some(params) = p.checked_add(size as usize).and_then(|end| d.get(p..end)) else {
            break;
        };
        if id == GAIN_MAP {
            if let Some(m) = parse_gain_map(params) {
                out.push(m);
            }
        }
        o = p + size as usize;
    }
    out
}

fn parse_gain_map(d: &[u8]) -> Option<GainMap> {
    let u32_at = |o: usize| Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?));
    let f64_at = |o: usize| Some(f64::from_be_bytes(d.get(o..o + 8)?.try_into().ok()?));
    let f32_at = |o: usize| Some(f32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?));
    let m = GainMap {
        top: u32_at(0)?,
        left: u32_at(4)?,
        bottom: u32_at(8)?,
        right: u32_at(12)?,
        plane: u32_at(16)?,
        planes: u32_at(20)?,
        row_pitch: u32_at(24)?,
        col_pitch: u32_at(28)?,
        points_v: u32_at(32)?,
        points_h: u32_at(36)?,
        spacing_v: f64_at(40)?,
        spacing_h: f64_at(48)?,
        origin_v: f64_at(56)?,
        origin_h: f64_at(64)?,
        map_planes: u32_at(72)?,
        gains: Vec::new(),
    };
    let count = (m.points_v as usize)
        .checked_mul(m.points_h as usize)?
        .checked_mul(m.map_planes as usize)?;
    let valid = m.bottom > m.top
        && m.right > m.left
        && m.planes > 0
        && m.row_pitch > 0
        && m.col_pitch > 0
        && m.points_v > 0
        && m.points_h > 0
        && m.map_planes > 0
        && (m.points_v == 1 || m.spacing_v > 0.0)
        && (m.points_h == 1 || m.spacing_h > 0.0)
        && m.origin_v.is_finite()
        && m.origin_h.is_finite()
        && count <= (d.len().saturating_sub(76)) / 4;
    if !valid {
        return None;
    }
    let gains: Vec<f32> = (0..count)
        .map(|i| f32_at(76 + i * 4))
        .collect::<Option<_>>()?;
    if gains.iter().any(|g| !g.is_finite()) {
        return None;
    }
    Some(GainMap { gains, ..m })
}

/// Index and fraction of a map coordinate for the pixel at `pos` of an image `len` pixels long,
/// clamped to the map (the map's edge values extend beyond it).
fn map_coord(pos: usize, len: usize, origin: f64, spacing: f64, points: u32) -> (usize, f32) {
    if points <= 1 {
        return (0, 0.0);
    }
    let f = ((pos as f64 + 0.5) / len as f64 - origin) / spacing;
    let f = f.clamp(0.0, (points - 1) as f64);
    let i = (f.floor() as usize).min(points as usize - 2);
    (i, (f - i as f64) as f32)
}

impl GainMap {
    /// Multiply `data` by the map. `data` is a `width × height` image with `cpp` interleaved
    /// planes, whose stage-2 image (the active area) starts at `(ax, ay)` and is `aw × ah`.
    pub(crate) fn apply(
        &self,
        data: &mut [f32],
        width: usize,
        cpp: usize,
        (ax, ay, aw, ah): (usize, usize, usize, usize),
    ) {
        if aw == 0 || ah == 0 || cpp == 0 {
            return;
        }
        let top = self.top as usize;
        let left = self.left as usize;
        let bottom = (self.bottom as usize).min(ah);
        let right = (self.right as usize).min(aw);
        let plane0 = self.plane as usize;
        let plane1 = (plane0 + self.planes as usize).min(cpp);
        if top >= bottom || left >= right || plane0 >= plane1 {
            return;
        }
        let (ph, mp) = (self.points_h as usize, self.map_planes as usize);
        let cols: Vec<(usize, usize, f32)> = (left..right)
            .step_by(self.col_pitch as usize)
            .map(|x| {
                let (i, t) = map_coord(x, aw, self.origin_h, self.spacing_h, self.points_h);
                (x, i, t)
            })
            .collect();
        let stride = width * cpp;
        data.par_chunks_mut(stride)
            .enumerate()
            .skip(ay + top)
            .take(bottom - top)
            .filter(|(y, _)| (y - ay - top).is_multiple_of(self.row_pitch as usize))
            .for_each(|(y, row)| {
                let (j, u) = map_coord(y - ay, ah, self.origin_v, self.spacing_v, self.points_v);
                let j1 = (j + 1).min(self.points_v as usize - 1);
                for &(x, i, t) in &cols {
                    let i1 = (i + 1).min(ph - 1);
                    for p in plane0..plane1 {
                        let q = (p - plane0).min(mp - 1);
                        let g = |v: usize, h: usize| self.gains[(v * ph + h) * mp + q];
                        let top = g(j, i) + t * (g(j, i1) - g(j, i));
                        let bot = g(j1, i) + t * (g(j1, i1) - g(j1, i));
                        let gain = top + u * (bot - top);
                        if let Some(v) = row.get_mut((ax + x) * cpp + p) {
                            *v *= gain;
                        }
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `GainMap` opcode's parameters, big-endian.
    fn encode(m: &GainMap) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [
            m.top,
            m.left,
            m.bottom,
            m.right,
            m.plane,
            m.planes,
            m.row_pitch,
            m.col_pitch,
            m.points_v,
            m.points_h,
        ] {
            b.extend(v.to_be_bytes());
        }
        for v in [m.spacing_v, m.spacing_h, m.origin_v, m.origin_h] {
            b.extend(v.to_be_bytes());
        }
        b.extend(m.map_planes.to_be_bytes());
        for g in &m.gains {
            b.extend(g.to_be_bytes());
        }
        b
    }

    /// An opcode list holding `(id, params)` opcodes.
    fn list(ops: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut b = (ops.len() as u32).to_be_bytes().to_vec();
        for (id, p) in ops {
            b.extend(id.to_be_bytes());
            b.extend(0x01030000u32.to_be_bytes()); // DNG version 1.3
            b.extend(0u32.to_be_bytes()); // flags
            b.extend((p.len() as u32).to_be_bytes());
            b.extend(p);
        }
        b
    }

    /// A 2×2 map over the whole image: gains 1 (top left), 2 (top right), 3, 4.
    fn corners(top: u32, left: u32, pitch: u32) -> GainMap {
        GainMap {
            top,
            left,
            bottom: 1000,
            right: 1000,
            plane: 0,
            planes: 1,
            row_pitch: pitch,
            col_pitch: pitch,
            points_v: 2,
            points_h: 2,
            spacing_v: 1.0,
            spacing_h: 1.0,
            origin_v: 0.0,
            origin_h: 0.0,
            map_planes: 1,
            gains: vec![1.0, 2.0, 3.0, 4.0],
        }
    }

    #[test]
    fn parses_gain_maps_and_skips_other_opcodes() {
        let a = corners(0, 0, 2);
        let b = corners(1, 1, 2);
        let blob = list(&[
            (3, vec![0u8; 40]), // FixVignetteRadial
            (GAIN_MAP, encode(&a)),
            (GAIN_MAP, encode(&b)),
        ]);
        assert_eq!(parse(&blob), vec![a, b]);
    }

    #[test]
    fn drops_truncated_and_inconsistent_maps() {
        let m = corners(0, 0, 1);
        let mut short = encode(&m);
        short.truncate(short.len() - 4);
        assert!(parse(&list(&[(GAIN_MAP, short)])).is_empty());
        let zero_pitch = GainMap {
            row_pitch: 0,
            ..m.clone()
        };
        assert!(parse(&list(&[(GAIN_MAP, encode(&zero_pitch))])).is_empty());
        // A size running past the end of the list stops parsing instead of panicking.
        let mut blob = list(&[(GAIN_MAP, encode(&m))]);
        blob[16] = 0xff; // parameter size, high byte
        assert!(parse(&blob).is_empty());
        assert!(parse(&[]).is_empty());
    }

    #[test]
    fn interpolates_bilinearly_and_clamps_outside_the_map() {
        // 4×4 image, map points at pixel centres of columns/rows 0.5 and 2.5 (fractions 1/8
        // and 5/8): spacing 0.5, origin 0.125.
        let m = GainMap {
            spacing_v: 0.5,
            spacing_h: 0.5,
            origin_v: 0.125,
            origin_h: 0.125,
            ..corners(0, 0, 1)
        };
        let mut px = vec![1.0f32; 16];
        m.apply(&mut px, 4, 1, (0, 0, 4, 4));
        let at = |x: usize, y: usize| px[y * 4 + x];
        // Pixel (0,0) is at fraction 1/8 = the map's first point.
        assert!((at(0, 0) - 1.0).abs() < 1e-6);
        // Pixel 2 is at 5/8 = the second point.
        assert!((at(2, 0) - 2.0).abs() < 1e-6);
        assert!((at(0, 2) - 3.0).abs() < 1e-6);
        assert!((at(2, 2) - 4.0).abs() < 1e-6);
        // Halfway: pixel 1 is at 3/8, midway between the points.
        assert!((at(1, 1) - 2.5).abs() < 1e-6);
        // Beyond the last point the edge value holds.
        assert!((at(3, 3) - 4.0).abs() < 1e-6);
    }

    #[test]
    fn applies_per_bayer_phase_inside_the_active_area() {
        // 6×6 sensor, active area (1,1) 4×4. Map: constant 2 on the (odd row, odd column)
        // phase of the active area only.
        let m = GainMap {
            points_v: 1,
            points_h: 1,
            gains: vec![2.0],
            ..corners(1, 1, 2)
        };
        let mut px = vec![1.0f32; 36];
        m.apply(&mut px, 6, 1, (1, 1, 4, 4));
        for y in 0..6 {
            for x in 0..6 {
                let inside = (1..5).contains(&x) && (1..5).contains(&y);
                let phase = inside && (x - 1) % 2 == 1 && (y - 1) % 2 == 1;
                let want = if phase { 2.0 } else { 1.0 };
                assert_eq!(px[y * 6 + x], want, "({x}, {y})");
            }
        }
    }

    #[test]
    fn applies_per_plane_for_linear_raw() {
        // 2×1 RGB image; map covers planes 1..3 with 2 map planes (gains 2 and 3).
        let m = GainMap {
            plane: 1,
            planes: 2,
            points_v: 1,
            points_h: 1,
            map_planes: 2,
            gains: vec![2.0, 3.0],
            ..corners(0, 0, 1)
        };
        let mut px = vec![1.0f32; 6];
        m.apply(&mut px, 2, 3, (0, 0, 2, 1));
        assert_eq!(px, vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
        // A single map plane is shared by every plane.
        let m = GainMap {
            map_planes: 1,
            gains: vec![5.0],
            ..m
        };
        let mut px = vec![1.0f32; 3];
        m.apply(&mut px, 1, 3, (0, 0, 1, 1));
        assert_eq!(px, vec![1.0, 5.0, 5.0]);
    }
}
