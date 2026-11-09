use naga::back::glsl;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use serde::Serialize;
use ts_rs::TS;

use crate::DEVELOP_WGSL;

/// GLSL ES 3.00 sources plus the names the WebGL side must bind to.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GlslProgram {
    pub vertex: String,
    pub fragment: String,
    /// Uniform block name for `params` in the fragment stage.
    pub params_block: String,
    /// Uniform block name for `view` in the vertex stage.
    pub view_block: String,
    /// Combined sampler uniform name for `image` + `image_sampler`.
    pub image_sampler: String,
    /// Combined sampler uniform name for `curve` + `curve_sampler`.
    pub curve_sampler: String,
    /// Combined sampler uniform name for `brush` (a 2D array) + `image_sampler`.
    pub brush_sampler: String,
}

pub fn preview_glsl() -> Result<GlslProgram, String> {
    let module = naga::front::wgsl::parse_str(DEVELOP_WGSL).map_err(|e| e.emit_to_string(DEVELOP_WGSL))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| format!("{e:?}"))?;

    let options = glsl::Options {
        version: glsl::Version::Embedded { version: 300, is_webgl: true },
        writer_flags: glsl::WriterFlags::empty(),
        ..Default::default()
    };

    let find_global = |name: &str| {
        module
            .global_variables
            .iter()
            .find(|(_, g)| g.name.as_deref() == Some(name))
            .map(|(h, _)| h)
            .ok_or_else(|| format!("global `{name}` missing from shader"))
    };
    let params_h = find_global("params")?;
    let view_h = find_global("view")?;

    let write = |stage, entry: &str| -> Result<(String, glsl::ReflectionInfo), String> {
        let mut out = String::new();
        let pipeline = glsl::PipelineOptions { shader_stage: stage, entry_point: entry.into(), multiview: None };
        let mut w = glsl::Writer::new(&mut out, &module, &info, &options, &pipeline, Default::default())
            .map_err(|e| format!("{e:?}"))?;
        let refl = w.write().map_err(|e| format!("{e:?}"))?;
        Ok((out, refl))
    };

    let (vertex, vrefl) = write(naga::ShaderStage::Vertex, "vs_main")?;
    let (fragment, frefl) = write(naga::ShaderStage::Fragment, "fs_main")?;

    let params_block = frefl.uniforms.get(&params_h).cloned().ok_or("params not used in fragment")?;
    let view_block = vrefl.uniforms.get(&view_h).cloned().ok_or("view not used in vertex")?;
    let sampler_for = |texture: &str| -> Result<String, String> {
        let h = find_global(texture)?;
        frefl
            .texture_mapping
            .iter()
            .find(|(_, m)| m.texture == h)
            .map(|(name, _)| name.clone())
            .ok_or_else(|| format!("no sampler for `{texture}` in fragment"))
    };
    let image_sampler = sampler_for("image")?;
    let curve_sampler = sampler_for("curve")?;
    let brush_sampler = sampler_for("brush")?;

    Ok(GlslProgram { vertex, fragment, params_block, view_block, image_sampler, curve_sampler, brush_sampler })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpiles_to_webgl2() {
        let p = preview_glsl().expect("transpile");
        assert!(p.vertex.starts_with("#version 300 es"));
        assert!(p.fragment.contains(&p.image_sampler));
        assert!(p.fragment.contains(&p.curve_sampler));
        assert_ne!(p.image_sampler, p.curve_sampler);
        assert!(p.fragment.contains(&p.brush_sampler));
        assert!(p.fragment.contains("sampler2DArray"));
        assert!(p.fragment.contains(&p.params_block));
        assert!(p.vertex.contains(&p.view_block));
        println!("{}\n----\n{}\n{p:?}", p.vertex, p.fragment.lines().take(40).collect::<Vec<_>>().join("\n"));
    }
}
