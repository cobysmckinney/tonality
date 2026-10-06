# Tonality

A raw photo editor: a library to keep photos in and an editor to develop them.

## The library

Tonality keeps one managed library, like Apple Photos. Importing copies photos into it and files them by the day they were taken; nothing depends on the card or folder they came from.

```
~/Pictures/Tonality/
  Originals/2026/2026-09-19/IMG_0462.CR2   your files, original names, browsable without the app
  Exports/IMG_0462.jpg                     exported pictures, unless you choose another folder
  .tonality/library.db                     the index: favorites, flags, albums, imports
  .tonality/thumbs, previews               generated images, safe to delete
```

- **Library, Favorites, Imports, Recently Deleted, Albums** in the sidebar. Deleted photos are kept for 30 days.
- **Import** from files, folders, drag and drop, or a camera card (any mounted volume with a `DCIM` folder). You review what was found before anything is copied, grouped by the day it was taken: a day's heading selects or clears the whole day, so one shoot can be imported and the rest of the card left. Photos already in the library are marked and skipped.
- **RAW + JPEG pairs** shot together are one photo.
- **Culling**: `F` favorite, `P` pick, `X` reject, `U` unflag, `Delete` to delete. Arrows move, Shift extends, Enter opens, Escape goes back, `I` toggles info.

## The editor

Opening a photo (double-click or Enter) loads it at full resolution onto the GPU. Every slider change is one pass of a single shader, `src-tauri/src/shaders/develop.wgsl`, drawn at exactly the size and crop on screen. Editing never touches the original: the recipe is stored in the library database, and the same shader redraws the photo's thumbnail so the grid matches.

