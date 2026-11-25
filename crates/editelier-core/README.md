# editelier-core

Platform-free editing core: project model, named serde `Command`s (shared by UI, CLI and agent), copy-on-write undo `Session`, render queries, project JSON.
Time type: **vendored**, not our own: `Tick`/`TimeRange`/`FrameRate` from craft-libs `craft-time` at `a4db799` (the rev FilmCraft `2d1d1b6` pins), MIT, trimmed to those types in `crates/vendor/craft-time` (see its `PROVENANCE`).
Paths are stored as given; the app layer validates them. Speed is fixed at 1.0 in M1.
