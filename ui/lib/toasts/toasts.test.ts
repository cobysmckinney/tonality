import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { describe as describeError, MAX_TOASTS, NOTICE_MS, Toasts } from "./toasts.svelte";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

const texts = (t: Toasts) => t.list.map((x) => x.text);

describe("toasts", () => {
  it("puts the context before what went wrong", () => {
    const t = new Toasts();
    t.error("Couldn't open DSC_0042.NEF", "unsupported camera");
    t.error("Couldn't save", new Error("disk full"));
    t.error("Something broke");
    expect(texts(t)).toEqual([
      "Couldn't open DSC_0042.NEF: unsupported camera",
      "Couldn't save: disk full",
      "Something broke",
    ]);
  });

  it("sends errors, not notices, to the log", () => {
    const log = vi.fn();
    const t = new Toasts({ log });
    t.notice("Moved 2 photos to the Trash");
    t.error("Couldn't move", "denied");
    expect(log).toHaveBeenCalledExactlyOnceWith("Couldn't move: denied");
  });

  it("coalesces the same message into one, with a count", () => {
    const t = new Toasts();
    const a = t.error("Couldn't render", "lost device");
    const b = t.error("Couldn't render", "lost device");
    t.error("Couldn't render", "out of memory");
    t.notice("Couldn't render: lost device"); // a notice isn't the same toast as an error
    expect(a).toBe(b);
    expect(t.list.map((x) => [x.kind, x.count])).toEqual([
      ["error", 2],
      ["error", 1],
      ["notice", 1],
    ]);
  });

  it("dismisses a notice after a few seconds, and keeps errors", () => {
    const t = new Toasts();
    t.notice("Reset 3 photos to as-shot");
    t.error("Couldn't export", "no GPU");
    vi.advanceTimersByTime(NOTICE_MS - 1);
    expect(t.list).toHaveLength(2);
    vi.advanceTimersByTime(1);
    expect(texts(t)).toEqual(["Couldn't export: no GPU"]);
    vi.advanceTimersByTime(60_000);
    expect(t.list).toHaveLength(1);
  });

  it("gives a repeated notice its full time again", () => {
    const t = new Toasts();
    t.notice("Saved");
    vi.advanceTimersByTime(3000);
    t.notice("Saved");
    vi.advanceTimersByTime(3000);
    expect(t.list).toHaveLength(1);
    expect(t.list[0].count).toBe(2);
    vi.advanceTimersByTime(NOTICE_MS - 3000);
    expect(t.list).toHaveLength(0);
  });

  it("holds notices while hovered, then gives them the time they had left", () => {
    const t = new Toasts();
    t.notice("Added 2 photos to “Picks”");
    vi.advanceTimersByTime(3000);
    t.hold();
    vi.advanceTimersByTime(60_000);
    expect(t.list).toHaveLength(1);
    // One arriving while held waits too.
    t.notice("Another");
    vi.advanceTimersByTime(60_000);
    expect(t.list).toHaveLength(2);
    t.release();
    vi.advanceTimersByTime(NOTICE_MS - 3000);
    expect(texts(t)).toEqual(["Another"]);
    vi.advanceTimersByTime(3000);
    expect(t.list).toHaveLength(0);
  });

  it("shows at most a few, letting the oldest notice go first", () => {
    const t = new Toasts();
    t.error("e1");
    t.notice("n1");
    t.error("e2");
    t.notice("n2");
    t.error("e3");
    expect(texts(t)).toEqual(["e1", "e2", "n2", "e3"]);
    t.error("e4");
    t.error("e5");
    expect(texts(t)).toEqual(["e2", "e3", "e4", "e5"]);
    expect(t.list).toHaveLength(MAX_TOASTS);
    // The notice that made room has no timer left behind.
    vi.advanceTimersByTime(NOTICE_MS);
    expect(t.list).toHaveLength(MAX_TOASTS);
  });

  it("dismisses one, or the newest", () => {
    const t = new Toasts();
    const a = t.error("a");
    t.notice("b");
    t.error("c");
    t.dismiss(a);
    t.dismissNewest();
    expect(texts(t)).toEqual(["b"]);
    t.dismissNewest();
    t.dismissNewest(); // nothing left: no-op
    expect(t.list).toHaveLength(0);
  });
});

describe("describe", () => {
  it("turns anything thrown into text", () => {
    expect(describeError("plain")).toBe("plain");
    expect(describeError(new TypeError("bad"))).toBe("bad");
    expect(describeError({ code: 3 })).toBe('{"code":3}');
  });
});
