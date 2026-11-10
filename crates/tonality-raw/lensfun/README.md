# lensfun database

Lens calibration data from the [lensfun](https://lensfun.github.io/) project
(`data/db` at commit 52cf1a7b0e82e07e0ecaa3391ee5106a6dd1ee54, 2026-11-04), used by tonality for
lens correction when a RAW file carries no correction data of its own.

The database is licensed under the Creative Commons Attribution-ShareAlike 3.0 license
(`COPYING.CC_BY-SA_3.0`). Copyright the lensfun contributors.

`build.rs` compacts these files into the form tonality embeds; to update, replace the `.xml`
files with a newer `data/db` and rebuild.
