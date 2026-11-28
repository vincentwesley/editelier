// Real-app E2E (D8): WebdriverIO + @wdio/tauri-service (official tauri-driver + msedgedriver)
// against the debug build with the agent channel. Build first (see README.md).
// WebView2 can't run headless, so the window is placed off-screen (EDITELIER_TEST_OFFSCREEN).
import { spawnSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { INFO } from "./agent.mjs";

const root = resolve(import.meta.dirname, "..");
const app = process.env.EDITELIER_APP ?? join(root, "target", "debug", "editelier.exe");
export const MEDIA = join(tmpdir(), "editelier-e2e");
export const OUT = join(tmpdir(), "editelier-e2e-out");

// Inherited by tauri-driver -> msedgedriver -> the app.
Object.assign(process.env, {
  EDITELIER_AGENT: "1",
  EDITELIER_AUDIO: "null",
  EDITELIER_TEST_OFFSCREEN: "1",
  // The export dialog's "Choose…" answers with this folder, no native picker (debug + agent only).
  EDITELIER_TEST_EXPORT_DIR: OUT,
  // Chromium treats an off-screen window as occluded and stops painting; keep it rendering.
  // Tauri's own defaults are repeated because this variable replaces them.
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--disable-features=CalculateNativeWinOcclusion,msWebOOUI,msPdfOOUI,msSmartScreenProtection",
});

/** Synthetic clips from the local ffmpeg on PATH (GPL test oracle, never shipped). */
function makeMedia() {
  rmSync(MEDIA, { recursive: true, force: true });
  mkdirSync(MEDIA, { recursive: true });
  const ff = (args) => {
    const r = spawnSync("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", ...args], { stdio: "inherit" });
    if (r.status !== 0) throw new Error(`ffmpeg failed: ${args.join(" ")}`);
  };
  const clip = (src, name) => ff(["-f", "lavfi", "-i", `${src}${src.includes("=") ? ":" : "="}size=1280x720:rate=30:duration=3`, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "30", join(MEDIA, name)]);
  clip("testsrc2", "a.mp4");
  clip("smptebars", "b.mp4");
  clip("color=c=0x2060c0", "c.mp4");
  ff(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=8", "-ac", "2", join(MEDIA, "tone.wav")]);
}

/** PIDs of the driver chain and the app (tasklist, no dependency). */
function e2ePids() {
  const pids = new Set();
  for (const exe of ["tauri-driver.exe", "msedgedriver.exe", "editelier.exe"]) {
    const r = spawnSync("tasklist", ["/FO", "CSV", "/NH", "/FI", `IMAGENAME eq ${exe}`], { encoding: "utf8" });
    for (const l of (r.stdout ?? "").split("\n")) {
      const pid = Number(l.split('","')[1]);
      if (pid) pids.add(pid);
    }
  }
  return pids;
}
let before = new Set();

export const config = {
  runner: "local",
  specs: ["./test/*.spec.mjs"],
  maxInstances: 1,
  logLevel: "warn",
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 600_000 },
  reporters: ["spec"],
  waitforTimeout: 20_000,
  services: [["@wdio/tauri-service", { driverProvider: "external", appBinaryPath: app, autoDownloadEdgeDriver: true }]],
  capabilities: [{ browserName: "tauri", "tauri:options": { application: app } }],
  onPrepare() {
    rmSync(INFO, { force: true });
    makeMedia();
    before = e2ePids();
  },
  // The service stops tauri-driver but not its msedgedriver child (nor a stuck app): stop only the
  // PIDs this run started, so `npm test` alone leaves nothing behind (xtask keeps a backstop).
  onComplete() {
    for (const pid of e2ePids()) if (!before.has(pid)) spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"]);
  },
};
