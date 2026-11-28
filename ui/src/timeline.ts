// Timeline on one <canvas>, drawn only for the visible span (binary search into the magnetic main track), so 200+ clips
// scroll at display rate. Native horizontal scrolling: a wide spacer scrolls under a sticky, viewport-sized canvas.
import { TICKS_PER_SECOND as S, type Project, type Ticks } from "./api/types";
import { $, h } from "./dom";
import { clipLength, editPoints, mainIndexAt, mainStarts, quantize, timelineEnd } from "./model";
import { edit, state, subscribe, update } from "./state";
import { formatTimecode } from "./timecode";

const RULER = 28;
const TITLES = 36;
const MAIN = 64;
const MUSIC = 48;
const Y = { titles: RULER, main: RULER + TITLES, music: RULER + TITLES + MAIN, end: RULER + TITLES + MAIN + MUSIC };
const EDGE = 6; // px either side of a clip edge that grabs a trim handle
const SNAP = 8; // px snapping distance
export const ZOOM = { min: 2, max: 2000 };

type Track = "titles" | "main" | "music";
type Hit = { track: Track; id: string; start: Ticks; end: Ticks; index: number; edge: "in" | "out" | null };
type Drag =
  | { kind: "scrub" }
  | { kind: "trim"; hit: Hit; x0: number; t: Ticks }
  | { kind: "move"; hit: Hit; x0: number; moved: boolean; t: Ticks; index: number };

let scroller: HTMLElement;
let spacer: HTMLElement;
let canvas: HTMLCanvasElement;
let ctx: CanvasRenderingContext2D;
let drag: Drag | null = null;
let dropMedia: string | null = null;
let dropAt: { track: Track; t: Ticks; index: number } | null = null;
let snapLine: Ticks | null = null;
let points: { project: Project | null; list: Ticks[] } = { project: null, list: [] };
const images = new Map<string, HTMLImageElement>();

const project = () => state.snap?.project ?? null;
const toX = (t: Ticks) => (t / S) * state.pxPerSec - scroller.scrollLeft;
const toT = (x: number) => Math.max(0, ((x + scroller.scrollLeft) / state.pxPerSec) * S);
const localX = (e: { clientX: number }) => e.clientX - canvas.getBoundingClientRect().left;
const localY = (e: { clientY: number }) => e.clientY - canvas.getBoundingClientRect().top;
const trackAt = (y: number): Track | null => (y < Y.titles ? null : y < Y.main ? "titles" : y < Y.music ? "main" : y < Y.end ? "music" : null);

/** Snaps `t` to the nearest edit point, mark or the playhead within SNAP px. Alt held = no snapping. */
function snap(t: Ticks, alt: boolean, skipPlayhead = false): Ticks {
  snapLine = null;
  const p = project();
  if (!p || !state.snapping || alt) return p ? quantize(t, p.canvas.frame_rate) : t;
  if (points.project !== p) points = { project: p, list: editPoints(p.timeline) };
  const cands = [...points.list, state.markIn, state.markOut, skipPlayhead ? null : state.playhead].filter((v): v is Ticks => v !== null);
  let best = quantize(t, p.canvas.frame_rate);
  let dist = (SNAP / state.pxPerSec) * S;
  for (const c of cands)
    if (Math.abs(c - t) < dist) {
      dist = Math.abs(c - t);
      best = c;
      snapLine = c;
    }
  return best;
}

function hitTest(x: number, y: number): Hit | null {
  const p = project();
  const track = trackAt(y);
  if (!p || !track) return null;
  const t = toT(x);
  const tl = p.timeline;
  const mk = (id: string, start: Ticks, end: Ticks, index: number): Hit => {
    const xs = toX(start);
    const xe = toX(end);
    const roomy = xe - xs > EDGE * 3;
    return { track, id, start, end, index, edge: roomy && Math.abs(x - xs) <= EDGE ? "in" : roomy && Math.abs(x - xe) <= EDGE ? "out" : null };
  };
  if (track === "main") {
    const starts = mainStarts(tl.main);
    // Near an edge, prefer the clip whose edge it is.
    const i = mainIndexAt(tl.main, starts, Math.max(0, t));
    if (i < 0) return null;
    return mk(tl.main[i]!.id, starts[i]!, starts[i]! + clipLength(tl.main[i]!), i);
  }
  const list = track === "titles" ? tl.titles.map((c) => [c.id, c.start, c.start + c.duration] as const) : tl.music.map((c) => [c.id, c.start, c.start + clipLength(c)] as const);
  for (let i = list.length - 1; i >= 0; i--) {
    const [id, s, e] = list[i]!;
    if (t >= s && t < e) return mk(id, s, e, i);
  }
  return null;
}

