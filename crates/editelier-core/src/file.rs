//! Project JSON. The file is the project's fields plus a top-level `version`.

use crate::{Error, Project};
use serde::{Deserialize, Serialize};

/// The project file format this build writes and reads.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize)]
struct Out<'a> {
    version: u32,
    #[serde(flatten)]
    project: &'a Project,
}

#[derive(Deserialize)]
struct Version {
    version: u64,
}

pub fn to_json(p: &Project) -> String {
    // Serialising plain structs with string keys can't fail.
    serde_json::to_string_pretty(&Out {
        version: FORMAT_VERSION,
        project: p,
    })
    .unwrap_or_default()
}

/// Parse a project file, refusing versions this build doesn't know before reading anything else.
pub fn from_json(s: &str) -> Result<Project, Error> {
    let json = |e: serde_json::Error| Error::Json(e.to_string());
    let Version { version } = serde_json::from_str(s).map_err(json)?;
    if version != u64::from(FORMAT_VERSION) {
        return Err(Error::UnsupportedVersion {
            found: version,
            supported: FORMAT_VERSION,
        });
    }
    let mut p = serde_json::from_str::<Project>(s).map_err(json)?;
    // Never hand out an id that's already in the file.
    let ids = p
        .media
        .iter()
        .map(|m| m.id)
        .chain(p.timeline.main.iter().map(|c| c.id));
    let ids = ids
        .chain(p.timeline.titles.iter().map(|t| t.id))
        .chain(p.timeline.music.iter().map(|m| m.id));
    p.next_id = ids.fold(p.next_id, |n, id| n.max(id.saturating_add(1)));
    Ok(p)
}
