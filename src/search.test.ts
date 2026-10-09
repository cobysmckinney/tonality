import { describe, expect, test } from "bun:test";
import { applySuggestion, parseDate, parseSearch, SearchFacets, SearchTerm, searching, suggest } from "./search";

const term = (change: Partial<SearchTerm>): SearchTerm => ({
  field: "any",
  text: null,
  exact: false,
  from: null,
  until: null,
  month: null,
  ...change,
});

describe("reading a search", () => {
  test("each word is a term of its own, found anywhere", () => {
    expect(parseSearch("  portra   fm2 ").terms).toEqual([term({ text: "portra" }), term({ text: "fm2" })]);
    expect(parseSearch("").terms).toEqual([]);
    expect(searching("   ")).toBe(false);
    expect(searching("portra")).toBe(true);
  });

  test("quotes keep words together and match the whole name", () => {
    expect(parseSearch('"Kodak Portra 400" r5').terms).toEqual([
      term({ text: "Kodak Portra 400", exact: true }),
      term({ text: "r5" }),
    ]);
    // Still being typed: the open quote runs to the end.
    expect(parseSearch('"Kodak Por').terms).toEqual([term({ text: "Kodak Por", exact: true })]);
  });

  test("a word can be kept to the camera, lens, film or file name", () => {
    expect(parseSearch('camera:"Canon EOS R5" Lens:50mm stock:hp5 file:IMG_01').terms).toEqual([
      term({ field: "camera", text: "Canon EOS R5", exact: true }),
      term({ field: "lens", text: "50mm" }),
      term({ field: "film", text: "hp5" }),
      term({ field: "name", text: "IMG_01" }),
    ]);
  });

  test("something that only looks like a field is a word", () => {
    expect(parseSearch("f/1.4 iso:400").terms).toEqual([term({ text: "f/1.4" }), term({ text: "iso:400" })]);
  });

  test("a field with nothing after it yet asks for nothing", () => {
    expect(parseSearch("camera: portra").terms).toEqual([term({ text: "portra" })]);
    expect(parseSearch('film:""').terms).toEqual([]);
  });

  test("a year, month or date typed as a word is also when a photo was taken", () => {
    expect(parseSearch("2026").terms).toEqual([term({ text: "2026", from: "2026", until: "2027" })]);
    expect(parseSearch("oct").terms).toEqual([term({ text: "oct", month: 10 })]);
    // Two words, both to match: October of 2026.
    expect(parseSearch("october 2026").terms).toEqual([
      term({ text: "october", month: 10 }),
      term({ text: "2026", from: "2026", until: "2027" }),
    ]);
  });

  test("date: takes dates only", () => {
    expect(parseSearch("date:2026-10").terms).toEqual([term({ from: "2026-10", until: "2026-11" })]);
    expect(parseSearch('date:"Oct 2026"').terms).toEqual([term({ from: "2026-10", until: "2026-11" })]);
    // Not a date, so nothing matches, rather than everything.
    expect(parseSearch("date:soon").terms).toEqual([term({})]);
  });
});

describe("reading a date", () => {
  test("a year, month or day runs until the next one starts", () => {
    expect(parseDate("2026")).toEqual({ from: "2026", until: "2027", month: null });
    expect(parseDate("2026-12")).toEqual({ from: "2026-12", until: "2027-01", month: null });
    expect(parseDate("2026/9")).toEqual({ from: "2026-09", until: "2026-10", month: null });
    expect(parseDate("2026-02-28")).toEqual({ from: "2026-02-28", until: "2026-03-01", month: null });
    expect(parseDate("2024-12-31")).toEqual({ from: "2024-12-31", until: "2025-01-01", month: null });
  });

  test("months by name, with a year or without", () => {
    expect(parseDate("September")).toEqual({ from: null, until: null, month: 9 });
    expect(parseDate("sept")).toEqual({ from: null, until: null, month: 9 });
    expect(parseDate("Mar.")).toEqual({ from: null, until: null, month: 3 });
    expect(parseDate("June 2025")).toEqual({ from: "2025-06", until: "2025-07", month: null });
    expect(parseDate("jun, 2025")).toEqual({ from: "2025-06", until: "2025-07", month: null });
  });

  test("a span between two dates, either end left open", () => {
    expect(parseDate("2026-06..2026-08")).toEqual({ from: "2026-06", until: "2026-09", month: null });
    expect(parseDate("2025..")).toEqual({ from: "2025", until: null, month: null });
    expect(parseDate("..2026-10-08")).toEqual({ from: null, until: "2026-10-09", month: null });
  });

  test("anything else is not a date", () => {
    for (const text of ["", "ma", "2026-13", "2026-02-30", "26", "12345", "..", "2026..soon", "R5", "0005"]) {
      expect(parseDate(text)).toBeNull();
    }
  });
});

const facets: SearchFacets = {
  cameras: [
    { value: "Canon EOS R5", count: 40 },
    { value: "Nikon FM2", count: 36 },
    { value: "Canon EOS R50", count: 2 },
  ],
  lenses: [{ value: "Nikkor 50mm f/1.4", count: 36 }],
  films: [
    { value: "Kodak Portra 400", count: 24 },
    { value: "Ilford HP5 Plus", count: 12 },
  ],
  months: [
    { value: "2026-10", count: 50 },
    { value: "2026-09", count: 20 },
    { value: "2025-10", count: 8 },
  ],
};

const labels = (text: string) => suggest(text, facets).map((s) => s.label);

describe("suggesting", () => {
  test("with nothing typed, a few of each kind", () => {
    expect(labels("")).toEqual(["Canon EOS R5", "Nikon FM2", "Kodak Portra 400", "Ilford HP5 Plus", "Nikkor 50mm f/1.4", "2026", "2025"]);
  });

  test("names that start with the word come before ones that only hold it", () => {
    expect(labels("n")).toEqual(["Nikon FM2", "Nikkor 50mm f/1.4", "Canon EOS R5", "Canon EOS R50"]);
    expect(labels("por")).toEqual(["Kodak Portra 400"]);
    expect(labels("r5")).toEqual(["Canon EOS R5", "Canon EOS R50"]);
  });

  test("dates by month name, year or number", () => {
    expect(labels("oct")).toEqual(["October 2026", "October 2025"]);
    expect(labels("2026-0")).toEqual(["September 2026"]);
    expect(labels("2025")).toEqual(["2025", "October 2025"]);
  });

  test("a field keeps the suggestions to its kind", () => {
    expect(labels("camera:")).toEqual(["Canon EOS R5", "Nikon FM2", "Canon EOS R50"]);
    expect(labels("film:il")).toEqual(["Ilford HP5 Plus"]);
    expect(labels('lens:"nik')).toEqual(["Nikkor 50mm f/1.4"]);
    expect(labels("name:img")).toEqual([]);
  });

  test("what is already searched for isn't offered again", () => {
    expect(labels('film:"Kodak Portra 400" ')).not.toContain("Kodak Portra 400");
  });

  test("choosing one replaces the word being typed", () => {
    const [portra] = suggest("canon por", facets);
    expect(applySuggestion("canon por", portra)).toBe('canon film:"Kodak Portra 400" ');
    const [october] = suggest("oct", facets);
    expect(applySuggestion("oct", october)).toBe("date:2026-10 ");
    const [r5] = suggest("camera:", facets);
    expect(applySuggestion("portra camera:", r5)).toBe('portra camera:"Canon EOS R5" ');
    // After a space, it's added.
    expect(applySuggestion("portra ", r5)).toBe('portra camera:"Canon EOS R5" ');
  });
});
