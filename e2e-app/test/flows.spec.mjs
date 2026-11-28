// A-UI flows in the real app. Native pickers can't be driven: media comes in through the agent's
// import_media (the shell's own drop path); the export folder picker answers with
// EDITELIER_TEST_EXPORT_DIR (debug + agent builds only), so the UI Export button runs for real.
import AxeBuilder from "@axe-core/webdriverio";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { browser, $, $$, expect } from "@wdio/globals";
import { connect } from "../agent.mjs";
import { MEDIA, OUT } from "../wdio.conf.mjs";

let agent;
const snap = () => agent.call("snapshot");
const tl = async () => (await snap()).project.timeline;

async function until(fn, what, timeout = 20_000) {
  await browser.waitUntil(fn, { timeout, interval: 100, timeoutMsg: `timed out: ${what}` });
}

async function axe(where) {
  const r = await new AxeBuilder({ client: browser }).analyze();
  const bad = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(bad.map((v) => `${where} ${v.id}: ${v.nodes.map((n) => n.target).join(" ")}`)).toEqual([]);
}

async function bothThemes(where) {
  const toggle = async () => {
    for (const b of await $$("[data-theme-toggle]")) if (await b.isDisplayed()) return b.click();
  };
  await axe(`${where} dark`);
  await toggle();
  await until(async () => (await $("html").getAttribute("data-theme")) === "light", "light theme");
  await axe(`${where} light`);
  await toggle();
  await until(async () => (await $("html").getAttribute("data-theme")) !== "light", "dark theme");
}

/** Seek through the agent; the UI follows from the shell's playhead event. */
async function seek(ticks) {
  await agent.call("seek", { ticks });
  await browser.pause(150);
}

async function key(...keys) {
  // Shortcuts fire from the page, not from inputs: focus the timeline first.
  await browser.execute(() => document.getElementById("tl-scroll").focus());
  await browser.keys(keys);
}

/** The 1080p export of the 3-clip timeline: H.264 1920x1080 + AAC, 9 s. */
function probe1080(file) {
  expect(existsSync(file) && statSync(file).size > 10_000).toBe(true);
  const r = spawnSync("ffprobe", ["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height:format=duration", "-of", "json", file], { encoding: "utf8" });
  const info = JSON.parse(r.stdout);
  const v = info.streams.find((s) => s.codec_type === "video");
  expect([v.codec_name, v.width, v.height]).toEqual(["h264", 1920, 1080]);
  expect(info.streams.some((s) => s.codec_type === "audio" && s.codec_name === "aac")).toBe(true);
  // 3 clips of 3 s; the split keeps the length.
  expect(Math.abs(Number(info.format.duration) - 9)).toBeLessThan(0.2);
}

