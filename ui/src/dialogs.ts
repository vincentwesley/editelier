// Export, Keyboard shortcuts and About dialogs (native <dialog>, opened with showModal()).
import { version } from "../package.json";
import * as api from "./api/commands";
import type { KeymapId, PresetId } from "./api/types";
import { $, h } from "./dom";
import { ACTIONS, displayChord, KEYMAPS, type Action } from "./keymap";
import { ASPECTS, PRESETS } from "./model";
import { setKeymap, state } from "./state";
import { showError } from "./toast";

const dlg = (id: string) => $<HTMLDialogElement>(`#${id}`);

// ---- export ----
let job: string | null = null;
let lastPath = "";
let folder = "Videos";

function exportMode(mode: "idle" | "running" | "done" | "failed", status = ""): void {
  $("#export-run").hidden = mode === "idle";
  $("#export-cancel").hidden = mode !== "running";
  $("#export-reveal").hidden = mode !== "done";
  const go = $<HTMLButtonElement>("#export-go");
  go.hidden = mode === "running";
  go.textContent = mode === "done" ? "Done" : mode === "failed" ? "Try again" : "Export";
  $("#export-status").textContent = status;
  $<HTMLFieldSetElement>(".presets").disabled = mode === "running";
}

// Drops characters Windows forbids in file names, and control characters.
const safeName = (s: string) => [...s].filter((ch) => ch >= " " && !'<>:"/\\|?*'.includes(ch)).join("").trim();

export async function openExport(): Promise<void> {
  const p = state.snap?.project;
  if (!p) return;
  const d = dlg("export-dialog");
  if (!job) {
    const grid = $("#presets");
    const pick = p.export_preset ?? ASPECTS[p.canvas.aspect].preset;
    grid.replaceChildren(
      ...(Object.entries(PRESETS) as [PresetId, (typeof PRESETS)[PresetId]][]).map(([id, pr]) =>
        h(
          "label",
          { class: "preset" },
          h("input", { type: "radio", name: "preset", value: id, checked: id === pick }),
          h("span", { class: "preset__name" }, pr.label),
          h("span", { class: "preset__sum" }, pr.summary),
          pr.aspect && pr.aspect !== p.canvas.aspect ? h("span", { class: "preset__warn" }, `Your canvas is ${p.canvas.aspect}; this adds bars or crops.`) : null,
        ),
      ),
    );
    $<HTMLInputElement>("#export-name").value = `${safeName(p.name) || "Untitled"}.mp4`;
    $("#export-folder").textContent = folder;
    exportMode("idle");
  }
  d.showModal();
  try {
    const enc = await api.exportEncoder();
    $("#export-encoder").textContent = enc.label;
  } catch {
    $("#export-encoder").textContent = "Auto";
  }
}

async function startExport(): Promise<void> {
  const nameInput = $<HTMLInputElement>("#export-name");
  let name = safeName(nameInput.value);
  if (!name) {
    nameInput.setAttribute("aria-invalid", "true");
    exportMode("failed", "Give the file a name.");
    return nameInput.focus();
  }
  nameInput.removeAttribute("aria-invalid");
  if (!/\.mp4$/i.test(name)) name += ".mp4";
  const preset = (new FormData($<HTMLFormElement>("#export-form")).get("preset") as PresetId) ?? "master";
  const range = state.markIn !== null || state.markOut !== null ? { start: state.markIn ?? 0, end: state.markOut ?? Number.MAX_SAFE_INTEGER } : null;
  try {
    exportMode("running", "Starting…");
    $<HTMLProgressElement>("#export-progress").value = 0;
    job = await api.exportStart(preset, `${folder}\\${name}`, range);
  } catch (e) {
    job = null;
    const err = e as { message?: string };
    exportMode("failed", err.message ?? "The export could not start.");
  }
}

// ---- shortcuts ----
function renderShortcuts(): void {
  const q = $<HTMLInputElement>("#sc-search").value.trim().toLowerCase();
  const keys = KEYMAPS[state.keymap].keys;
  const rows = (Object.entries(ACTIONS) as [Action, string][])
    .map(([a, label]) => ({ label, chords: (keys[a] ?? []).map(displayChord) }))
    .filter((r) => !q || r.label.toLowerCase().includes(q) || r.chords.some((c) => c.toLowerCase().includes(q)));
  $("#sc-rows").replaceChildren(
    ...rows.map((r) =>
      h("tr", {}, h("td", {}, r.label), h("td", {}, ...(r.chords.length ? r.chords.flatMap((c, i) => [i ? " or " : "", h("kbd", {}, c)]) : ["Menu only"]))),
    ),
  );
  $("#sc-empty").hidden = rows.length > 0;
}

export function openShortcuts(): void {
  $<HTMLSelectElement>("#sc-keymap").value = state.keymap;
  renderShortcuts();
  dlg("shortcuts-dialog").showModal();
}

export function initDialogs(): void {
  $("#about-version").textContent = version;
  document.addEventListener("click", (e) => {
    const t = e.target as HTMLElement;
    const open = t.closest<HTMLElement>("[data-open]");
    if (open) dlg(open.dataset.open!).showModal();
    if (t.closest("[data-close]")) t.closest("dialog")?.close();
  });
  // Light dismiss: a click on the backdrop lands on the <dialog> itself, outside its box.
  for (const d of document.querySelectorAll("dialog")) {
    d.setAttribute("closedby", "any");
    d.addEventListener("click", (e) => {
      const r = d.getBoundingClientRect();
      if (e.target === d && (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom)) d.close();
    });
  }

  // Export.
  $("#export-form").addEventListener("submit", (e) => {
    e.preventDefault();
    if ($("#export-go").textContent === "Done") {
      exportMode("idle");
      return dlg("export-dialog").close();
    }
    void startExport();
  });
  $("#export-pick").addEventListener("click", async () => {
    try {
      const f = await api.pickFolder();
      if (f) $("#export-folder").textContent = folder = f;
    } catch (e) {
      showError(e);
    }
  });
  $("#export-cancel").addEventListener("click", () => {
    if (job) void api.exportCancel(job);
    job = null;
    exportMode("idle");
    $("#export-run").hidden = false;
    $("#export-status").textContent = "Export cancelled. Nothing was saved.";
  });
  $("#export-reveal").addEventListener("click", () => void api.revealPath(lastPath).catch(showError));
  void api.listen("export-progress", ({ job: j, progress }) => {
      if (j !== job) return;
      $<HTMLProgressElement>("#export-progress").value = progress;
      $("#export-status").textContent = `Exporting… ${Math.round(progress)}%`;
    });
  void api.listen("export-done", ({ job: j, path }) => {
      if (j !== job) return;
      job = null;
      lastPath = path;
      $<HTMLProgressElement>("#export-progress").value = 100;
      exportMode("done", `Exported to ${path}`);
    });
  void api.listen("export-failed", ({ job: j, error }) => {
      if (j !== job) return;
      job = null;
      exportMode("failed", `${error.message} Your settings are kept; try again.`);
    });

  // Shortcuts.
  const sel = $<HTMLSelectElement>("#sc-keymap");
  sel.replaceChildren(...(Object.entries(KEYMAPS) as [KeymapId, { label: string }][]).map(([id, k]) => h("option", { value: id }, k.label)));
  sel.addEventListener("change", () => {
    setKeymap(sel.value as KeymapId);
    renderShortcuts();
  });
  $("#sc-search").addEventListener("input", renderShortcuts);
}
