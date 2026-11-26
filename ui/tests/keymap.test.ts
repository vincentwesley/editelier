import { describe, expect, it } from "vitest";
import { ACTIONS, chordOf, displayChord, indexKeymap, KEYMAPS, type Action } from "../src/keymap";

type Ev = Parameters<typeof chordOf>[0];
const key = (key: string, code: string, mods: Partial<Ev> = {}): Ev => ({ key, code, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods });
const action = (id: keyof typeof KEYMAPS, e: Ev) => indexKeymap(KEYMAPS[id].keys).get(chordOf(e)) ?? null;

describe("chordOf", () => {
  it("names letters by physical key, modifiers in a fixed order", () => {
    expect(chordOf(key("z", "KeyZ", { ctrlKey: true }))).toBe("Ctrl+Z");
    expect(chordOf(key("Z", "KeyZ", { ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+Z");
    expect(chordOf(key("≈", "KeyX", { altKey: true }))).toBe("Alt+X");
  });
  it("keeps Shift as a modifier on punctuation", () => {
    expect(chordOf(key("?", "Slash", { shiftKey: true }))).toBe("Shift+/");
    expect(chordOf(key("{", "BracketLeft", { ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+[");
    expect(chordOf(key(" ", "Space"))).toBe("Space");
    expect(chordOf(key("ArrowLeft", "ArrowLeft"))).toBe("Left");
  });
  it("treats Cmd as Ctrl", () => expect(chordOf(key("z", "KeyZ", { metaKey: true }))).toBe("Ctrl+Z"));
});

describe("keymap resolution", () => {
  it("default keymap: research §5 single keys", () => {
    expect(action("editelier", key(" ", "Space"))).toBe("playPause");
    expect(action("editelier", key("j", "KeyJ"))).toBe("shuttleReverse");
    expect(action("editelier", key("k", "KeyK"))).toBe("shuttleStop");
    expect(action("editelier", key("l", "KeyL"))).toBe("shuttleForward");
    expect(action("editelier", key("s", "KeyS"))).toBe("split");
    expect(action("editelier", key("q", "KeyQ"))).toBe("rippleTrimStart");
    expect(action("editelier", key("w", "KeyW"))).toBe("rippleTrimEnd");
    expect(action("editelier", key("Delete", "Delete"))).toBe("rippleDelete");
    expect(action("editelier", key("z", "KeyZ", { ctrlKey: true }))).toBe("undo");
    expect(action("editelier", key("Z", "KeyZ", { ctrlKey: true, shiftKey: true }))).toBe("redo");
    expect(action("editelier", key("+", "Equal", { shiftKey: true }))).toBe("zoomIn");
    expect(action("editelier", key("-", "Minus"))).toBe("zoomOut");
    expect(action("editelier", key("?", "Slash", { shiftKey: true }))).toBe("cheatSheet");
    expect(action("editelier", key("x", "KeyX"))).toBeNull();
  });
  it("Premiere-style and Resolve-style remap the same actions", () => {
    expect(action("premiere", key("k", "KeyK", { ctrlKey: true }))).toBe("split");
    expect(action("premiere", key("s", "KeyS"))).toBe("toggleSnap");
    expect(action("premiere", key("Delete", "Delete", { shiftKey: true }))).toBe("rippleDelete");
    expect(action("resolve", key("b", "KeyB", { ctrlKey: true }))).toBe("split");
    expect(action("resolve", key("{", "BracketLeft", { ctrlKey: true, shiftKey: true }))).toBe("rippleTrimStart");
    expect(action("resolve", key("Backspace", "Backspace"))).toBe("rippleDelete");
    expect(action("resolve", key("s", "KeyS"))).toBeNull();
  });
  it("no keymap binds one chord to two actions", () => {
    for (const { keys } of Object.values(KEYMAPS)) {
      const seen = new Map<string, Action>();
      for (const [a, chords] of Object.entries(keys) as [Action, string[]][])
        for (const c of chords) {
          expect(seen.get(c) ?? a, `${c} bound twice`).toBe(a);
          seen.set(c, a);
        }
    }
  });
  it("every action has a label and a key in the default keymap", () => {
    for (const a of Object.keys(ACTIONS) as Action[]) expect(KEYMAPS.editelier.keys[a]?.length, a).toBeGreaterThan(0);
  });
  it("shows friendly key names", () => {
    expect(displayChord("Shift+/")).toBe("?");
    expect(displayChord("Left")).toBe("←");
    expect(displayChord("Ctrl+Z")).toBe("Ctrl+Z");
  });
});
