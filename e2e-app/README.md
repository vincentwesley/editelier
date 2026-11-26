# e2e-app: real-app UI tests (D8)

WebdriverIO + `@wdio/tauri-service` (official `tauri-driver`, matching msedgedriver auto-downloaded) driving the **debug** build with the agent channel (D7). Milestone-only: WebView2 can't run headless, so the window is placed off-screen (`EDITELIER_TEST_OFFSCREEN=1`) and never shows on the desktop.

Setup (once): `cargo install tauri-driver --version 2.1.0 --locked`, `scripts/fetch-ffmpeg.ps1`, system `ffmpeg`/`ffprobe` on PATH (GPL test oracle: makes the synthetic clips and probes the export; never shipped).

Run:

```sh
npm --prefix ui run build                                   # UI embedded via custom-protocol
cargo build -p editelier --features agent,custom-protocol   # debug only; release + agent is a compile error
cd e2e-app && npm ci --ignore-scripts && npm test
```

The suite sets `EDITELIER_AGENT=1`, `EDITELIER_AUDIO=null` (no sound) and `EDITELIER_TEST_OFFSCREEN=1`. Native pickers are never opened (they would show a window): media comes in through the agent's `import_media` (the shell's drop path) and the export through `export_start`. Media and the export go to `%TEMP%\editelier-e2e*`. `tauri-driver`/`msedgedriver` can outlive the run; stop them by PID. `agent.mjs` is the pipe client (`%TEMP%\editelier-agent.json` holds the pipe name and token while the app runs).
