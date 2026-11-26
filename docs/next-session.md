# Next session

## Before anything
Read `CLAUDE.md`, `docs/DECISIONS.md`, `docs/PLAN.md`, `docs/ACCEPTANCE.md`. Owner steps: `docs/NEEDS-VIN.md`.

## How to work here
Orchestrate; all subagents on Opus (D17). Builders work in worktrees (`.claude/worktrees/`, ignored) on `m1-*` branches. Branches forked from 06cbb44 conflict on root `Cargo.toml`/`Cargo.lock`: union the `members`, keep main's lock, run `cargo metadata`, commit.

## Current state (2026-11-26)
- End goal (owner): easy, really good colour-graded cinematic video and photos; colour is the centre (D20, `docs/research/06-colour-grading.md`).
- M1 crates `core`, `media`, `export` and `ui` merged to `main` (0f589ca). FFmpeg runs as LGPL helper processes (D3 amendment).
- Next: integration (render crate with the identity grade stage + DRT and A-COL-01, Tauri command wiring of core/media/export, playback with cpal audio + music mix), then MSIX via winapp, acceptance runner, tag v0.1.0. Then M2 Looks.

## Known gaps
- Export integration test fails intermittently on first run (likely parallel QSV sessions): serialise those tests.
- `logo.spec` "Logo by" links test fails (house markup, pre-existing).
- MP4 `moov` relocation limited to ≤ 4 GiB (D3 amendment).
- 150% DPI untested; flicker in live drag-resize not eyeballed; no CI yet (D16).
