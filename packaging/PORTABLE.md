Editelier portable (Windows 11, x64)
====================================

Free video and photo editor by Vin's Atelier. Provided as is, without warranty (see licences\LICENSE).

Run it
------
1. Unzip the folder anywhere you can write to (for example your Desktop or a USB drive).
2. Run editelier.exe. Nothing is installed and nothing is written to the registry.

Needs the Microsoft Edge WebView2 Runtime. It is preinstalled on Windows 11; if it is missing,
get the Evergreen runtime from https://developer.microsoft.com/microsoft-edge/webview2/

Where your data lives
---------------------
- Projects and exports: wherever you save them.
- Cache (thumbnails, waveforms, frame indexes) and app data (recent projects):
  %LOCALAPPDATA%\dev.vwesley.editelier\cache and \data
- UI settings (theme, keyboard layout): the WebView2 profile in %LOCALAPPDATA%\dev.vwesley.editelier\
Delete that folder to remove everything the app stored. The app makes no network connections.

Replacing FFmpeg (LGPL)
-----------------------
Editelier runs FFmpeg as separate helper programs beside the exe. FFmpeg is licensed under the
GNU LGPL v2.1 or later (licences\FFmpeg\LICENSE.txt) and is not owned by Vin's Atelier; you may
replace it with your own build:

1. Build or obtain an LGPL FFmpeg 8.x *shared* build for Windows x64.
2. Close Editelier.
3. Replace, beside editelier.exe, the files with the same names: ffmpeg.exe, ffprobe.exe and the
   DLLs avcodec-62.dll, avfilter-11.dll, avformat-62.dll, avutil-60.dll, swresample-6.dll,
   swscale-9.dll (plus any extra DLLs your build needs). Keep the file names.
4. Start Editelier again.

Corresponding source for the bundled FFmpeg: the source archives (FFmpeg, dav1d, zlib), the build
script and the configure line are attached to the same GitHub Release as this ZIP
(https://github.com/vincentwesley/editelier/releases). The build script is also in the repository
at packaging/build-ffmpeg.sh; licences\FFmpeg\BUILD-FFMPEG.txt records the exact configuration.

H.264, HEVC and AAC are decoded and encoded by Windows Media Foundation, not FFmpeg; a replacement
FFmpeg does not change that.

Licences
--------
licences\ holds the Editelier licence (MIT), NOTICE.md (logo and art: all rights reserved),
THIRD_PARTY_NOTICES.md, the font licence (OFL.txt) and the FFmpeg licence and build record.
