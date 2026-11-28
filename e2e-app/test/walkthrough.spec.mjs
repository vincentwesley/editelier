// Usability walkthroughs (next-session step 5) in the real app, a screenshot at every step.
// A: first-time user, buttons only. B: switcher from a pro editor, keys only (pro-style keymap).
// Then Welcome / editor / export / About in both themes at 1280x720 and 1920x1200.
// Screenshots go to qa-output/walkthrough/ (gitignored) via the agent's PrintWindow capture.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { crc32, deflateSync } from "node:zlib";
import { browser, expect } from "@wdio/globals";
import { connect } from "../agent.mjs";
import { MEDIA, OUT } from "../wdio.conf.mjs";

const QA = resolve(import.meta.dirname, "..", "..", "qa-output", "walkthrough");
let agent;
const tl = async () => (await agent.call("snapshot")).project.timeline;
const until = (fn, what, timeout = 20_000) => browser.waitUntil(fn, { timeout, interval: 100, timeoutMsg: `timed out: ${what}` });

// Plain execute() calls: @wdio/tauri-service probes for its plugin (~7 s) before every $/click/find,
// so this spec drives the DOM directly. el.click() runs the same handlers a pointer click does.
const click = (sel) => browser.execute((s) => document.querySelector(s).click(), sel);
const text = (sel) => browser.execute((s) => document.querySelector(s)?.textContent ?? "", sel);
const shown = (sel) => browser.execute((s) => !!document.querySelector(s)?.checkVisibility(), sel);
const count = (sel) => browser.execute((s) => document.querySelectorAll(s).length, sel);
const attr = (sel, a) => browser.execute((s, n) => document.querySelector(s)?.getAttribute(n), sel, a);
const fill = (sel, v) =>
  browser.execute(
    (s, val) => {
      const e = document.querySelector(s);
      e.value = val;
      e.dispatchEvent(new Event("input", { bubbles: true }));
      e.dispatchEvent(new Event("change", { bubbles: true }));
    },
    sel,
    v,
  );
const fmt = (label) => browser.execute((l) => [...document.querySelectorAll("label.fmt")].find((x) => x.textContent.includes(l)).click(), label);
const clip = (n) => `button[aria-label="Add ${n} to the timeline"]`;
const playLabel = () => attr("#play", "aria-label");