/** Main-track insertion index nearest to `t` (a boundary between clips). */
function insertIndex(t: Ticks): { index: number; t: Ticks } {
  const p = project()!;
  const starts = mainStarts(p.timeline.main);
  let best = { index: 0, t: 0 };
  for (let i = 0; i <= starts.length; i++) {
    const b = i < starts.length ? starts[i]! : starts.length ? starts.at(-1)! + clipLength(p.timeline.main.at(-1)!) : 0;
    if (Math.abs(b - t) <= Math.abs(best.t - t)) best = { index: i, t: b };
  }
  return best;
}

function colors() {
  const cs = getComputedStyle(document.documentElement);
  const v = (n: string) => cs.getPropertyValue(n).trim();
  return { bg: v("--bg"), elev: v("--bg-elev"), fg: v("--fg"), muted: v("--fg-muted"), soft: v("--border-soft"), border: v("--border"), accent: v("--accent"), fx2: v("--fx-2"), fx3: v("--fx-3"), warn: v("--warn"), focus: v("--focus") };
}

function hash(s: string): number {
  let x = 2166136261;
  for (const ch of s) x = Math.imul(x ^ ch.charCodeAt(0), 16777619);
  return x >>> 0;
}

function thumb(url: string): HTMLImageElement | null {
  let img = images.get(url);
  if (!img) {
    img = new Image();
    img.onload = () => draw();
    img.src = url;
    images.set(url, img);
  }
  return img.complete && img.naturalWidth ? img : null;
}

function clipBox(x0: number, x1: number, y: number, hgt: number, fill: string, label: string, selected: boolean, c: ReturnType<typeof colors>) {
  const x = Math.max(x0, -2);
  const w = Math.min(x1, canvas.clientWidth + 2) - x;
  if (w <= 0) return;
  ctx.globalAlpha = 0.28;
  ctx.fillStyle = fill;
  ctx.fillRect(x, y + 3, w, hgt - 6);
  ctx.globalAlpha = 1;
  ctx.strokeStyle = selected ? c.fg : fill;
  ctx.lineWidth = selected ? 2 : 1;
  ctx.strokeRect(x0 + 0.5, y + 3.5, x1 - x0 - 1, hgt - 7);
  if (selected) {
    ctx.fillStyle = c.fg;
    ctx.fillRect(x0, y + 3, 4, hgt - 6);
    ctx.fillRect(x1 - 4, y + 3, 4, hgt - 6);
  }
  if (w > 24) {
    ctx.save();
    ctx.beginPath();
    ctx.rect(x, y, w - 4, hgt);
    ctx.clip();
    ctx.fillStyle = c.fg;
    ctx.fillText(label, Math.max(x0, 0) + 8, y + 18);
    ctx.restore();
  }
}

function rulerStep(): number {
  for (const s of [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800]) if (s * state.pxPerSec >= 90) return s;
  return 3600;
}

