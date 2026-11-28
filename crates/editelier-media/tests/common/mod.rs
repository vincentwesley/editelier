//! Shared helpers for the integration tests: `test-media/` lookup, the LGPL helper and the oracle.
#![allow(dead_code)]

use editelier_media::Tools;
use std::path::{Path, PathBuf};

/// `test-media/<name>`, or `None` with a SKIP hint when it hasn't been generated.
pub fn media(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-media")
        .join(name);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!(
            "SKIP: {} missing; run scripts/make-test-media.ps1",
            p.display()
        );
        None
    }
}

/// The shipped LGPL FFmpeg helper, or `None` with a SKIP message.
pub fn tools() -> Option<Tools> {
    Tools::locate().map_err(|e| eprintln!("SKIP: {e}")).ok()
}

/// The local (GPL, test-only) oracle ffmpeg.
pub fn oracle() -> String {
    std::env::var("EDITELIER_ORACLE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}
