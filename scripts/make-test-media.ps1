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
