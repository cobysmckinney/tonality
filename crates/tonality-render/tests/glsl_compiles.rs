//! The Edit preview runs the develop shader as GLSL ES 3.00, transpiled from the WGSL by naga.
//! Nothing on the Rust side ever compiled that GLSL: the first compiler to see it was the
//! WebView's, on a user's machine. Here the reference compiler (glslang) reads both stages.
//!
//! Skipped (passes with a note) where `glslangValidator` isn't installed, unless
//! `TONALITY_REQUIRE_GLSLANG` is set, as CI does.

use std::process::Command;

use tonality_render::preview_glsl;

const TOOL: &str = "glslangValidator";

#[test]
fn the_preview_shaders_compile_as_glsl_es() {
    if Command::new(TOOL).arg("--version").output().is_err() {
        if std::env::var_os("TONALITY_REQUIRE_GLSLANG").is_some() {
            panic!("TONALITY_REQUIRE_GLSLANG is set, but {TOOL} isn't installed");
        }
        eprintln!("skipped: {TOOL} isn't installed");
        return;
    }
    let program = preview_glsl().expect("the develop shader transpiles");
    let dir = std::env::temp_dir().join(format!("tonality-glsl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (stage, source) in [("vert", &program.vertex), ("frag", &program.fragment)] {
        assert!(
            source.starts_with("#version 300 es"),
            "{stage}: not GLSL ES 3.00"
        );
        // The stage comes from the extension. Without -V or -G glslang checks the source
        // against the GLSL ES specification its #version names.
        let file = dir.join(format!("develop.{stage}"));
        std::fs::write(&file, source).unwrap();
        let out = Command::new(TOOL).arg(&file).output().unwrap();
        assert!(
            out.status.success(),
            "the {stage} shader doesn't compile:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // And the names the WebGL side binds by are in the source it will compile.
    for name in [
        &program.image_sampler,
        &program.curve_sampler,
        &program.brush_sampler,
        &program.presence_sampler,
        &program.params_block,
    ] {
        assert!(program.fragment.contains(name.as_str()), "{name}");
    }
    assert!(program.vertex.contains(program.view_block.as_str()));
    std::fs::remove_dir_all(&dir).unwrap();
}
