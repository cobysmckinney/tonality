import { describe, expect, test } from "bun:test";
import { focusToRestore } from "./focus";

const page = { isConnected: true };
const button = { isConnected: true };

describe("when a menu closes the focus", () => {
  test("goes back to the button that opened it", () => {
    expect(focusToRestore(button, page, page)).toBe(button);
    expect(focusToRestore(button, null, page)).toBe(button);
  });

  test("stays where the chosen item put it", () => {
    const dialogButton = { isConnected: true };
    expect(focusToRestore(button, dialogButton, page)).toBeNull();
  });

  test("is left alone when the button has gone from the page", () => {
    expect(focusToRestore({ isConnected: false }, page, page)).toBeNull();
  });

  test("is left alone when nothing had it before", () => {
    expect(focusToRestore(null, page, page)).toBeNull();
    expect(focusToRestore(page, page, page)).toBeNull();
  });
});
