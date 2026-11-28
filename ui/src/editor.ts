// The editor screen: actions (shared by keys, buttons and the All actions menu), top bar, media bin, viewer
// transport and inspector. The timeline canvas lives in timeline.ts, dialogs in dialogs.ts.
import * as api from "./api/commands";
import { TICKS_PER_SECOND as S, type MediaRef, type Project, type Snapshot, type Ticks, type TitleFont } from "./api/types";
import { openExport, openShortcuts } from "./dialogs";
import { $, announce, h } from "./dom";
import { ACTIONS, chordOf, displayChord, KEYMAPS, type Action } from "./keymap";
import { ASPECTS, clipLength, editPoints, frameTicks, mainIndexAt, mainStarts, quantize, timelineEnd } from "./model";
import { mountPreview } from "./preview";
import { applySnap, edit, setKeymap, state, subscribe, update } from "./state";
import { formatTimecode } from "./timecode";
import { initTimeline, setDropMedia, setZoom, zoomFit } from "./timeline";
import { showError, toast } from "./toast";

const project = () => state.snap?.project ?? null;
const fr = () => project()?.canvas.frame_rate ?? { num: 30000, den: 1001 };
const tc = (t: Ticks) => formatTimecode(t, fr());

export function seek(t: Ticks): void {
  const p = project();
  const end = p ? timelineEnd(p.timeline) : 0;
  const v = quantize(Math.max(0, Math.min(t, end)), fr());
  update({ playhead: v });
  void api.transportSeek(v);
}

function setRate(rate: number): void {
  update({ rate });
  void api.transportPlay(rate);
}

const mainAtPlayhead = (p: Project) => {
  const starts = mainStarts(p.timeline.main);
  const i = mainIndexAt(p.timeline.main, starts, state.playhead);
  return { i, start: starts[i] ?? 0, clip: p.timeline.main[i] };
};

async function importPaths(paths: string[]): Promise<MediaRef[]> {
  const before = new Set(project()?.media.map((m) => m.id));
  try {
    const snap = await api.importMedia(paths);
    applySnap(snap);
    const added = snap.project.media.filter((m) => !before.has(m.id));
    announce(`Imported ${added.length} ${added.length === 1 ? "file" : "files"}`);
    return added;
  } catch (e) {
    showError(e, () => void importPaths(paths));
    return [];
  }
}

export async function run(action: Action): Promise<void> {
  const p = project();
  if (!p) return;
  const frame = frameTicks(p.canvas.frame_rate);
  const end = timelineEnd(p.timeline);
  switch (action) {
    case "playPause":
      if (state.rate) return setRate(0);
      if (state.playhead >= end) seek(0);
      return setRate(1);
    case "shuttleForward":
      return setRate(state.rate > 0 ? Math.min(state.rate * 2, 8) : 1);
    case "shuttleReverse":
      return setRate(state.rate < 0 ? Math.max(state.rate * 2, -8) : -1);
    case "shuttleStop":
      return setRate(0);
    case "frameBack":
    case "frameForward":
      if (state.rate) setRate(0);
      return seek(state.playhead + (action === "frameBack" ? -frame : frame));
    case "prevEdit":
    case "nextEdit": {
      const pts = editPoints(p.timeline);
      const t = action === "prevEdit" ? pts.filter((x) => x < state.playhead).at(-1) : pts.find((x) => x > state.playhead);
      return seek(t ?? (action === "prevEdit" ? 0 : end));
    }
    case "goStart":
      return seek(0);
    case "goEnd":
      return seek(end);
    case "markIn":
      return update({ markIn: state.playhead, markOut: state.markOut !== null && state.markOut <= state.playhead ? null : state.markOut });
    case "markOut":
      return update({ markOut: state.playhead, markIn: state.markIn !== null && state.markIn >= state.playhead ? null : state.markIn });
    case "clearInOut":
      return update({ markIn: null, markOut: null });
    case "split":
      return void edit("clip.split", { at: state.playhead, ...(state.selection ? { clip_id: state.selection } : {}) });
    case "rippleTrimStart":
    case "rippleTrimEnd": {
      const { start } = mainAtPlayhead(p);
      if (await edit("clip.ripple_trim", { at: state.playhead, edge: action === "rippleTrimStart" ? "start" : "end" }))
        if (action === "rippleTrimStart") seek(start);
      return;
    }
    case "rippleDelete": {
      const id = state.selection ?? mainAtPlayhead(p).clip?.id;
      if (!id) return void toast("Select a clip, or move the playhead over one, to delete it.");
      return void edit("clip.delete", { clip_id: id });
    }
    case "toggleSnap":
      update({ snapping: !state.snapping });
      return announce(state.snapping ? "Snapping on" : "Snapping off");
    case "undo":
      return void (state.snap?.undo_label && edit("history.undo", {}));
    case "redo":
      return void (state.snap?.redo_label && edit("history.redo", {}));
    case "zoomIn":
      return setZoom(state.pxPerSec * 1.5);
    case "zoomOut":
      return setZoom(state.pxPerSec / 1.5);
    case "zoomFit":
      return zoomFit();
    case "append": {
      if (!state.selectedMedia) return void toast("Select something in the Media panel first.");
      return void edit("timeline.append", { media_id: state.selectedMedia });
    }
    case "addTitle":
      if (await edit("title.add", { start: state.playhead, duration: 3 * S, text: "Your title" }))
        update({ selection: project()!.timeline.titles.at(-1)?.id ?? null });
      return;
    case "import": {
      const paths = await api.pickMedia().catch((e) => (showError(e), null));
      if (paths?.length) await importPaths(paths);
      return;
    }
    case "save":
      try {
        applySnap(await api.projectSave());
        toast("Project saved", { kind: "ok" });
      } catch (e) {
        showError(e, () => void run("save"));
      }
      return;
    case "export":
      return openExport();
    case "cheatSheet":
      return openShortcuts();
  }
}

