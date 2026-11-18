//! Raster mask layers (brush and AI), cached so that only masks that changed are rebuilt.
//!
//! Building a layer means decoding the AI model's PNG, warping it for lens correction and
//! painting every stroke. Most requests change one mask or none (a slider drag, a thumbnail of
//! the same state), so finished layers are kept by content. A layer that only gained strokes
//! (painting) starts from the cached one and paints the new strokes on top.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tonality_core::lens::{warp_to_uv, LensTable};
use tonality_core::mask::{mask_layers, mask_size, BrushLayers, Mask, MaskShape, Stroke};

/// Decoded model rasters kept (≈ 0.7 MB each).
const RASTERS: usize = 16;
/// Finished layers kept (≈ 0.7 MB each): enough for a few photos' masks plus undo steps.
const LAYERS: usize = 64;

/// A small least-recently-used map (linear scan: it only ever holds a few dozen entries).
struct Lru<K, V> {
    cap: usize,
    items: Vec<(K, V)>,
}

impl<K: PartialEq, V: Clone> Lru<K, V> {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            items: Vec::new(),
        }
    }

    fn get(&mut self, k: &K) -> Option<V> {
        let i = self.items.iter().position(|(key, _)| key == k)?;
        let item = self.items.remove(i);
        let v = item.1.clone();
        self.items.push(item);
        Some(v)
    }

    fn put(&mut self, k: K, v: V) {
        self.items.retain(|(key, _)| *key != k);
        self.items.push((k, v));
        if self.items.len() > self.cap {
            self.items.remove(0);
        }
    }
}

/// What a finished layer was built from, apart from its strokes.
#[derive(Clone, PartialEq)]
struct Base {
    /// Photo contents hash and raster size.
    photo: String,
    size: (u32, u32),
    /// `None` for brush masks; the model raster's path and the lens geometry for AI masks.
    ai: Option<(PathBuf, Vec<u32>)>,
}

#[derive(Clone)]
struct Layer {
    base: Base,
    strokes: Arc<Vec<Stroke>>,
    data: Arc<Vec<u8>>,
}

#[derive(Default)]
pub struct LayerCache {
    inner: Mutex<Option<Inner>>,
}

struct Inner {
    rasters: Lru<PathBuf, Arc<Vec<u8>>>,
    layers: Vec<Layer>,
    /// Bumped by `forget`: work loaded or built before then may hold the old raster, so it isn't
    /// stored (it's still returned, once).
    generation: u64,
}

impl Inner {
    fn new() -> Self {
        Self {
            rasters: Lru::new(RASTERS),
            layers: Vec::new(),
            generation: 0,
        }
    }

    fn exact(&mut self, base: &Base, strokes: &[Stroke]) -> Option<Arc<Vec<u8>>> {
        let i = self
            .layers
            .iter()
            .position(|l| l.base == *base && *l.strokes == strokes)?;
        let l = self.layers.remove(i);
        let data = l.data.clone();
        self.layers.push(l);
        Some(data)
    }

    /// The cached layer with the longest stroke list that `strokes` extends.
    fn prefix(&self, base: &Base, strokes: &[Stroke]) -> Option<(usize, Arc<Vec<u8>>)> {
        self.layers
            .iter()
            .filter(|l| {
                l.base == *base
                    && l.strokes.len() < strokes.len()
                    && strokes.starts_with(&l.strokes)
            })
            .max_by_key(|l| l.strokes.len())
            .map(|l| (l.strokes.len(), l.data.clone()))
    }

    fn put(&mut self, layer: Layer) {
        self.layers
            .retain(|l| !(l.base == layer.base && l.strokes == layer.strokes));
        self.layers.push(layer);
        if self.layers.len() > LAYERS {
            self.layers.remove(0);
        }
    }
}

/// The lens geometry an AI raster is warped by (just the green distortion; vignetting and the
/// like don't move pixels), or empty when it doesn't move anything.
fn geometry(table: &LensTable) -> Vec<u32> {
    if !table.active || table.knots.iter().all(|k| k[1] == 1.0) {
        return Vec::new();
    }
    table.knots.iter().map(|k| k[1].to_bits()).collect()
}