- **Adjust**: light (exposure, contrast, highlights, shadows, whites, blacks), color (temperature, tint, vibrance, saturation), tone curve with per-channel curves, an eight-band color mixer, detail (sharpening, noise reduction, clarity, dehaze), effects (vignette, grain).
- **Crop** (`C`): the photo is shown whole with an upright crop frame over it. Shapes (free, original, 1:1, 5:4, 4:3, 3:2, 16:9), a straighten slider that turns the photo under the frame and keeps the frame on the photo, quarter-turns and flips. Pasting edits onto another photo leaves that photo's own crop alone.
- **Starting look**: RAW files open with a built-in tone curve fitted to match camera JPEGs. The constants at the top of the shader hold it.
- **Histogram** with clipping markers; `J` shows clipped areas on the photo.
- **Tools** sit in a rail of icons along the right edge; `A` adjust, `C` crop, `M` masks, `Shift+P` presets, `H` history, `I` info; `Ctrl+Z` / `Ctrl+Shift+Z` undo and redo, hold `\` for the original, `Z` or double-click for 100%, scroll to zoom, `Ctrl+C` / `Ctrl+V` copy and paste edits (in the grid too, onto a whole selection), double-click a slider to reset it.

### Presets

A preset is a named look to lay over a photo's edits. Each one *covers* some settings and leaves the rest alone, so a look keeps the exposure you already corrected, and a grain preset goes on top of a look. No preset touches the crop.

- **The Presets tool** (`Shift+P`) shows each preset as a small picture of the open photo wearing it, drawn over the photo's current edits. Point at one, or walk the grid with the arrow keys, to see it large; click or Enter applies it. An outline marks the presets the photo is wearing.
- **Favorites**: the star on a preset lists it again in a Favorites group at the top, in the order you starred them. Stars are kept in the library.
- **Applying is a step** in the history ("Preset: Warm fade"), so it undoes and branches like any other edit.
- **Amount**: once applied, a slider under the list fades the preset from none of it (0%) to double (200%). It stays for as long as the settings the preset covers are left as it set them, and comes back when undo or redo lands on a step it made. Applying the same preset again, or another look in its place, keeps the amount measured from the photo as it was before either.
- **Built in**: six colour looks and four black-and-whites, which all cover the same settings (vibrance, saturation, the curves and the colour mixer), so picking another replaces the last one cleanly; and four finishing touches (grain, vignette, crispness) that cover one thing each.
- **Your own**: **+** saves the photo's edits as a preset. You choose which groups it covers (Light, Color, Curve, Color mixer, Detail, Effects); the ones you changed are ticked to begin with. Right-click one of yours, or use its **…** button, to rename it, update it from the photo in front of you, export it or delete it.
- **In the grid**, right-click a selection and choose **Apply preset** to put one on every photo in it.
- **Files**: a preset exports as a small `.tonality-preset` file, and **+** imports them, to share or to carry to another library.

`src-tauri/src/presets.rs` holds the built-in presets and the rules, with `tests/presets.rs`; `src/presets.ts` works out the amount.

### Masks

Masks change one part of the photo. Each mask is an area, built from parts, with its own sliders (light, color, clarity, dehaze, sharpening, noise reduction) that add to the photo's own wherever it covers.

- **The Masks tool** (`M`) starts a mask from the subject, the background, the sky, a selected object, a brush, a linear gradient, a radial gradient or a brightness range, each described in a line.
- **Subject, background and sky**: small models find the photo's main subject (a person, an animal, a building) and its sky; the background is everything but the subject. Each takes a second or two the first time; after that it is kept in the library. They are parts like any other, so they can be added, subtracted or intersected ("the sky, less the subject"), and a pasted mask finds each photo's own.
- **Select object**: draw a rough loop around something on the photo and its outline is found. The first loop on a photo takes a few seconds while the model looks the photo over (it starts as soon as you choose to draw); after that each loop takes a moment. **Draw again** replaces the loop; Esc cancels one being drawn.
- **Layers**: while the tool is open, the photo's masks float as layers over its corner, each with a black-and-white thumbnail of what it covers. The eye hides a mask's effect without removing it; **…** (or a right-click) renames, inverts or deletes it, and **+** starts another. The side column holds the chosen mask's sliders.
- **Parts**: the chosen layer opens to list its parts. **Add**, **Subtract** and **Intersect** put another one on: it adds to the area, takes from it, or keeps only what both cover ("the sky, but only its bright part"). The mark in front of a part changes how it combines.
- **On the photo**: a gradient's ends and a radial's edges, centre and turning knob are dragged into place. With a brush part chosen, dragging paints; Alt erases, `[` and `]` change the size. `O` tints the mask red.
- **They stay on the photo**: parts are kept on the file itself, so they follow crops, turns and flips. Presets never include masks; pasting edits does.
- **Limits**: 8 masks, 32 parts, 8 brush parts and 8 found parts (subject, sky, objects) a photo.

Gradients and brightness ranges are worked out per pixel in the shader. Brush strokes are painted into a coverage map per brush part (`src-tauri/src/masks.rs`); while you paint only the newest length of the stroke is painted again. `tests/masks.rs` checks each kind on the GPU; `src/masks.ts` places them on screen.

Found parts come from three small models bundled in the app (`src-tauri/models`, about 50 MB together), run on the CPU by tract, which is pure Rust, so mobile builds need no native runtime (`src-tauri/src/segment.rs`):

- the subject from U²-Netp (Apache 2.0), a salient-object model that sees the photo at 320 x 320;
- the sky from a U²-Netp trained on skies (MIT), converted from ncnn to ONNX, at 384 x 384;
- an object from EfficientSAM-Ti (Apache 2.0), given the loop's box as a prompt. Its encoder looks the whole photo over once, in a few seconds, and is kept for the session; each loop then costs a fraction of a second.

A guided filter then fits each answer to the photo's own edges at up to 3072 pixels. Mattes are worked out from the file before any edits, so they follow crops and turns, and are kept in `.tonality/mattes`, so each is found once per photo (an object's is named after its loop).

### History and branches

Every change is a step in the photo's history, kept in the library, so undo and redo still work after a restart. The History tab (`H`) lists the steps of the current branch, newest first; click one to go back to it.

History works like a small git repository per photo, and takes the place of "virtual copies":

- **Branch** from any step to take the edit in another direction. The branch you were on keeps everything it had.
- **Switch** between branches from the picker. The library grid shows whichever branch is current, with a marker when a photo has more than one.
- **Undo then edit** replaces the undone steps on that branch, as usual. Branch first if you want to keep both. Steps another branch still stands on are never removed.
- Pasting edits, resetting, and "Revert to original" are ordinary steps, so they can be undone too.

`src-tauri/src/history.rs` holds the model (steps, branches, tip and head) and `tests/history.rs` its rules.

### Exporting

**Export** (`Ctrl+E`, the button in the editor, or the selection bar and right-click menu in the grid) writes pictures out as files. The sheet shows what is about to be written before anything is, and what was written afterwards. Enter exports.

- **The name** sits at the top, with its extension, and can be typed over. Left alone it is the photo's name, plus the branch when the photo has more than one: `IMG_0462 (Warm).jpg`. A name you type is used as it is. When an export writes several files they share one name built from placeholders (`{name}`, `{branch}`, `{date}`, `{n}`), and the list under the field shows what each file comes out as.
- **Where**: the folder is shown in full. It starts as `Exports` inside the library; **Change…** picks another, and the choice is remembered along with the format, quality and size.
- **Which edits**: what you see, unless you say otherwise. A photo is exported from the branch it is on, at the step that branch is on. For one photo, tick any of its branches to get a file for each; for several photos, choose between the branch each is on and all branches.
- **Format**: JPEG with a quality from 1 to 100, PNG, or TIFF. TIFF is 16 bits a channel, drawn at that depth rather than widened from 8, and uncompressed.
- **Size**: full size, or a longest side in pixels (a few presets, or any number). The sheet shows the pixel size each file will have.
- **Nothing is replaced**: a name that is taken, in the folder or by another file in the same export, gets a number (`IMG_0462-2.jpg`), and the sheet shows the exact name beforehand.
- **Afterwards**: the step that was exported is marked in the History tab with the file it became; click the mark to show the file.

Exports are sRGB and carry the capture date, camera, lens and exposure. `src-tauri/src/export.rs` holds the rules and `tests/export.rs` checks them.

## Development

```sh
bun install
bun run tauri dev
```

Needs Rust, Bun, the [Tauri prerequisites](https://tauri.app/start/prerequisites/), and `libheif` for HEIC support (build with `--no-default-features` to leave HEIC out).

```sh
bun test                                  # crop arithmetic (src/crop.ts) and preset amounts (src/presets.ts)
cd src-tauri && cargo test                # library, history, preset, export and GPU pipeline tests, synthetic files
TONALITY_SAMPLES=/path/to/raws cargo test real_samples -- --ignored --nocapture
# Render real photos through the editor (optionally with a recipe) next to the camera's JPEG:
TONALITY_SAMPLES=/path/to/raws TONALITY_OUT=/tmp/out TONALITY_RECIPE='{"shadows":60}' \
  cargo test gpu_render -- --ignored --nocapture
