> Update at the end of any session that changed state. Keep it lean.

Continue work on **Editelier** (repo `vincentwesley/editelier`, local only, not pushed).
Handover 2026-11-25: M0 scaffold done, UI checks green. Only do what the owner asks.

## Before anything
1. Local path `C:\Coding Projects\editelier`. No remote yet.
2. Read `CLAUDE.md` and `docs/research/` (if present).
3. Don't hand-edit `docs/research/`.

## How to work here
- `main` only for now, commit locally, no push.
- UI: `cd ui && npm ci --ignore-scripts && npm run check && npm run build`.
- Rust: none yet; Tauri 2 workspace goes at the repo root, `frontendDist` = `ui/dist`.

## Current state
- M0 scaffold done: repo skeleton, house kit in `ui/src/house/`, welcome empty state + credits card, strict CSP meta.
- Research in progress (`docs/research/`).
- Rust toolchain installing.

## Known gaps / ideas
1. Create the Tauri 2 crate (`src-tauri` or workspace) and wire `frontendDist`.
2. Windows Rust CI (no `.github/workflows/` yet).
3. MSIX packaging + signing (never commit `*.pfx`).
Report outcomes tersely.
