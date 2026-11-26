// In-memory stand-in for the Rust core, used when the page is not running inside Tauri.
// Same command names, snapshot shape and events as the real core; media is faked from file names.
import { ASPECTS, clipLength, frameTicks, mainIndexAt, mainStarts, PRESETS, quantize } from "../model";
import type { Backend } from "./commands";
import {
  TICKS_PER_SECOND as S,
  type ApiError,
  type AspectId,
  type CommandMap,
  type CommandName,
  type EventMap,
  type KeymapId,
  type MediaRef,
  type PresetId,
  type Project,
  type RecentProject,
  type Snapshot,
  type Ticks,
} from "./types";

const fail = (code: string, message: string): never => {
  throw { code, message } satisfies ApiError;
};

function hash(s: string): number {
  let h = 2166136261;
  for (const ch of s) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
  return h >>> 0;
}

const AUDIO = /\.(mp3|wav|m4a|aac|flac|ogg|opus)$/i;
const IMAGE = /\.(jpe?g|png|webp|heic|gif)$/i;

/** A fake probe: duration from the name's hash, 4K / VFR / audio from words and extensions in the name. */
function fakeMedia(path: string, n: number): MediaRef {
  const name = path.split(/[\\/]/).pop() || path;
  const h = hash(name);
  const kind = AUDIO.test(name) ? "audio" : IMAGE.test(name) ? "image" : "video";
  const is4k = /4k|2160/i.test(name);
  const isVfr = /vfr|phone|pxl_|img_/i.test(name);
  const seconds = kind === "audio" ? 60 + (h % 120) : kind === "image" ? 5 : 4 + (h % 17);
  const hue = h % 360;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><defs><linearGradient id="g" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 45% 35%)"/><stop offset="1" stop-color="hsl(${(hue + 60) % 360} 50% 18%)"/></linearGradient></defs><rect width="160" height="90" fill="url(#g)"/></svg>`;
  return {
    id: `m${n}`,
    path,
    name,
    kind,
    probe: {
      duration: seconds * S,
      width: kind === "audio" ? 0 : is4k ? 3840 : 1920,
      height: kind === "audio" ? 0 : is4k ? 2160 : 1080,
      frame_rate: { num: 30000, den: 1001 },
      is_vfr: isVfr,
      has_video: kind !== "audio",
      has_audio: kind !== "image",
      codec: kind === "audio" ? "aac" : kind === "image" ? "jpeg" : is4k ? "hevc" : "h264",
    },
    proxy: { status: "none" },
    thumbnail: kind === "audio" ? null : `data:image/svg+xml,${encodeURIComponent(svg)}`,
  };
}

const SAMPLE_MEDIA = ["Beach day 4K.mp4", "Phone clip VFR.mp4", "Interview.mp4", "Theme song.mp3"];

export interface MockOptions {
  /** Milliseconds per progress tick for imports, proxies and exports (tests use 0-10). */
  tick?: number;
}

