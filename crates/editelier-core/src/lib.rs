//! Editelier's platform-free editing core: tick time, the project model, edit commands shared by
//! UI, CLI and agent, copy-on-write undo, render queries and project JSON. No OS, GPU or IO here.

mod command;
mod file;
mod model;
mod session;

pub use command::Command;
pub use craft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};
pub use file::{FORMAT_VERSION, from_json, to_json};
pub use model::*;
pub use session::Session;

use std::fmt;

/// Why a command or a project load was refused. Messages are safe to show users.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// No clip, title, music clip or media item has this id.
    NotFound(Id),
    /// The command's parameters can't be applied to the current project.
    Invalid(&'static str),
    /// The project file isn't valid JSON for this format.
    Json(String),
    /// The project file was written by a newer (or unknown) format version.
    UnsupportedVersion { found: u64, supported: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(id) => write!(f, "nothing with id {id} in this project"),
            Error::Invalid(why) => f.write_str(why),
            Error::Json(e) => write!(f, "project file is not valid: {e}"),
            Error::UnsupportedVersion { found, supported } => write!(
                f,
                "project file version {found} is not supported (this Editelier reads version {supported}); update Editelier to open it"
            ),
        }
    }
}

impl std::error::Error for Error {}
