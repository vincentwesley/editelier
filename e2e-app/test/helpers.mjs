// Shared spec helpers. Plain execute() where possible: @wdio/tauri-service probes for its plugin
// before every $/click/find.
import AxeBuilder from "@axe-core/webdriverio";
import { browser, expect } from "@wdio/globals";

export const until = (fn, what, timeout = 20_000) => browser.waitUntil(fn, { timeout, interval: 100, timeoutMsg: `timed out: ${what}` });

/** The open project's timeline, from the agent's snapshot. */
export const tl = async (agent) => (await agent.call("snapshot")).project.timeline;

/** Keyboard shortcuts fire from the page, not from inputs: focus the timeline first. */
export async function key(...keys) {
  await browser.execute(() => document.getElementById("tl-scroll").focus());
  await browser.keys(keys);
}

/** No serious or critical axe violations on the current page. */
export async function axe(where) {
  const r = await new AxeBuilder({ client: browser }).analyze();
  const bad = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(bad.map((v) => `${where} ${v.id}: ${v.nodes.map((n) => n.target).join(" ")}`)).toEqual([]);
}

const isLight = () => browser.execute(() => document.documentElement.getAttribute("data-theme") === "light");

/** Switches to the light (`true`) or dark theme with the visible toggle, if not already there. */
export async function setTheme(light) {
  if ((await isLight()) === light) return;
  await browser.execute(() => [...document.querySelectorAll("[data-theme-toggle]")].find((b) => b.checkVisibility()).click());
  await until(async () => (await isLight()) === light, light ? "light theme" : "dark theme");
}

/** axe in dark, then light, then back to dark. */
export async function bothThemes(where) {
  await setTheme(false);
  await axe(`${where} dark`);
  await setTheme(true);
  await axe(`${where} light`);
  await setTheme(false);
}