export function draw(): void {
  if (!ctx) return;
  const p = project();
  const W = canvas.clientWidth;
  const c = colors();
  const dpr = devicePixelRatio;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.fillStyle = c.bg;
  ctx.fillRect(0, 0, W, Y.end);
  // Track lanes.
  ctx.fillStyle = c.elev;
  ctx.fillRect(0, 0, W, RULER);
  ctx.fillStyle = c.soft;
  for (const y of [Y.titles, Y.main, Y.music, Y.end - 1]) ctx.fillRect(0, y, W, 1);
  if (!p) return;
  const fr = p.canvas.frame_rate;
  const t0 = toT(0);
  const t1 = toT(W);
  ctx.font = "500 12px Inter, system-ui, sans-serif";
  ctx.textBaseline = "alphabetic";

  // Ruler.
  const step = rulerStep();
  ctx.fillStyle = c.muted;
  for (let s = Math.floor(t0 / S / step) * step; s * S <= t1; s += step) {
    const x = Math.round(toX(s * S)) + 0.5;
    ctx.fillRect(x, RULER - 10, 1, 10);
    ctx.font = "11px 'JetBrains Mono', ui-monospace, monospace";
    ctx.fillText(formatTimecode(s * S, fr).slice(3), x + 4, 13);
  }
  ctx.font = "500 12px Inter, system-ui, sans-serif";
  if (state.markIn !== null || state.markOut !== null) {
    const a = toX(state.markIn ?? 0);
    const b = toX(state.markOut ?? timelineEnd(p.timeline));
    ctx.fillStyle = c.focus;
    ctx.globalAlpha = 0.1; // the marked range shades every track, so it reads as "this part"
    ctx.fillRect(a, RULER, b - a, Y.end - RULER);
    ctx.globalAlpha = 0.6;
    ctx.fillRect(a, RULER - 6, b - a, 6);
    ctx.globalAlpha = 1;
    ctx.fillRect(a, RULER - 12, 2, 12);
    ctx.fillRect(b - 2, RULER - 12, 2, 12);
  }

  const tl = p.timeline;
  const media = new Map(p.media.map((m) => [m.id, m]));
  // Empty-state hint in the video lane (research 03 §6): the next step, not just a blank track.
  if (!tl.main.length && !dropAt) {
    ctx.fillStyle = c.muted;
    ctx.fillText(p.media.length ? "Drag a clip here, or press + next to it in Media" : "Import clips in the Media panel, then drag them here", 16, Y.main + MAIN / 2 + 4);
  }
  // Main track: binary search to the first visible clip, stop past the right edge.
  const starts = mainStarts(tl.main);
  let i = Math.max(0, mainIndexAt(tl.main, starts, t0));
  if (tl.main.length && t0 >= starts.at(-1)! + clipLength(tl.main.at(-1)!)) i = tl.main.length;
  for (; i < tl.main.length && starts[i]! <= t1; i++) {
    const clip = tl.main[i]!;
    const m = media.get(clip.media_id);
    const x0 = toX(starts[i]!);
    const x1 = toX(starts[i]! + clipLength(clip));
    const moving = drag?.kind === "move" && drag.moved && drag.hit.id === clip.id;
    if (moving) ctx.globalAlpha = 0.4;
    clipBox(x0, x1, Y.main, MAIN, c.accent, m?.name ?? "Missing media", state.selection === clip.id, c);
    const img = m?.thumbnail ? thumb(m.thumbnail) : null;
    if (img && x1 - x0 > 72) {
      const th = MAIN - 32;
      ctx.drawImage(img, Math.max(x0, 0) + 6, Y.main + 24, (th * 16) / 9, th);
    }
    ctx.globalAlpha = 1;
  }
  for (const t of tl.titles) {
    if (t.start > t1 || t.start + t.duration < t0) continue;
    let s = t.start;
    if (drag?.kind === "move" && drag.moved && drag.hit.id === t.id) s = drag.t;
    clipBox(toX(s), toX(s + t.duration), Y.titles, TITLES, c.fx3, t.text || "Title", state.selection === t.id, c);
  }
  for (const a of tl.music) {
    const len = clipLength(a);
    let s = a.start;
    if (drag?.kind === "move" && drag.moved && drag.hit.id === a.id) s = drag.t;
    if (s > t1 || s + len < t0) continue;
    const x0 = toX(s);
    const x1 = toX(s + len);
    // Waveform placeholder until the core sends peaks: stable pseudo-random bars, scaled by gain.
    ctx.fillStyle = c.fx2;
    const gain = Math.min(1.6, 10 ** (a.gain_db / 20));
    const mid = Y.music + MUSIC / 2 + 4;
    for (let x = Math.max(x0, 0) + 2; x < Math.min(x1, W) - 2; x += 3) {
      const k = Math.floor((x + scroller.scrollLeft) / 3);
      const amp = ((hash(a.id + k) % 100) / 100) * 0.7 + 0.2;
      const hgt = Math.min(MUSIC - 22, amp * (MUSIC - 22) * gain);
      ctx.fillRect(x, mid - hgt / 2, 2, hgt);
    }
    clipBox(x0, x1, Y.music, MUSIC, c.fx2, media.get(a.media_id)?.name ?? "Music", state.selection === a.id, c);
    // Fade ramps.
    ctx.strokeStyle = c.fg;
    ctx.lineWidth = 1;
    ctx.beginPath();
    if (a.fade_in) {
      ctx.moveTo(x0, Y.music + MUSIC - 4);
      ctx.lineTo(toX(s + a.fade_in), Y.music + 4);
    }
    if (a.fade_out) {
      ctx.moveTo(toX(s + len - a.fade_out), Y.music + 4);
      ctx.lineTo(x1, Y.music + MUSIC - 4);
    }
    ctx.stroke();
  }

  // Trim preview: outline the clip's new extent.
  if (drag?.kind === "trim") {
    const { hit } = drag;
    const a = hit.edge === "in" ? drag.t : hit.start;
    const b = hit.edge === "out" ? drag.t : hit.end;
    const y = Y[hit.track];
    ctx.strokeStyle = c.focus;
    ctx.lineWidth = 2;
    ctx.strokeRect(toX(a), y + 2, toX(b) - toX(a), (hit.track === "main" ? MAIN : hit.track === "titles" ? TITLES : MUSIC) - 4);
  }
  // Insertion marker (reorder or drop from the bin).
  const ins = drag?.kind === "move" && drag.moved && drag.hit.track === "main" ? insertIndex(drag.t) : dropAt;
  if (ins && (!("track" in ins) || ins.track === "main")) {
    ctx.fillStyle = c.focus;
    ctx.fillRect(toX(ins.t) - 1, Y.main, 3, MAIN);
  }
  if (dropAt && dropAt.track !== "main") {
    ctx.fillStyle = c.focus;
    ctx.fillRect(toX(dropAt.t) - 1, Y[dropAt.track], 3, MUSIC);
  }
  // Snapping indicator.
  if (snapLine !== null && drag) {
    const x = Math.round(toX(snapLine)) + 0.5;
    ctx.strokeStyle = c.warn;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    ctx.moveTo(x, RULER);
    ctx.lineTo(x, Y.end);
    ctx.stroke();
    ctx.setLineDash([]);
  }
  // Playhead.
  const px = Math.round(toX(state.playhead));
  ctx.fillStyle = c.accent;
  ctx.fillRect(px - 1, 0, 2, Y.end);
  ctx.beginPath();
  ctx.moveTo(px - 6, 0);
  ctx.lineTo(px + 6, 0);
  ctx.lineTo(px, 8);
  ctx.fill();
}

