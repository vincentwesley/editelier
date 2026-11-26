# Test media

Everything here except this file is gitignored and **self-generated**: run `scripts/make-test-media.ps1` (needs the local system `ffmpeg` on PATH, the GPL test oracle; it never ships). Add `-Force` to regenerate. No third-party footage, no people.

| File | What it tests |
|---|---|
| `cfr30_1080p_h264_aac.mp4` | 1080p30 H.264 (GOP 60, 2 B-frames) + AAC 48 kHz, burned frame numbers, BT.709 tags |
| `ntsc2997_720p_h264.mp4` / `ntsc5994_720p_h264.mp4` | 30000/1001 and 60000/1001 rates |
| `uhd60_h264.mp4` | 4K60 H.264 (proxy speed) |
| `vfr_jitter_720p_h264.mp4` | VFR: PTS jitter < half a frame + every 7th frame dropped, 90 kHz timescale |
| `hlg10_720p_hevc.mp4` | HEVC Main10, BT.2020 + HLG tags (MF decode needs the HEVC Video Extensions) |
| `rot90_h264.mov` | MOV with a 90° display matrix |
| `vp9_opus_360p.mkv` | VP9 + Opus (royalty-free path) |
| `still_alpha.png`, `still.jpg` | stills (PNG with alpha) |
| `tone_48k.wav`, `tone_44k.mp3` | PCM 48 kHz stereo, MP3 44.1 kHz (resample path) |
| `marker_360p_h264_aac.mp4` | 640x360 H.264 with an asymmetric corner marker (red top-left, lime top-right, blue bottom-left) |
| `phone_rot90cw_h264.mp4` | the marker clip remuxed with a phone-style display matrix: rotate 90° clockwise (ffprobe rotation −90) |
| `av_offset_h264_aac.mp4` | audio starts 250 ms after video (empty edit) + AAC priming edit + B-frame composition edit; 1 kHz burst at exactly 1.000 s |
| `loud_peaky_44k.wav` | quiet 220 Hz bed + near-full-scale transients at 44.1 kHz: reaching −14 LUFS needs the true-peak limiter |
| `orient_base.jpg`, `orient_1.jpg` … `orient_8.jpg` | the corner marker as a JPEG; `orient_N` adds an Exif APP1 with Orientation = N (same stored pixels) |
| `orient_base.png`, `orient_6.png` | the marker as PNG; `orient_6` adds an `eXIf` chunk with Orientation = 6 |

Sources: FFmpeg `testsrc2`, `sine` and `drawtext` (Windows Consolas font, only to render digits into local files). Tests in `crates/editelier-media/tests/media.rs` skip with a message when a file is missing. Oracle frames come from the system ffmpeg (`EDITELIER_ORACLE_FFMPEG` overrides the path).

Not yet covered: HEIC (no local encoder; it goes through the same WIC orientation path), real phone VFR/HDR clips (owner-shot, see `docs/ACCEPTANCE.md`).
