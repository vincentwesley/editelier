// Renders brand SVGs to PNG with headless Chromium (Playwright from ui/node_modules).
// Usage: node brand/render.mjs   -> brand/png/{previews, icon-1024.png}
import { createRequire } from "node:module";
import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const { chromium } = createRequire(join(here, "../ui/package.json"))("@playwright/test");
const out = join(here, "png");
mkdirSync(out, { recursive: true });

const browser = await chromium.launch();
const page = await browser.newPage();
async function shot(svgFile, size, file, bg = "transparent") {
  const svg = readFileSync(join(here, svgFile), "utf8").replace("<svg ", `<svg width="${size}" height="${size}" `);
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(`<style>html,body{margin:0;background:${bg}}svg{display:block}</style>${svg}`);
  writeFileSync(join(out, file), await page.screenshot({ omitBackground: bg === "transparent" }));
}

await shot("icon-tile.svg", 1024, "icon-1024.png"); // cargo tauri icon source
await shot("mark-dark.svg", 1024, "mark-dark-1024.png"); // packaging/make-assets.ps1 source
for (const s of [16, 32, 64, 256]) {
  await shot("icon-tile.svg", s, `icon-${s}.png`);
  for (const f of ["a-tracks", "b-channels", "c-chart"]) await shot(`explore/${f}.svg`, s, `explore-${f}-${s}.png`, "#11150e");
  await shot("mark-light.svg", s, `mark-light-${s}.png`, "#eef7f1");
}
// Contact sheet: each option at real 16/32/64/256 px on dark, plus the chosen mark on light, for eyeballing.
const row = (f, bg) => `<div style="background:${bg};display:flex;gap:16px;align-items:end;padding:12px">${[16, 32, 64, 256]
  .map((s) => `<img src="${f}-${s}.png" width="${s}" height="${s}">`).join("")}</div>`;
await page.setViewportSize({ width: 480, height: 1500 });
writeFileSync(join(out, "sheet.html"), `<body style="margin:0">${["a-tracks", "b-channels", "c-chart"].map((f) => row(`explore-${f}`, "#11150e")).join("")}${row("mark-light", "#eef7f1")}${row("icon", "#333")}${["dark:#11150e", "light:#eef7f1"].map((t) => `<div style="background:${t.split(":")[1]};padding:12px"><img src="../wordmark-${t.split(":")[0]}.svg" height="48"> <img src="../wordmark-${t.split(":")[0]}.svg" height="24"></div>`).join("")}</body>`);
await page.goto("file:///" + join(out, "sheet.html").replace(/\\/g, "/"));
writeFileSync(join(out, "sheet.png"), await page.screenshot({ fullPage: true }));
await browser.close();
