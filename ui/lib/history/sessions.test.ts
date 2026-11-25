import { describe, expect, it } from "vitest";
import { sessionLabel, sessionLabels, stepTime } from "./sessions";

const MIN = 60_000;

describe("sessionLabels", () => {
  it("labels the newest step of each session with the session's start", () => {
    const t = 1_700_000_000_000;
    // Newest first: 4 and 3 are one session, 2 and 1 (an hour earlier) another.
    const steps = [
      { id: 4, time: t + 70 * MIN },
      { id: 3, time: t + 62 * MIN },
      { id: 2, time: t + 5 * MIN },
      { id: 1, time: t },
    ];
    expect([...sessionLabels(steps)]).toEqual([
      [4, t + 62 * MIN],
      [2, t],
    ]);
  });

  it("keeps a session going while each gap is short, however long it runs", () => {
    const steps = Array.from({ length: 10 }, (_, i) => ({ id: 10 - i, time: (10 - i) * 20 * MIN }));
    expect([...sessionLabels(steps)]).toEqual([[10, 20 * MIN]]);
  });

  it("is empty without steps", () => {
    expect(sessionLabels([]).size).toBe(0);
  });
});

describe("sessionLabel", () => {
  const now = new Date(2026, 10, 24, 16, 0).getTime();
  it("says today and yesterday with a time, older days by date", () => {
    expect(sessionLabel(new Date(2026, 10, 24, 14, 2).getTime(), now)).toMatch(/^Today 14.02/);
    expect(sessionLabel(new Date(2026, 10, 23, 9, 30).getTime(), now)).toMatch(/^Yesterday 09.30/);
    const older = sessionLabel(new Date(2026, 10, 12, 9, 30).getTime(), now);
    expect(older).toMatch(/12/);
    expect(older).not.toMatch(/2026/);
    expect(sessionLabel(new Date(2025, 10, 12).getTime(), now)).toMatch(/2025/);
  });

  it("gives a step's own time, with the date on another day", () => {
    expect(stepTime(new Date(2026, 10, 24, 14, 5).getTime(), now)).toMatch(/^14.05$/);
    expect(stepTime(new Date(2026, 10, 12, 14, 5).getTime(), now)).toMatch(/12.*14.05/);
  });
});