impl LayerCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Inner>> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.get_or_insert_with(Inner::new);
        g
    }

    /// Forget a model raster (it was just regenerated).
    pub fn forget(&self, path: &Path) {
        let mut g = self.lock();
        let inner = g.as_mut().unwrap();
        inner.generation += 1;
        inner.rasters.items.retain(|(p, _)| p != path);
        inner
            .layers
            .retain(|l| l.base.ai.as_ref().is_none_or(|(p, _)| p != path));
    }

    /// Every raster layer of `masks` for a photo of `(w, h)` (contents `hash`). `raster(mask)` names
    /// an AI mask's model raster file; `load(path)` reads it (source space, mask size).
    pub fn layers(
        &self,
        hash: &str,
        masks: &[Mask],
        (w, h): (u32, u32),
        table: &LensTable,
        raster: impl Fn(&Mask) -> Option<PathBuf>,
        load: impl Fn(&Path) -> Option<Vec<u8>>,
    ) -> BrushLayers {
        let size = mask_size(w, h);
        let geo = geometry(table);
        let mut out = BrushLayers {
            width: size.0,
            height: size.1,
            layers: 0,
            data: Vec::new(),
            missing: Vec::new(),
        };
        for (i, m) in masks.iter().enumerate() {
            let strokes = match &m.shape {
                MaskShape::Brush { strokes } => strokes,
                MaskShape::Ai { refine, .. } => refine,
                _ => continue,
            };
            let path = raster(m);
            let base = Base {
                photo: hash.to_string(),
                size,
                ai: path.clone().map(|p| (p, geo.clone())),
            };
            out.layers += 1;

            let (exact, start, generation) = {
                let mut g = self.lock();
                let inner = g.as_mut().unwrap();
                let exact = inner.exact(&base, strokes);
                // Otherwise start from a layer these strokes extend, or from scratch.
                let start = if exact.is_none() {
                    inner.prefix(&base, strokes)
                } else {
                    None
                };
                (exact, start, inner.generation)
            };
            if let Some(data) = exact {
                out.data.extend_from_slice(&data);
                continue;
            }
            let (data, found) = match start {
                Some((n, data)) => {
                    let mut layer = data.to_vec();
                    for s in &strokes[n..] {
                        tonality_core::mask::paint(&mut layer, size.0, size.1, s);
                    }
                    (layer, true)
                }
                None => {
                    let src = path.as_deref().and_then(|p| self.raster(p, &load));
                    let found = path.is_none() || src.is_some();
                    let built = mask_layers(std::slice::from_ref(m), w, h, |_, _| {
                        let src = src.as_ref()?;
                        Some(if geo.is_empty() {
                            src.to_vec()
                        } else {
                            warp_to_uv(src, size.0, size.1, table, w, h)
                        })
                    });
                    (built.data, found)
                }
            };
            if found {
                let mut g = self.lock();
                let inner = g.as_mut().unwrap();
                if inner.generation == generation {
                    inner.put(Layer {
                        base,
                        strokes: Arc::new(strokes.clone()),
                        data: Arc::new(data.clone()),
                    });
                }
            } else {
                // Not cached: it changes once the raster is detected.
                out.missing.push(i);
            }
            out.data.extend_from_slice(&data);
        }
        out
    }

    fn raster(&self, path: &Path, load: impl Fn(&Path) -> Option<Vec<u8>>) -> Option<Arc<Vec<u8>>> {
        let key = path.to_path_buf();
        let generation = {
            let mut g = self.lock();
            let inner = g.as_mut().unwrap();
            if let Some(r) = inner.rasters.get(&key) {
                return Some(r);
            }
            inner.generation
        };
        let r = Arc::new(load(path)?);
        let mut g = self.lock();
        let inner = g.as_mut().unwrap();
        if inner.generation == generation {
            inner.rasters.put(key, r.clone());
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use tonality_core::mask::AiTarget;

    fn mask(shape: MaskShape) -> Mask {
        Mask {
            shape,
            ..Mask::default()
        }
    }

    fn stroke(x: f32) -> Stroke {
        Stroke {
            points: vec![[x, 0.5]],
            radius: 0.05,
            ..Stroke::default()
        }
    }

    fn ai(refine: Vec<Stroke>) -> Mask {
        mask(MaskShape::Ai {
            target: AiTarget::Subject,
            model: "subject@1".into(),
            refine,
        })
    }

    #[test]
    fn layers_match_a_fresh_build_and_reuse_work() {
        let (w, h) = (400, 300);
        let (lw, lh) = mask_size(w, h);
        let src: Vec<u8> = (0..lw * lh).map(|i| (i % 251) as u8).collect();
        let loads = Cell::new(0);
        let load = |_: &Path| {
            loads.set(loads.get() + 1);
            Some(src.clone())
        };
        let raster = |m: &Mask| {
            matches!(m.shape, MaskShape::Ai { .. }).then(|| PathBuf::from("subject.png"))
        };
        let table = LensTable::identity();
        let cache = LayerCache::default();
        let fresh = |masks: &[Mask]| {
            mask_layers(masks, w, h, |_, m| {
                matches!(m.shape, MaskShape::Ai { .. }).then(|| src.clone())
            })
        };

        let mut masks = vec![
            mask(MaskShape::Brush {
                strokes: vec![stroke(0.2)],
            }),
            ai(vec![stroke(0.7)]),
        ];
        let a = cache.layers("h", &masks, (w, h), &table, raster, load);
        assert_eq!(a.data, fresh(&masks).data);
        assert_eq!((a.layers, a.missing.len(), loads.get()), (2, 0, 1));

        // More strokes: painted on top of the cached layer, same result as from scratch.
        if let MaskShape::Brush { strokes } = &mut masks[0].shape {
            strokes.push(stroke(0.4));
        }
        if let MaskShape::Ai { refine, .. } = &mut masks[1].shape {
            refine.push(stroke(0.3));
        }
        let b = cache.layers("h", &masks, (w, h), &table, raster, load);
        assert_eq!(b.data, fresh(&masks).data);
        assert_eq!(loads.get(), 1, "the raster is decoded once");

        // A different photo doesn't share layers.
        let c = cache.layers(
            "other",
            &masks,
            (w, h),
            &table,
            |m| matches!(m.shape, MaskShape::Ai { .. }).then(|| PathBuf::from("other.png")),
            |_| None,
        );
        assert_eq!(c.missing, vec![1]);
    }

    #[test]
    fn missing_rasters_are_not_cached() {
        let (w, h) = (200, 100);
        let cache = LayerCache::default();
        let masks = vec![ai(vec![])];
        let raster = |_: &Mask| Some(PathBuf::from("sky.png"));
        let table = LensTable::identity();
        assert_eq!(
            cache
                .layers("h", &masks, (w, h), &table, raster, |_| None)
                .missing,
            vec![0]
        );
        let (lw, lh) = mask_size(w, h);
        let l = cache.layers("h", &masks, (w, h), &table, raster, |_| {
            Some(vec![200; (lw * lh) as usize])
        });
        assert!(l.missing.is_empty() && l.data.iter().all(|&v| v == 200));

        cache.forget(Path::new("sky.png"));
        let l = cache.layers("h", &masks, (w, h), &table, raster, |_| {
            Some(vec![9; (lw * lh) as usize])
        });
        assert!(
            l.data.iter().all(|&v| v == 9),
            "a regenerated raster replaces the old one"
        );

        // Regenerated while being loaded: the stale load is returned but not kept.
        let stale = |_: &Path| {
            cache.forget(Path::new("sky.png"));
            Some(vec![1; (lw * lh) as usize])
        };
        cache.forget(Path::new("sky.png"));
        assert!(cache
            .layers("h", &masks, (w, h), &table, raster, stale)
            .data
            .iter()
            .all(|&v| v == 1));
        let l = cache.layers("h", &masks, (w, h), &table, raster, |_| {
            Some(vec![7; (lw * lh) as usize])
        });
        assert!(
            l.data.iter().all(|&v| v == 7),
            "the stale load wasn't cached"
        );
    }
}
