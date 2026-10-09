//! Presets: named looks to lay over a photo's edits.
//!
//! A preset holds part of a recipe, the settings it *covers*, and applying
//! it changes those and leaves every other setting as it was. So a look can
//! keep the exposure you already corrected, and a grain preset can go on top
//! of a look. No preset covers the crop, straightening, turns or flips.
//!
//! The built-in ones are defined here; your own are rows in the library, and
//! can be written out as files and read back in.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::edit::{Adjustments, Band, Curves};
use crate::library::Library;

pub(crate) const SCHEMA: &str = "
CREATE TABLE presets (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    -- The settings the preset covers: part of a recipe, as JSON.
    settings    TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
";

/// The presets starred as favorites, built-in or your own, in the order they were starred.
pub(crate) const FAVORITES_SCHEMA: &str = "
CREATE TABLE preset_favorites (
    -- Counts up as presets are starred, so the order holds within the same second.
    position    INTEGER PRIMARY KEY AUTOINCREMENT,
    preset_id   INTEGER NOT NULL UNIQUE,
    starred_at  INTEGER NOT NULL
);
";

/// The extension of a preset written out as a file.
pub const FILE_EXTENSION: &str = "tonality-preset";

/// Declares `Settings`, one optional field for each setting a preset can
/// cover, along with laying them over a recipe and keeping them in range.
/// The framing settings (crop, straightening, turns and flips) belong to
/// the photo and have no field, so no preset can hold them.
macro_rules! settings {
    (sliders { $($slider:ident: $min:literal..=$max:literal,)* } $($other:ident: $kind:ty,)*) => {
        /// Part of a recipe: values for the settings a preset covers, under
        /// the names they have in a stored recipe. Those it leaves out are
        /// not written at all.
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default, rename_all = "camelCase")]
        pub struct Settings {
            $(#[serde(skip_serializing_if = "Option::is_none")] pub $slider: Option<f32>,)*
            $(#[serde(skip_serializing_if = "Option::is_none")] pub $other: Option<$kind>,)*
        }

        impl Settings {
            pub fn is_empty(&self) -> bool {
                $(self.$slider.is_none() &&)* $(self.$other.is_none() &&)* true
            }

            /// Lays these over a recipe: the settings they cover take their
            /// values, the rest stay as they are.
            pub fn apply(&self, onto: &Adjustments) -> Adjustments {
                let mut recipe = onto.clone();
                $(if let Some(value) = self.$slider { recipe.$slider = value; })*
                $(if let Some(value) = &self.$other { recipe.$other = value.clone(); })*
                recipe
            }

            /// Brings every slider inside the range the editor gives it.
            fn clamp_sliders(&mut self) -> Result<()> {
                $(if let Some(value) = &mut self.$slider {
                    ensure!(value.is_finite(), "{} is not a number", stringify!($slider));
                    *value = value.clamp($min, $max);
                })*
                Ok(())
            }
        }
    };
}

settings! {
    sliders {
        exposure: -5.0..=5.0,
        contrast: -100.0..=100.0,
        highlights: -100.0..=100.0,
        shadows: -100.0..=100.0,
        whites: -100.0..=100.0,
        blacks: -100.0..=100.0,
        temperature: -2000.0..=2000.0,
        tint: -2000.0..=2000.0,
        vibrance: -100.0..=100.0,
        saturation: -100.0..=100.0,
        clarity: -100.0..=100.0,
        dehaze: -100.0..=100.0,
        sharpening: 0.0..=100.0,
        noise_reduction: 0.0..=100.0,
        vignette: -100.0..=100.0,
        grain: 0.0..=100.0,
    }
    curves: Curves,
    mixer: [Band; 8],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    /// Built-in presets count down from -1; the library's own count up from 1.
    pub id: i64,
    pub name: String,
    /// The heading a built-in preset is listed under; none for one of your own.
    pub group: Option<&'static str>,
    pub settings: Settings,
}

/// Checks settings from outside (the interface, a file, the database) and
/// brings every value inside the range the editor allows.
pub fn clean(mut settings: Settings) -> Result<Settings> {
    ensure!(!settings.is_empty(), "a preset has to cover at least one group of settings");
    settings.clamp_sliders()?;
    if let Some(curves) = &mut settings.curves {
        for points in [&mut curves.master, &mut curves.red, &mut curves.green, &mut curves.blue] {
            ensure!(points.len() >= 2, "a curve needs at least two points");
            for point in points.iter_mut() {
                ensure!(point.iter().all(|v| v.is_finite()), "a curve point is not a number");
                *point = point.map(|v| v.clamp(0.0, 1.0));
            }
        }
    }
    for band in settings.mixer.iter_mut().flatten() {
        for value in [&mut band.hue, &mut band.saturation, &mut band.luminance] {
            ensure!(value.is_finite(), "a colour mixer value is not a number");
            *value = value.clamp(-100.0, 100.0);
        }
    }
    Ok(settings)
}

/// Reads settings written as JSON by anything other than this version:
/// settings it does not know, and the framing ones, are left out.
pub fn read(settings: &Value) -> Result<Settings> {
    ensure!(settings.is_object(), "a preset's settings should be a set of named values");
    clean(Settings::deserialize(settings).context("these are not settings Tonality understands")?)
}

/// Lays a preset over a recipe: the settings it covers take its values, the
/// rest stay as they are.
pub fn apply(settings: &Settings, onto: &Adjustments) -> Adjustments {
    settings.apply(onto)
}

// ---- the built-in presets ----

/// What every colour and black-and-white look covers: tone through the
/// curve, colour through saturation and the mixer. Covering the same
/// settings means one look replaces another cleanly, and none of them
/// touches the corrections made to a particular photo (exposure, white
/// balance, highlights and shadows) or its detail and effects.
fn look(recipe: Adjustments) -> Settings {
    Settings {
        vibrance: Some(recipe.vibrance),
        saturation: Some(recipe.saturation),
        curves: Some(recipe.curves),
        mixer: Some(recipe.mixer),
        ..Default::default()
    }
}

const RED: usize = 0;
const ORANGE: usize = 1;
const YELLOW: usize = 2;
const GREEN: usize = 3;
const AQUA: usize = 4;
const BLUE: usize = 5;
const PURPLE: usize = 6;
const MAGENTA: usize = 7;

/// A colour mixer with these bands changed: (band, hue, saturation, luminance).
fn mixer(bands: &[(usize, f32, f32, f32)]) -> [Band; 8] {
    let mut mixer = [Band::default(); 8];
    for &(band, hue, saturation, luminance) in bands {
        mixer[band] = Band { hue, saturation, luminance };
    }
    mixer
}

fn curve(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    points.to_vec()
}

/// The built-in presets, in the order they are listed.
pub fn built_in() -> &'static [Preset] {
    static BUILT_IN: LazyLock<Vec<Preset>> = LazyLock::new(make_built_in);
    &BUILT_IN
}

