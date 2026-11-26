import { describe, expect, it } from "vitest";
import { TICKS_PER_SECOND as S } from "../src/api/types";
import { frameTicks } from "../src/model";
import { formatTimecode, ticksToFrames } from "../src/timecode";

const NTSC = { num: 30000, den: 1001 };
const at = (frame: number, r = NTSC) => frame * frameTicks(r);

describe("ticks per frame", () => {
  it("is an exact integer for common rates", () => {
    for (const r of [NTSC, { num: 24, den: 1 }, { num: 25, den: 1 }, { num: 30, den: 1 }, { num: 60000, den: 1001 }, { num: 24000, den: 1001 }])
      expect(Number.isInteger(frameTicks(r))).toBe(true);
    expect(frameTicks(NTSC)).toBe(8_475_667_200);
  });
});

describe("formatTimecode at 29.97 drop-frame", () => {
  it.each([
    [0, "00:00:00;00"],
    [29, "00:00:00;29"],
    [1799, "00:00:59;29"],
    [1800, "00:01:00;02"], // frames ;00 and ;01 are skipped at the minute
    [3598, "00:02:00;02"],
    [17982, "00:10:00;00"], // every tenth minute keeps ;00
    [107892, "01:00:00;00"], // one real hour
  ])("frame %i is %s", (f, want) => expect(formatTimecode(at(f), NTSC)).toBe(want));

  it("tracks the wall clock: one hour of ticks reads 01:00:00;00", () => {
    expect(formatTimecode(3600 * S, NTSC)).toBe("01:00:00;00");
  });
});

describe("formatTimecode at 29.97 non-drop", () => {
  it.each([
    [1800, "00:01:00:00"],
    [17982, "00:09:59:12"],
    [107892, "00:59:56:12"],
  ])("frame %i is %s", (f, want) => expect(formatTimecode(at(f), NTSC, false)).toBe(want));
});

describe("formatTimecode at integer rates", () => {
  it("uses colons and the nominal rate", () => {
    const r25 = { num: 25, den: 1 };
    expect(formatTimecode(at(25 * 61 + 3, r25), r25)).toBe("00:01:01:03");
  });
});

describe("ticksToFrames", () => {
  it("floors, stays exact past 2^53 intermediate values, and handles long timelines", () => {
    expect(ticksToFrames(frameTicks(NTSC) - 1, NTSC)).toBe(0);
    expect(ticksToFrames(9 * 3600 * S, NTSC)).toBe(Math.floor((9 * 3600 * 30000) / 1001));
  });
});
