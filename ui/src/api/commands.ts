// Typed bridge to the Rust core. Inside Tauri it calls the real commands; anywhere else (plain browser, Vitest,
// Playwright) it falls back to the in-memory mock so the UI runs on its own.
import { createMock } from "./mock";
import type {
  AspectId,
  CommandMap,
  CommandName,
  Encoder,
  EventMap,
  KeymapId,
  PresetId,
  RecentProject,
  Snapshot,
  Ticks,
} from "./types";

export interface Backend {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<K extends keyof EventMap>(event: K, cb: (payload: EventMap[K]) => void): Promise<() => void>;
}

interface TauriInternals {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  transformCallback: (cb: (e: { payload: unknown; id: number }) => void, once?: boolean) => number;
}

function tauriBackend(t: TauriInternals): Backend {
  return {
    invoke: <T>(cmd: string, args?: Record<string, unknown>) => t.invoke(cmd, args) as Promise<T>,
    // Same calls @tauri-apps/api/event makes, without the dependency.
    async listen(event, cb) {
      const handler = t.transformCallback((e) => cb(e.payload as never));
      const id = (await t.invoke("plugin:event|listen", { event, target: { kind: "Any" }, handler })) as number;
      return () => void t.invoke("plugin:event|unlisten", { event, eventId: id });
    },
  };
}

const internals = (globalThis as { __TAURI_INTERNALS__?: TauriInternals }).__TAURI_INTERNALS__;
export const inTauri = !!internals;
let backend: Backend = internals ? tauriBackend(internals) : createMock();

/** Tests swap in a fresh mock. */
export function useBackend(b: Backend): void {
  backend = b;
}

export const runCommand = <K extends CommandName>(name: K, args: CommandMap[K]) =>
  backend.invoke<Snapshot>("run_command", { name, args });
export const undo = () => runCommand("history.undo", {});
export const redo = () => runCommand("history.redo", {});

// Tauri command arguments are camelCase on the wire (`export_preset` -> `exportPreset`).
export const projectNew = ({ export_preset, ...a }: { name: string; aspect: AspectId; export_preset: PresetId; keymap: KeymapId }) =>
  backend.invoke<Snapshot>("project_new", { ...a, exportPreset: export_preset });
export const projectOpen = (path: string) => backend.invoke<Snapshot>("project_open", { path });
export const projectSave = (path?: string) => backend.invoke<Snapshot>("project_save", { path: path ?? null });
export const recentProjects = () => backend.invoke<RecentProject[]>("recent_projects");

export const importMedia = (paths: string[]) => backend.invoke<Snapshot>("import_media", { paths });

// Native pickers (file dialogs live in the shell); each resolves null when the user cancels.
export const pickMedia = () => backend.invoke<string[] | null>("dialog_open_media");
export const pickProject = () => backend.invoke<string | null>("dialog_open_project");
export const pickFolder = () => backend.invoke<string | null>("dialog_pick_folder");

export const exportEncoder = () => backend.invoke<Encoder>("export_encoder");
export const exportStart = (preset: PresetId, path: string, range: { start: Ticks; end: Ticks } | null = null) =>
  backend.invoke<string>("export_start", { preset, path, range });
export const exportCancel = (job: string) => backend.invoke<void>("export_cancel", { job });
export const revealPath = (path: string) => backend.invoke<void>("reveal_path", { path });

export const transportPlay = (rate: number) => backend.invoke<void>("transport_play", { rate });
export const transportSeek = (ticks: Ticks) => backend.invoke<void>("transport_seek", { ticks });

export const listen = <K extends keyof EventMap>(event: K, cb: (p: EventMap[K]) => void) => backend.listen(event, cb);