fn make_built_in() -> Vec<Preset> {
    let looks = [
        (
            "Color",
            "Warm fade",
            Adjustments {
                vibrance: 8.0,
                saturation: -12.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.06], [0.25, 0.24], [0.5, 0.52], [0.75, 0.77], [1.0, 0.96]]),
                    red: curve(&[[0.0, 0.02], [0.5, 0.52], [1.0, 1.0]]),
                    blue: curve(&[[0.0, 0.03], [0.5, 0.47], [1.0, 0.94]]),
                    ..Default::default()
                },
                mixer: mixer(&[(ORANGE, 0.0, 8.0, 0.0), (GREEN, 12.0, -20.0, 0.0), (BLUE, -8.0, -15.0, 0.0)]),
                ..Default::default()
            },
        ),
        (
            "Color",
            "Cool matte",
            Adjustments {
                vibrance: 5.0,
                saturation: -18.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.08], [0.22, 0.22], [0.5, 0.5], [0.8, 0.8], [1.0, 0.95]]),
                    red: curve(&[[0.0, 0.0], [0.5, 0.48], [1.0, 0.97]]),
                    blue: curve(&[[0.0, 0.04], [0.5, 0.52], [1.0, 1.0]]),
                    ..Default::default()
                },
                mixer: mixer(&[
                    (ORANGE, 0.0, -8.0, 0.0),
                    (YELLOW, 0.0, -20.0, 0.0),
                    (AQUA, 0.0, 10.0, 0.0),
                    (MAGENTA, 0.0, -12.0, 0.0),
                ]),
                ..Default::default()
            },
        ),
        (
            "Color",
            "Punchy",
            Adjustments {
                vibrance: 28.0,
                saturation: 4.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.0], [0.25, 0.2], [0.5, 0.51], [0.75, 0.81], [1.0, 1.0]]),
                    ..Default::default()
                },
                mixer: mixer(&[(BLUE, 0.0, 10.0, -8.0)]),
                ..Default::default()
            },
        ),
        (
            "Color",
            "Golden hour",
            Adjustments {
                vibrance: 18.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.0], [0.25, 0.23], [0.5, 0.52], [0.75, 0.79], [1.0, 1.0]]),
                    red: curve(&[[0.0, 0.0], [0.5, 0.54], [1.0, 1.0]]),
                    blue: curve(&[[0.0, 0.0], [0.5, 0.46], [1.0, 0.93]]),
                    ..Default::default()
                },
                mixer: mixer(&[(ORANGE, 0.0, 15.0, 4.0), (YELLOW, -15.0, 12.0, 0.0), (GREEN, -20.0, -10.0, 0.0)]),
                ..Default::default()
            },
        ),
        (
            "Color",
            "Cinema",
            Adjustments {
                vibrance: 10.0,
                saturation: -6.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.02], [0.25, 0.21], [0.5, 0.5], [0.75, 0.79], [1.0, 0.98]]),
                    red: curve(&[[0.0, 0.0], [0.25, 0.22], [0.75, 0.78], [1.0, 1.0]]),
                    blue: curve(&[[0.0, 0.04], [0.25, 0.29], [0.75, 0.72], [1.0, 0.95]]),
                    ..Default::default()
                },
                mixer: mixer(&[
                    (ORANGE, 0.0, 14.0, 0.0),
                    (YELLOW, -20.0, -10.0, 0.0),
                    (GREEN, 30.0, -30.0, 0.0),
                    (BLUE, -35.0, 0.0, -6.0),
                ]),
                ..Default::default()
            },
        ),
        (
            "Color",
            "Pastel",
            Adjustments {
                vibrance: 22.0,
                saturation: -22.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.05], [0.25, 0.31], [0.5, 0.56], [0.75, 0.79], [1.0, 0.98]]),
                    ..Default::default()
                },
                mixer: mixer(&[(GREEN, 0.0, -10.0, 8.0), (BLUE, 0.0, 0.0, 8.0)]),
                ..Default::default()
            },
        ),
        (
            "Black & white",
            "Soft mono",
            Adjustments {
                saturation: -100.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.03], [0.25, 0.25], [0.5, 0.52], [0.75, 0.77], [1.0, 0.98]]),
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
        (
            "Black & white",
            "High contrast",
            Adjustments {
                saturation: -100.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.0], [0.25, 0.16], [0.5, 0.5], [0.75, 0.85], [1.0, 1.0]]),
                    ..Default::default()
                },
                mixer: mixer(&[(ORANGE, 0.0, 0.0, 6.0)]),
                ..Default::default()
            },
        ),
        (
            "Black & white",
            "Dark sky",
            Adjustments {
                saturation: -100.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.0], [0.25, 0.21], [0.5, 0.5], [0.75, 0.8], [1.0, 1.0]]),
                    ..Default::default()
                },
                mixer: mixer(&[
                    (RED, 0.0, 0.0, 10.0),
                    (ORANGE, 0.0, 0.0, 12.0),
                    (YELLOW, 0.0, 0.0, 8.0),
                    (AQUA, 0.0, 0.0, -45.0),
                    (BLUE, 0.0, 0.0, -65.0),
                    (PURPLE, 0.0, 0.0, -30.0),
                ]),
                ..Default::default()
            },
        ),
        (
            "Black & white",
            "Faded mono",
            Adjustments {
                saturation: -100.0,
                curves: Curves {
                    master: curve(&[[0.0, 0.1], [0.25, 0.27], [0.5, 0.5], [0.75, 0.74], [1.0, 0.93]]),
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
    ];
    // Finishing touches cover one thing each, so they go on top of any look.
    let finishes = [
        ("Fine grain", Settings { grain: Some(22.0), ..Default::default() }),
        ("Heavy grain", Settings { grain: Some(55.0), ..Default::default() }),
        ("Vignette", Settings { vignette: Some(-28.0), ..Default::default() }),
        ("Crisp", Settings { sharpening: Some(40.0), clarity: Some(18.0), ..Default::default() }),
    ];

    let looks = looks.into_iter().map(|(group, name, recipe)| (group, name, look(recipe)));
    let finishes = finishes.into_iter().map(|(name, settings)| ("Finishing", name, settings));
    looks
        .chain(finishes)
        .enumerate()
        .map(|(index, (group, name, settings))| Preset {
            id: -(index as i64) - 1,
            name: name.to_string(),
            group: Some(group),
            settings,
        })
        .collect()
}

// ---- your own, in the library ----

fn tidy_name(name: &str) -> Result<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(!name.is_empty(), "A preset needs a name.");
    Ok(name)
}

