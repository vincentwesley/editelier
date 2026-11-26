// Welcome: "What are you making?" (canvas + export preset), optional "Coming from" (keymap), recent projects.
import * as api from "./api/commands";
import type { AspectId, KeymapId } from "./api/types";
import { $, h } from "./dom";
import { enterEditor } from "./editor";
import { ASPECTS } from "./model";
import { showError } from "./toast";

const COMING: [KeymapId, string, string][] = [
  ["editelier", "New to editing, or CapCut", "Simple single keys"],
  ["premiere", "Premiere Pro", "Premiere-style keys"],
  ["resolve", "DaVinci Resolve", "Resolve-style keys"],
];

async function open(path: string | null): Promise<void> {
  if (!path) return;
  try {
    enterEditor(await api.projectOpen(path));
  } catch (e) {
    showError(e);
  }
}

async function renderRecent(): Promise<void> {
  const list = await api.recentProjects().catch(() => []);
  $("#recent-empty").hidden = list.length > 0;
  $("#recent-list").replaceChildren(
    ...list.slice(0, 8).map((r) =>
      h(
        "li",
        {},
        h("button", { type: "button", class: "recent__item", onclick: () => void open(r.path) }, h("span", { class: "recent__name" }, r.name), h("span", { class: "recent__path muted" }, r.path)),
      ),
    ),
  );
}

export function initWelcome(): void {
  $("#formats").replaceChildren(
    ...(Object.entries(ASPECTS) as [AspectId, (typeof ASPECTS)[AspectId]][]).map(([id, a], i) =>
      h(
        "label",
        { class: "fmt" },
        h("input", { type: "radio", name: "aspect", value: id, checked: i === 0, class: "sr-only" }),
        h("span", { class: "fmt__frame", "data-aspect": id, "aria-hidden": "true" }),
        h("span", { class: "fmt__label" }, a.label),
        h("span", { class: "fmt__hint mono" }, `${a.hint}, ${a.canvas.width}×${a.canvas.height}`),
      ),
    ),
  );
  let saved: string | null = null;
  try {
    saved = localStorage.getItem("keymap");
  } catch {
    /* no storage */
  }
  $("#coming").replaceChildren(
    ...COMING.map(([id, label, hint]) =>
      h("label", { class: "pick" }, h("input", { type: "radio", name: "keymap", value: id, checked: id === (saved ?? "editelier"), "aria-describedby": "coming-hint" }), h("span", {}, label, h("span", { class: "muted" }, ` (${hint})`))),
    ),
  );

  const form = $<HTMLFormElement>("#new-project");
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const f = new FormData(form);
    const aspect = (f.get("aspect") as AspectId) ?? "16:9";
    try {
      const snap = await api.projectNew({
        name: String(f.get("name") || "Untitled project"),
        aspect,
        export_preset: ASPECTS[aspect].preset,
        keymap: (f.get("keymap") as KeymapId) ?? "editelier",
      });
      enterEditor(snap);
    } catch (err) {
      showError(err);
    }
  });
  $("#open-project").addEventListener("click", async () => open(await api.pickProject().catch((e) => (showError(e), null))));
  void renderRecent();
}