let raf = 0;
const redraw = () => {
  if (!raf) raf = requestAnimationFrame(() => ((raf = 0), draw()));
};

function sizeSpacer(): void {
  const p = project();
  const end = p ? timelineEnd(p.timeline) : 0;
  spacer.style.width = `${Math.max(scroller.clientWidth, (end / S + 30) * state.pxPerSec)}px`;
}

function resize(): void {
  const w = scroller.clientWidth;
  canvas.width = Math.round(w * devicePixelRatio);
  canvas.height = Math.round(Y.end * devicePixelRatio);
  canvas.style.width = `${w}px`;
  canvas.style.height = `${Y.end}px`;
  sizeSpacer();
  draw();
}

/** Zoom keeping the time under `anchorX` (canvas px; default the playhead) still. */
export function setZoom(px: number, anchorX?: number): void {
  const next = Math.min(ZOOM.max, Math.max(ZOOM.min, px));
  const ax = anchorX ?? Math.min(Math.max(toX(state.playhead), 0), scroller.clientWidth);
  const t = toT(ax);
  state.pxPerSec = next;
  sizeSpacer();
  scroller.scrollLeft = (t / S) * next - ax;
  update({ pxPerSec: next });
}

export function zoomFit(): void {
  const p = project();
  const secs = Math.max(10, p ? timelineEnd(p.timeline) / S : 0);
  setZoom((scroller.clientWidth - 40) / secs, 0);
  scroller.scrollLeft = 0;
}

export function setDropMedia(id: string | null): void {
  dropMedia = id;
  if (!id) dropAt = null;
  redraw();
}

function syncList(p: Project | null): void {
  const list = $("#tl-clips");
  if (!p) return list.replaceChildren();
  const fr = p.canvas.frame_rate;
  const names = new Map(p.media.map((m) => [m.id, m.name]));
  const starts = mainStarts(p.timeline.main);
  const n = p.timeline.main.length;
  list.replaceChildren(
    ...p.timeline.main.map((c, i) => h("li", { "data-id": c.id }, `Clip ${i + 1} of ${n}: ${names.get(c.media_id) ?? "missing"}, ${formatTimecode(clipLength(c), fr)} long, at ${formatTimecode(starts[i]!, fr)}`)),
    ...p.timeline.titles.map((t) => h("li", { "data-id": t.id }, `Title: ${t.text}, at ${formatTimecode(t.start, fr)}`)),
    ...p.timeline.music.map((a) => h("li", { "data-id": a.id }, `Music: ${names.get(a.media_id) ?? "missing"}, at ${formatTimecode(a.start, fr)}`)),
  );
}

