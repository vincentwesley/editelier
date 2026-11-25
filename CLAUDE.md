# CLAUDE.md (Editelier)

Follows `../CLAUDE.md` house rules (orchestrate, house style, safety, identity).

## Purpose
Free Windows desktop video + photo editor. Tauri 2: Rust core at the repo root (workspace, not created yet), TypeScript + Vite UI in `ui/` (`frontendDist` = `ui/dist`), packaged as MSIX.

## Commands (in `ui/`, Node 24, `npm ci --ignore-scripts`)
- `npm run check` = typecheck + lint + unit; `npm run build`; `npm run test:e2e` (headless).

## Notes
- House kit copied into `ui/src/house/`; `ui/public/theme-init.js` is a copy of `house/js/theme-init.js` (classic pre-paint script).
- CSP is the meta tag in `ui/index.html` (mirror it in `tauri.conf.json` when the Rust side lands).
- `docs/research/` is written by research agents; don't edit by hand.
- Keep `docs/next-session.md` current.
