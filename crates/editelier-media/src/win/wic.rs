//! Still images through WIC (JPEG, PNG, HEIC with the Store HEIF extension, ...).

use super::{err, init};
use crate::{Error, ImageInfo, MediaKind, ProbeInfo, Result};
use std::path::Path;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VT_UI2;
use windows_core::{HSTRING, w};

const HEIF_HELP: &str = "This HEIC/HEIF photo needs the free \"HEIF Image Extensions\" from the \
Microsoft Store: ms-windows-store://pdp/?ProductId=9PMMSR1CGPWG";

/// Dimensions, format and EXIF orientation of a still.
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
        let mut orientation = 1;
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
