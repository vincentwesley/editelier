// Timeline maths and format data shared by the UI and the mock backend.
import { TICKS_PER_SECOND, type AspectId, type Canvas, type Clip, type PresetId, type Rational, type Ticks, type Timeline } from "./api/types";

/** Ticks per frame; exact for every common rate (that is why the tick rate is 254,016,000,000). */
export const frameTicks = (r: Rational): Ticks => (TICKS_PER_SECOND * r.den) / r.num;
export const quantize = (t: Ticks, r: Rational): Ticks => Math.round(t / frameTicks(r)) * frameTicks(r);
export const clipLength = (c: { source_in: Ticks; source_out: Ticks }): Ticks => c.source_out - c.source_in;

/** Start of each main-track clip (magnetic: packed from zero). */
export function mainStarts(main: Clip[]): Ticks[] {
  const out: Ticks[] = [];
  let t = 0;
  for (const c of main) {
    out.push(t);
    t += clipLength(c);
  }
  return out;
}

export function timelineEnd(tl: Timeline): Ticks {
  const main = tl.main.reduce((n, c) => n + clipLength(c), 0);
  const titles = Math.max(0, ...tl.titles.map((c) => c.start + c.duration));
  const music = Math.max(0, ...tl.music.map((c) => c.start + clipLength(c)));
  return Math.max(main, titles, music);
}

/** Index of the main-track clip covering `t` (start <= t < end), or -1. Binary search: 200+ clips stay cheap. */
export function mainIndexAt(main: Clip[], starts: Ticks[], t: Ticks): number {
  let lo = 0;
  let hi = main.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (t < starts[mid]!) hi = mid - 1;
    else if (t >= starts[mid]! + clipLength(main[mid]!)) lo = mid + 1;
    else return mid;
  }
  return -1;
}

/** Every edit point on the timeline, sorted (for Up/Down and snapping). */
export function editPoints(tl: Timeline): Ticks[] {
  const s = new Set<Ticks>([0]);
  let t = 0;
  for (const c of tl.main) s.add((t += clipLength(c)));
  for (const c of tl.titles) s.add(c.start).add(c.start + c.duration);
  for (const c of tl.music) s.add(c.start).add(c.start + clipLength(c));
  return [...s].sort((a, b) => a - b);
}

export const NTSC: Rational = { num: 30000, den: 1001 };

export const ASPECTS: Record<AspectId, { label: string; hint: string; canvas: Canvas; preset: PresetId }> = {
  "16:9": { label: "YouTube video", hint: "16:9 landscape", canvas: { aspect: "16:9", width: 1920, height: 1080, frame_rate: NTSC }, preset: "youtube-1080" },
  "9:16": { label: "Short, Reel or TikTok", hint: "9:16 vertical", canvas: { aspect: "9:16", width: 1080, height: 1920, frame_rate: NTSC }, preset: "tiktok" },
  "1:1": { label: "Square post", hint: "1:1", canvas: { aspect: "1:1", width: 1080, height: 1080, frame_rate: NTSC }, preset: "master" },
  "4:5": { label: "Instagram post", hint: "4:5 portrait", canvas: { aspect: "4:5", width: 1080, height: 1350, frame_rate: NTSC }, preset: "instagram-45" },
};

export const PRESETS: Record<PresetId, { label: string; summary: string; aspect: AspectId | null }> = {
  "youtube-1080": { label: "YouTube 1080p", summary: "1920×1080, H.264 + AAC, up to 60 fps", aspect: "16:9" },
  "youtube-4k": { label: "YouTube 4K", summary: "3840×2160, H.264 + AAC, high bitrate", aspect: "16:9" },
  tiktok: { label: "TikTok", summary: "1080×1920 vertical, H.264 + AAC", aspect: "9:16" },
  "instagram-reel": { label: "Instagram Reel", summary: "1080×1920 vertical, H.264 + AAC", aspect: "9:16" },
  "instagram-45": { label: "Instagram 4:5", summary: "1080×1350 portrait, H.264 + AAC", aspect: "4:5" },
  master: { label: "Master file", summary: "Project size and frame rate, highest quality", aspect: null },
};