export function initTimeline(seek: (t: Ticks) => void): void {
  scroller = $("#tl-scroll");
  spacer = $("#tl-spacer");
  canvas = $<HTMLCanvasElement>("#tl-canvas");
  ctx = canvas.getContext("2d", { alpha: false })!;
  new ResizeObserver(resize).observe(scroller);
  new MutationObserver(redraw).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  scroller.addEventListener("scroll", redraw, { passive: true });

  let lastProject: Project | null = null;
  let lastPx = 0;
  subscribe((s) => {
    const p = s.snap?.project ?? null;
    if (p !== lastProject || s.pxPerSec !== lastPx) {
      if (p !== lastProject) syncList(p);
      lastProject = p;
      lastPx = s.pxPerSec;
      sizeSpacer();
    }
    // Follow the playhead while playing.
    if (s.rate) {
      const x = toX(s.playhead);
      if (x < 0 || x > scroller.clientWidth - 20) scroller.scrollLeft += x - 40;
    }
    draw();
  });

  // Ctrl+wheel zooms around the pointer; plain wheel scrolls sideways.
  scroller.addEventListener(
    "wheel",
    (e) => {
      if (e.ctrlKey) {
        e.preventDefault();
        setZoom(state.pxPerSec * Math.exp(-e.deltaY * 0.0015), localX(e));
      } else if (!e.shiftKey && Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        e.preventDefault();
        scroller.scrollLeft += e.deltaY;
      }
    },
    { passive: false },
  );

  canvas.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || !project()) return;
    const x = localX(e);
    const y = localY(e);
    scroller.focus({ preventScroll: true });
    canvas.setPointerCapture(e.pointerId);
    if (y < RULER) {
      drag = { kind: "scrub" };
      seek(snap(toT(x), e.altKey, true));
      return;
    }
    const hit = hitTest(x, y);
    update({ selection: hit?.id ?? null });
    if (!hit) {
      drag = { kind: "scrub" };
      seek(snap(toT(x), e.altKey, true));
    } else if (hit.edge) drag = { kind: "trim", hit, x0: x, t: hit.edge === "in" ? hit.start : hit.end };
    else drag = { kind: "move", hit, x0: x, moved: false, t: hit.start, index: hit.index };
  });

  canvas.addEventListener("pointermove", (e) => {
    const x = localX(e);
    if (!drag) {
      const hit = hitTest(x, localY(e));
      canvas.style.cursor = hit?.edge ? "ew-resize" : hit ? "grab" : "default";
      return;
    }
    if (drag.kind === "scrub") seek(snap(toT(x), e.altKey, true));
    else if (drag.kind === "trim") drag.t = snap(toT(x), e.altKey);
    else {
      if (!drag.moved && Math.abs(x - drag.x0) < 4) return;
      drag.moved = true;
      canvas.style.cursor = "grabbing";
      const raw = drag.hit.start + ((x - drag.x0) / state.pxPerSec) * S;
      drag.t = drag.hit.track === "main" ? toT(x) : snap(Math.max(0, raw), e.altKey);
    }
    redraw();
  });

  const finish = (commit: boolean) => {
    const d = drag;
    drag = null;
    snapLine = null;
    canvas.style.cursor = "default";
    redraw();
    if (!d || !commit) return;
    if (d.kind === "trim") {
      const delta = d.t - (d.hit.edge === "in" ? d.hit.start : d.hit.end);
      if (delta) void edit("clip.trim", { clip_id: d.hit.id, edge: d.hit.edge!, delta });
    } else if (d.kind === "move" && d.moved) {
      if (d.hit.track === "main") {
        const { index } = insertIndex(d.t);
        if (index !== d.index && index !== d.index + 1) void edit("clip.move", { clip_id: d.hit.id, index });
      } else if (d.t !== d.hit.start) void edit(d.hit.track === "titles" ? "title.update" : "music.update", { id: d.hit.id, start: d.t });
    }
  };
  canvas.addEventListener("pointerup", () => finish(true));
  canvas.addEventListener("pointercancel", () => finish(false));
  canvas.addEventListener("lostpointercapture", () => drag && finish(true));

  // Drops from the media bin.
  const dropTarget = (e: DragEvent) => {
    const p = project();
    const m = p?.media.find((x) => x.id === dropMedia);
    if (!m) return null;
    const t = toT(localX(e));
    return m.kind === "audio" ? { track: "music" as const, t: snap(t, e.altKey), index: 0 } : { track: "main" as const, ...insertIndex(t) };
  };
  scroller.addEventListener("dragover", (e) => {
    dropAt = dropTarget(e);
    if (!dropAt) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = "copy";
    redraw();
  });
  scroller.addEventListener("dragleave", (e) => {
    if (scroller.contains(e.relatedTarget as Node | null)) return;
    dropAt = null;
    redraw();
  });
  scroller.addEventListener("drop", (e) => {
    const at = dropTarget(e);
    const id = dropMedia;
    setDropMedia(null);
    if (!at || !id) return;
    e.preventDefault();
    if (at.track === "music") void edit("music.add", { media_id: id, start: at.t });
    else void edit("timeline.insert", { media_id: id, index: at.index });
  });
}
