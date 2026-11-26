// Mirrors the Rust core model (crates/core, serde snake_case). Times are integer ticks (D2, D4).
// ponytail: ticks travel as JS numbers, exact up to 2^53 ticks (about 9.8 h of timeline); switch to string/BigInt if projects get longer.

export const TICKS_PER_SECOND = 254_016_000_000;
export type Ticks = number;

export interface Rational {
  num: number;
  den: number;
}

export type AspectId = "16:9" | "9:16" | "1:1" | "4:5";
export type KeymapId = "editelier" | "premiere" | "resolve";
export type PresetId = "youtube-1080" | "youtube-4k" | "tiktok" | "instagram-reel" | "instagram-45" | "master";

export interface Canvas {
  aspect: AspectId;
  width: number;
  height: number;
  frame_rate: Rational;
}

export interface Probe {
  duration: Ticks;
  width: number;
  height: number;
  frame_rate: Rational;
  is_vfr: boolean;
  has_video: boolean;
  has_audio: boolean;
  codec: string;
}

export type ProxyState =
  | { status: "none" }
  | { status: "building"; progress: number }
  | { status: "ready" }
  | { status: "failed"; error: string };

export interface MediaRef {
  id: string;
  path: string;
  name: string;
  kind: "video" | "audio" | "image";
  probe: Probe;
  proxy: ProxyState;
  /** URL the webview can load (Tauri asset protocol, or data: in the mock). */
  thumbnail: string | null;
  /** Missing, unreadable or not on a local drive (project paths never reach network shares). */
  offline?: boolean;
}

/** Main track clip. The main track is magnetic: its position is the sum of the clips before it. */
export interface Clip {
  id: string;
  media_id: string;
  source_in: Ticks;
  source_out: Ticks;
}

export type TitleFont = "Space Grotesk" | "Inter" | "JetBrains Mono";
export interface TitleStyle {
  font: TitleFont;
  size: number;
  color: string;
  position: "top" | "center" | "lower-third";
}

export interface TitleClip {
  id: string;
  start: Ticks;
  duration: Ticks;
  text: string;
  style: TitleStyle;
}

export interface MusicClip {
  id: string;
  media_id: string;
  start: Ticks;
  source_in: Ticks;
  source_out: Ticks;
  gain_db: number;
  fade_in: Ticks;
  fade_out: Ticks;
}

export interface Timeline {
  main: Clip[];
  titles: TitleClip[];
  music: MusicClip[];
}

export interface Project {
  id: string;
  name: string;
  path: string | null;
  canvas: Canvas;
  export_preset: PresetId;
  keymap: KeymapId;
  media: MediaRef[];
  timeline: Timeline;
}

export interface Snapshot {
  project: Project;
  undo_label: string | null;
  redo_label: string | null;
}

export interface RecentProject {
  name: string;
  path: string;
  modified: string;
}

export interface Encoder {
  label: string;
  hardware: boolean;
}

/** Error shape the core returns from any command; `code` picks the friendly message and fix button. */
export interface ApiError {
  code: string;
  message: string;
}

// Events the core emits.
export interface ImportProgress {
  done: number;
  total: number;
  name: string;
}
export interface ProxyProgress {
  media_id: string;
  proxy: ProxyState;
}
export interface ExportProgress {
  job: string;
  progress: number;
}
export interface ExportDone {
  job: string;
  path: string;
}
export interface ExportFailed {
  job: string;
  error: ApiError;
}
export interface EventMap {
  "import-progress": ImportProgress;
  "proxy-progress": ProxyProgress;
  "export-progress": ExportProgress;
  "export-done": ExportDone;
  "export-failed": ExportFailed;
  playhead: { ticks: Ticks; playing: boolean };
  autosaved: { at: string };
  /** Tauri's own event for files dropped from Explorer (physical-pixel position). */
  "tauri://drag-drop": { paths: string[]; position: { x: number; y: number } };
}

/** Every edit, by name, with its arguments. The core's command registry uses the same names. */
export interface CommandMap {
  "history.undo": Record<string, never>;
  "history.redo": Record<string, never>;
  "project.rename": { name: string };
  "project.set_canvas": { aspect: AspectId };
  "timeline.append": { media_id: string };
  "timeline.insert": { media_id: string; index: number };
  "clip.split": { at: Ticks; clip_id?: string };
  "clip.trim": { clip_id: string; edge: "in" | "out"; delta: Ticks };
  "clip.ripple_trim": { at: Ticks; edge: "start" | "end" };
  "clip.delete": { clip_id: string };
  "clip.move": { clip_id: string; index: number };
  "title.add": { start: Ticks; duration: Ticks; text: string };
  "title.update": { id: string; start?: Ticks; text?: string; style?: Partial<TitleStyle> };
  "music.add": { media_id: string; start: Ticks };
  "music.update": { id: string; start?: Ticks; gain_db?: number; fade_in?: Ticks; fade_out?: Ticks };
}
export type CommandName = keyof CommandMap;
