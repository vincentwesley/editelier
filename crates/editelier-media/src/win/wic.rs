//! Still images through WIC (JPEG, PNG, HEIC with the Store HEIF extension, ...).

use super::{err, init};
use crate::{Error, ImageInfo, MediaKind, ProbeInfo, Result};
use std::path::Path;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VT_UI2;
use windows_core::{HSTRING, Interface, w};

const HEIF_HELP: &str = "This HEIC/HEIF photo needs the free \"HEIF Image Extensions\" from the \
Microsoft Store: ms-windows-store://pdp/?ProductId=9PMMSR1CGPWG";

/// Orientation tag (0x0112) of a TIFF/EXIF block.
fn tiff_orientation(t: &[u8]) -> Option<u16> {
    let le = t.get(..2)? == b"II";
    let u16_at = |o: usize| {
        let b = [*t.get(o)?, *t.get(o + 1)?];
        Some(if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |o: usize| {
        let b: [u8; 4] = t.get(o..o + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let ifd = u32_at(4)? as usize;
    (0..u16_at(ifd)? as usize)
        .map(|i| ifd + 2 + i * 12)
        .find(|&e| u16_at(e) == Some(0x0112))
        .and_then(|e| u16_at(e + 8))
        .filter(|o| (1..=8).contains(o))
}

/// Orientation from a PNG `eXIf` chunk (WIC doesn't expose it). Reads chunk headers up to IDAT.
fn png_orientation(path: &Path) -> Option<u16> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 8];
    f.read_exact(&mut head).ok()?;
    (head == *b"\x89PNG\r\n\x1a\n").then_some(())?;
    loop {
        f.read_exact(&mut head).ok()?;
        let len = u32::from_be_bytes(head[..4].try_into().ok()?) as usize;
        match &head[4..] {
            b"IDAT" | b"IEND" => return None,
            b"eXIf" if len <= 1 << 20 => {
                let mut t = vec![0u8; len];
                f.read_exact(&mut t).ok()?;
                return tiff_orientation(&t);
            }
            _ => {
                f.seek(SeekFrom::Current(len as i64 + 4)).ok()?;
            }
        }
    }
}

/// EXIF orientation (1..=8) of a decoded frame (PNG: from its `eXIf` chunk); 1 when absent.
unsafe fn orientation(path: &Path, frame: &IWICBitmapFrameDecode) -> u16 {
    if let Some(o) = png_orientation(path) {
        return o;
    }
    let mut orientation = 1;
    unsafe {
        if let Ok(q) = frame.GetMetadataQueryReader() {
            let mut pv = PROPVARIANT::default();
            if q.GetMetadataByName(w!("System.Photo.Orientation"), &mut pv)
                .is_ok()
            {
                let inner = &pv.Anonymous.Anonymous;
                if inner.vt == VT_UI2 && (1..=8).contains(&inner.Anonymous.uiVal) {
                    orientation = inner.Anonymous.uiVal;
                }
                let _ = PropVariantClear(&mut pv);
            }
        }
    }
    orientation
}

/// The WIC transform that shows an EXIF-oriented picture upright.
fn transform(orientation: u16) -> WICBitmapTransformOptions {
    let (r90, r180, r270, fh, fv) = (
        WICBitmapTransformRotate90.0,
        WICBitmapTransformRotate180.0,
        WICBitmapTransformRotate270.0,
        WICBitmapTransformFlipHorizontal.0,
        WICBitmapTransformFlipVertical.0,
    );
    WICBitmapTransformOptions(match orientation {
        2 => fh,
        3 => r180,
        4 => fv,
        5 => r270 | fh,
        6 => r90,
        7 => r90 | fh,
        8 => r270,
        _ => 0,
    })
}

/// Decodes a still to RGBA8 (straight alpha), turned upright by its EXIF orientation.
pub fn decode_image(path: &Path) -> Result<crate::Frame> {
    init()?;
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(err("WIC"))?;
        let decoder = factory
            .CreateDecoderFromFilename(
                &HSTRING::from(path),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(|e| {
                Error::Unsupported(format!("Windows can't open this image: {}", e.message()))
            })?;
        let frame = decoder.GetFrame(0).map_err(err("image frame"))?;
        let source: IWICBitmapSource = match orientation(path, &frame) {
            1 => frame.cast().map_err(err("image frame"))?,
            o => {
                let rot = factory
                    .CreateBitmapFlipRotator()
                    .map_err(err("flip rotator"))?;
                rot.Initialize(&frame, transform(o))
                    .map_err(err("orient image"))?;
                rot.cast().map_err(err("orient image"))?
            }
        };
        let conv = factory
            .CreateFormatConverter()
            .map_err(err("format converter"))?;
        conv.Initialize(
            &source,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .map_err(err("convert to RGBA"))?;
        let (mut width, mut height) = (0, 0);
        conv.GetSize(&mut width, &mut height)
            .map_err(err("image size"))?;
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(Error::Unsupported(format!(
                "images up to 16384 pixels per side are supported (this one is {width}x{height})"
            )));
        }
        let stride = width as usize * 4;
        let mut bytes = vec![0u8; stride * height as usize];
        conv.CopyPixels(std::ptr::null(), stride as u32, &mut bytes)
            .map_err(err("image pixels"))?;
        Ok(crate::Frame {
            pts: 0,
            width,
            height,
            format: crate::PixelFormat::Rgba8,
            data: crate::FrameData::Cpu { bytes, stride },
        })
    }
}

/// Display dimensions (orientation applied), format and EXIF orientation of a still.
pub fn probe_image(path: &Path) -> Result<ProbeInfo> {
    init()?;
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(err("WIC"))?;
        let heif = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"));
        let decoder = factory
            .CreateDecoderFromFilename(
                &HSTRING::from(path),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(|e| {
                Error::Unsupported(if heif {
                    HEIF_HELP.into()
                } else {
                    format!("Windows can't open this image: {}", e.message())
                })
            })?;
        let frame = decoder.GetFrame(0).map_err(err("image frame"))?;
        let (mut width, mut height) = (0, 0);
        frame
            .GetSize(&mut width, &mut height)
            .map_err(err("image size"))?;
        let fmt = decoder.GetContainerFormat().map_err(err("image format"))?;
        let format = [
            (GUID_ContainerFormatJpeg, "jpeg"),
            (GUID_ContainerFormatPng, "png"),
            (GUID_ContainerFormatHeif, "heif"),
            (GUID_ContainerFormatBmp, "bmp"),
            (GUID_ContainerFormatTiff, "tiff"),
            (GUID_ContainerFormatGif, "gif"),
            (GUID_ContainerFormatWebp, "webp"),
        ]
        .iter()
        .find(|(g, _)| *g == fmt)
        .map_or("image", |(_, n)| n)
        .to_string();
        let orientation = orientation(path, &frame);
        if orientation >= 5 {
            (width, height) = (height, width);
        }
        Ok(ProbeInfo {
            path: path.into(),
            kind: MediaKind::Image,
            container: format.clone(),
            duration_us: None,
            video: Vec::new(),
            audio: Vec::new(),
            image: Some(ImageInfo {
                width,
                height,
                orientation,
                format,
            }),
        })
    }
}