describe("Editelier real app", () => {
  before(async () => {
    mkdirSync(OUT, { recursive: true });
    agent = await connect();
  });
  after(() => agent?.close());

  it("A-UI-01 A-UI-05: welcome -> YouTube video -> editor (axe both themes)", async () => {
    await expect($("#welcome")).toBeDisplayed();
    // Off-screen: never pops up on the owner's desktop.
    expect(await browser.execute(() => window.screenX)).toBeLessThan(-10_000);
    // rAF and painting still run off-screen (occlusion off): a frame callback must land.
    expect(await browser.executeAsync((done) => requestAnimationFrame(() => done(document.visibilityState)))).toBe("visible");
    await bothThemes("welcome");
    await $("label.fmt*=YouTube video").click();
    await $("#new-project button[type=submit]").click();
    await expect($("#editor")).toBeDisplayed();
    const p = (await snap()).project;
    expect([p.canvas.width, p.canvas.height, p.export_preset]).toEqual([1920, 1080, "youtube-1080"]);
    await bothThemes("editor");
  });

  it("A-UI-02: imports three clips and appends them via the UI", async () => {
    const files = ["a.mp4", "b.mp4", "c.mp4", "tone.wav"].map((f) => join(MEDIA, f));
    await agent.call("import_media", { paths: files });
    await until(async () => (await $$("#bin-list > li")).length === 4, "4 media in the bin", 60_000);
    for (const n of ["a.mp4", "b.mp4", "c.mp4"]) await $(`button[aria-label="Add ${n} to the timeline"]`).click();
    await until(async () => (await tl()).main.length === 3, "3 clips on the timeline");
  });

  it("A-UI-02: preview frame changes after a seek (frame_hash)", async () => {
    const main = (await tl()).main;
    const len = main[0].source_out - main[0].source_in;
    await seek(0);
    const a = await agent.call("frame_hash");
    await seek(len + len / 2); // middle of clip 2 (bars)
    const b = await agent.call("frame_hash");
    expect(a.width).toBe(1920);
    expect(a.hash).not.toBe(b.hash);
    await seek(0);
    expect((await agent.call("frame_hash")).hash).toBe(a.hash);
  });

  it("A-UI-02 A-UI-04: splits at the playhead with S, then undo/redo by keys and buttons", async () => {
    const main = (await tl()).main;
    await seek((main[0].source_out - main[0].source_in) / 2);
    await until(async () => (await $("#timecode").getText()) !== "00:00:00;00", "UI playhead moved");
    await key("s");
    await until(async () => (await tl()).main.length === 4, "split -> 4 clips");
    await key("Control", "z");
    await until(async () => (await tl()).main.length === 3, "undo -> 3");
    await key("Control", "Shift", "z");
    await until(async () => (await tl()).main.length === 4, "redo -> 4");
    await $("#undo").click();
    await until(async () => (await tl()).main.length === 3, "Undo button -> 3");
    await $("#redo").click();
    await until(async () => (await tl()).main.length === 4, "Redo button -> 4");
  });

  it("A-UI-02: adds a title and music via the UI", async () => {
    await $('button[data-action="addTitle"]').click();
    await until(async () => (await tl()).titles.length === 1, "title added");
    await $('button[aria-label="Add tone.wav to the timeline"]').click();
    await until(async () => (await tl()).music.length === 1, "music added");
  });

  it("A-UI-04 A-UI-05: keyboard shortcuts: ? sheet, Ctrl+T title, Escape", async () => {
    await key("Shift", "?");
    await expect($("#shortcuts-dialog")).toBeDisplayed();
    await axe("shortcuts");
    await browser.keys("Escape");
    await expect($("#shortcuts-dialog")).not.toBeDisplayed();
    await key("Control", "t");
    await until(async () => (await tl()).titles.length === 2, "Ctrl+T title");
    await key("Control", "z");
    await until(async () => (await tl()).titles.length === 1, "undo title");
  });

  it("A-UI-02 A-UI-05: exports 1080p to a temp folder and the file probes right", async () => {
    await $('button[data-action="export"]').click();
    await expect($("#export-dialog")).toBeDisplayed();
    await axe("export");
    await browser.keys("Escape");
    // The folder field only takes the native picker (never opened here: it would show a window),
    // so the export itself starts through the agent with the same preset and path checks.
    const name = `e2e-${Date.now()}`;
    const { job } = await agent.call("export_start", { preset: "youtube-1080", path: join(OUT, `${name}.mp4`) });
    let st;
    await until(async () => ["done", "failed"].includes((st = await agent.call("export_status", { job })).state), "export done", 300_000);
    expect(st.error ?? null).toBe(null);
    expect(st.state).toBe("done");
    probe1080(st.path);
  });

  it("A-UI-02: exports through the UI: Choose folder, Export button, file probes right", async () => {
    await $('button[data-action="export"]').click();
    await expect($("#export-dialog")).toBeDisplayed();
    await $('input[name="preset"][value="youtube-1080"]').click();
    await $("#export-pick").click();
    await until(async () => (await $("#export-folder").getText()) === OUT, "test folder picked");
    const name = `e2e-ui-${Date.now()}.mp4`;
    await $("#export-name").setValue(name);
    await $("#export-go").click();
    let status;
    await until(async () => /^Exported to |try again/i.test((status = await $("#export-status").getText())), "UI export done", 300_000);
    expect(status).toBe(`Exported to ${join(OUT, name)}`);
    probe1080(join(OUT, name));
    await $("#export-go").click(); // "Done" closes the dialog
    await expect($("#export-dialog")).not.toBeDisplayed();
  });

  it("agent screenshot lands only in temp", async () => {
    const path = join(tmpdir(), `editelier-e2e-${Date.now()}.bmp`);
    const s = await agent.call("screenshot", { path });
    expect(s.width).toBeGreaterThan(0);
    expect(statSync(path).size).toBe(54 + s.width * s.height * 4);
    await expect(agent.call("screenshot", { path: "C:\\Windows\\x.bmp" })).rejects.toThrow(/temp folder/);
  });
});
