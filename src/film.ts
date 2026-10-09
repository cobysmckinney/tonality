/**
 * Film details: what a photo of film was shot on and with. A camera scan's
 * own details describe the scanning camera, so these are typed in, for a
 * whole roll at once or one frame at a time.
 */

export interface FilmDetails {
  /** The film stock: "Kodak Portra 400". */
  stock: string | null;
  /** The speed the film was shot at, which may not be the box speed. */
  iso: number | null;
  camera: string | null;
  lens: string | null;
  /** The frame's number on the roll. */
  frame: number | null;
}

export interface FilmPhoto {
  id: number;
  /** Camera-local `YYYY-MM-DDTHH:MM:SS`: for a scan, when the frame was scanned. */
  takenAt: string;
  fileName: string;
  film: FilmDetails;
}

/** Stocks, cameras and lenses already in the library, the most used first. */
export interface FilmSuggestions {
  stocks: string[];
  cameras: string[];
  lenses: string[];
}

export const NO_FILM: FilmDetails = { stock: null, iso: null, camera: null, lens: null, frame: null };

/** Whether any detail is given, which makes the photo one of film. */
export const hasFilm = (film: FilmDetails) => Object.values(film).some((value) => value !== null);

/** What the sheet's fields hold, as typed. */
export interface FilmForm {
  stock: string;
  iso: string;
  camera: string;
  lens: string;
  /** The frame of one photo, or the first frame of several. */
  frame: string;
}

const DETAILS = ["stock", "iso", "camera", "lens"] as const;

/** The order frames were scanned in: by when, then by file name, which a camera counts up. */
export function captureOrder<T extends Pick<FilmPhoto, "id" | "takenAt" | "fileName">>(photos: T[]): T[] {
  return [...photos].sort(
    (a, b) =>
      a.takenAt.localeCompare(b.takenAt) ||
      a.fileName.localeCompare(b.fileName, undefined, { numeric: true }) ||
      a.id - b.id,
  );
}

/** A whole number typed in a field, or null if there isn't one. */
export function wholeNumber(text: string): number | null {
  const trimmed = text.trim();
  return /^\d{1,9}$/.test(trimmed) ? Number(trimmed) : null;
}

/** Only the digits of what was typed, for the number fields. */
export const digitsOnly = (text: string) => text.replace(/\D/g, "").slice(0, 9);

const shown = (value: string | number | null) => (value === null ? "" : String(value));

export interface StartingForm {
  form: FilmForm;
  /** Fields the photos differ on, which start blank and keep each photo's own unless changed. */
  mixed: Set<keyof FilmForm>;
}

/**
 * What the sheet starts with. Each detail the photos share is filled in;
 * one they differ on starts blank. Several photos without frame numbers
 * start counting at 1, and ones already numbered in order show where they
 * start; numbered any other way, each keeps its own.
 */
export function startingForm(photos: FilmPhoto[]): StartingForm {
  const form: FilmForm = { stock: "", iso: "", camera: "", lens: "", frame: "" };
  const mixed = new Set<keyof FilmForm>();
  for (const key of DETAILS) {
    const values = new Set(photos.map((photo) => photo.film[key]));
    if (values.size === 1) form[key] = shown([...values][0]);
    else mixed.add(key);
  }
  const frames = captureOrder(photos).map((photo) => photo.film.frame);
  if (photos.length === 1) {
    form.frame = shown(frames[0]);
  } else if (frames.every((frame) => frame === null)) {
    form.frame = "1";
  } else if (frames.every((frame, i) => frame !== null && frame === frames[0]! + i)) {
    form.frame = String(frames[0]);
  } else {
    mixed.add("frame");
  }
  return { form, mixed };
}

/**
 * Each photo's details once the sheet's fields are laid over them. A field
 * left as it started changes nothing, so a blank one keeps each photo's
 * own; one that was changed applies to every photo, and one emptied clears
 * it. Frames count up from the first frame, in the order they were scanned.
 */
export function applyForm(photos: FilmPhoto[], start: FilmForm, form: FilmForm): FilmPhoto[] {
  const changed = (key: keyof FilmForm) => form[key].trim() !== start[key].trim();
  const text = (value: string) => value.trim() || null;
  const first = wholeNumber(form.frame);
  const order = new Map(captureOrder(photos).map((photo, i) => [photo.id, i]));
  return photos.map((photo) => {
    const film = { ...photo.film };
    if (changed("stock")) film.stock = text(form.stock);
    if (changed("camera")) film.camera = text(form.camera);
    if (changed("lens")) film.lens = text(form.lens);
    if (changed("iso")) film.iso = wholeNumber(form.iso) || null;
    if (first !== null) film.frame = first + order.get(photo.id)!;
    else if (changed("frame")) film.frame = null;
    return { ...photo, film };
  });
}

/** Whether saving would change anything. */
export const changesAnything = (before: FilmPhoto[], after: FilmPhoto[]) =>
  after.some((photo, i) => JSON.stringify(photo.film) !== JSON.stringify(before[i]?.film));

/**
 * What to offer while typing: the ones containing what is typed, those
 * starting with it first, and not what is already typed in full.
 */
export function suggestionsFor(options: string[], typed: string, limit = 6): string[] {
  const wanted = typed.trim().toLocaleLowerCase();
  const starts: string[] = [];
  const contains: string[] = [];
  for (const option of options) {
    const lower = option.toLocaleLowerCase();
    if (lower === wanted) continue;
    if (lower.startsWith(wanted)) starts.push(option);
    else if (lower.includes(wanted)) contains.push(option);
  }
  return [...starts, ...contains].slice(0, limit);
}
