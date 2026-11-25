# CLAUDE.md (Editelier)

Follows `../CLAUDE.md` house rules (orchestrate, house style, safety, identity). Project overrides below win where they conflict.

## Before anything
Read `docs/next-session.md` first, then `docs/PLAN.md` and `docs/DECISIONS.md`. `docs/research/` is written by research agents; don't hand-edit it.

## Purpose
Free Windows-only desktop video + photo editor by Vin's Atelier: pro depth, beginner ease, everything on-device (no account, watermark, upload, ads or telemetry). Tauri 2 shell, Rust workspace core, TS + Vite UI in `ui/`, wgpu preview in a child HWND, MSIX for the Microsoft Store.

## How to work
- **All subagents run on Opus** (owner override, D17). The orchestrator still only dispatches; builders write code, in parallel, each prompt naming allowed folders, exact checks and "no push".
- Branch per milestone step (`m1-*`); commit locally; no push or merge unless the owner says so. Tag `v0.1.0` at M1.
- Record every new decision as the next `D-n` in `docs/DECISIONS.md` with reason + research link.
- Keep `docs/next-session.md`, `docs/HANDOVER.md` and `docs/NEEDS-VIN.md` current.

## Layout (planned, created in M1; full map in `docs/PLAN.md`)
`src-tauri/` shell · `crates/{core,media,media-win,media-ffmpeg,render,audio,proxy,agent}` · `crates/vendor/` (FilmCraft MIT crates, headers + PROVENANCE kept) · `xtask/` · `ui/` · `site/` (landing, after M1).
Layering: `core` and `media` are platform-free (no Windows, UI or GPU crates); checked by `cargo xtask layers`.

## Commands
- UI (in `ui/`, Node 24, `npm ci --ignore-scripts`): `npm run check` (typecheck + lint + unit), `npm run build`, `npm run test:e2e` (headless).
- Rust (planned): `cargo xtask check` (fmt --check + clippy -D warnings + test), `cargo xtask layers`, `cargo xtask accept <id>` / `--milestone mN` (see `docs/ACCEPTANCE.md`), `cargo xtask fetch-media`, `cargo xtask package` (MSIX via winapp).
- Verification tiers: root `CLAUDE.md` §4. Per change = only the checks the change touches. Perf, VMAF, E2E (WebView2 can't run headless) and MSIX ids run at milestones only.

## Clean-room and legal (hard rules)
- Never copy proprietary assets, icons, layouts or trade dress. Never put Premiere, DaVinci, Resolve or CapCut in the name, icon, Store listing or marketing; factual comparisons in docs and descriptive keymap names only (D19).
- GPL/AGPL projects (LosslessCut, Kdenlive, Shotcut, Palmier, OpenTake, OpenChatCut) are study-only: no code, shaders or text.
- H.264/HEVC/AAC only through Media Foundation; shipped FFmpeg is our LGPL shared build with those software codecs disabled (D3). Never ship the local GPL ffmpeg (test oracle only).
- LGPL libraries only as DLLs or a helper process. No RMBG models. Every bundled third-party file goes in `THIRD_PARTY_NOTICES.md` when it lands (only what's actually in the repo).
- Test media: synthetic fixtures committed; external media fetched by manifest with SHA-256; never Pexels (D15).
- Automation channel is debug + feature-gated only (D7); run `security-review` when it or file-input code changes.
- Release checklist: `docs/LEGAL-REVIEW.md`.

## House style
House kit copied into `ui/src/house/`; `ui/public/theme-init.js` is a copy of `house/js/theme-init.js` (classic pre-paint script). Dark default. Credits card + draggable logo live in the Welcome footer and About dialog (D13). CSP is the meta tag in `ui/index.html` (mirror it in `tauri.conf.json` when the shell lands).

## Hardware
Test laptop i5-1335U / Iris Xe / 16 GB / 1920×1200 (QSV); AMD desktop from Jan 2027.
