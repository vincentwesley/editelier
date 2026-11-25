> Update at the end of any session that changed state. Keep it lean.

Continue work on **Editelier** (repo `vincentwesley/editelier`, local only, not pushed).
Handover 2026-11-25: M0 scaffold + research + planning docs done (branch `m0-docs`, not merged). Only do what the owner asks.

## Before anything
1. Local repo folder `editelier/` under Coding Projects. No remote yet.
2. Read `CLAUDE.md`, then `docs/PLAN.md`, `docs/DECISIONS.md`, `docs/ACCEPTANCE.md`. Research index: `docs/RESEARCH.md`.
3. Don't hand-edit `docs/research/`.

## How to work here
- Subagents on Opus (D17). Branch per step, commit locally, no push.
- UI: `cd ui && npm ci --ignore-scripts && npm run check && npm run build`.
- Rust: workspace not created yet; layout in `docs/PLAN.md`.

## Current state
- M0 scaffold done; UI checks green.
- Docs on `m0-docs`: RESEARCH, DECISIONS (D1–D19), PLAN (M1–M10), ACCEPTANCE, LEGAL-REVIEW, NEEDS-VIN, PRIVACY, HANDOVER.
- Rust toolchain install status: check `rustc -V` / VS 2022 Build Tools before M1.

## Next
1. Merge `m0-docs` when the owner OKs it.
2. M1 step 1: preview spike (wgpu DX12 child HWND under WebView2, D1); fallback egui/wgpu pane.
3. Then M1 scope in `docs/PLAN.md`.

## Known gaps
- Owner steps open: trademark searches, Partner Center + name reservation (`docs/NEEDS-VIN.md`).
- Open questions: ProRes/DNx patent status; BiRefNet LICENSE; MSIX DLL-swap interpretation for LGPL.
- No CI until M1 end (D16). Never commit `*.pfx`.
