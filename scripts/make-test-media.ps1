# Generates small synthetic clips into test-media/ (gitignored) with the LOCAL system ffmpeg.
# The system ffmpeg (gyan full, GPL) is a test oracle only: never ship it or its output codecs' encoders.
# Usage: scripts/make-test-media.ps1 [-Ffmpeg path\to\ffmpeg.exe] [-Force]
param([string]$Ffmpeg = 'ffmpeg', [switch]$Force)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'test-media'
New-Item -ItemType Directory -Force $out | Out-Null

$font = "fontfile='C\:/Windows/Fonts/consola.ttf'"
$num = "drawtext=${font}:text='%{frame_num}':fontsize=120:fontcolor=white:box=1:boxcolor=black@0.8:x=40:y=40"
$sdr = "$num,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv"
$aac = @('-c:a', 'aac', '-b:a', '128k')
$tone = @('-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000')
$x264 = @('-c:v', 'libx264', '-preset', 'veryfast', '-crf', '20', '-pix_fmt', 'yuv420p',
  '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv')

function Make([string]$name, [string[]]$a) {
  $dst = Join-Path $out $name
  if ((Test-Path $dst) -and -not $Force) { return }
  & $Ffmpeg -hide_banner -v error -y @a $dst
  if ($LASTEXITCODE -ne 0) { throw "ffmpeg failed for $name" }
  Write-Host "made $name"
}

# 1080p30 H.264 (GOP 60, B-frames) + AAC, burned frame numbers
Make 'cfr30_1080p_h264_aac.mp4' (@('-f', 'lavfi', '-i', "testsrc2=size=1920x1080:rate=30:duration=4,$sdr") + $tone +
  @('-t', '4') + $x264 + @('-g', '60', '-bf', '2') + $aac + @('-movflags', '+faststart'))
# A-CUT-01: long GOP (250) with B-frames, 10 s
Make 'cfr30_longgop_720p_h264.mp4' (@('-f', 'lavfi', '-i', "testsrc2=size=1280x720:rate=30:duration=10,$sdr") +
  @('-t', '10') + $x264 + @('-g', '250', '-keyint_min', '250', '-sc_threshold', '0', '-bf', '2'))
Make 'ntsc2997_720p_h264.mp4' (@('-f', 'lavfi', '-i', "testsrc2=size=1280x720:rate=30000/1001:duration=3,$sdr") + $tone +
  @('-t', '3') + $x264 + @('-g', '30') + $aac)
Make 'ntsc5994_720p_h264.mp4' (@('-f', 'lavfi', '-i', "testsrc2=size=1280x720:rate=60000/1001:duration=3,$sdr") + $tone +
  @('-t', '3') + $x264 + @('-g', '60') + $aac)
Make 'uhd60_h264.mp4' (@('-f', 'lavfi', '-i', "testsrc2=size=3840x2160:rate=60:duration=3,$sdr") + $tone +
  @('-t', '3') + $x264 + @('-g', '120') + $aac)
# VFR: PTS jitter (< half a frame) plus every 7th frame dropped, 90 kHz timescale
Make 'vfr_jitter_720p_h264.mp4' (@('-f', 'lavfi', '-i', "testsrc2=size=1280x720:rate=30:duration=4,$sdr,settb=1/90000,setpts='(N/30+random(1)*0.012)/TB',select='not(eq(mod(n\,7)\,3))'") +
  @('-fps_mode', 'vfr', '-enc_time_base', '1/90000', '-video_track_timescale', '90000') + $x264 + @('-g', '30'))
# HEVC 10-bit HLG (MF decode needs the HEVC Video Extensions)
Make 'hlg10_720p_hevc.mp4' (@('-f', 'lavfi', '-i', "testsrc2=size=1280x720:rate=30:duration=2,$num") + $tone +
  @('-t', '2', '-c:v', 'libx265', '-preset', 'fast', '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1',
    '-x265-params', 'log-level=error:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc',
    '-color_primaries', 'bt2020', '-color_trc', 'arib-std-b67', '-colorspace', 'bt2020nc', '-color_range', 'tv') + $aac)
# MOV with 90 degree rotation metadata (remuxed from the 29.97 clip)
Make 'rot90_h264.mov' (@('-display_rotation', '90', '-i', (Join-Path $out 'ntsc2997_720p_h264.mp4'), '-c', 'copy'))
# MKV VP9 + Opus (royalty-free path)
Make 'vp9_opus_360p.mkv' (@('-f', 'lavfi', '-i', "testsrc2=size=640x360:rate=30:duration=2,$num") + $tone +
  @('-t', '2', '-c:v', 'libvpx-vp9', '-deadline', 'realtime', '-b:v', '1M', '-g', '30', '-c:a', 'libopus'))
# Stills and audio
Make 'still_alpha.png' @('-f', 'lavfi', '-i', 'testsrc2=size=640x480,format=rgba,geq=r=r(X\,Y):g=g(X\,Y):b=b(X\,Y):a=X/3', '-frames:v', '1')
Make 'still.jpg' @('-f', 'lavfi', '-i', 'testsrc2=size=800x600', '-frames:v', '1', '-q:v', '3')
Make 'tone_48k.wav' @('-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=3', '-ac', '2', '-c:a', 'pcm_s16le')
Make 'tone_44k.mp3' @('-f', 'lavfi', '-i', 'sine=frequency=1000:sample_rate=44100:duration=3', '-c:a', 'libmp3lame', '-b:a', '128k')

