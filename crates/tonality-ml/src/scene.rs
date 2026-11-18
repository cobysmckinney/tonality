//! Scene segmentation (ADE20K classes): one pass gives sky, people and more.

use std::path::Path;

use image::RgbImage;

use crate::input::{to_nchw, Normalize};
use crate::session::{run, Tensor32};
use crate::{MlError, Result};

/// ADE20K class indices (0-based, 150 classes).
pub mod ade {
    pub const SKY: usize = 2;
    pub const PERSON: usize = 12;
}

#[derive(Debug, Clone, Copy)]
pub struct SceneModel {
    pub input: u32,
    pub input_name: &'static str,
    pub output_name: &'static str,
}

impl SceneModel {
    /// SegFormer and UperNet exports both take 512² ImageNet-normalised input → `logits`.
    pub const ADE_512: SceneModel = SceneModel {
        input: 512,
        input_name: "pixel_values",
        output_name: "logits",
    };
}

/// Per-class probability maps for `classes` (softmax over all classes), at the logits'
/// resolution. Returns `(maps, w, h)` with `maps[k]` the map of `classes[k]`.
pub fn detect(
    model: &Path,
    m: SceneModel,
    img: &RgbImage,
    classes: &[usize],
) -> Result<(Vec<Vec<f32>>, u32, u32)> {
    let n = m.input;
    let x = Tensor32::new(
        [1, 3, n as i64, n as i64],
        to_nchw(img, n, n, Normalize::IMAGENET),
    );
    let out = run(model, vec![(m.input_name, x)], &[m.output_name])?.remove(0);
    let [_, c, h, w] = out.shape[..] else {
        return Err(MlError::Inference(format!(
            "unexpected logits shape {:?}",
            out.shape
        )));
    };
    let (c, h, w) = (c as usize, h as usize, w as usize);
    let plane = h * w;
    let mut maps = vec![vec![0.0f32; plane]; classes.len()];
    #[allow(clippy::needless_range_loop)] // indexes the logits planes and every map
    for i in 0..plane {
        let max = (0..c)
            .map(|k| out.data[k * plane + i])
            .fold(f32::MIN, f32::max);
        let sum: f32 = (0..c).map(|k| (out.data[k * plane + i] - max).exp()).sum();
        for (j, &k) in classes.iter().enumerate() {
            maps[j][i] = (out.data[k * plane + i] - max).exp() / sum;
        }
    }
    Ok((maps, w as u32, h as u32))
}