// ---- keyboard ----
function onKey(e: KeyboardEvent): void {
  if (e.defaultPrevented || e.isComposing || $("#editor").hidden || document.querySelector(":modal")) return;
  const t = e.target as HTMLElement;
  if (t.closest("input, textarea, select, [contenteditable]")) return;
  if (t.closest("button, a, [popover]") && (e.key === " " || e.key === "Enter")) return;
  const action = state.keys.get(chordOf(e));
  if (!action) return;
  e.preventDefault();
  void run(action);
}

// ---- media bin ----
function badge(text: string, title: string, mod = ""): HTMLElement {
  return h("span", { class: `badge ${mod}`, title }, text);
}

function proxyBadge(m: MediaRef): HTMLElement | null {
  const px = m.proxy;
  if (px.status === "building")
    return h("span", { class: "badge badge--proxy", title: "Making a smooth preview copy in the background" }, `Proxy ${Math.round(px.progress)}%`, h("progress", { max: 100, value: px.progress, "aria-label": `Preview copy for ${m.name}` }));
  if (px.status === "ready") return badge("Proxy", "A smooth preview copy is ready; export still uses the original");
  if (px.status === "failed") return badge("Proxy failed", px.error, "badge--bad");
  return null;
}

function renderBin(p: Project): void {
  const list = $("#bin-list");
  $("#bin-empty").hidden = p.media.length > 0;
  list.hidden = !p.media.length;
  list.replaceChildren(
    ...p.media.map((m) => {
      const sel = state.selectedMedia === m.id;
      const pick = h(
        "button",
        {
          type: "button",
          class: "bin__item",
          draggable: "true",
          "aria-pressed": String(sel),
          "data-media": m.id,
          title: "Double-click to add to the end of the timeline, or drag onto it",
          onclick: () => update({ selectedMedia: m.id }),
          ondblclick: () => void edit("timeline.append", { media_id: m.id }),
          ondragstart: (e: Event) => {
            const d = (e as DragEvent).dataTransfer!;
            d.setData("text/plain", m.name);
            d.effectAllowed = "copy";
            setDropMedia(m.id);
          },
          ondragend: () => setDropMedia(null),
        },
        m.thumbnail ? h("img", { class: "bin__thumb", src: m.thumbnail, alt: "", width: 64, height: 36, draggable: "false" }) : h("span", { class: "bin__thumb bin__thumb--audio", "aria-hidden": "true" }),
        h(
          "span",
          { class: "bin__meta" },
          h("span", { class: "bin__name" }, m.offline ? `${m.name} (offline)` : m.name),
          h(
            "span",
            { class: "bin__badges" },
            h("span", { class: "mono muted" }, tc(m.probe.duration)),
            m.probe.is_vfr ? badge("VFR", "Phone footage with a variable frame rate: Editelier keeps it in sync") : null,
            m.probe.width >= 3840 ? badge("4K", `${m.probe.width}×${m.probe.height}`) : null,
            proxyBadge(m),
          ),
        ),
      );
      return h("li", {}, pick, h("button", { type: "button", class: "ibtn bin__add", "aria-label": `Add ${m.name} to the timeline`, onclick: () => void edit("timeline.append", { media_id: m.id }) }, "+"));
    }),
  );
}

