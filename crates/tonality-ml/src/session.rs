//! ONNX Runtime sessions, created on first use and kept for the life of the app.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;

use crate::{MlError, Result};

/// A named float tensor going into or coming out of a model.
#[derive(Debug, Clone)]
pub struct Tensor32 {
    pub shape: Vec<i64>,
    pub data: Vec<f32>,
}

impl Tensor32 {
    pub fn new(shape: impl Into<Vec<i64>>, data: Vec<f32>) -> Self {
        Self { shape: shape.into(), data }
    }
}

fn environment() -> Result<&'static Environment> {
    static ENV: OnceLock<Environment> = OnceLock::new();
    if let Some(e) = ENV.get() {
        return Ok(e);
    }
    let env = ort::environment::init().with_name("tonality").build().map_err(ort_err)?;
    Ok(ENV.get_or_init(|| env))
}

fn sessions() -> &'static Mutex<HashMap<PathBuf, Session>> {
    static S: OnceLock<Mutex<HashMap<PathBuf, Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Physical cores, roughly: ONNX Runtime gains nothing from hyper-threads and the UI needs some CPU.
fn threads() -> usize {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (n / 2).clamp(1, 8)
}

fn ort_err(e: impl std::fmt::Display) -> MlError {
    MlError::Inference(e.to_string())
}

/// Run `model` with `inputs` (by name) and return the named `outputs`.
pub fn run(model: &Path, inputs: Vec<(&str, Tensor32)>, outputs: &[&str]) -> Result<Vec<Tensor32>> {
    let mut map = sessions().lock().map_err(|_| MlError::Inference("session lock poisoned".into()))?;
    if !map.contains_key(model) {
        let s = Session::builder(environment()?)
            .map_err(ort_err)?
            .with_intra_threads(threads())
            .map_err(ort_err)?
            .commit_from_file(model)
            .map_err(ort_err)?;
        map.insert(model.to_path_buf(), s);
    }
    let session = map.get_mut(model).expect("inserted above");
    let mut values = Vec::with_capacity(inputs.len());
    for (name, t) in inputs {
        let v = Tensor::from_array((t.shape, t.data)).map_err(ort_err)?;
        values.push((std::borrow::Cow::Borrowed(name), ort::session::SessionInputValue::from(v)));
    }
    let out = session.run(values).map_err(ort_err)?;
    outputs
        .iter()
        .map(|name| {
            let v = out.get(*name).ok_or_else(|| MlError::Inference(format!("model has no output `{name}`")))?;
            let (shape, data) = v.try_extract_tensor::<f32>().map_err(ort_err)?;
            Ok(Tensor32 { shape: shape.to_vec(), data: data.to_vec() })
        })
        .collect()
}

/// Input and output names of a model (for diagnostics and the benchmark).
pub fn describe(model: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let s = Session::builder(environment()?).map_err(ort_err)?.commit_from_file(model).map_err(ort_err)?;
    Ok((
        s.inputs().iter().map(|o| format!("{} {:?}", o.name(), o.dtype())).collect(),
        s.outputs().iter().map(|o| format!("{} {:?}", o.name(), o.dtype())).collect(),
    ))
}
