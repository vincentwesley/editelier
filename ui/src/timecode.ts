// Ticks -> SMPTE timecode. BigInt for the tick maths: ticks x frame rate passes 2^53 within minutes.
import { TICKS_PER_SECOND, type Rational, type Ticks } from "./api/types";

const TPS = BigInt(TICKS_PER_SECOND);

/** Whole frames elapsed at `t` (floor), exact for any rational rate. */
export function ticksToFrames(t: Ticks, r: Rational): number {
  return Number((BigInt(Math.trunc(t)) * BigInt(r.num)) / (BigInt(r.den) * TPS));
}

/** Drop-frame applies to the NTSC rates (29.97, 59.94) only. */
export const isDropRate = (r: Rational) => r.den === 1001 && r.num % 30000 === 0;

/**
 * HH:MM:SS:FF. Drop-frame (`;` before the frames) skips frame numbers 0 and 1 (0-3 at 59.94) at the start of each
 * minute except every tenth, so the label tracks the wall clock. NDF counts frames at the nominal rate.
 */
export function formatTimecode(t: Ticks, r: Rational, drop = isDropRate(r)): string {
  const fps = Math.round(r.num / r.den);
  let f = ticksToFrames(Math.max(0, t), r);
  const sep = drop && isDropRate(r) ? ";" : ":";
  if (sep === ";") {
    const d = (fps / 30) * 2; // frames dropped per minute
    const per10 = fps * 600 - d * 9;
    const perMin = fps * 60 - d;
    const tens = Math.floor(f / per10);
    const rem = f % per10;
    f += d * 9 * tens + (rem > d ? d * Math.floor((rem - d) / perMin) : 0);
  }
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(Math.floor(f / (fps * 3600)))}:${p(Math.floor(f / (fps * 60)) % 60)}:${p(Math.floor(f / fps) % 60)}${sep}${p(f % fps)}`;
}
