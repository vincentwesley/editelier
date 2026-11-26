//! `+faststart` for MP4 files: move `moov` in front of `mdat` and shift chunk offsets.
//! Media Foundation's own MF_MPEG4SINK_MOOV_BEFORE_MDAT produced broken files, so the
//! exporter runs this after `Finalize`. Streams the media data; only `moov` is in memory.

use crate::ExportError;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

struct TopBox {
    kind: [u8; 4],
    offset: u64,
    size: u64,
}

fn top_level_boxes(f: &mut File, len: u64) -> io::Result<Vec<TopBox>> {
    let mut boxes = Vec::new();
    let mut pos = 0;
    while pos + 8 <= len {
        let mut h = [0u8; 16];
        f.seek(SeekFrom::Start(pos))?;
        f.read_exact(&mut h[..8])?;
        let mut size = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
        if size == 1 {
            f.read_exact(&mut h[8..])?;
            size = u64::from_be_bytes(h[8..].try_into().unwrap());
        } else if size == 0 {
            size = len - pos;
        }
        if size < 8 || pos + size > len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad MP4 box size",
            ));
        }
        boxes.push(TopBox {
            kind: h[4..8].try_into().unwrap(),
            offset: pos,
            size,
        });
        pos += size;
    }
    Ok(boxes)
}

/// Add `delta` to every `stco`/`co64` entry under `data` (a box payload sequence).
fn shift_chunk_offsets(data: &mut [u8], delta: u64) -> Result<(), ExportError> {
    let bad = || ExportError::Media("malformed moov".into());
    let mut i = 0;
    while i + 8 <= data.len() {
        let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        if size < 8 || i + size > data.len() {
            return Err(bad());
        }
        let kind: [u8; 4] = data[i + 4..i + 8].try_into().unwrap();
        let body = &mut data[i + 8..i + size];
        match &kind {
            b"trak" | b"mdia" | b"minf" | b"stbl" => shift_chunk_offsets(body, delta)?,
            b"stco" | b"co64" => {
                let wide = &kind == b"co64";
                let w = if wide { 8 } else { 4 };
                let n = u32::from_be_bytes(body.get(4..8).ok_or_else(bad)?.try_into().unwrap());
                let entries = body.get_mut(8..8 + n as usize * w).ok_or_else(bad)?;
                for e in entries.chunks_exact_mut(w) {
                    if wide {
                        let v = u64::from_be_bytes(e.try_into().unwrap()) + delta;
                        e.copy_from_slice(&v.to_be_bytes());
                    } else {
                        // ponytail: no stco->co64 upgrade; MF writes co64 itself for files >4 GiB.
                        let v = u32::from_be_bytes(e.try_into().unwrap()) as u64 + delta;
                        let v = u32::try_from(v).map_err(|_| {
                            ExportError::Unsupported("faststart needs co64 (file > 4 GiB)".into())
                        })?;
                        e.copy_from_slice(&v.to_be_bytes());
                    }
                }
            }
            _ => {}
        }
        i += size;
    }
    Ok(())
}

/// Rewrite `path` with `moov` before the first `mdat`. Returns `false` if it already was.
pub fn faststart(path: &Path) -> Result<bool, ExportError> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let boxes = top_level_boxes(&mut f, len)?;
    let find = |k: &[u8; 4]| boxes.iter().position(|b| &b.kind == k);
    let (Some(moov), Some(mdat)) = (find(b"moov"), find(b"mdat")) else {
        return Err(ExportError::Media("MP4 without moov/mdat".into()));
    };
    if moov < mdat {
        return Ok(false);
    }
    if boxes[moov + 1..].iter().any(|b| &b.kind == b"mdat") {
        return Err(ExportError::Unsupported("mdat after moov".into()));
    }
    let m = &boxes[moov];
    let mut moov_data = vec![0u8; m.size as usize];
    f.seek(SeekFrom::Start(m.offset))?;
    f.read_exact(&mut moov_data)?;
    if m.size > u32::MAX as u64 {
        return Err(ExportError::Unsupported("moov > 4 GiB".into()));
    }
    shift_chunk_offsets(&mut moov_data[8..], m.size)?;

    let tmp = path.with_extension("faststart.tmp");
    let result = (|| -> Result<(), ExportError> {
        let mut out = io::BufWriter::new(File::create(&tmp)?);
        for (i, b) in boxes.iter().enumerate() {
            if i == mdat {
                out.write_all(&moov_data)?;
            }
            if i != moov {
                f.seek(SeekFrom::Start(b.offset))?;
                io::copy(&mut (&mut f).take(b.size), &mut out)?;
            }
        }
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(|_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn moves_moov_and_patches_offsets() {
        let ftyp = bx(b"ftyp", b"isom0000");
        let mdat = bx(b"mdat", b"SAMPLE01SAMPLE02");
        let first = (ftyp.len() + 8) as u32; // first sample's offset
        let mut stco = vec![0u8; 4];
        stco.extend_from_slice(&2u32.to_be_bytes());
        stco.extend_from_slice(&first.to_be_bytes());
        stco.extend_from_slice(&(first + 8).to_be_bytes());
        let stbl = bx(b"stbl", &bx(b"stco", &stco));
        let moov = bx(b"moov", &bx(b"trak", &bx(b"mdia", &bx(b"minf", &stbl))));
        let file = [ftyp.clone(), mdat, moov.clone()].concat();
        let dir = std::env::temp_dir().join("editelier-export-mp4-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.mp4");
        std::fs::write(&path, &file).unwrap();

        assert!(faststart(&path).unwrap());
        let out = std::fs::read(&path).unwrap();
        assert_eq!(out.len(), file.len());
        assert_eq!(&out[ftyp.len() + 4..ftyp.len() + 8], b"moov");
        // Read the patched offsets back and check they still point at the samples.
        let at = out.windows(4).position(|w| w == b"stco").unwrap() + 12;
        for (k, want) in [b"SAMPLE01", b"SAMPLE02"].iter().enumerate() {
            let o = u32::from_be_bytes(out[at + k * 4..at + k * 4 + 4].try_into().unwrap());
            assert_eq!(&out[o as usize..o as usize + 8], *want);
        }
        assert!(!faststart(&path).unwrap(), "second run is a no-op");
        std::fs::remove_file(&path).unwrap();
    }
}
