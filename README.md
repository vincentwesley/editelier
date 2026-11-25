# Editelier

Editelier is a free video and photo editor for Windows by Vin's Atelier. Trim and arrange clips, touch up photos and export, in a fast native app built with Tauri 2 (Rust core, TypeScript + Vite UI) and packaged as MSIX. Everything happens on your machine: no account, no cloud, no uploads.

Status: early scaffold (M0). The UI shell lives in `ui/`; the Rust core arrives next. Plan and decisions: `docs/PLAN.md`, `docs/DECISIONS.md`; privacy policy: `docs/PRIVACY.md`.

## Run the UI
```
cd ui
npm ci --ignore-scripts
npm run dev        # browser preview of the UI shell
npm run check      # typecheck + lint + unit
npm run build      # ui/dist (Tauri frontendDist)
```

## Secrets, network and data
- Secrets: none.
- Network calls: none. No telemetry, no analytics.
- Data: all media and projects stay on your device. The UI stores only small preferences (theme, logo size) in local storage.

## Credits
Designed and built by Vin, Vin's Atelier (https://vwesley.dev). Logo by Portus Workstation, Instagram @portus_workstation (art) and @wason25_pz (main); the logo is all rights reserved (see `NOTICE.md`), not covered by the MIT licence.

## Licence
MIT, see `LICENSE`; reserved artwork in `NOTICE.md`; third-party material in `THIRD_PARTY_NOTICES.md`.

Provided as is, no warranty.