// ---- inspector ----
const field = (label: string, control: HTMLElement, out?: HTMLElement) => h("label", { class: "field" }, h("span", {}, label, out ?? null), control);

function renderInspector(p: Project): void {
  const box = $("#inspector");
  const focused = document.activeElement?.id;
  const sel = state.selection;
  const tl = p.timeline;
  const title = tl.titles.find((c) => c.id === sel);
  const music = tl.music.find((c) => c.id === sel);
  const clipI = tl.main.findIndex((c) => c.id === sel);
  const media = (id: string) => p.media.find((m) => m.id === id);
  const del = (id: string) => h("button", { type: "button", class: "tbtn", onclick: () => void edit("clip.delete", { clip_id: id }) }, "Delete");
  let kids: (HTMLElement | null)[];

  if (title) {
    const fonts: TitleFont[] = ["Space Grotesk", "Inter", "JetBrains Mono"];
    const up = (style: Parameters<typeof edit<"title.update">>[1]["style"]) => void edit("title.update", { id: title.id, style });
    kids = [
      h("h3", {}, "Title"),
      field("Text", h("textarea", { id: "in-title-text", class: "hs-textarea", rows: 3, maxlength: 500, onchange: (e: Event) => void edit("title.update", { id: title.id, text: (e.target as HTMLTextAreaElement).value }) }, title.text)),
      field("Font", h("select", { id: "in-title-font", class: "hs-select", onchange: (e: Event) => up({ font: (e.target as HTMLSelectElement).value as TitleFont }) }, ...fonts.map((f) => h("option", { value: f, selected: f === title.style.font }, f)))),
      h(
        "div",
        { class: "field-row" },
        field("Size", h("input", { id: "in-title-size", class: "hs-input", type: "number", min: 12, max: 400, value: title.style.size, onchange: (e: Event) => up({ size: Math.min(400, Math.max(12, Number((e.target as HTMLInputElement).value) || 72)) }) })),
        field("Colour", h("input", { id: "in-title-color", class: "swatch", type: "color", value: title.style.color, onchange: (e: Event) => up({ color: (e.target as HTMLInputElement).value }) })),
      ),
      field(
        "Position",
        h(
          "select",
          { id: "in-title-pos", class: "hs-select", onchange: (e: Event) => up({ position: (e.target as HTMLSelectElement).value as "top" }) },
          ...(["top", "center", "lower-third"] as const).map((v) => h("option", { value: v, selected: v === title.style.position }, v === "lower-third" ? "Lower third" : v === "top" ? "Top" : "Centre")),
        ),
      ),
      h("p", { class: "muted mono" }, `${tc(title.start)} for ${tc(title.duration)}`),
      del(title.id),
    ];
  } else if (music) {
    const out = (id: string, v: string) => h("output", { id: id + "-out", for: id, class: "mono" }, v);
    const range = (id: string, min: number, max: number, step: number, value: number, fmt: (v: number) => string, commit: (v: number) => void) =>
      h("input", {
        id,
        type: "range",
        class: "range",
        min,
        max,
        step,
        value,
        oninput: (e: Event) => ($(`#${id}-out`).textContent = fmt(Number((e.target as HTMLInputElement).value))),
        onchange: (e: Event) => commit(Number((e.target as HTMLInputElement).value)),
      });
    const db = (v: number) => `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`;
    const sec = (v: number) => `${v.toFixed(1)} s`;
    const maxFade = Math.min(10, clipLength(music) / S);
    kids = [
      h("h3", {}, media(music.media_id)?.name ?? "Music"),
      field("Volume", range("in-gain", -30, 12, 0.5, music.gain_db, db, (v) => void edit("music.update", { id: music.id, gain_db: v })), out("in-gain", db(music.gain_db))),
      field("Fade in", range("in-fin", 0, maxFade, 0.1, music.fade_in / S, sec, (v) => void edit("music.update", { id: music.id, fade_in: v * S })), out("in-fin", sec(music.fade_in / S))),
      field("Fade out", range("in-fout", 0, maxFade, 0.1, music.fade_out / S, sec, (v) => void edit("music.update", { id: music.id, fade_out: v * S })), out("in-fout", sec(music.fade_out / S))),
      h("p", { class: "muted mono" }, `${tc(music.start)} for ${tc(clipLength(music))}`),
      del(music.id),
    ];
  } else if (clipI >= 0) {
    const c = tl.main[clipI]!;
    const m = media(c.media_id);
    kids = [
      h("h3", {}, m?.name ?? "Missing media"),
      h(
        "dl",
        { class: "facts" },
        h("dt", {}, "Clip"),
        h("dd", {}, `${clipI + 1} of ${tl.main.length}`),
        h("dt", {}, "Source in"),
        h("dd", { class: "mono" }, tc(c.source_in)),
        h("dt", {}, "Source out"),
        h("dd", { class: "mono" }, tc(c.source_out)),
        h("dt", {}, "Length"),
        h("dd", { class: "mono" }, tc(clipLength(c))),
        m ? h("dt", {}, "Size") : null,
        m ? h("dd", {}, `${m.probe.width}×${m.probe.height}, ${m.probe.codec.toUpperCase()}`) : null,
      ),
      m?.probe.is_vfr ? h("p", { class: "note" }, "Phone footage with a variable frame rate. Editelier keeps every frame in sync and evens it out when you export.") : null,
      h("div", { class: "row" }, h("button", { type: "button", class: "tbtn", "data-action": "split" }, "Split at playhead"), del(c.id)),
    ];
  } else {
    kids = [
      h("h3", {}, "Project"),
      field(
        "Canvas",
        h(
          "select",
          { id: "in-canvas", class: "hs-select", onchange: (e: Event) => void edit("project.set_canvas", { aspect: (e.target as HTMLSelectElement).value as keyof typeof ASPECTS }) },
          ...Object.entries(ASPECTS).map(([k, a]) => h("option", { value: k, selected: k === p.canvas.aspect }, `${a.label} (${a.hint})`)),
        ),
      ),
      h("p", { class: "muted" }, `${p.canvas.width}×${p.canvas.height} at ${(p.canvas.frame_rate.num / p.canvas.frame_rate.den).toFixed(2)} fps`),
      h("p", { class: "muted" }, tl.main.length ? "Select a clip on the timeline to edit it." : "Add clips from the Media panel to start your edit."),
      // Hints follow the active key set (a pro-style S is snapping, not split).
      h(
        "ul",
        { class: "hints muted" },
        ...(
          [
            ["split", " splits at the playhead"],
            ["rippleDelete", " removes a clip and closes the gap"],
            ["cheatSheet", " lists every shortcut"],
          ] as const
        ).map(([a, text]) => h("li", {}, h("kbd", {}, keyFor(a) ?? "?"), text)),
      ),
    ];
  }
  box.replaceChildren(...kids.filter((k): k is HTMLElement => !!k));
  if (focused) document.getElementById(focused)?.focus();
}