export function createMock({ tick = 120 }: MockOptions = {}): Backend {
  let project: Project | null = null;
  let undoStack: { label: string; project: Project }[] = [];
  let redoStack: { label: string; project: Project }[] = [];
  let nextId = 1;
  const id = (p: string) => `${p}${nextId++}`;
  const saved = new Map<string, Project>();
  const recent: RecentProject[] = [];
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const jobs = new Map<string, ReturnType<typeof setInterval>>();
  let playTimer: ReturnType<typeof setInterval> | undefined;
  let playhead: Ticks = 0;

  const emit = <K extends keyof EventMap>(event: K, payload: EventMap[K]) =>
    listeners.get(event)?.forEach((cb) => cb(structuredClone(payload)));
  const need = () => project ?? fail("no_project", "No project is open.");
  const snap = (): Snapshot => ({
    project: structuredClone(need()),
    undo_label: undoStack.at(-1)?.label ?? null,
    redo_label: redoStack.at(-1)?.label ?? null,
  });
  const media = (p: Project, mid: string) => p.media.find((m) => m.id === mid) ?? fail("not_found", "That media is no longer in the project.");

  /** Runs one edit on a copy of the project; returns the undo label. */
  function apply<K extends CommandName>(p: Project, name: K, a: CommandMap[K]): string {
    const tl = p.timeline;
    const fr = p.canvas.frame_rate;
    const minLen = frameTicks(fr);
    const floorFrame = (t: Ticks) => Math.floor(t / minLen) * minLen;
    switch (name) {
      case "project.rename": {
        const n = (a as CommandMap["project.rename"]).name.trim();
        if (!n) fail("invalid", "A project needs a name.");
        p.name = n.slice(0, 120);
        return "Rename project";
      }
      case "project.set_canvas": {
        const asp = (a as CommandMap["project.set_canvas"]).aspect;
        p.canvas = structuredClone(ASPECTS[asp].canvas);
        return "Change canvas";
      }
      case "timeline.append":
      case "timeline.insert": {
        const { media_id, index } = a as CommandMap["timeline.insert"];
        const m = media(p, media_id);
        if (m.kind === "audio") {
          tl.music.push({ id: id("a"), media_id, start: 0, source_in: 0, source_out: m.probe.duration, gain_db: 0, fade_in: 0, fade_out: 0 });
          return "Add music";
        }
        const clip = { id: id("c"), media_id, source_in: 0, source_out: floorFrame(m.probe.duration) };
        tl.main.splice(name === "timeline.append" ? tl.main.length : Math.max(0, Math.min(index, tl.main.length)), 0, clip);
        return "Add clip";
      }
      case "clip.split": {
        const { at, clip_id } = a as CommandMap["clip.split"];
        const t = quantize(at, fr);
        const title = tl.titles.find((c) => c.id === clip_id);
        if (title) {
          if (t <= title.start || t >= title.start + title.duration) fail("invalid", "Move the playhead over the title to split it.");
          tl.titles.push({ ...structuredClone(title), id: id("t"), start: t, duration: title.start + title.duration - t });
          title.duration = t - title.start;
          return "Split title";
        }
        const music = tl.music.find((c) => c.id === clip_id);
        if (music) {
          const off = t - music.start;
          if (off <= 0 || off >= clipLength(music)) fail("invalid", "Move the playhead over the music to split it.");
          tl.music.push({ ...structuredClone(music), id: id("a"), start: t, source_in: music.source_in + off, fade_in: 0 });
          music.source_out = music.source_in + off;
          music.fade_out = 0;
          return "Split music";
        }
        const starts = mainStarts(tl.main);
        const i = mainIndexAt(tl.main, starts, t);
        const c = tl.main[i];
        if (!c || t === starts[i]) return fail("invalid", "Move the playhead inside a clip to split it.");
        const cut = c.source_in + (t - starts[i]!);
        tl.main.splice(i + 1, 0, { ...c, id: id("c"), source_in: cut });
        c.source_out = cut;
        return "Split clip";
      }
      case "clip.ripple_trim": {
        const { at, edge } = a as CommandMap["clip.ripple_trim"];
        const t = quantize(at, fr);
        const starts = mainStarts(tl.main);
        const i = mainIndexAt(tl.main, starts, t);
        const c = tl.main[i];
        if (!c || t === starts[i]) return fail("invalid", "Move the playhead inside a clip to trim it.");
        if (edge === "start") c.source_in += t - starts[i]!;
        else c.source_out = c.source_in + (t - starts[i]!);
        return edge === "start" ? "Trim start" : "Trim end";
      }
      case "clip.trim": {
        const { clip_id, edge, delta } = a as CommandMap["clip.trim"];
        const d = quantize(delta, fr);
        const music = tl.music.find((x) => x.id === clip_id);
        const c = tl.main.find((x) => x.id === clip_id) ?? music;
        if (c) {
          const dur = media(p, c.media_id).probe.duration;
          if (edge === "in") {
            // Music keeps its place on the timeline, so its start moves with the in point (never before zero).
            const v = Math.max(music ? Math.max(0, music.source_in - music.start) : 0, Math.min(c.source_out - minLen, c.source_in + d));
            if (music) music.start += v - music.source_in;
            c.source_in = v;
          } else c.source_out = Math.min(floorFrame(dur), Math.max(c.source_in + minLen, c.source_out + d));
          return "Trim";
        }
        const tc = tl.titles.find((x) => x.id === clip_id) ?? fail("not_found", "That clip is gone.");
        if (edge === "in") {
          const v = Math.max(-tc.start, Math.min(tc.duration - minLen, d));
          tc.start += v;
          tc.duration -= v;
        } else tc.duration = Math.max(minLen, tc.duration + d);
        return "Trim";
      }
      case "clip.delete": {
        const cid = (a as CommandMap["clip.delete"]).clip_id;
        for (const k of ["main", "titles", "music"] as const) {
          const i = tl[k].findIndex((c) => c.id === cid);
          if (i >= 0) {
            tl[k].splice(i, 1);
            return "Delete";
          }
        }
        return fail("not_found", "That clip is gone.");
      }
      case "clip.move": {
        const { clip_id, index } = a as CommandMap["clip.move"];
        const i = tl.main.findIndex((c) => c.id === clip_id);
        if (i < 0) fail("not_found", "That clip is gone.");
        const [c] = tl.main.splice(i, 1);
        tl.main.splice(Math.max(0, Math.min(index > i ? index - 1 : index, tl.main.length)), 0, c!);
        return "Move clip";
      }
      case "title.add": {
        const { start, duration, text } = a as CommandMap["title.add"];
        tl.titles.push({
          id: id("t"),
          start: quantize(Math.max(0, start), fr),
          duration: Math.max(minLen, quantize(duration, fr)),
          text: text.slice(0, 500),
          style: { font: "Space Grotesk", size: 72, color: "#ffffff", position: "lower-third" },
        });
        return "Add title";
      }
      case "title.update": {
        const u = a as CommandMap["title.update"];
        const c = tl.titles.find((x) => x.id === u.id) ?? fail("not_found", "That title is gone.");
        if (u.start !== undefined) c.start = quantize(Math.max(0, u.start), fr);
        if (u.text !== undefined) c.text = u.text.slice(0, 500);
        if (u.style) Object.assign(c.style, u.style);
        return u.start !== undefined ? "Move title" : "Edit title";
      }
      case "music.add": {
        const { media_id, start } = a as CommandMap["music.add"];
        const m = media(p, media_id);
        if (!m.probe.has_audio) fail("no_audio", "That file has no sound to use as music.");
        tl.music.push({ id: id("a"), media_id, start: quantize(Math.max(0, start), fr), source_in: 0, source_out: m.probe.duration, gain_db: 0, fade_in: 0, fade_out: 0 });
        return "Add music";
      }
      case "music.update": {
        const u = a as CommandMap["music.update"];
        const c = tl.music.find((x) => x.id === u.id) ?? fail("not_found", "That music is gone.");
        const len = clipLength(c);
        if (u.start !== undefined) c.start = quantize(Math.max(0, u.start), fr);
        if (u.gain_db !== undefined) c.gain_db = Math.max(-60, Math.min(12, u.gain_db));
        if (u.fade_in !== undefined) c.fade_in = Math.max(0, Math.min(len, u.fade_in));
        if (u.fade_out !== undefined) c.fade_out = Math.max(0, Math.min(len, u.fade_out));
        return u.start !== undefined ? "Move music" : "Music volume and fades";
      }
      default:
        return fail("unknown_command", `Unknown command ${String(name)}.`);
    }
  }

  function runCommand(name: CommandName, args: Record<string, unknown>): Snapshot {
    const p = need();
    if (name === "history.undo" || name === "history.redo") {
      const [from, to] = name === "history.undo" ? [undoStack, redoStack] : [redoStack, undoStack];
      const step = from.pop();
      if (!step) fail("nothing_to_" + name.slice(8), `Nothing to ${name.slice(8)}.`);
      to.push({ label: step!.label, project: p });
      project = step!.project;
      return snap();
    }
    const next = structuredClone(p);
    const label = apply(next, name, args as never);
    undoStack.push({ label, project: p });
    redoStack = [];
    project = next;
    setTimeout(() => emit("autosaved", { at: new Date().toISOString() }), tick);
    return snap();
  }

  function startProxy(m: MediaRef) {
    let pct = 0;
    const t = setInterval(() => {
      pct = Math.min(100, pct + 20);
      const proxy = pct < 100 ? ({ status: "building", progress: pct } as const) : ({ status: "ready" } as const);
      const live = project?.media.find((x) => x.id === m.id);
      if (live) live.proxy = proxy;
      emit("proxy-progress", { media_id: m.id, proxy });
      if (pct >= 100) clearInterval(t);
    }, tick);
  }

  const handlers: Record<string, (a: Record<string, unknown>) => unknown> = {
    project_new(a) {
      const asp = (a.aspect as AspectId) in ASPECTS ? (a.aspect as AspectId) : "16:9";
      const preset = (a.export_preset as PresetId) in PRESETS ? (a.export_preset as PresetId) : ASPECTS[asp].preset;
      project = {
        id: id("p"),
        name: String(a.name || "Untitled project").slice(0, 120),
        path: null,
        canvas: structuredClone(ASPECTS[asp].canvas),
        export_preset: preset,
        keymap: (a.keymap as KeymapId) ?? "editelier",
        media: [],
        timeline: { main: [], titles: [], music: [] },
      };
      undoStack = [];
      redoStack = [];
      return snap();
    },
    project_open(a) {
      const p = saved.get(String(a.path)) ?? fail("not_found", "That project file was moved or deleted.");
      project = structuredClone(p);
      undoStack = [];
      redoStack = [];
      return snap();
    },
    project_save(a) {
      const p = need();
      p.path = (a.path as string | null) ?? p.path ?? `${p.name}.editelier`;
      saved.set(p.path, structuredClone(p));
      const i = recent.findIndex((r) => r.path === p.path);
      if (i >= 0) recent.splice(i, 1);
      recent.unshift({ name: p.name, path: p.path, modified: new Date().toISOString() });
      return snap();
    },
    recent_projects: () => structuredClone(recent),
    run_command: (a) => runCommand(a.name as CommandName, (a.args ?? {}) as Record<string, unknown>),
    import_media(a) {
      const p = need();
      const paths = (a.paths as string[]).filter((x) => typeof x === "string" && x.length > 0);
      if (!paths.length) fail("invalid", "Nothing to import.");
      paths.forEach((path, i) => {
        const m = fakeMedia(path, nextId++);
        p.media.push(m);
        emit("import-progress", { done: i + 1, total: paths.length, name: m.name });
        if (m.kind === "video" && (m.probe.width > 1920 || m.probe.is_vfr)) {
          m.proxy = { status: "building", progress: 0 };
          startProxy(m);
        }
      });
      return snap();
    },
    dialog_open_media: () => [...SAMPLE_MEDIA],
    dialog_open_project: () => recent[0]?.path ?? null,
    dialog_pick_folder: () => "Videos\\Editelier",
    export_encoder: () => ({ label: "Auto: software H.264 (browser preview)", hardware: false }),
    export_start(a) {
      const p = need();
      if (!p.timeline.main.length && !p.timeline.titles.length) fail("empty_timeline", "Add a clip to the timeline before exporting.");
      if (!((a.preset as PresetId) in PRESETS)) fail("invalid", "Unknown export preset.");
      const path = String(a.path);
      const job = id("job");
      let pct = 0;
      jobs.set(
        job,
        setInterval(() => {
          pct += 10;
          emit("export-progress", { job, progress: Math.min(pct, 100) });
          if (pct >= 100) {
            clearInterval(jobs.get(job));
            jobs.delete(job);
            emit("export-done", { job, path });
          }
        }, tick),
      );
      return job;
    },
    export_cancel(a) {
      clearInterval(jobs.get(String(a.job)));
      jobs.delete(String(a.job));
    },
    reveal_path: () => undefined,
    transport_seek(a) {
      playhead = Math.max(0, Number(a.ticks) || 0);
    },
    transport_play(a) {
      clearInterval(playTimer);
      const rate = Number(a.rate) || 0;
      if (!rate) return emit("playhead", { ticks: playhead, playing: false });
      const step = 1000 / 30;
      playTimer = setInterval(() => {
        playhead = Math.max(0, playhead + (rate * step * S) / 1000);
        emit("playhead", { ticks: playhead, playing: playhead > 0 || rate > 0 });
        if (playhead === 0 && rate < 0) clearInterval(playTimer);
      }, step);
    },
  };

  return {
    async invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
      const h = handlers[cmd] ?? fail("unknown_command", `Unknown command ${cmd}.`);
      return h(args) as T;
    },
    async listen(event, cb) {
      const set = listeners.get(event) ?? new Set();
      listeners.set(event, set);
      set.add(cb as (p: unknown) => void);
      return () => set.delete(cb as (p: unknown) => void);
    },
  };
}
