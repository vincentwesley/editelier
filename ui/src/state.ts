// UI state: the latest project snapshot from the core plus view-only state (playhead, selection, zoom).
// Subscribers re-render once per animation frame however many changes land in it.
import { runCommand } from "./api/commands";
import type { CommandMap, CommandName, KeymapId, Snapshot, Ticks } from "./api/types";
import { announce } from "./dom";
import { indexKeymap, KEYMAPS } from "./keymap";
import { showError } from "./toast";

export const state = {
  snap: null as Snapshot | null,
  playhead: 0 as Ticks,
  rate: 0,
  selection: null as string | null,
  selectedMedia: null as string | null,
  markIn: null as Ticks | null,
  markOut: null as Ticks | null,
  snapping: true,
  /** Timeline zoom in CSS pixels per second. */
  pxPerSec: 60,
  keymap: "editelier" as KeymapId,
  keys: indexKeymap(KEYMAPS.editelier.keys),
};
export type State = typeof state;

const subs = new Set<(s: State) => void>();
let queued = false;

export function subscribe(fn: (s: State) => void): void {
  subs.add(fn);
}

export function update(patch: Partial<State>): void {
  Object.assign(state, patch);
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => {
    queued = false;
    subs.forEach((fn) => fn(state));
  });
}

export function setKeymap(id: KeymapId): void {
  update({ keymap: id, keys: indexKeymap(KEYMAPS[id].keys) });
  try {
    localStorage.setItem("keymap", id);
  } catch {
    /* storage blocked: the choice lasts for this session */
  }
}

/** Applies a snapshot from the core, dropping a selection that no longer exists. */
export function applySnap(snap: Snapshot): void {
  const tl = snap.project.timeline;
  const alive = (id: string | null) => !!id && [...tl.main, ...tl.titles, ...tl.music].some((c) => c.id === id);
  update({ snap, selection: alive(state.selection) ? state.selection : null });
}

/** Runs a named edit; the core answers with the new snapshot and undo label. */
export async function edit<K extends CommandName>(name: K, args: CommandMap[K]): Promise<boolean> {
  try {
    const snap = await runCommand(name, args);
    applySnap(snap);
    if (name === "history.undo") announce(`Undid ${snap.redo_label ?? ""}`);
    else if (name === "history.redo") announce(`Redid ${snap.undo_label ?? ""}`);
    else if (snap.undo_label) announce(snap.undo_label);
    return true;
  } catch (e) {
    showError(e);
    return false;
  }
}
