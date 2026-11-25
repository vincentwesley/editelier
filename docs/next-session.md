# Next session

## Before anything
Read `CLAUDE.md`, `docs/DECISIONS.md`, `docs/PLAN.md`, `docs/ACCEPTANCE.md`. Owner steps: `docs/NEEDS-VIN.md`.

## How to work here
Orchestrate; all subagents on Opus (D17). Builders work in worktrees (`.claude/worktrees/`, ignored) on `m1-*` branches. Branches forked from 06cbb44 conflict on root `Cargo.toml`/`Cargo.lock`: union the `members`, keep main's lock, run `cargo metadata`, commit.

## Current state (2026-11-25, session cut by usage limit)
- M0 done and pushed (research, decisions D1-D19, plan, acceptance, legal, privacy).
- Preview spike merged: wgpu DX12 child HWND above WebView2, 60 fps on Iris Xe (`docs/spikes/preview-surface.md`).
- `m1-core` merged locally (editelier-core + vendored craft-time, 11 tests).
- In flight when the session ended (check each branch / worktree for commits or uncommitted work; re-dispatch if incomplete): `m1-media` (probe/index/MF decode/proxies/test media), `m1-export` (MF HW encoders, presets, ebur128), `m1-ui` (editor UI with mock backend).
- Next: merge those, then integration (Tauri commands wiring core+media+export, render crate: NV12→linear compositor + titles via glyphon, audio playback cpal + music mix), MSIX via winapp CLI, acceptance runner (`cargo xtask accept`), walkthroughs, tag v0.1.0.

## Known gaps
150% DPI untested; flicker in live drag-resize not eyeballed; no CI yet (D16).
