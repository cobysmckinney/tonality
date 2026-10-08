import { describe, expect, test } from "bun:test";
import { canStartExport, canStartImport } from "./sheets";

const nothing = { importState: null, exportState: null, confirmRequest: null };

describe("one sheet at a time", () => {
  test("either can start when nothing is open", () => {
    expect(canStartImport(nothing)).toBe(true);
    expect(canStartExport(nothing)).toBe(true);
  });

  test("an import doesn't open over the export sheet or a question", () => {
    expect(canStartImport({ ...nothing, exportState: { phase: "setup" } })).toBe(false);
    expect(canStartImport({ ...nothing, exportState: { phase: "exporting" } })).toBe(false);
    expect(canStartImport({ ...nothing, confirmRequest: { title: "Delete?" } })).toBe(false);
  });

  test("an import waits for one that is looking or copying, but can replace one being reviewed", () => {
    expect(canStartImport({ ...nothing, importState: { phase: "scanning" } })).toBe(false);
    expect(canStartImport({ ...nothing, importState: { phase: "importing" } })).toBe(false);
    expect(canStartImport({ ...nothing, importState: { phase: "review" } })).toBe(true);
  });

  test("an export doesn't open over the import sheet", () => {
    expect(canStartExport({ ...nothing, importState: { phase: "review" } })).toBe(false);
    expect(canStartExport({ ...nothing, exportState: { phase: "setup" } })).toBe(false);
  });
});