// ---- menu ----
/** The first key for an action in the active key set, as shown to people ("Ctrl+K", "?"). */
const keyFor = (a: Action) => {
  const c = KEYMAPS[state.keymap].keys[a]?.[0];
  return c ? displayChord(c) : null;
};

function renderMenu(): void {
  // Every action button says what it does and its key on hover (e.g. "Trim clip start to playhead (Q)").
  for (const b of document.querySelectorAll<HTMLElement>("#editor [data-action]")) {
    const a = b.dataset.action as Action;
    const k = keyFor(a);
    b.title = k ? `${ACTIONS[a]} (${k})` : ACTIONS[a];
  }
  $("#actions-list").replaceChildren(
    ...(Object.entries(ACTIONS) as [Action, string][]).map(([a, label]) => {
      const k = keyFor(a);
      return h("li", {}, h("button", { type: "button", class: "menu__item", "data-action": a, "data-menu": "" }, h("span", {}, label), k ? h("kbd", {}, k) : null));
    }),
  );
}

export function enterEditor(snap: Snapshot): void {
  $("#welcome").hidden = true;
  $("#editor").hidden = false;
  setKeymap(snap.project.keymap);
  applySnap(snap);
  update({ playhead: 0, rate: 0, selection: null, selectedMedia: null, markIn: null, markOut: null });
  requestAnimationFrame(() => {
    zoomFit();
    document.dispatchEvent(new Event("editelier:layout"));
  });
  $("#save-state").textContent = snap.project.path ? "Saved" : "Autosave on";
}

let lastProject: Project | null = null;
let lastSel: string | null | undefined;
let lastMedia: string | null | undefined;
let lastKeymap = "";

