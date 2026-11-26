// Actions and keymaps as data (D14, research 03 §5). Key-to-action conventions only; the keymap names are descriptive.
import type { KeymapId } from "./api/types";

export const ACTIONS = {
  playPause: "Play / pause",
  shuttleReverse: "Play backwards (press again for faster)",
  shuttleStop: "Stop",
  shuttleForward: "Play forwards (press again for faster)",
  frameBack: "Step back one frame",
  frameForward: "Step forward one frame",
  prevEdit: "Go to previous edit",
  nextEdit: "Go to next edit",
  goStart: "Go to start",
  goEnd: "Go to end",
  markIn: "Mark in",
  markOut: "Mark out",
  clearInOut: "Clear in and out",
  split: "Split at playhead",
  rippleTrimStart: "Trim clip start to playhead",
  rippleTrimEnd: "Trim clip end to playhead",
  rippleDelete: "Delete and close the gap",
  toggleSnap: "Snapping on or off",
  undo: "Undo",
  redo: "Redo",
  zoomIn: "Zoom in",
  zoomOut: "Zoom out",
  zoomFit: "Zoom to fit",
  append: "Add selected media to the end",
  addTitle: "Add title at playhead",
  import: "Import media",
  save: "Save project",
  export: "Export",
  cheatSheet: "Keyboard shortcuts",
} as const;
export type Action = keyof typeof ACTIONS;
export type Keymap = Partial<Record<Action, string[]>>;

const base: Keymap = {
  playPause: ["Space"],
  shuttleReverse: ["J"],
  shuttleStop: ["K"],
  shuttleForward: ["L"],
  frameBack: ["Left"],
  frameForward: ["Right"],
  prevEdit: ["Up"],
  nextEdit: ["Down"],
  goStart: ["Home"],
  goEnd: ["End"],
  markIn: ["I"],
  markOut: ["O"],
  clearInOut: ["Alt+X"],
  split: ["S"],
  rippleTrimStart: ["Q"],
  rippleTrimEnd: ["W"],
  rippleDelete: ["Delete", "Backspace"],
  toggleSnap: ["N"],
  undo: ["Ctrl+Z"],
  redo: ["Ctrl+Shift+Z", "Ctrl+Y"],
  zoomIn: ["=", "Shift+="],
  zoomOut: ["-"],
  zoomFit: ["Shift+Z"],
  append: ["E"],
  addTitle: ["Ctrl+T"],
  import: ["Ctrl+I"],
  save: ["Ctrl+S"],
  export: ["Ctrl+E"],
  cheatSheet: ["Shift+/"],
};

export const KEYMAPS: Record<KeymapId, { label: string; keys: Keymap }> = {
  editelier: { label: "Editelier (simple single keys)", keys: base },
  premiere: {
    label: "Premiere-style",
    keys: { ...base, split: ["Ctrl+K"], rippleDelete: ["Shift+Delete"], toggleSnap: ["S"], zoomFit: ["\\"], export: ["Ctrl+M"], clearInOut: ["Ctrl+Shift+X"] },
  },
  resolve: {
    label: "Resolve-style",
    keys: {
      ...base,
      split: ["Ctrl+B"],
      rippleTrimStart: ["Ctrl+Shift+["],
      rippleTrimEnd: ["Ctrl+Shift+]"],
      rippleDelete: ["Backspace"],
      zoomIn: ["Ctrl+="],
      zoomOut: ["Ctrl+-"],
      append: ["Shift+F12"],
      export: ["Ctrl+Shift+,"],
    },
  },
};

// Physical keys whose character changes with Shift: name them by the unshifted key so Shift stays a modifier.
const CODES: Record<string, string> = {
  Equal: "=", Minus: "-", BracketLeft: "[", BracketRight: "]", Backslash: "\\", Comma: ",", Period: ".", Slash: "/", Backquote: "`",
  Space: "Space", NumpadAdd: "=", NumpadSubtract: "-",
};
const KEYS: Record<string, string> = { ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", " ": "Space" };

/** A KeyboardEvent as a chord string, e.g. "Ctrl+Shift+Z". Modifiers in the fixed order Ctrl, Alt, Shift. */
export function chordOf(e: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">): string {
  let k: string;
  if (/^Key[A-Z]$/.test(e.code)) k = e.code.slice(3);
  else if (/^Digit\d$/.test(e.code)) k = e.code.slice(5);
  else k = CODES[e.code] ?? KEYS[e.key] ?? (e.key.length === 1 ? e.key.toUpperCase() : e.key);
  const mods = [(e.ctrlKey || e.metaKey) && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift"].filter(Boolean);
  return [...mods, k].join("+");
}

/** Reverse index for one keymap: chord -> action. Later entries never shadow earlier ones. */
export function indexKeymap(map: Keymap): Map<string, Action> {
  const idx = new Map<string, Action>();
  for (const [action, chords] of Object.entries(map) as [Action, string[]][]) for (const c of chords) if (!idx.has(c)) idx.set(c, action);
  return idx;
}


const SHOW: Record<string, string> = { "Shift+/": "?", "Shift+=": "+", Left: "←", Right: "→", Up: "↑", Down: "↓" };
export const displayChord = (c: string) => SHOW[c] ?? c;