# Render real photos with every built-in preset, to judge them by eye:
TONALITY_SAMPLES=/path/to/raws TONALITY_OUT=/tmp/out cargo test preset_sheet -- --ignored
```

- `TONALITY_LIBRARY=/some/folder` runs against another library instead of `~/Pictures/Tonality`.
- `TONALITY_FAKE_VOLUMES=/some/folder` (debug builds) treats a folder containing `DCIM` as a camera card.
- `TONALITY_EVAL=/some/folder` (debug builds) runs each `.js` file put in the folder inside the window, then deletes it: a way to script pointer drags when checking the mask tools.

Layout: `src-tauri/src` is the backend (`library.rs` database, `import.rs` scan and copy, `media.rs` decoding and metadata, `thumbs.rs` generated images, `volumes.rs` card detection, `develop.rs` RAW to linear light, `geometry.rs` crop and rotation arithmetic, `gpu.rs` the GPU pipeline, `edit.rs` the edit recipe, `history.rs` steps and branches, `presets.rs` presets, `masks.rs` mask packing and brush coverage, `segment.rs` finding the subject, `export.rs` writing pictures out, `commands.rs` the interface's API). `src` is the React interface, with all state in `store.ts` and the editor under `components/editor`. The GPU tests skip themselves on a machine without a graphics adapter.