/// `name`, or `name 2`, `name 3`… if another preset already goes by it.
/// Two presets by one name could not be told apart in the list.
fn free_name(db: &Connection, name: &str, except: Option<i64>) -> Result<String> {
    let mut taken: Vec<String> = built_in().iter().map(|preset| preset.name.to_lowercase()).collect();
    let mut stmt = db.prepare("SELECT name FROM presets WHERE id IS NOT ?1")?;
    for other in stmt.query_map([except], |r| r.get::<_, String>(0))? {
        taken.push(other?.to_lowercase());
    }
    let mut candidate = name.to_string();
    let mut number = 1;
    while taken.contains(&candidate.to_lowercase()) {
        number += 1;
        candidate = format!("{name} {number}");
    }
    Ok(candidate)
}

fn stored(db: &Connection, id: i64) -> Result<Preset> {
    let row: Option<(String, String)> = db
        .query_row("SELECT name, settings FROM presets WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let (name, settings) = row.context("That preset is no longer in the library.")?;
    Ok(Preset { id, name, group: None, settings: serde_json::from_str(&settings)? })
}

/// Adds a preset of your own. `settings` must already be clean.
fn insert(db: &Connection, name: &str, settings: Settings) -> Result<Preset> {
    let name = free_name(db, &tidy_name(name)?, None)?;
    db.execute(
        "INSERT INTO presets (name, settings, created_at) VALUES (?1, ?2, ?3)",
        params![name, serde_json::to_string(&settings)?, chrono::Utc::now().timestamp()],
    )?;
    Ok(Preset { id: db.last_insert_rowid(), name, group: None, settings })
}

/// A preset as a file: what `export_preset` writes and `import_presets` reads.
#[derive(Serialize, Deserialize)]
struct PresetFile {
    /// Always "preset"; tells these files apart from other JSON.
    tonality: String,
    version: u32,
    name: String,
    settings: Value,
}

/// What reading preset files came to.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedPresets {
    pub imported: Vec<Preset>,
    pub failed: Vec<FailedFile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedFile {
    pub file_name: String,
    pub reason: String,
}

fn read_file(path: &Path) -> Result<(String, Settings)> {
    // A preset is a few hundred bytes; anything large is some other file.
    ensure!(fs::metadata(path)?.len() < 1 << 20, "this is not a Tonality preset");
    let file: PresetFile =
        serde_json::from_str(&fs::read_to_string(path)?).ok().context("this is not a Tonality preset")?;
    ensure!(file.tonality == "preset", "this is not a Tonality preset");
    ensure!(file.version == 1, "this preset was made by a newer version of Tonality");
    Ok((tidy_name(&file.name)?, read(&file.settings)?))
}

impl Library {
    /// Every preset: the built-in ones in their order, then your own by name.
    pub fn presets(&self) -> Result<Vec<Preset>> {
        let mut presets = built_in().to_vec();
        let db = self.db();
        let mut stmt = db.prepare("SELECT id, name, settings FROM presets ORDER BY name COLLATE NOCASE, id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        for row in rows {
            let (id, name, settings) = row?;
            // One damaged row should not hide the others.
            if let Ok(settings) = serde_json::from_str(&settings) {
                presets.push(Preset { id, name, group: None, settings });
            }
        }
        Ok(presets)
    }

    pub fn preset(&self, id: i64) -> Result<Preset> {
        if id < 0 {
            return built_in().iter().find(|preset| preset.id == id).cloned().context("there is no such built-in preset");
        }
        stored(&self.db(), id)
    }

    /// Saves settings as a new preset of your own.
    pub fn create_preset(&self, name: &str, settings: Settings) -> Result<Preset> {
        let settings = clean(settings)?;
        insert(&self.db(), name, settings)
    }

    pub fn rename_preset(&self, id: i64, name: &str) -> Result<Preset> {
        let db = self.db();
        let name = free_name(&db, &tidy_name(name)?, Some(id))?;
        db.execute("UPDATE presets SET name = ?2 WHERE id = ?1", params![id, name])?;
        stored(&db, id)
    }

    /// Replaces what a preset of your own holds, keeping its name.
    pub fn update_preset(&self, id: i64, settings: Settings) -> Result<Preset> {
        let settings = serde_json::to_string(&clean(settings)?)?;
        let db = self.db();
        db.execute("UPDATE presets SET settings = ?2 WHERE id = ?1", params![id, settings])?;
        stored(&db, id)
    }

    pub fn delete_preset(&self, id: i64) -> Result<()> {
        let db = self.db();
        db.execute("DELETE FROM presets WHERE id = ?1", [id])?;
        db.execute("DELETE FROM preset_favorites WHERE preset_id = ?1", [id])?;
        Ok(())
    }

    /// The favorite presets' ids, in the order they were starred. Only presets that still exist.
    pub fn favorite_presets(&self) -> Result<Vec<i64>> {
        let existing: std::collections::HashSet<i64> = self.presets()?.iter().map(|preset| preset.id).collect();
        let db = self.db();
        let mut stmt = db.prepare("SELECT preset_id FROM preset_favorites ORDER BY position")?;
        let ids = stmt.query_map([], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids.into_iter().filter(|id| existing.contains(id)).collect())
    }

    /// Stars a preset, or takes its star away.
    pub fn set_preset_favorite(&self, id: i64, favorite: bool) -> Result<Vec<i64>> {
        {
            let db = self.db();
            if favorite {
                db.execute(
                    "INSERT OR IGNORE INTO preset_favorites (preset_id, starred_at) VALUES (?1, ?2)",
                    params![id, chrono::Utc::now().timestamp()],
                )?;
            } else {
                db.execute("DELETE FROM preset_favorites WHERE preset_id = ?1", [id])?;
            }
        }
        self.favorite_presets()
    }

    /// Writes a preset out as a file, to share or to keep.
    pub fn export_preset(&self, id: i64, path: &Path) -> Result<()> {
        let preset = self.preset(id)?;
        let file = PresetFile {
            tonality: "preset".into(),
            version: 1,
            name: preset.name,
            settings: serde_json::to_value(preset.settings)?,
        };
        fs::write(path, serde_json::to_string_pretty(&file)?).with_context(|| format!("writing {}", path.display()))
    }

    /// Reads preset files into the library. A file that is not a preset is
    /// reported and the others still come in.
    pub fn import_presets(&self, paths: &[PathBuf]) -> Result<ImportedPresets> {
        let mut result = ImportedPresets { imported: Vec::new(), failed: Vec::new() };
        for path in paths {
            // Read before taking the library: the file may be on a slow drive.
            let read = read_file(path).and_then(|(name, settings)| insert(&self.db(), &name, settings));
            match read {
                Ok(preset) => result.imported.push(preset),
                Err(error) => result.failed.push(FailedFile {
                    file_name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                    reason: error.to_string(),
                }),
            }
        }
        Ok(result)
    }
}
