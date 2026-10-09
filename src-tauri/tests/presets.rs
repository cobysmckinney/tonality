//! Presets: what they cover, the built-in ones, and keeping your own.

use std::collections::BTreeSet;

use serde_json::json;
use tempfile::TempDir;
use tonality_lib::edit::{Adjustments, Crop};
use tonality_lib::film::{Film, Kind};
use tonality_lib::library::Library;
use tonality_lib::presets::{self, Preset, Settings};

fn library() -> (TempDir, Library) {
    let dir = TempDir::new().unwrap();
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    (dir, library)
}

fn own(library: &Library) -> Vec<Preset> {
    library.presets().unwrap().into_iter().filter(|preset| preset.group.is_none()).collect()
}

fn names(presets: &[Preset]) -> Vec<&str> {
    presets.iter().map(|preset| preset.name.as_str()).collect()
}

fn covered(preset: &Preset) -> BTreeSet<String> {
    serde_json::to_value(&preset.settings).unwrap().as_object().unwrap().keys().cloned().collect()
}

fn settings(value: serde_json::Value) -> Settings {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_preset_changes_only_what_it_covers() {
    let photo = Adjustments {
        exposure: 0.7,
        contrast: 12.0,
        vibrance: 30.0,
        grain: 10.0,
        straighten: 3.0,
        crop: Crop { x: 0.4, y: 0.5, width: 0.5, height: 0.5 },
        ..Default::default()
    };
    let settings = presets::read(&json!({ "contrast": 40, "vibrance": 0, "saturation": -20 })).unwrap();
    let after = presets::apply(&settings, &photo);

    assert_eq!((after.contrast, after.saturation), (40.0, -20.0));
    assert_eq!(after.vibrance, 0.0, "a covered setting goes back to zero if the preset says so");
    assert_eq!((after.exposure, after.grain), (0.7, 10.0), "the rest is left alone");
    assert_eq!((after.straighten, after.crop), (photo.straighten, photo.crop));

    // A scan's film settings belong to it, like its framing.
    let scan = Adjustments { film: Film { kind: Kind::Colour, base: Some([0.7, 0.4, 0.2]), range: None }, ..photo };
    assert_eq!(presets::apply(&settings, &scan).film, scan.film);
}

#[test]
fn a_preset_never_holds_framing_or_things_it_does_not_understand() {
    let settings = presets::read(&json!({
        "grain": 30,
        "crop": { "x": 0.5, "y": 0.5, "width": 0.2, "height": 0.2 },
        "rotation": 1,
        "flipHorizontal": true,
        "film": { "kind": "colour", "base": [0.7, 0.4, 0.2], "range": null },
        "somethingFromTheFuture": 4,
    }))
    .unwrap();
    assert_eq!(settings, Settings { grain: Some(30.0), ..Default::default() });
    // Only what it covers is written; single-precision values keep their short form.
    let short = presets::read(&json!({ "exposure": 0.35, "curves": { "master": [[0, 0.03], [1, 0.9]] } })).unwrap();
    let written = serde_json::to_string(&short).unwrap();
    assert!(written.starts_with(r#"{"exposure":0.35,"curves":{"master":[[0.0,0.03],[1.0,0.9]]"#), "{written}");

    assert!(presets::read(&json!({ "rotation": 1 })).is_err(), "nothing left to cover");
    assert!(presets::read(&json!({ "grain": "lots" })).is_err());
    assert!(presets::read(&json!({ "mixer": [1, 2, 3] })).is_err());
    assert!(presets::read(&json!("grain")).is_err());
}

#[test]
fn a_preset_stays_inside_the_editor_ranges() {
    let wild = presets::read(&json!({
        "exposure": 50,
        "vignette": 5000,
        "grain": -3,
        "curves": { "master": [[-1, 0.2], [2, 7]] },
        "mixer": [{ "hue": 400 }, {}, {}, {}, {}, {}, {}, { "luminance": -250 }],
    }))
    .unwrap();
    assert_eq!((wild.exposure, wild.vignette, wild.grain), (Some(5.0), Some(100.0), Some(0.0)));
    assert_eq!(wild.curves.unwrap().master, [[0.0, 0.2], [1.0, 1.0]]);
    let mixer = wild.mixer.unwrap();
    assert_eq!((mixer[0].hue, mixer[7].luminance), (100.0, -100.0));
    // White balance goes as far as a RAW's Kelvin sliders take it, and no further.
    let max = tonality_lib::edit::MAX_WHITE_BALANCE;
    let warm = presets::read(&json!({ "temperature": 450, "tint": -9000 })).unwrap();
    assert_eq!((warm.temperature, warm.tint), (Some(450.0), Some(-max)));

    // A number too large for a recipe is refused outright, before anything is stored.
    assert!(presets::read(&json!({ "grain": 1e40 })).is_err());
    assert!(presets::read(&json!({ "curves": { "master": [[0, 0]] } })).is_err(), "a curve needs two points");
    let (_dir, library) = library();
    assert!(library.create_preset("Huge", settings(json!({ "grain": 1e40 }))).is_err());
    assert!(own(&library).is_empty());
    assert_eq!(library.create_preset("Huge", settings(json!({ "grain": 10 }))).unwrap().name, "Huge");
}

#[test]
fn the_built_in_presets_are_sound() {
    let built_in = presets::built_in();
    assert!(built_in.len() >= 10);
    assert_eq!(names(built_in).iter().collect::<BTreeSet<_>>().len(), built_in.len(), "no two share a name");
    assert!(built_in.iter().all(|preset| preset.id < 0 && preset.group.is_some()));

    for preset in built_in {
        // Each is already in its stored form, and does something.
        assert_eq!(presets::clean(preset.settings.clone()).unwrap(), preset.settings, "{}", preset.name);
        assert!(!presets::apply(&preset.settings, &Adjustments::default()).is_default(), "{}", preset.name);
        // The corrections made to one photo are never a preset's business.
        for correction in ["exposure", "temperature", "tint", "highlights", "shadows"] {
            assert!(!covered(preset).contains(correction), "{} covers {correction}", preset.name);
        }
    }

    // Looks cover the same settings, so one replaces another without leftovers.
    let looks: Vec<&Preset> = built_in.iter().filter(|preset| preset.group != Some("Finishing")).collect();
    assert!(looks.iter().all(|look| covered(look) == covered(looks[0])));
    let recipe = |preset: &Preset, onto: &Adjustments| presets::apply(&preset.settings, onto);
    let direct = recipe(looks[1], &Adjustments::default());
    assert_eq!(recipe(looks[1], &recipe(looks[0], &Adjustments::default())), direct);

    // Finishing touches go on top of a look and leave it standing.
    let grain = built_in.iter().find(|preset| preset.name == "Fine grain").unwrap();
    let both = recipe(grain, &direct);
    assert_eq!(both, Adjustments { grain: both.grain, ..direct.clone() });
    assert!(both.grain > 0.0);
}

#[test]
fn your_own_presets_are_kept_in_the_library() {
    let (dir, library) = library();
    assert_eq!(library.presets().unwrap(), presets::built_in());

    let evening = library.create_preset("  Wedding   evening ", settings(json!({ "contrast": 15, "vignette": -20 }))).unwrap();
    assert_eq!(evening.name, "Wedding evening");
    assert!(evening.id > 0 && evening.group.is_none());
    library.create_preset("beach", settings(json!({ "vibrance": 40 }))).unwrap();
    assert_eq!(names(&own(&library)), ["beach", "Wedding evening"], "listed by name, after the built-in ones");

    // A name is used once, whatever its capitals, built-in names included.
    assert_eq!(library.create_preset("Beach", settings(json!({ "grain": 5 }))).unwrap().name, "Beach 2");
    assert_eq!(library.create_preset("punchy", settings(json!({ "grain": 5 }))).unwrap().name, "punchy 2");
    assert_eq!(library.rename_preset(evening.id, "beach").unwrap().name, "beach 3");
    assert_eq!(library.rename_preset(evening.id, "Beach 3").unwrap().name, "Beach 3", "its own name is free to it");
    assert!(library.create_preset("   ", settings(json!({ "grain": 5 }))).is_err());
    assert!(library.rename_preset(evening.id, "").is_err());
    assert!(library.create_preset("Nothing", settings(json!({}))).is_err());

    let updated = library.update_preset(evening.id, settings(json!({ "grain": 12 }))).unwrap();
    assert_eq!((updated.name.as_str(), covered(&updated)), ("Beach 3", BTreeSet::from(["grain".to_string()])));
    assert_eq!(library.preset(evening.id).unwrap(), updated);

    library.delete_preset(evening.id).unwrap();
    assert!(library.preset(evening.id).is_err());

    // They are still there the next time the library is opened.
    drop(library);
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    assert_eq!(names(&own(&library)), ["beach", "Beach 2", "punchy 2"]);
    assert_eq!(library.preset(-1).unwrap(), presets::built_in()[0]);
}

#[test]
fn favorites_are_kept_in_the_order_they_were_starred() {
    let (dir, library) = library();
    assert!(library.favorite_presets().unwrap().is_empty());
    let mine = library.create_preset("Mine", settings(json!({ "grain": 5 }))).unwrap();
    library.set_preset_favorite(mine.id, true).unwrap();
    library.set_preset_favorite(-3, true).unwrap();
    assert_eq!(library.set_preset_favorite(mine.id, true).unwrap(), [mine.id, -3], "starring twice changes nothing");
    assert_eq!(library.set_preset_favorite(-3, false).unwrap(), [mine.id]);

    // A deleted preset takes its star with it, and the stars outlast the library being closed.
    library.set_preset_favorite(-1, true).unwrap();
    library.delete_preset(mine.id).unwrap();
    drop(library);
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    assert_eq!(library.favorite_presets().unwrap(), [-1]);
}

#[test]
fn presets_travel_as_files() {
    let (dir, library) = library();
    let made = library
        .create_preset("Warm evening", settings(json!({ "curves": { "master": [[0, 0.05], [1, 0.95]] }, "vibrance": 12 })))
        .unwrap();
    let file = dir.path().join(format!("Warm evening.{}", presets::FILE_EXTENSION));
    library.export_preset(made.id, &file).unwrap();
    let built_in = dir.path().join("built-in.json");
    library.export_preset(-1, &built_in).unwrap();

    let other = dir.path().join("holiday.json");
    std::fs::write(&other, r#"{"destination": "Lisbon"}"#).unwrap();
    let newer = dir.path().join("newer.json");
    std::fs::write(&newer, r#"{"tonality": "preset", "version": 2, "name": "Later", "settings": {"grain": 5}}"#).unwrap();
    let missing = dir.path().join("missing.json");

    // Into another library entirely.
    let (_dir, second) = self::library();
    let result = second.import_presets(&[file.clone(), other, built_in, newer, missing]).unwrap();
    assert_eq!(names(&result.imported), ["Warm evening", "Warm fade 2"]);
    assert_eq!(result.imported[0].settings, made.settings);
    assert_eq!(result.imported[1].settings, presets::built_in()[0].settings);
    let failed: Vec<&str> = result.failed.iter().map(|failure| failure.file_name.as_str()).collect();
    assert_eq!(failed, ["holiday.json", "newer.json", "missing.json"]);
    assert_eq!(own(&second), result.imported);

    // Reading the same file again gives a second preset rather than replacing the first.
    assert_eq!(second.import_presets(&[file]).unwrap().imported[0].name, "Warm evening 2");
}

/// Renders real photos with every built-in preset, for judging them by eye:
/// `TONALITY_SAMPLES=… TONALITY_OUT=… cargo test preset_sheet -- --ignored`
#[test]
#[ignore = "needs TONALITY_SAMPLES and a GPU"]
fn preset_sheet() {
    use std::path::PathBuf;
    let samples = PathBuf::from(std::env::var_os("TONALITY_SAMPLES").expect("set TONALITY_SAMPLES"));
    let out = PathBuf::from(std::env::var_os("TONALITY_OUT").expect("set TONALITY_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let gpu = tonality_lib::gpu::Gpu::new().unwrap();

    for entry in walkdir::WalkDir::new(samples).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        let Some(kind) = tonality_lib::media::kind_of(entry.path()) else { continue };
        let image = tonality_lib::develop::load(entry.path(), kind == tonality_lib::media::Kind::Raw).unwrap();
        let session = gpu.open(image).unwrap();
        let name = entry.path().file_stem().unwrap().to_string_lossy().into_owned();
        let as_shot = Adjustments::default();
        gpu.render_image(&session, &as_shot, 640).unwrap().save(out.join(format!("{name} 00 As shot.jpg"))).unwrap();
        for (index, preset) in presets::built_in().iter().enumerate() {
            let recipe = presets::apply(&preset.settings, &as_shot);
            let file = format!("{name} {:02} {}.jpg", index + 1, preset.name);
            gpu.render_image(&session, &recipe, 640).unwrap().save(out.join(file)).unwrap();
        }
    }
}
