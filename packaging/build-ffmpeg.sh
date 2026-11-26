#!/usr/bin/env bash
# STUB, untested: builds the shipped LGPL FFmpeg (packaging/ffmpeg-build.md) with BtbN's scripts.
# Needs Docker (Linux or WSL2). Output: a win64 lgpl-shared 8.1 zip in the clone's artifacts/.
set -euo pipefail
BTBN_COMMIT="${BTBN_COMMIT:?pin a BtbN/FFmpeg-Builds commit}"
work="$(mktemp -d)"
git clone https://github.com/BtbN/FFmpeg-Builds "$work/btbn"
git -C "$work/btbn" checkout "$BTBN_COMMIT"
# BtbN addins are sourced shell snippets that may append to FF_CONFIGURE (see its addins/ folder).
cat > "$work/btbn/addins/editelier.sh" <<'EOF'
#!/bin/bash
ffbuild_configure() {
  echo --disable-decoder=h264,hevc,aac,aac_fixed,aac_latm \
    --disable-encoder=aac,h264_amf,h264_mf,h264_nvenc,h264_qsv,h264_vaapi,hevc_amf,hevc_mf,hevc_nvenc,hevc_qsv,hevc_vaapi \
    --disable-parser=h264,hevc,aac,aac_latm \
    --disable-hwaccel=h264_d3d11va,h264_d3d11va2,h264_d3d12va,h264_dxva2,h264_nvdec,h264_vulkan,hevc_d3d11va,hevc_d3d11va2,hevc_d3d12va,hevc_dxva2,hevc_nvdec,hevc_vulkan \
    --disable-ffplay
}
EOF
cd "$work/btbn"
./makeimage.sh win64 lgpl-shared 8.1 editelier
./build.sh win64 lgpl-shared 8.1 editelier
echo "Built in $work/btbn/artifacts; record ffmpeg -buildconf + $BTBN_COMMIT in BUILD-FFMPEG.txt"