/** The agent's 32-bit top-down BMP as a PNG (no image dependency). */
function png(bmp, w, h) {
  const row = w * 4 + 1;
  const raw = Buffer.alloc(row * h);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const s = 54 + (y * w + x) * 4;
      const d = y * row + 1 + x * 4;
      raw[d] = bmp[s + 2];
      raw[d + 1] = bmp[s + 1];
      raw[d + 2] = bmp[s];
      raw[d + 3] = 255;
    }
  const chunk = (type, data) => {
    const td = Buffer.concat([Buffer.from(type), data]);
    const out = Buffer.alloc(8 + td.length);
    out.writeUInt32BE(data.length, 0);
    td.copy(out, 4);
    out.writeUInt32BE(crc32(td), 4 + td.length);
    return out;
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

async function shot(name) {
  await browser.pause(250); // let toasts, dialogs and the preview settle
  const bmp = join(tmpdir(), `editelier-wt-${name}.bmp`);
  const s = await agent.call("screenshot", { path: bmp });
  writeFileSync(join(QA, `${name}.png`), png(readFileSync(bmp), s.width, s.height));
  rmSync(bmp, { force: true });
}

async function key(...keys) {
  await browser.execute(() => document.getElementById("tl-scroll").focus());
  await browser.keys(keys);
}

async function importAll() {
  await agent.call("import_media", { paths: ["a.mp4", "b.mp4", "c.mp4", "tone.wav"].map((f) => join(MEDIA, f)) });
  await until(async () => (await count("#bin-list > li")) === 4, "4 media in the bin", 60_000).catch(async (e) => {
    const why = await browser.execute(() => [document.querySelector("#bin-list")?.innerText, document.querySelector(".hs-toasts")?.innerText, document.querySelector("#import-status")?.innerText].join(" | "));
    throw new Error(`${e.message}: ${why}`);
  });
}

async function toWelcome() {
  await browser.refresh();
  await until(() => shown("#welcome"), "welcome");
}

async function theme(light) {
  if (((await attr("html", "data-theme")) === "light") === light) return;
  await browser.execute(() => [...document.querySelectorAll("[data-theme-toggle]")].find((b) => b.checkVisibility()).click());
  await until(async () => ((await attr("html", "data-theme")) === "light") === light, "theme switched");
}

describe("Usability walkthroughs", () => {
  before(async () => {
    rmSync(QA, { recursive: true, force: true });
    mkdirSync(QA, { recursive: true });
    mkdirSync(OUT, { recursive: true });
    agent = await connect();
  });
  after(() => agent?.close());
  afterEach(() => browser.execute(() => document.querySelectorAll("dialog[open]").forEach((d) => d.close())));

  it("A: first-time user, buttons only: import, trim, split, delete, title, music, export", async () => {
    await until(() => shown("#welcome"), "welcome");
    // First paint of a fresh window: fonts loaded and two frames out before the capture.
    await browser.executeAsync((done) => document.fonts.ready.then(() => requestAnimationFrame(() => requestAnimationFrame(done))));
    await browser.pause(1000);
    await shot("a01-welcome");
    await fmt("YouTube video");
    await click("#new-project button[type=submit]");
    await until(() => shown("#editor"), "editor");
    await shot("a02-empty-editor");

    await importAll();
    await shot("a03-imported");
    for (const n of ["a.mp4", "b.mp4", "c.mp4"]) await click(clip(n));
    await until(async () => (await tl()).main.length === 3, "3 clips");
    await shot("a04-three-clips");

    const len = (await tl()).main[0].source_out;
    await agent.call("seek", { ticks: Math.round(len / 3) });
    await until(async () => (await text("#timecode")) !== "00:00:00;00", "playhead moved");
    await click('.tl-bar [data-action="rippleTrimStart"]');
    await until(async () => (await tl()).main[0].source_in > 0, "trimmed start");
    await shot("a05-trim-start");

    await agent.call("seek", { ticks: Math.round(len / 3) });
    await browser.pause(150);
    await click('.tl-bar [data-action="split"]');
    await until(async () => (await tl()).main.length === 4, "split -> 4");
    await shot("a06-split");

    await click('.tl-bar [data-action="rippleDelete"]');
    await until(async () => (await tl()).main.length === 3, "delete -> 3");
    await shot("a07-deleted");

    await agent.call("seek", { ticks: 0 });
    await browser.pause(150);
    await click('.tl-bar [data-action="addTitle"]');
    await until(async () => (await tl()).titles.length === 1, "title");
    await shot("a08-title-inspector");
    await fill("#in-title-text", "My first video");
    await until(async () => (await tl()).titles[0].text === "My first video", "title text");
    await shot("a09-title-text");

    await click(clip("tone.wav"));
    await until(async () => (await tl()).music.length === 1, "music");
    await shot("a10-music");

    await click('.bar [data-action="export"]');
    await until(() => shown("#export-dialog"), "export dialog");
    await shot("a11-export-dialog");
    await click("#export-pick");
    await until(async () => (await text("#export-folder")) === OUT, "folder");
    await fill("#export-name", `walk-a-${Date.now()}.mp4`);
    await click("#export-go");
    await browser.pause(400);
    await shot("a12-exporting");
    let status;
    await until(async () => /^Exported to |try again/i.test((status = await text("#export-status"))), "export done", 300_000);
    await shot("a13-exported");
    expect(status).toMatch(/^Exported to /);
    await click("#export-go");
  });

  it("B: switcher with the pro-style keymap: ?, Ctrl+K, J/K/L, I/O, Shift+Delete, Ctrl+M", async () => {
    await toWelcome();
    await click('input[name="keymap"][value="premiere"]');
    await shot("b01-welcome-keymap");
    await click("#new-project button[type=submit]");
    await until(() => shown("#editor"), "editor");
    await importAll();
    for (const n of ["a.mp4", "b.mp4", "c.mp4"]) await click(clip(n));
    await until(async () => (await tl()).main.length === 3, "3 clips");
    await shot("b02-editor");
    // The inspector's key hints follow the pro-style set: split is Ctrl+K there, not S.
    expect(await text("#inspector .hints")).toContain("Ctrl+K");
    expect(await attr('.tl-bar [data-action="split"]', "title")).toBe("Split at playhead (Ctrl+K)");

    await key("Shift", "?");
    await until(() => shown("#shortcuts-dialog"), "shortcuts");
    await shot("b03-shortcuts");
    await browser.keys("Escape");

    await key("l");
    await until(async () => (await playLabel()) === "Pause", "L plays");
    await shot("b04-playing-L");
    await key("k");
    await until(async () => (await playLabel()) === "Play", "K stops");

    const len = (await tl()).main[0].source_out;
    await agent.call("seek", { ticks: Math.round(len / 2) });
    await until(async () => (await text("#timecode")) !== "00:00:00;00", "playhead moved");
    await key("Control", "k");
    await until(async () => (await tl()).main.length === 4, "Ctrl+K split");
    await shot("b05-ctrl-k");

    await key("i");
    await agent.call("seek", { ticks: Math.round(len * 1.5) });
    await browser.pause(200);
    await key("o");
    await shot("b06-in-out");

    await key("Shift", "Delete");
    await until(async () => (await tl()).main.length === 3, "Shift+Delete ripple delete");
    await shot("b07-ripple-delete");

    await key("Control", "m");
    await until(() => shown("#export-dialog"), "Ctrl+M export");
    // The in/out marks narrow the export: the dialog says so and offers the whole timeline.
    expect(await text("#export-range-text")).toMatch(/^Only the marked part is exported/);
    await shot("b08-ctrl-m-export");
    await click("#export-all");
    await until(async () => !(await shown("#export-range")), "range note gone");
    await browser.keys("Escape");
  });

  it("themes x sizes: editor, export, About, Welcome", async () => {
    const sizes = [
      [1280, 720],
      [1920, 1200],
    ];
    let resized = true;
    const size = async (w, h) => {
      // setWindowRect leaves WebView2 unpainted and there's no BiDi, so emulate the page size over
      // msedgedriver's CDP endpoint (the window itself stays off-screen). Page at 1 CSS px = 1 device px,
      // top-left of the window; the native preview keeps the real DPI, so it can sit off its box here.
      try {
        const o = browser.options;
        const r = await fetch(`${o.protocol}://${o.hostname}:${o.port}/session/${browser.sessionId}/ms/cdp/execute`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ cmd: "Emulation.setDeviceMetricsOverride", params: { width: w, height: h, deviceScaleFactor: 1, mobile: false } }),
        });
        if (!r.ok) throw new Error(`${r.status} ${(await r.text()).slice(0, 200)}`);
        await browser.pause(300);
      } catch (e) {
        resized = false;
        console.warn(`viewport emulation unsupported (${e.message}); shooting at the default size`);
      }
      expect(await browser.execute(() => window.screenX)).toBeLessThan(-10_000);
    };
    for (const [w, h] of sizes) {
      await size(w, h);
      for (const light of [false, true]) {
        const t = `${resized ? `${w}x${h}` : "default"}-${light ? "light" : "dark"}`;
        await theme(light);
        await shot(`c-editor-${t}`);
        await click('.bar [data-action="export"]');
        await shot(`c-export-${t}`);
        await browser.keys("Escape");
        await click('.bar [data-open="about-dialog"]');
        await shot(`c-about-${t}`);
        await browser.keys("Escape");
      }
      if (!resized) break;
    }
    await toWelcome();
    for (const [w, h] of resized ? sizes : [sizes[0]]) {
      if (resized) await size(w, h);
      for (const light of [false, true]) {
        await theme(light);
        await shot(`c-welcome-${resized ? `${w}x${h}` : "default"}-${light ? "light" : "dark"}`);
      }
    }
    await theme(false);
  });
});