export function initEditor(): void {
  mountPreview($("#preview"));
  initTimeline(seek);
  window.addEventListener("keydown", onKey);
  // Every [data-action] button anywhere runs its action (top bar, bin, transport, timeline bar, menu, inspector).
  document.addEventListener("click", (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>("[data-action]");
    if (!b || (b as HTMLButtonElement).disabled) return;
    if (b.hasAttribute("data-menu")) $("#actions-menu").hidePopover();
    void run(b.dataset.action as Action);
  });

  const name = $<HTMLInputElement>("#project-name");
  name.addEventListener("change", () => {
    if (name.value.trim()) void edit("project.rename", { name: name.value });
    else name.value = project()?.name ?? "";
  });
  const zoom = $<HTMLInputElement>("#zoom");
  // Slider is logarithmic: 0..1000 -> 2..2000 px per second.
  zoom.addEventListener("input", () => setZoom(2 * 1000 ** (Number(zoom.value) / 1000)));

  // Drops from Explorer onto the bin (browser: names only, the mock fakes the rest).
  const drop = $("#bin-drop");
  drop.addEventListener("dragover", (e) => {
    if (!e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    drop.classList.add("is-over");
  });
  drop.addEventListener("dragleave", () => drop.classList.remove("is-over"));
  drop.addEventListener("drop", (e) => {
    drop.classList.remove("is-over");
    const files = [...(e.dataTransfer?.files ?? [])];
    if (!files.length) return;
    e.preventDefault();
    void importPaths(files.map((f) => f.name));
  });
  // Inside Tauri the shell reports real paths; dropped over the timeline they are appended too.
  void api.listen("tauri://drag-drop", async ({ paths, position }) => {
    const added = await importPaths(paths);
    const r = $("#tl-scroll").getBoundingClientRect();
    const x = position.x / devicePixelRatio;
    const y = position.y / devicePixelRatio;
    if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) for (const m of added) await edit("timeline.append", { media_id: m.id });
  });

  void api.listen("import-progress", ({ done, total, name: n }) => {
    const st = $("#import-status");
    st.hidden = done >= total;
    st.textContent = `Importing ${done} of ${total}: ${n}`;
  });
  void api.listen("proxy-progress", ({ media_id, proxy }) => {
    const m = project()?.media.find((x) => x.id === media_id);
    if (!m) return;
    m.proxy = proxy;
    renderBin(project()!);
    if (proxy.status === "failed") toast(`Couldn't make a preview copy of ${m.name}. It still plays from the original.`, { kind: "warn" });
  });
  void api.listen("playhead", ({ ticks, playing }) => {
    const p = project();
    const end = p ? timelineEnd(p.timeline) : 0;
    if ((state.rate > 0 && ticks >= end) || (state.rate < 0 && ticks <= 0) || !playing) {
      if (state.rate) setRate(0);
      update({ playhead: Math.max(0, Math.min(ticks, end)) });
    } else update({ playhead: ticks });
  });
  void api.listen("autosaved", ({ at }) => {
    $("#save-state").textContent = `Saved ${new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  });

  subscribe((s) => {
    const p = s.snap?.project;
    if (!p) return;
    $("#timecode").textContent = tc(s.playhead);
    const play = $("#play");
    play.setAttribute("aria-label", s.rate ? "Pause" : "Play");
    play.firstElementChild!.className = `hs-icon ${s.rate ? "i-pause" : "i-play"}`;
    $("#snap").setAttribute("aria-pressed", String(s.snapping));
    const zoom = $<HTMLInputElement>("#zoom");
    zoom.value = String(Math.round((Math.log(s.pxPerSec / 2) / Math.log(1000)) * 1000));
    const keymapChanged = s.keymap !== lastKeymap;
    if (keymapChanged) {
      lastKeymap = s.keymap;
      renderMenu();
    }
    if (p !== lastProject) {
      const undo = $<HTMLButtonElement>("#undo");
      const redo = $<HTMLButtonElement>("#redo");
      undo.disabled = !s.snap!.undo_label;
      redo.disabled = !s.snap!.redo_label;
      undo.title = s.snap!.undo_label ? `Undo ${s.snap!.undo_label}` : "Nothing to undo";
      redo.title = s.snap!.redo_label ? `Redo ${s.snap!.redo_label}` : "Nothing to redo";
      if (document.activeElement !== $("#project-name")) $<HTMLInputElement>("#project-name").value = p.name;
      $("#duration").textContent = tc(timelineEnd(p.timeline));
      $("#preview").dataset.aspect = p.canvas.aspect;
      $("#preview").setAttribute("aria-label", `Preview, ${p.canvas.width} by ${p.canvas.height}`);
      if (lastProject && lastProject.id === p.id && s.snap!.undo_label) $("#save-state").textContent = "Saving…";
    }
    if (p !== lastProject || s.selectedMedia !== lastMedia) renderBin(p);
    if (p !== lastProject || s.selection !== lastSel || keymapChanged) renderInspector(p);
    lastProject = p;
    lastSel = s.selection;
    lastMedia = s.selectedMedia;
  });
}
