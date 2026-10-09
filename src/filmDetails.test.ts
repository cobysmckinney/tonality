import { describe, expect, test } from "bun:test";
import {
  applyForm,
  captureOrder,
  changesAnything,
  digitsOnly,
  FilmDetails,
  FilmPhoto,
  hasFilm,
  NO_FILM,
  startingForm,
  suggestionsFor,
  wholeNumber,
} from "./filmDetails";

const photo = (id: number, takenAt: string, film: Partial<FilmDetails> = {}, fileName = `DSC0${id}.ARW`): FilmPhoto => ({
  id,
  takenAt,
  fileName,
  film: { ...NO_FILM, ...film },
});

/** A roll scanned out of the order the grid lists it in (newest first). */
const roll = [
  photo(3, "2026-10-01T10:00:20"),
  photo(1, "2026-10-01T10:00:00"),
  photo(2, "2026-10-01T10:00:10"),
];

const framesOf = (photos: FilmPhoto[]) => Object.fromEntries(photos.map((p) => [p.id, p.film.frame]));

describe("capture order", () => {
  test("frames go by when they were scanned, then by file name", () => {
    expect(captureOrder(roll).map((p) => p.id)).toEqual([1, 2, 3]);
    const sameSecond = [photo(7, "2026-10-01T10:00:00", {}, "DSC10.ARW"), photo(8, "2026-10-01T10:00:00", {}, "DSC9.ARW")];
    expect(captureOrder(sameSecond).map((p) => p.id)).toEqual([8, 7]);
  });
});

describe("starting the sheet", () => {
  test("a roll without frame numbers starts counting at 1", () => {
    const { form, mixed } = startingForm(roll);
    expect(form).toEqual({ stock: "", iso: "", camera: "", lens: "", frame: "1" });
    expect(mixed.size).toBe(0);
  });

  test("what the photos share is filled in, and what they differ on is left blank", () => {
    const photos = [
      photo(1, "2026-10-01T10:00:00", { stock: "Kodak Portra 400", iso: 400, camera: "Nikon FM2" }),
      photo(2, "2026-10-01T10:00:10", { stock: "Kodak Portra 400", iso: 800, camera: "Nikon FM2" }),
    ];
    const { form, mixed } = startingForm(photos);
    expect(form.stock).toBe("Kodak Portra 400");
    expect(form.camera).toBe("Nikon FM2");
    expect(form.iso).toBe("");
    expect([...mixed]).toEqual(["iso"]);
  });

  test("frames already in order show where they start; any other way, each keeps its own", () => {
    const inOrder = roll.map((p) => ({ ...p, film: { ...p.film, frame: p.id + 12 } }));
    expect(startingForm(inOrder).form.frame).toBe("13");
    const gap = roll.map((p) => ({ ...p, film: { ...p.film, frame: p.id === 3 ? 20 : p.id } }));
    const { form, mixed } = startingForm(gap);
    expect(form.frame).toBe("");
    expect(mixed.has("frame")).toBe(true);
  });

  test("one photo shows its own frame, or none", () => {
    expect(startingForm([photo(1, "2026-10-01T10:00:00", { frame: 0 })]).form.frame).toBe("0");
    expect(startingForm([photo(1, "2026-10-01T10:00:00")]).form.frame).toBe("");
  });
});

describe("saving the sheet", () => {
  test("a roll is numbered in the order it was scanned, from the first frame chosen", () => {
    const { form } = startingForm(roll);
    expect(framesOf(applyForm(roll, form, form))).toEqual({ 1: 1, 2: 2, 3: 3 });
    expect(framesOf(applyForm(roll, form, { ...form, frame: "0" }))).toEqual({ 1: 0, 2: 1, 3: 2 });
  });

  test("fields left as they were keep each photo's own; changed ones apply to all", () => {
    const photos = [
      photo(1, "2026-10-01T10:00:00", { stock: "Kodak Portra 400", camera: "Nikon FM2", frame: 5 }),
      photo(2, "2026-10-01T10:00:10", { stock: "Kodak Portra 160", camera: "Nikon FM2", frame: 9 }),
    ];
    const { form } = startingForm(photos);
    const saved = applyForm(photos, form, { ...form, lens: " Nikkor 50mm f/1.4 ", iso: "400" });
    expect(saved.map((p) => p.film.stock)).toEqual(["Kodak Portra 400", "Kodak Portra 160"]);
    expect(saved.map((p) => p.film.lens)).toEqual(["Nikkor 50mm f/1.4", "Nikkor 50mm f/1.4"]);
    expect(saved.map((p) => p.film.iso)).toEqual([400, 400]);
    expect(framesOf(saved)).toEqual({ 1: 5, 2: 9 });
  });

  test("emptying a field the photos shared clears it", () => {
    const photos = roll.map((p) => ({ ...p, film: { ...p.film, camera: "Nikon FM2", frame: p.id } }));
    const { form } = startingForm(photos);
    const saved = applyForm(photos, form, { ...form, camera: "  ", frame: "" });
    expect(saved.every((p) => p.film.camera === null && p.film.frame === null)).toBe(true);
  });

  test("one frame's number can be corrected", () => {
    const one = [photo(2, "2026-10-01T10:00:10", { stock: "Ilford HP5 Plus", frame: 2 })];
    const { form } = startingForm(one);
    const saved = applyForm(one, form, { ...form, frame: "14" });
    expect(saved[0].film).toEqual({ ...NO_FILM, stock: "Ilford HP5 Plus", frame: 14 });
  });

  test("saving what was there changes nothing", () => {
    const photos = roll.map((p) => ({ ...p, film: { ...p.film, stock: "Kodak Gold 200", frame: p.id } }));
    const { form } = startingForm(photos);
    expect(changesAnything(photos, applyForm(photos, form, form))).toBe(false);
    expect(changesAnything(photos, applyForm(photos, form, { ...form, iso: "200" }))).toBe(true);
  });
});

describe("typing", () => {
  test("numbers are whole and typed as digits", () => {
    expect(wholeNumber(" 400 ")).toBe(400);
    expect(wholeNumber("")).toBeNull();
    expect(wholeNumber("4a")).toBeNull();
    expect(digitsOnly("ISO 1,600")).toBe("1600");
  });

  test("suggestions start with what is typed, then contain it, and leave out what is typed in full", () => {
    const stocks = ["Kodak Portra 400", "Kodak Gold 200", "Fujifilm Superia 400", "Portra 160"];
    expect(suggestionsFor(stocks, "")).toEqual(stocks);
    expect(suggestionsFor(stocks, "portra")).toEqual(["Portra 160", "Kodak Portra 400"]);
    expect(suggestionsFor(stocks, "kodak gold 200")).toEqual([]);
    expect(suggestionsFor(stocks, "400", 1)).toEqual(["Kodak Portra 400"]);
  });

  test("a photo with any detail given is one of film", () => {
    expect(hasFilm(NO_FILM)).toBe(false);
    expect(hasFilm({ ...NO_FILM, frame: 0 })).toBe(true);
  });
});
