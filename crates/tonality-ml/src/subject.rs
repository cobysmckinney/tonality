//! Salient-subject models (BiRefNet, ISNet, U²-Net family): one probability map of "the subject".

use std::path::Path;

use image::RgbImage;

use crate::input::{to_nchw, Normalize};
use crate::session::{run, Tensor32};
use crate::Result;

/// How a particular subject model is fed and read.
#[derive(Debug, Clone, Copy)]
pub struct SubjectModel {
    /// Square input edge (the photo is squashed to it; the map is stretched back).
    pub input: u32,
    pub norm: Normalize,
    pub input_name: &'static str,
    pub output_name: &'static str,
    /// Output is logits (apply a sigmoid) rather than probabilities.
    pub logits: bool,
}

impl SubjectModel {
    pub const BIREFNET_LITE: SubjectModel = SubjectModel {
        input: 1024,
        norm: Normalize::IMAGENET,
        input_name: "input_image",
        output_name: "output_image",
        logits: true,
    };
    pub const ISNET: SubjectModel = SubjectModel {
        input: 1024,
        norm: Normalize { mean: [0.5; 3], std: [1.0; 3] },
        input_name: "input_image",
        output_name: "1890",
        logits: false,
    };
    /// U²-Net and its pruned variants (silueta, u2netp).
    pub const U2NET: SubjectModel = SubjectModel {
        input: 320,
        norm: Normalize::IMAGENET,
        input_name: "input.1",
        output_name: "1959",
        logits: false,
    };
}

/// The subject probability map, `input × input`, 0..1.
pub fn detect(model: &Path, m: SubjectModel, img: &RgbImage) -> Result<(Vec<f32>, u32, u32)> {
    let n = m.input;
    let x = Tensor32::new([1, 3, n as i64, n as i64], to_nchw(img, n, n, m.norm));
    let out = run(model, vec![(m.input_name, x)], &[m.output_name])?.remove(0);
    let mut p = out.data;
    if m.logits {
        p.iter_mut().for_each(|v| *v = 1.0 / (1.0 + (-*v).exp()));
    }
    // Stretch to the full range (these models are often shy of 0 and 1).
    let (lo, hi) = p.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    if hi - lo > 1e-3 {
        p.iter_mut().for_each(|v| *v = (*v - lo) / (hi - lo));
    }
    Ok((p, n, n))
}
