# Next session

## Before anything
1. Read `CLAUDE.md`, then this file, then `docs/HANDOVER.md`. Use `docs/DECISIONS.md` (D1–D22), `docs/PLAN.md`, `docs/ACCEPTANCE.md` as reference. Owner steps: `docs/NEEDS-VIN.md`.
2. `git status` + `git worktree list`: leftover worktrees under `.claude/worktrees/` are stale agent copies; remove them (`git worktree remove --force --force <path>`, then `git worktree prune`) once their branch is merged.
3. Node 24 (fnm), Rust 1.100 pinned (`rust-toolchain.toml`), VS 2022 Build Tools, tauri-cli 2.12.6, tauri-driver 2.1.0, winapp 0.8.0 are installed on the laptop. Dev FFmpeg (LGPLv3 BtbN, dev only): `scripts/fetch-ffmpeg.ps1` → `third_party/ffmpeg/`. Test media: `cargo xtask fetch-media` (runs `scripts/make-test-media.ps1`; uses the local GPL ffmpeg as generator/oracle only).

## Owner's goal (don't lose this)
Easy, really good colour-graded cinematic video AND photos (vibrant, film-like, Eyes-of-Leo style): one-tap Looks with a strength slider, skin protect, film layer, then pro depth. Colour is the centre (D20, `docs/research/06-colour-grading.md`). Photos share the video pipeline; no retouch/healing/library (D21). Each project has its own logo; the Vin's Atelier logo is only in the credits sign-off (D22, root CLAUDE.md §2).

## How to work here
- Orchestrate only; all subagents on Opus (D17). Dispatch builders with `isolation: worktree`, each told to `git reset --hard main` first, a branch name, allowed folders, exact checks, "no push". Run the Agent tool from inside the repo (cwd must be the repo, or worktree creation fails).
- Merge with `git merge --no-ff <branch>`; if code conflicts, abort and dispatch a builder to resolve (don't hand-merge code).
- Builders sometimes switch the main checkout's branch; check `git branch --show-current` is `main` before merging.
- Usage limits hit often with 4 parallel Opus builders: resume them with SendMessage after the reset instead of re-dispatching.
- Tests: always `EDITELIER_AUDIO=null` (silent sink; the owner heard a test beep once). MF encoder tests run serially (`--test-threads=1`). Never leave windows open: app runs use `--measure` (auto-exits) or the off-screen e2e (`EDITELIER_TEST_OFFSCREEN=1`). Kill `tauri-driver`/`msedgedriver` by PID after e2e.
- `cargo xtask check | layers | fetch-media | package [--dev-cert] | accept <id>|--milestone m1`.
- Security review is required whenever file input, the agent channel (`src-tauri/src/agent.rs`, debug + `agent` feature only) or network/CSP code changes.

## Current state (2026-11-26, end of session 2)
M1 is feature-complete on `main` (pushed): editing core, media (probe/index/MF decode/proxies, rotation, EXIF, A/V offset, streamed audio), renderer (identity grade stage, titles, NV12 out, orientation), playback (cpal + null sink), export (MF QSV/AMF/NVENC/software, presets, loudness + true-peak limiter, zero-copy pipelined), UI (welcome, editor, timeline, keymaps, export dialog, About, own Editelier logo), xtask + MSIX packaging (dev-signed), debug agent channel + off-screen WebdriverIO suite (9/9, incl. export via the UI button), security fix for UNC/WebDAV paths in project files + ffmpeg `-protocol_whitelist file`.

## Next (in order) to finish M1 and tag v0.1.0
1. **Own LGPLv2.1 FFmpeg build** (`packaging/ffmpeg-build.md`, `build-ffmpeg.sh`: no `--enable-version3`, `--disable-network`, h264/hevc/aac software codecs disabled). Needs Docker or WSL2; check which exists, else MSYS2. Then A-LEGAL-01 must pass.
2. **MSIX install checks** A-MSIX-02..04 (blocked until the owner trusts the dev cert; see NEEDS-VIN).
3. ~~Export through the UI in e2e~~ done (`m1-e2e-ui-export`): debug + `agent` builds with `EDITELIER_AGENT=1` answer the folder picker with `EDITELIER_TEST_EXPORT_DIR` (compiled out of release; unit test checks both); e2e 9/9.
4. **Acceptance run** `cargo xtask accept --milestone m1` on an idle laptop; fill gaps marked "not yet automated" (A-UI ids, A-PERF 60 s budgets, A-CUT FFV1 lossless path via FFmpeg helper).
5. **Usability walkthroughs** (first-time user; Premiere switcher) via the agent channel + screenshots; fix what confuses. Then `simplify` + `code-review` (low), `design-review` on headless screenshots, `docs/LEGAL-REVIEW.md` checklist.
6. Add Windows CI (D16), tag `v0.1.0`, GitHub Release with the dev MSIX + cert instructions + portable zip.
7. Then **M2 Looks** (PLAN.md M2, research 06 §6), then the landing page.

## Known gaps
- 10-bit (P010) frames still go through the CPU on the zero-copy path.
- HDR (HLG/PQ) not tone-mapped yet (M2). Video flips from display matrix not handled (rotation is). HEIC untested (no local encoder).
- Shipped ffprobe can't read H.264/HEVC bitstreams; MF fills bit depth + colour for the first video stream only (D23). 4:2:2/4:4:4/12-bit profiles and DCI-P3 primaries stay unknown.
- MF limits: no B-frames, AAC max 192 kbps, moov relocation ≤ 4 GiB, no lossless H.264.
- 150% DPI untested; live drag-resize flicker not eyeballed.
- `e2e-app` npm audit: high findings only via `extract-zip` (puppeteer download, dev-only, unused).