# Orientation: asymmetric corner marker (red top-left, lime top-right, blue bottom-left, black bottom-right)
$mark = "drawbox=x=0:y=0:w=iw/4:h=ih/4:c=red:t=fill,drawbox=x=iw*3/4:y=0:w=iw/4:h=ih/4:c=lime:t=fill,drawbox=x=0:y=ih*3/4:w=iw/4:h=ih/4:c=blue:t=fill"
Make 'marker_360p_h264_aac.mp4' (@('-f', 'lavfi', '-i', "color=c=gray:s=640x360:r=30:d=2,$mark,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv") + $tone +
  @('-t', '2') + $x264 + @('-g', '30') + $aac)
# Phone-style portrait clip: landscape-coded, display matrix says rotate 90 degrees clockwise (ffprobe rotation -90)
Make 'phone_rot90cw_h264.mp4' (@('-display_rotation', '-90', '-i', (Join-Path $out 'marker_360p_h264_aac.mp4'), '-c', 'copy'))
# A/V offset: audio starts 250 ms after video (empty edit) + AAC encoder priming (1024-sample edit),
# video has B-frames (composition edit). The 1 kHz burst sits at exactly 1.000 s after the first frame.
Make 'av_offset_h264_aac.mp4' (@('-f', 'lavfi', '-i', "testsrc2=size=640x360:rate=30:duration=3,$sdr",
  '-itsoffset', '0.25', '-f', 'lavfi', '-i', "aevalsrc='if(gte(t,0.75),0.5*sin(2*PI*1000*(t-0.75)),0)':s=48000:d=2.5:c=stereo") +
  @('-map', '0:v', '-map', '1:a') + $x264 + @('-g', '30', '-bf', '2') + $aac)
# Loud, peaky "music": quiet bed + full-scale transients at 44.1 kHz (needs gain AND a true-peak limiter at -14 LUFS)
Make 'loud_peaky_44k.wav' @('-f', 'lavfi', '-i', "aevalsrc='0.08*sin(2*PI*220*t)+0.95*exp(-200*mod(t\,0.5))*sin(2*PI*3000*t)':s=44100:d=10:c=stereo", '-c:a', 'pcm_s16le')

# Export/preview benches (ignored tests export_bench, preview_bench): 20 s, GOP 60 + AAC
foreach ($b in @(@('1080p', '1920x1080'), @('2160p', '3840x2160'))) {
  Make "bench20_$($b[0])30.mp4" (@('-f', 'lavfi', '-i', "testsrc2=size=$($b[1]):rate=30:duration=20,$sdr") + $tone +
    @('-t', '20') + $x264 + @('-g', '60') + $aac + @('-movflags', '+faststart'))
}

# EXIF orientation 1..8: same stored pixels, only the tag differs (APP1 Exif spliced in after SOI).
function Exif-Tiff([int]$o) {
  # Big-endian TIFF, one IFD entry: 0x0112 Orientation, SHORT, count 1
  [byte[]](0x4D,0x4D,0x00,0x2A,0,0,0,8, 0,1, 0x01,0x12,0,3,0,0,0,1,0,$o,0,0, 0,0,0,0)
}
Make 'orient_base.jpg' @('-f', 'lavfi', '-i', "color=c=gray:s=320x240,$mark", '-frames:v', '1', '-q:v', '2')
$base = [IO.File]::ReadAllBytes((Join-Path $out 'orient_base.jpg'))
foreach ($o in 1..8) {
  $dst = Join-Path $out "orient_$o.jpg"
  if ((Test-Path $dst) -and -not $Force) { continue }
  $payload = [byte[]](0x45,0x78,0x69,0x66,0,0) + (Exif-Tiff $o)
  $len = $payload.Length + 2
  $app1 = [byte[]](0xFF,0xE1,[byte]($len -shr 8),[byte]($len -band 255)) + $payload
  [IO.File]::WriteAllBytes($dst, [byte[]]($base[0..1] + $app1 + $base[2..($base.Length - 1)]))
  Write-Host "made orient_$o.jpg"
}
# PNG with an eXIf chunk (orientation 6), inserted after IHDR
function Crc32([byte[]]$b) {
  $c = [uint32]::MaxValue
  foreach ($x in $b) { $c = $c -bxor $x; for ($k = 0; $k -lt 8; $k++) { if ($c -band 1) { $c = ($c -shr 1) -bxor [uint32]3988292384 } else { $c = $c -shr 1 } } }
  $c -bxor [uint32]::MaxValue
}
Make 'orient_base.png' @('-f', 'lavfi', '-i', "color=c=gray:s=320x240,$mark", '-frames:v', '1')
$dst = Join-Path $out 'orient_6.png'
if (-not (Test-Path $dst) -or $Force) {
  $png = [IO.File]::ReadAllBytes((Join-Path $out 'orient_base.png'))
  $data = Exif-Tiff 6
  $body = [byte[]](0x65,0x58,0x49,0x66) + $data
  $crc = [uint32](Crc32 $body)
  $chunk = [byte[]](0,0,0,$data.Length) + $body + [byte[]]@((($crc -shr 24) -band 255), (($crc -shr 16) -band 255), (($crc -shr 8) -band 255), ($crc -band 255))
  [IO.File]::WriteAllBytes($dst, [byte[]]($png[0..32] + $chunk + $png[33..($png.Length - 1)]))
  Write-Host 'made orient_6.png'
}
