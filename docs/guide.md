# Tonality user guide

How Tonality works, area by area. New here? The [README](../README.md) has the short version; this is the long one.

- [The library](#the-library)
- [The editor](#the-editor)
- [Presets](#presets)
- [Masks](#masks)
- [History and branches](#history-and-branches)
- [Exporting](#exporting)
- [Keyboard shortcuts](#keyboard-shortcuts)

## The library

Tonality keeps one managed library, like Apple Photos. Importing copies photos into it and files them by the day they were taken; nothing depends on the card or folder they came from.

```
~/Pictures/Tonality/
  Originals/2026/2026-09-19/IMG_0462.CR2   your files, original names, browsable without the app
  Exports/IMG_0462.jpg                     exported pictures, unless you choose another folder
  .tonality/library.db                     the index: favorites, flags, albums, imports
  .tonality/thumbs, previews               generated images, safe to delete
```

- **One window.** Opening Tonality while it's already running brings up the open window rather than starting a second copy.
- **Library, Favorites, Imports, Recently Deleted, Albums** in the sidebar. Deleted photos are kept for 30 days. If a file can't be deleted at the end of that (it's open in another program, say), its photo stays in Recently Deleted until it can be.
- **Import** from files, folders, drag and drop, or a camera card (any mounted volume with a `DCIM` folder). You review what was found before anything is copied, grouped by the day it was taken: a day's heading selects or clears the whole day, so one shoot can be imported and the rest of the card left. Photos already in the library are marked and skipped. Each copy is read back and checked against the original before the photo is added, so a failing card shows up as an error, not a damaged photo.
- **Moving or deleting files in `Originals` yourself**: a photo whose original is gone keeps its thumbnail but gets a crossed-out file mark in the grid, and can't be edited or exported until the file is put back where it was.
- **RAW + JPEG pairs** shot together are one photo.
- **Culling**: `F` favorite, `P` pick, `X` reject, `U` unflag, `Delete` to delete. Arrows move, Shift extends, Enter opens, Escape goes back. When a photo leaves the grid (deleted, or hidden by the filter after you reject it under Hide rejected, say), the next one is selected, so you can keep going without reaching for the mouse; in the editor, the next photo opens.
- **If the library can't open** (say the folder can't be changed), the window says why. Fix the problem and choose Try again.

## The editor

Opening a photo (double-click or Enter) loads it at full resolution onto the graphics card, and every slider change redraws it straight away. A photo larger than the graphics card can hold (16,384 pixels a side on most, 8,192 on some) is scaled down just enough to fit, and that is the largest it can be exported at. Editing never touches the original: your edits are stored in the library, and the photo's thumbnail is redrawn with them so the grid matches.

- **Adjust**: light (exposure, contrast, highlights, shadows, whites, blacks), color (temperature, tint, vibrance, saturation), tone curve with per-channel curves, an eight-band color mixer, detail (sharpening, noise reduction, clarity, dehaze), effects (vignette, grain).
- **Crop** (`C`): the photo is shown whole with an upright crop frame over it. Shapes (free, original, 1:1, 5:4, 4:3, 3:2, 16:9) that hold while you drag the frame (undo back to a crop of another shape and it is free again), a straighten slider that turns the photo under the frame and keeps the frame on the photo, quarter-turns and flips. Pasting edits onto another photo leaves that photo's own crop alone.
- **Starting look**: RAW files open with a built-in tone curve fitted to match camera JPEGs.
- **Histogram** with clipping markers; `J` shows clipped areas on the photo.
- **Tools** sit in a rail of icons along the right edge; `A` adjust, `C` crop, `M` masks, `Shift+P` presets, `H` history, `I` info; `Ctrl+Z` / `Ctrl+Shift+Z` undo and redo, hold `\` for the original, `Z` or double-click for 100%, scroll to zoom, `Ctrl+C` / `Ctrl+V` copy and paste edits (in the grid too, onto a whole selection), double-click a slider to reset it.

## Presets

A preset is a named look to lay over a photo's edits. Each one *covers* some settings and leaves the rest alone, so a look keeps the exposure you already corrected, and a grain preset goes on top of a look. No preset touches the crop.

- **The Presets tool** (`Shift+P`) shows each preset as a small picture of the open photo wearing it, drawn over the photo's current edits. Point at one, or walk the grid with the arrow keys, to see it large; click or Enter applies it. An outline marks the presets the photo is wearing.
- **Favorites**: the star on a preset lists it again in a Favorites group at the top, in the order you starred them. Stars are kept in the library.
- **Applying is a step** in the history ("Preset: Warm fade"), so it undoes and branches like any other edit.
- **Amount**: once applied, a slider under the list fades the preset from none of it (0%) to double (200%). It stays for as long as the settings the preset covers are left as it set them, and comes back when undo or redo lands on a step it made. Applying the same preset again, or another look in its place, keeps the amount measured from the photo as it was before either.
- **Built in**: six colour looks and four black-and-whites, which all cover the same settings (vibrance, saturation, the curves and the colour mixer), so picking another replaces the last one cleanly; and four finishing touches (grain, vignette, crispness) that cover one thing each.
- **Your own**: **+** saves the photo's edits as a preset. You choose which groups it covers (Light, Color, Curve, Color mixer, Detail, Effects); the ones you changed are ticked to begin with. Right-click one of yours, or use its **…** button, to rename it, update it from the photo in front of you, export it or delete it.
- **In the grid**, right-click a selection and choose **Apply preset** to put one on every photo in it.
- **Files**: a preset exports as a small `.tonality-preset` file, and **+** imports them, to share or to carry to another library.

## Masks

Masks change one part of the photo. Each mask is an area, built from parts, with its own sliders (light, color, clarity, dehaze, sharpening, noise reduction) that add to the photo's own wherever it covers. Added up, each still stops at its own limit, so overlapping masks can't take exposure past +5 or -5.

- **The Masks tool** (`M`) is one column: the photo's masks at the top, then a tile for each way to start one (subject, background, sky, object, brush, linear gradient, radial gradient, brightness range; hover a tile for what it picks out), or, once a mask is chosen, its settings and sliders.
- **Subject, background and sky**: small models find the photo's main subject (a person, an animal, a building) and its sky; the background is everything but the subject. The subject takes about six seconds the first time and the sky about four; after that it is kept in the library. They are parts like any other, so they can be added, subtracted or intersected ("the sky, less the subject"), and a pasted mask finds each photo's own.
- **Select object**: draw a rough loop around something on the photo and its outline is found. The first loop on a photo takes a few seconds while the model looks the photo over (it starts as soon as you choose to draw); after that each loop takes a moment. **Draw again** replaces the loop; Esc cancels one being drawn. A loop drawn, or edits pasted, while another part is being found waits its turn. A loop pasted onto a much smaller photo may be too small to find anything in; that part is left empty, the editor says so, and you can draw it again.
- **The list**: each mask has a black-and-white thumbnail of what it covers. The eye hides a mask's effect without removing it; **…** (or a right-click) renames, inverts or deletes it. Clicking the chosen mask again, **New mask**, or Esc lets go of it, back to the tiles.
- **Parts**: the chosen mask opens to list its parts. **Add**, **Subtract** and **Intersect** put another one on: it adds to the area, takes from it, or keeps only what both cover ("the sky, but only its bright part"). The mark after a part changes how it combines, and the button at the end of the row inverts the whole mask. The chosen part's settings (a brush's size, a radial's feather, a range's ends) come first under the list.
- **On the photo**: a gradient's ends and a radial's edges, centre and turning knob are dragged into place. With a brush part chosen, dragging paints; Alt erases, `[` and `]` change the size. `O` (or **Overlay**) tints the chosen mask red.
- **They stay on the photo**: parts are kept on the file itself, so they follow crops, turns and flips. Presets never include masks; pasting edits does.
- **Limits**: 8 masks, 32 parts, 8 brush parts and 8 found parts (subject, sky, objects) a photo.

Subject, sky and object finding all run on your computer, from small models bundled with the app. Nothing is uploaded. Each photo's answer is kept in the library, so it is only worked out once.

## History and branches

Every change is a step in the photo's history, kept in the library, so undo and redo still work after a restart. The History tab (`H`) lists the steps of the current branch, newest first; click one to go back to it.

History works like a small git repository per photo, and takes the place of "virtual copies":

- **Branch** from any step to take the edit in another direction. The branch you were on keeps everything it had.
- **Switch** between branches from the picker. The library grid shows whichever branch is current, with a marker when a photo has more than one.
- **Undo then edit** replaces the undone steps on that branch, as usual. Branch first if you want to keep both. Steps another branch still stands on are never removed.
- Pasting edits, resetting, and "Revert to original" are ordinary steps, so they can be undone too.

## Exporting

**Export** (`Ctrl+E`, the button in the editor, or the selection bar and right-click menu in the grid) writes pictures out as files. The sheet shows what is about to be written before anything is, and what was written afterwards. Enter exports.

- **The name** sits at the top, with its extension, and can be typed over. Left alone it is the photo's name, plus the branch when the photo has more than one: `IMG_0462 (Warm).jpg`. A name you type is used as it is. When an export writes several files they share one name built from placeholders (`{name}`, `{branch}`, `{date}`, `{n}`), and the list under the field shows what each file comes out as.
- **Where**: the folder is shown in full. It starts as `Exports` inside the library; **Change…** picks another, and the choice is remembered along with the format, quality and size.
- **Which edits**: what you see, unless you say otherwise. A photo is exported from the branch it is on, at the step that branch is on. For one photo, tick any of its branches to get a file for each; for several photos, choose between the branch each is on and all branches.
- **Format**: JPEG with a quality from 1 to 100, PNG, or TIFF. TIFF is 16 bits a channel, drawn at that depth rather than widened from 8, and uncompressed.
- **Size**: full size, or a longest side in pixels (a few presets, or any number). The sheet shows the pixel size each file will have, and says so when a photo is too large for the graphics card and comes out smaller.
- **Nothing is replaced**: a name that is taken, in the folder or by another file in the same export, gets a number (`IMG_0462-2.jpg`), and the sheet shows the exact name beforehand.
- **Afterwards**: the step that was exported is marked in the History tab with the file it became; click the mark to show the file.

Exports are sRGB and carry the capture date, camera, lens and exposure.

## Keyboard shortcuts

| Where | Key | Does |
|---|---|---|
| Grid | Arrows, Shift+arrows | Move, extend the selection |
| Grid | `Ctrl+A` | Select all |
| Grid | Enter, double-click | Open the photo |
| Grid, editor | `F` / `P` / `X` / `U` | Favorite / pick / reject / unflag |
| Grid, editor | `Delete` | Move to Recently Deleted (kept 30 days). In the editor, not while a button or slider has focus: click the photo first |
| Grid, editor | `Ctrl+C` / `Ctrl+V` | Copy / paste edits (in the grid, onto every selected photo) |
| Grid, editor | `Ctrl+E` | Export |
| Editor | `Esc` | Back to the grid |
| Editor | Left / Right | Previous / next photo |
| Editor | `A` / `C` / `M` / `Shift+P` / `H` / `I` | Adjust / crop / masks / presets / history / info |
| Editor | `Ctrl+Z` / `Ctrl+Shift+Z` or `Ctrl+Y` | Undo / redo |
| Editor | hold `\` | Show the original |
| Editor | `Z`, double-click | Fit / 100% |
| Editor | `J` | Show clipped areas |
| Editor | `O` | Mask overlay |
| Editor | `[` / `]` | Brush size |
| Editor | double-click a slider | Reset it |
