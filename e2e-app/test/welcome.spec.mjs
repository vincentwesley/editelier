// A-UI-01 (Short), A-UI-03 (keymaps), A-UI-05/06 (About) in a fresh app launch (own session).
import { join } from "node:path";
import { browser, $, $$, expect } from "@wdio/globals";
import { connect } from "../agent.mjs";
import { MEDIA } from "../wdio.conf.mjs";
import { axe, key, tl, until } from "./helpers.mjs";

let agent;

/** The keys the open shortcuts sheet lists for "Split at playhead". */
async function splitKeys() {
  return browser.execute(() => {
    const row = [...document.querySelectorAll("#sc-rows tr")].find((r) => r.cells[0].textContent === "Split at playhead");
    return [...row.cells[1].querySelectorAll("kbd")].map((k) => k.textContent);
  });
}

describe("Editelier welcome choices", () => {
  before(async () => {
    agent = await connect();
  });
  after(() => agent?.close());
  // A failed test must not leave a modal open over the next one.
  afterEach(() => browser.execute(() => document.querySelectorAll("dialog[open]").forEach((d) => d.close())));

  it("A-UI-05 A-UI-06: About dialog credits, logo, artist and FFmpeg LGPL notice (axe)", async () => {
    await $('.top button[data-open="about-dialog"]').click();
    const about = $("#about-dialog");
    await expect(about).toBeDisplayed();
    const text = await about.getText();
    // D13: the Welcome footer's house credits card (artist handles, draggable logo) moves into About while open.
    for (const s of ["Designed and built by Vin", "@portus_workstation", "@wason25_pz", "LGPLv2.1"]) expect(text).toContain(s);
    expect(await $("#about-dialog .creator").isExisting()).toBe(true);
    const hrefs = await browser.execute(() => [...document.querySelectorAll("#about-dialog a")].map((a) => a.href));
    for (const h of ["https://ffmpeg.org", "https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html", "https://github.com/vincentwesley/editelier/releases"]) {
      expect(hrefs.some((x) => x.startsWith(h))).toBe(true);
    }
    expect(await $("#about-dialog img.about__logo").isExisting()).toBe(true);
    await axe("about dark");
    await browser.keys("Escape");
    await expect(about).not.toBeDisplayed();
    // ...and back to the Welcome footer on close.
    expect(await $(".welcome__footer .creator").isExisting()).toBe(true);
    expect(await $("#about-dialog .creator").isExisting()).toBe(false);
  });

  it("A-UI-01: Short + Premiere-style -> 1080x1920 canvas, TikTok preset", async () => {
    await $("label.fmt*=Short, Reel or TikTok").click();
    await $('input[name="keymap"][value="premiere"]').click();
    await $("#new-project button[type=submit]").click();
    await expect($("#editor")).toBeDisplayed();
    const p = (await agent.call("snapshot")).project;
    expect([p.canvas.width, p.canvas.height, p.export_preset, p.keymap]).toEqual([1080, 1920, "tiktok", "premiere"]);
  });

  it("A-UI-03: Premiere-style splits with Ctrl+K, Resolve-style with Ctrl+B; ? lists the active map", async () => {
    await agent.call("import_media", { paths: [join(MEDIA, "a.mp4")] });
    await until(async () => (await $$("#bin-list > li")).length === 1, "media in the bin", 60_000);
    await $('button[aria-label="Add a.mp4 to the timeline"]').click();
    await until(async () => (await tl(agent)).main.length === 1, "clip on the timeline");
    const len = (await tl(agent)).main[0].source_out;
    await agent.call("seek", { ticks: Math.round(len / 4) });
    await until(async () => (await $("#timecode").getText()) !== "00:00:00;00", "playhead moved");

    await key("Shift", "?");
    await expect($("#shortcuts-dialog")).toBeDisplayed();
    expect(await splitKeys()).toEqual(["Ctrl+K"]);
    await browser.keys("Escape");
    await key("s"); // Premiere-style S is snapping, not split
    await browser.pause(300);
    expect((await tl(agent)).main.length).toBe(1);
    await key("Control", "k");
    await until(async () => (await tl(agent)).main.length === 2, "Ctrl+K split");

    await key("Shift", "?");
    await $("#sc-keymap").selectByAttribute("value", "resolve");
    expect(await splitKeys()).toEqual(["Ctrl+B"]);
    await browser.keys("Escape");
    await expect($("#shortcuts-dialog")).not.toBeDisplayed();
    await agent.call("seek", { ticks: Math.round((len * 3) / 4) });
    await browser.pause(150);
    await key("Control", "b");
    await until(async () => (await tl(agent)).main.length === 3, "Ctrl+B split");
  });
});
