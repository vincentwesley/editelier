# 04 · Windows media pipeline, packaging and legal research

Researched 2026-11-25. Target: Editelier, a free Windows-only video and photo editor built on Tauri 2 (Rust + TS webview UI) with wgpu compositing, shipped free through the Microsoft Store as MSIX. Reference hardware: i5-1335U with Iris Xe (Quick Sync), 16 GB. An AMD desktop comes later, so NVENC and AMF paths stay open.

This is engineering research, not legal advice. Items marked **(verify)** have only secondary sources or need a human to check them.

---

## 1. Preview surface in Tauri 2 on Windows

### Options

| Option | How it works | Throughput / latency | Known issues |
|---|---|---|---|
| **(a) Native wgpu surface in its own HWND, beside or under the WebView2** | wry/Tauri builds the webview as a *child* of the window. wgpu renders to the parent window or to a sibling child HWND, positioned to a rectangle the UI reports. | Zero copy: decoded D3D11 textures go straight to wgpu (section 2) and present with no readback. Latency is one swapchain present, as in any native player. | **Airspace:** HTML can't draw over a sibling HWND unless the webview is transparent and stacked above it. Transparency over wgpu on Windows needed winit `with_clip_children(false)` (WS_CLIPCHILDREN). wry fixed that in its example, Jan 2025 ([wry#1331](https://github.com/tauri-apps/wry/issues/1331)). WebView2 also has a documented black-or-blank output bug with hardware overlay planes that reproduces in wry/Tauri 2 ([WebView2Feedback#5574 mirror](https://monitoring-git.chickenkiller.com/MicrosoftEdge/WebView2Feedback/issues/5574)). Drawing a transparent DComp surface *over* WebView2 is reported unworkable ([Construct blog](https://www.construct.net/en/blogs/ashleys-blog-2/trying-show-steam-overlay-1861)). |
| **(b) Stream frames to a `<canvas>`** (Tauri IPC or custom protocol, or WebView2 `PostSharedBufferToScript`) | Rust renders, reads back RGBA, then ships the bytes to JS, which uploads them to canvas or WebGPU. | 1080p RGBA is 8.3 MB a frame, so 30 fps is about 250 MB/s and 4K about 1 GB/s. One reported Tauri binary IPC run on Windows took about 200 ms for 10 MB (about 50 MB/s; anecdotal) ([tauri discussion #11915](https://github.com/orgs/tauri-apps/discussions/11915)). SharedBuffer uses OS shared memory and gives each post its own ArrayBuffer view ([ICoreWebView2SharedBuffer](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2sharedbuffer), [PostSharedBufferToScript](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2.postsharedbuffertoscript)). That removes the IPC copy but still needs a GPU→CPU readback in Rust plus a CPU→GPU upload in the renderer. I found no published fps benchmark. | Two copies across the GPU bus every frame, plus JSON or serialization on the webview main thread ([IPC guide](https://dev.to/hiyoyok/ipc-in-tauri-tauri-commands-vs-custom-ipc-what-to-use-when-2ab4)). Tauri doesn't expose SharedBuffer itself, so you'd reach the raw `ICoreWebView2_17` through `with_webview`. |
| **(c) WebGPU inside WebView2, frames from Rust** | Compositing runs in JS WebGPU, and Rust sends decoded frames. | Same transport cost as (b): a native D3D texture can't be handed to the page's WebGPU device. `importExternalTexture` only takes `<video>`/`VideoFrame` sources ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/GPUDevice/importExternalTexture)). WebGPU itself has shipped in Chromium on Windows since 113 ([web.dev](https://web.dev/blog/webgpu-supported-major-browsers)). | You'd lose the Rust wgpu compositor that export needs anyway, so there'd be two renderers. Older Tauri notes say WebView2's WebGPU state was unclear ([tauri#6381](https://github.com/tauri-apps/tauri/issues/6381)). Check `navigator.gpu` at runtime. |
| **(d) Different shell** (pure Rust UI: egui, iced, Slint, Xilem) | Everything is native, so wgpu is first-class. | Best possible. | Throws away the TS/web UI and the house style kit. That's a large rewrite with weaker accessibility than web. |

### Prior art
- The wry `wgpu.rs` example puts a child webview on a wgpu window ([docs.rs wry example](https://docs.rs/crate/wry/latest/source/examples/wgpu.rs)). There's also a Tauri v2 port, archived in Dec 2024 ([FabianLars/tauri-v2-wgpu](https://github.com/FabianLars/tauri-v2-wgpu)).
- `tauri-wgpu-cam`: camera frames rendered with wgpu inside a Tauri window ([clearlysid/tauri-wgpu-cam](https://uithub.com/clearlysid/tauri-wgpu-cam)).
- Bevy behind a transparent Tauri HUD window ([native-bevy-with-tauri-hud-demo](https://www.github.com/langyo/native-bevy-with-tauri-hud-demo)).

### Recommendation: (a), with no-overlap layout
Use a **dedicated child HWND for the viewer** (a wgpu `Surface` on DX12), as a sibling of the WebView2 controller HWND. JS reports the viewer rectangle (ResizeObserver + `devicePixelRatio`) and Rust calls `SetWindowPos`. **Draw every on-viewer overlay in wgpu**: safe areas, crop handles, text-edit gizmos, scopes. Keep HTML popovers out of the viewer rectangle; the native layer can briefly hide itself during modal dialogs. This avoids the transparency and overlay-plane bugs and gives zero-copy decode→composite→present. That's the only path where 4K-proxy (1080p/1440p) playback on Iris Xe isn't bus-bound. Keep (b) with SharedBuffer as a fallback only for thumbnails, waveforms and filmstrips, which are small and infrequent.

---

## 2. Decode and encode

### Media Foundation (windows-rs)
- **Formats:** The MPEG-4 source covers .mp4/.mov/.m4v. Built-in decoders and encoders cover H.264 and AAC, plus MP3 decode ([Supported media formats](https://learn.microsoft.com/en-us/windows/win32/medfound/supported-media-formats-in-media-foundation)). MKV has been supported natively since Windows 10, with H.264/AAC/subtitles inside ([MF MKV support](https://learn.microsoft.com/en-us/windows/win32/medfound/mkv-support)). HEVC needs the HEVC Video Extensions Store package, paid at $0.99. The free OEM variant was removed from the Store ([Windows Central](https://www.windowscentral.com/microsoft-now-charging-hevc-video-extensions), [How-To Geek](https://www.howtogeek.com/804013/how-to-install-free-hevc-codecs-on-windows-11-for-h265-video/)). N editions have no MF until the user adds the Media Feature Pack ([Microsoft support](https://support.microsoft.com/windows/media-feature-pack-for-windows-n-8622b390-4ce6-43c9-9b42-549e5328e407)).
- **Hardware:** Vendor hardware MFTs (Intel QSV, AMD AMF, NVIDIA) are enumerated through `MFTEnumEx` with `MFT_ENUM_FLAG_HARDWARE`. Decode can output into a D3D11 device through `IMFDXGIDeviceManager`, so frames stay on the GPU.
- **Gaps:**
  - **Seeking isn't frame-accurate.** `IMFSourceReader::SetCurrentPosition` "does not guarantee exact seeking", typically lands on the previous keyframe, and the app must ReadSample forward ([docs](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-setcurrentposition)). `IMFSeekInfo::GetNearestKeyFrames` helps ([docs](https://learn.microsoft.com/en-us/windows/desktop/api/mfidl/nf-mfidl-imfseekinfo-getnearestkeyframes)).
  - **VFR:** MF just passes through sample timestamps. An editor needs its own PTS→frame index.
  - **Formats:** No VP9/AV1/ProRes/DNx without Store extensions. MOV edge cases (edit lists, multiple tracks, timecode tracks) are limited.
- **Mitigation if MF is the backbone:** Build a demux index yourself (a Rust MP4/MKV parser, or FFmpeg's libavformat) with a keyframe and PTS table. Seek to keyframe, then decode forward and discard frames. Cache decoded GOPs around the playhead. Edit on proxies with short GOPs (all-intra or GOP 1 s), so seek cost is bounded.

### FFmpeg (LGPL) with Windows hardware acceleration
- `d3d11va` hwaccel decode (H.264/HEVC/VP9/AV1, depending on the driver) outputs `AV_PIX_FMT_D3D11` textures. `h264_qsv`/`hevc_qsv`/`av1_qsv`, `*_amf` and `*_nvenc` encoders are all available in LGPL builds, since they're vendor SDK headers rather than GPL libraries. The demuxers cover MOV/MP4/MKV/WebM with edit lists, VFR timestamps and frame-accurate seek (`avformat_seek_file` + decode-forward).
- **Rust bindings:**
  - **rsmpeg** 0.18 (`+ffmpeg.8.0`, larksuite) supports FFmpeg 6/7/8 and has Windows/vcpkg docs ([docs.rs](https://docs.rs/crate/rsmpeg/0.18.0), [Windows guide](https://docs.rs/crate/rsmpeg/latest/source/doc/windows.md)).
  - **ffmpeg-next** is frozen by policy, maintenance only ([State of media in Rust 2026](https://dev.to/yeauty/the-state-of-media-processing-in-rust-2026-what-each-crate-actually-covers-2k2c)). The fork **ffmpeg-the-third** 5.0 tracks FFmpeg 8.1 ([docs.rs](https://docs.rs/crate/ffmpeg-the-third/latest)).
  - **ffmpeg-sidecar** drives the `ffmpeg.exe` CLI over pipes. It's fine for export or batch work but gives no GPU texture handoff.
- **LGPL shared build source:** BtbN `FFmpeg-Builds` publishes `ffmpeg-master-latest-win64-lgpl-shared.zip` plus versioned `n8.1`/`n9.0` lgpl-shared zips. The LGPL variant leaves out GPL-only libraries such as libx264/libx265. It requires Windows 10 22H2+ and UCRT ([releases](https://github.com/BtbN/FFmpeg-Builds/releases/tag/latest), [repo README](https://github.com/BtbN/FFmpeg-Builds)). Pin a **versioned release tag** (for example n8.1), never `latest`. Record its configure line, which `ffmpeg -buildconf` prints.

### Zero-copy D3D11 → wgpu (DX12)
Create (or have FFmpeg/MF output into) a D3D11 texture with `D3D11_RESOURCE_MISC_SHARED` (better: `SHARED_NTHANDLE`). Open it on wgpu's D3D12 device (`ID3D12Device::OpenSharedHandle`), wrap it as a wgpu-hal DX12 texture, then `device.create_texture_from_hal::<Dx12>()` (unsafe). The `cef` crate's D3D11 importer ([source](https://docs.rs/crate/cef/latest/source/src/osr_texture_import/d3d11.rs)) and `vtsampler` ([source](https://docs.rs/crate/vtsampler/0.1.0/source/src/bridge/d3d11.rs)) both do this. Sync with a shared `ID3D11Fence`/`ID3D12Fence` (or a keyed mutex). Cache the opened resources per decoder surface pool slot, as Dawn does ([Dawn commit](https://dawn.googlesource.com/dawn/+/9c3aefa4bdc24c72edc0282e94c71bdef6747596)). NV12 → RGB conversion happens in a wgpu shader by sampling the R8/RG8 plane views.

### Recommendation
- **Decode:** FFmpeg LGPL (BtbN lgpl-shared, pinned) via **rsmpeg**, with **d3d11va** hwaccel and zero-copy into wgpu DX12. FFmpeg gives the demux, seeking and VFR robustness MF lacks, with one code path for Intel, AMD and NVIDIA.
- **Encode:** Media Foundation hardware MFTs for **H.264 + AAC** (OS-licensed codecs, see section 3). Optional FFmpeg `*_qsv`/`*_amf`/`*_nvenc` behind a capability probe. AV1 export through FFmpeg (`av1_qsv` on Arc and newer, `av1_amf`/`av1_nvenc` on newer GPUs, otherwise libsvtav1, which is BSD and LGPL-compatible).
- Iris Xe (Gen12) supports hardware decode of H.264, HEVC 8/10-bit, VP9 and AV1, but encode of H.264/HEVC only. **(verify on the test laptop with `ffmpeg -hwaccels` and `vainfo`-equivalent `dxdiag`)**

---

## 3. Codec patents for free distribution

| Codec | Pool(s) as of 2026 | What applies to a free app |
|---|---|---|
| **H.264/AVC** | Via LA, formed by the merger of MPEG LA and Via Licensing ([StreamingMedia](https://www.streamingmediaglobal.com/Articles/News/Featured-News/Via-LAs-Heath-Hoglund-Talks-MPEG-LA-Via-Licensing-Patent-Pool-Merger-158552.aspx)). | Codec royalties: "No Royalty" for units 1–100,000/yr per legal entity, then $0.20, then $0.10 ([Via LA AVC fees](https://www.via-la.com/licensing-2/avc-h-264/avc-h-264-license-fees)). Streaming fees were restructured in 2026, but that's irrelevant to a desktop app ([Tom's Hardware](https://www.tomshardware.com/service-providers/streaming/h264-streaming-license-fees-jump-from-100000-to-4-5-million)). Free download counts still count as units. |
| **HEVC** | Access Advance (HEVC Advance). It acquired the Via LA HEVC/VVC program in Dec 2025, now "VCL Advance" ([Access Advance](https://accessadvance.com/2026/01/22/access-advance-closes-2025-with-record-quarter-eight-major-licensees-100-renewal-rate-litigations-resolved/)). Avanci Video also exists ([StreamingMedia](https://www.streamingmedia.com/Articles/News/Online-Video-News/Avanci-Video-Signs-First-Licensee-and-Publishes-Rates-173422.aspx)). | HEVC Software is $1.00/$0.50 per copy (Region 1/2), with a $25,000 annual enterprise credit. Royalties are due on software that *installs* a decoder or encoder ([HEVC Advance Program Overview, July 2026](https://accessadvance.com/wp-content/uploads/2025/07/HEVC-Advance-Program-Overview-July-2026.pdf), [what we license](https://accessadvance.com/topic-what-do-we-license/)). No free-software exemption is stated. |
| **AAC** | Via LA AAC; unaffected by the 2025 HEVC deal ([Law360](https://www.law360.com/mlex/articles/2422258/combined-access-advance-via-la-codec-pools-could-fend-off-av1-threat)). | Per-unit codec royalties apply. There's a PC-software category; distributing AAC *bitstreams* costs nothing ([Via LA AAC FAQ](https://www.via-la.com/?p=1996), [AAC fee structure](https://www.via-la.com/aac-license-fees-structures/)). FDK-AAC's licence grants no patent rights ([SPDX FDK-AAC](https://spdx.org/licenses/FDK-AAC)). |
| **AV1** | AOMedia Patent License 1.0: royalty-free, with a defensive-termination clause ([AOM PATENTS](https://aomedia.googlesource.com/aom/+/HEAD/PATENTS)). Sisvel runs a separate AV1 pool claim, and Access Advance began exploring an AV1/AV2 pool in 2026 ([BusinessWire](https://secure.businesswire.com/news/home/20260825292774/en/Access-Advance-Launches-Exploration-Phase-for-an-AV1AV2-Device-and-Software-Patent-Pool-Invites-Participation-of-Potential-Licensors)). | Royalty-free under AOM. Third-party pool claims exist but are unproven. |
| **VP9** | Google royalty-free patent grant. **(verify, no primary source fetched)** | Same risk class as AV1. |
| **Opus** | RFC 6716, BSD reference code, royalty-free patent licences from Xiph, Microsoft and Broadcom ([opus-codec.org/license](https://opus-codec.org/license/), [Xiph press](https://xiph.org/press/2012/rfc-6716/)). Some RAND disclosures exist (Qualcomm, Huawei). | Effectively royalty-free. |

### Does using OS codecs shift licensing to Microsoft or the OEM?
- If the app **calls the Windows-provided H.264/AAC MFTs** (or the user-installed HEVC extension, or the GPU vendor's hardware MFT), the app **ships no codec implementation**. The royalty-bearing "unit" is the Windows copy or device, which Microsoft or the OEM licensed, or the $0.99 HEVC extension the user bought. Firefox has relied on this for H.264/AAC since Firefox 20. Mozilla said H.264 licence terms "prevent distributing it with open source products" ([Mozilla blog](https://blog.mozilla.org/en/wp-json/wp/v2/posts/62042), [Tom's Hardware, Firefox 20](https://www.tomshardware.com/news/mozilla-firefox-browsers-h264-video,20012.html)). Mozilla still lists AAC as supported "only when the OS provides it" ([Bugzilla 1737116](https://bugzilla.mozilla.org/show_bug.cgi?id=1737116)).
- Pools license whoever "installs" a codec in software at First Sale ([Access Advance](https://accessadvance.com/topic-what-do-we-license/)). No pool document I found explicitly blesses OS-API use, so this is the **industry-standard practice, not a written safe harbour. (verify with counsel if the app ever becomes commercial)**
- **Shipping FFmpeg's own H.264/HEVC/AAC decoders or encoders** (native `h264`, `hevc`, `aac` in libavcodec) *is* shipping a codec implementation. Free distribution doesn't change that: the FFmpeg project itself warns about it ([ffmpeg.org/legal](https://ffmpeg.org/legal.html)). Hardware paths are different: FFmpeg's `d3d11va` hwaccel still uses libavcodec's H.264/HEVC *parsing and slice-header code*, while the decode runs on the GPU's licensed hardware. That's a grey zone. `h264_qsv`/`*_amf`/`*_nvenc` wrap vendor-licensed hardware.

### Safest option (chosen)
1. **Build FFmpeg with the patent-encumbered software codecs disabled.** Drop the software `h264`, `hevc`, `aac` and `aac_fixed` decoders and encoders and the `mpeg4`/`mpeg2video` software codecs. Keep the demuxers, muxers and parsers, and royalty-free codecs: `av1` (dav1d is BSD; libaom), `vp8`/`vp9`, `opus`/`libopus`, `flac`, `pcm_*`, `mjpeg`, `png`, `prores`/`dnxhd` decode **(verify ProRes/DNx patent status)**.
2. **Decode and encode H.264, HEVC and AAC only through Windows Media Foundation MFTs.** Use hardware MFTs where present and the Microsoft software MFTs otherwise. HEVC needs the user's HEVC Video Extensions; detect it and show a "Get HEVC support from the Store" link, never bundle it.
3. **Default export: H.264 + AAC MP4 via MF**, for maximum compatibility, plus **AV1 + Opus (WebM/MP4)** via FFmpeg as the royalty-free option.

The cost: MF-only HEVC means no HEVC on N editions without the feature pack, and none without the extension. You also lose FFmpeg's demux→MF handoff convenience. FFmpeg demuxes and MF decodes from the elementary stream via `IMFTransform` fed with Annex-B samples, which is a known, workable pattern.

---

## 4. MSIX packaging for Tauri 2

- **tauri-bundler has no MSIX.** Issue [tauri#4818](https://github.com/tauri-apps/tauri/issues/4818) is still open. On 2026-01-25 a maintainer said "No explicit plans… not the highest priority", and v3 has no ETA. The 2026 bundler changelog covers MSI and NSIS only ([tauri-bundler releases](https://v2.tauri.app/release/tauri-bundler/)). Community tools exist: `tauri-windows-bundle` and `msixbundle-rs` (Choochmeque, linked in #4818).
- **Recommended tool: Microsoft `winapp` CLI** (public preview since 2026-01-22; v0.3.2 added msixbundle) ([docs](https://learn.microsoft.com/en-us/windows/apps/dev-tools/winapp-cli/), [blog](https://devblogs.microsoft.com/ifdef-windows/?p=1181), [repo](https://github.com/microsoft/winappcli)). It has an official Tauri guide ([Tauri + winapp](https://learn.microsoft.com/en-us/windows/apps/dev-tools/winapp-cli/guides/tauri)):
  ```powershell
  winget install microsoft.winappcli --source winget
  winapp init                         # Package.appxmanifest + Assets
  npm run tauri -- build              # release exe
  # stage exe + FFmpeg DLLs + models + licences into .\dist
  winapp cert generate --if-exists skip   # self-signed, Publisher = manifest Publisher
  winapp pack .\dist --cert .\devcert.pfx
  winapp cert install .\devcert.pfx       # admin, once per cert
  Add-AppxPackage .\editelier_1.0.0.0_x64.msix
  ```
  The guide says: "The Microsoft Store will sign the MSIX for you, no need to sign before submission." Bump `Version` for every reinstall. The manual fallback is `makeappx pack /d dist /p app.msix`, then `signtool sign /fd SHA256 /f cert.pfx app.msix`, with a cert from `New-SelfSignedCertificate` whose Subject equals the manifest Publisher ([create a certificate for package signing](https://learn.microsoft.com/en-us/windows/msix/package/create-certificate-package-signing)).
- **Manifest requirements:**
  - `Identity Name`/`Publisher` and `PublisherDisplayName` must be **copied from Partner Center** (Product identity page) before the Store upload.
  - `EntryPoint="Windows.FullTrustApplication"` with the **`runFullTrust` restricted capability**. It's required for a Win32 app in MSIX, and Partner Center accepts it for desktop apps ([capability declarations](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations), [Partner Center thread](https://techcommunity.microsoft.com/t5/msix/msix-package-not-accepted-at-partner-center/m-p/1106994)).
  - **Don't declare `broadFileSystemAccess`.** It's an AppContainer capability, user-toggled in Settings, that needs Store approval. A full-trust app already has normal user file access ([file access permissions](https://docs.microsoft.com/windows/uwp/files/file-access-permissions), [Q&A](https://learn.microsoft.com/en-us/answers/questions/952796/access-denied-when-using-broadfilesystemaccess-and)). Store policy 10.6 requires declared capabilities to relate to real features.
  - **File associations:** `uap:FileTypeAssociation` under `<Extensions>`, for .mp4/.mov/.mkv/.png/.jpg and the project file type ([schema](https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-uap-filetypeassociation)).
  - **Runtime behaviour:** MSIX redirects writes to `%AppData%` into the package's private store, so keep caches in `ApplicationData.LocalCacheFolder` or `%LOCALAPPDATA%`, and leave the install dir read-only ([behind the scenes](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes)).
- **Dependencies:**
  - **WebView2:** the Evergreen runtime is preinstalled on Windows 11 ([WebView2 distribution](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)). Set `MinVersion` to Windows 11 (10.0.22000), or detect the runtime on Windows 10 and link to it. MSIX can't run the bootstrapper at install time.
  - **VC++ runtime:** the Tauri CLI build sets `STATIC_VCRUNTIME`, so vcruntime is linked statically and UCRT stays dynamic (it ships in the OS) ([tauri#4292](https://www.github.com/tauri-apps/tauri/pull/4292), [tauri-build source](https://docs.rs/crate/tauri-build/2.6.2/source/src/lib.rs)). So **VCLibs isn't needed for the exe**. Confirm with `dumpbin /dependents`. FFmpeg BtbN DLLs need UCRT only. ONNX Runtime DLLs may import `VCRUNTIME140`/`MSVCP140`; in that case add a `PackageDependency` on `Microsoft.VCLibs.140.00.UWPDesktop` (verify).
- **Store policies (v7.20, 2026-09-14)** ([policies](https://learn.microsoft.com/en-us/windows/apps/publish/store-policies)):
  - 10.5.1: a privacy policy URL is required if the app accesses, collects or transmits personal information. Video editing of the user's own media plus any crash or telemetry reporting arguably triggers this, so **publish one anyway**.
  - 11.11: complete the **IARC questionnaire** in Partner Center; expect a general/3+ rating.
  - 10.2.2: no dynamic code that changes functionality. Model downloads are data, not code; disclose them in the description.
  - 10.1: the title must be unique and contain no keywords.
  - 11.16: only applies to *live generative* AI. Transcription and segmentation aren't generative, but add a description note anyway.
  - 10.14 / the account itself: individual registration is free and needs ID plus selfie verification ([Windows Developer blog](https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/)).

---

## 5. On-device AI licences

| Component | Licence | Redistribute in a free app? | Size |
|---|---|---|---|
| whisper.cpp | MIT ([repo](https://github.com/ggml-org/whisper.cpp)) | Yes | Small lib |
| Whisper weights (OpenAI) | "code and model weights are released under the MIT License" ([openai/whisper README](https://github.com/openai/whisper)). The HF card metadata says apache-2.0 ([HF whisper-tiny](https://huggingface.co/openai/whisper-tiny)). Both are permissive. | Yes, include LICENSE | ggml base 142 MB, small 466 MB, large-v3-turbo q5 ≈ 550 MB **(verify exact)** |
| distil-whisper distil-large-v3 | MIT ([HF card](https://huggingface.co/distil-whisper/distil-large-v3)), with a ggml build ([HF ggml](https://huggingface.co/distil-whisper/distil-large-v3-ggml)) | Yes | ≈ 1.5 GB fp16, smaller quantized. **English only.** |
| MODNet | Apache-2.0 (current upstream README; old forks were CC BY-NC-SA) ([ZHKKKe/MODNet](https://github.com/ZHKKKe/MODNet)) | Yes. Use upstream weights only. | ≈ 25 MB ONNX |
| BiRefNet | MIT (repo badge + HF metadata) ([HF ZhengPeng7/BiRefNet](https://huggingface.co/ZhengPeng7/BiRefNet)) **(verify LICENSE file)** | Yes | ≈ 970 MB full, "lite" ≈ 170 MB |
| U²-Net | Apache-2.0 (per ports; upstream xuebinqin/U-2-Net) ([HF port](https://huggingface.co/Heliosoph/u2net-onnx)) **(verify upstream)** | Yes | u2netp 4.7 MB, u2net 176 MB |
| MediaPipe Selfie Segmentation | Apache-2.0 from Google. Qualcomm repackages carry other terms, so avoid those ([Qualcomm AI Hub](https://aihub.qualcomm.com/models/mediapipe_selfie), [HF onnx-community](https://huggingface.co/onnx-community/mediapipe_selfie_segmentation/discussions)) | Yes (Google original) | ≈ 0.5 MB |
| SAM 2 | Apache-2.0 ([Roboflow summary](https://playground.roboflow.com/models/meta/sam-2); upstream facebookresearch/sam2) **(verify upstream)** | Yes | tiny ≈ 150 MB |
| **RMBG-1.4 / RMBG-2.0 (BRIA)** | **Non-commercial (CC BY-NC 4.0 / bria licence)** ([HF RMBG-2.0](https://huggingface.co/briaai/rmbg-2.0)) | **Exclude** | n/a |
| ONNX Runtime | MIT | Yes | `onnxruntime.dll` + `DirectML.dll` ≈ 41 MB per Microsoft ([Windows ML distribution](https://learn.microsoft.com/en-nz/windows/ai/new-windows-ml/distributing-your-app)) |
| DirectML EP | Supported but in "sustained engineering"; new work moved to **Windows ML** ([DirectML EP docs](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html), [ORT PR #25901](https://github.com/microsoft/onnxruntime/pull/25901)). Check the redistributable licence in the `Microsoft.AI.DirectML` NuGet ([NuGet](https://www.nuget.org/packages/Microsoft.AI.DirectML/)) **(verify)**. | Yes, under the NuGet licence terms | ≈ 20 MB |

**Recommended set:**
- **Transcription:** whisper.cpp with `base` (bundled or first-run) and optional `small` / `large-v3-turbo` downloads. It uses the Vulkan or CPU backend on Iris Xe, so it doesn't need ORT. Offer distil-large-v3 as an English-only "fast accurate" option.
- **Background removal:**
  - Video, real-time preview: MediaPipe Selfie (people) or MODNet (portrait matting) through ONNX Runtime + DirectML.
  - Stills, high quality: **BiRefNet-lite** as an optional download.
  - Object selection: **SAM 2 tiny** later.
- **Runtime:** ONNX Runtime + DirectML now, since it's stable and ships with the app. Keep the EP choice behind an interface so you can move to Windows ML once it's GA for unpackaged and Rust callers.
- **Housekeeping:** List every model with its licence and source in `THIRD_PARTY_NOTICES.md`. Download from pinned URLs with a SHA-256 check.

---

## 6. Name and trademark check: "Editelier"

Searched 2026-11-25:

| Where | Query | Result |
|---|---|---|
| USPTO (tmsearch API, `wordmark` + pseudo-text fields; query validated with "photoshop" → 11 hits) | `editelier`, `editelier*`, `edit*lier`, `"edit atelier" OR editatelier OR editelia` | **0 hits.** Nearby: 35 live "ATELIER" marks in class 9 (e.g. ATELIER, ATELIER TOTORI, MAGICAL ATELIER, CHEFS ATELIER); none start with EDIT. |
| Web search (Justia trademark index etc.) | `"Editelier"`, `Editelier trademark OR app OR software` | **No exact hits.** Near names: EDITEE (cl. 42), EDIT (Edit, LLC), EDITORSAPP, MYEDIT; Éditions de l'Atelier (French publisher), Atelier Éditions (LA publisher). |
| Microsoft Store (`storeedgefd` manifestSearch API) | `Editelier`, `Edit atelier` | **0 results.** The apps.microsoft.com web search couldn't be rendered headlessly. |
| GitHub repo search API | `editelier` | **0 repositories** |
| npm / PyPI / crates.io | `editelier` | All 404 (unclaimed) |
| Domains (RDAP / DNS) | editelier.com | **Registered** 2017-02-12 (GoDaddy), expires 2027-02-12, last changed 2026-11-16. Points at Google hosting and returns HTTP 404, so there's no live site. |
| | editelier.app / .dev / .io / .net / .sg | Not registered (NXDOMAIN / RDAP 404) |
| IPOS (digitalhub.ipos.gov.sg) | — | **Not reachable by automated means** (URL 404 / JS app). Owner check needed. |
| WIPO Global Brand Database | — | **Blocked by an ALTCHA captcha.** Owner check needed. |
| EUIPO eSearch / TMview | — | **Connection failed / "Request Rejected".** Owner check needed. |

**Conclusion:** No conflicting mark or product was found in the US register, the Store, GitHub, package registries or the web. **editelier.com is held by an unknown third party, with no site.** Singapore (IPOS), WIPO Madrid and the EU still need manual searches. Do them in classes **9** (downloadable software) and **42** (SaaS/software design).

---

## 7. FFmpeg LGPL compliance checklist

Based on [ffmpeg.org/legal.html](https://ffmpeg.org/legal.html):

1. Build or download a build **without `--enable-gpl` or `--enable-nonfree`**. BtbN `lgpl-shared` satisfies this. Run `ffmpeg -buildconf` and confirm no gpl/nonfree flags.
2. **Link dynamically**: ship `avcodec-*.dll`, `avformat-*.dll`, etc. beside the exe. Don't rename or obfuscate the DLL names; a prefix or suffix is allowed.
3. Ship **COPYING.LGPLv2.1** (and the licences of bundled libs such as dav1d BSD and libopus BSD) inside the package and in `THIRD_PARTY_NOTICES.md`.
4. **Provide the exact corresponding source**: the FFmpeg tarball for the pinned tag, the BtbN build scripts at that commit, and any patches (`git diff`). Host it alongside the binary, in a GitHub release asset of the Editelier repo. "Same server" in practice means a link on the product and download page.
5. Include a **text file with the configure line and toolchain** (`BUILD-FFMPEG.txt` = `ffmpeg -buildconf` output + BtbN commit + tag).
6. **About box:** "This software uses libraries from the FFmpeg project under the LGPLv2.1" with links to ffmpeg.org, the LGPL and our source archive.
7. The **EULA/Store terms** must say FFmpeg is used under LGPLv2.1, that we don't own it, and must **not prohibit reverse engineering** (for the FFmpeg parts). Apply the same to every translation.
8. Spell it **FFmpeg**.
9. **No GPL libraries**: no libx264, libx265, libxvid or rubberband-gpl. Also no `nonfree` (libfdk_aac).
10. Users must be able to **swap the DLLs**. MSIX installs are read-only, so document how to run the app unpackaged with replacement DLLs, or ship a portable zip on GitHub. **(verify the MSIX DLL replaceability interpretation)**
11. Re-run this checklist for every FFmpeg version bump.

---

## Decisions I recommend

1. **Preview:** wgpu (DX12) in a dedicated child HWND sized to a layout rect from the web UI. Draw all viewer overlays in wgpu, with no HTML over the viewer. Use SharedBuffer only for thumbnails and waveforms.
2. **Decode:** FFmpeg 8.1 LGPL shared (BtbN, pinned tag) via **rsmpeg**, with d3d11va hwaccel and zero-copy D3D11→D3D12 shared-handle import into wgpu with fence sync.
3. **Patent-encumbered codecs (H.264/HEVC/AAC) only through Windows Media Foundation MFTs** (hardware first, then the Microsoft software MFT). Build or pick FFmpeg with its software h264/hevc/aac codecs **disabled**. HEVC only if the user has HEVC Video Extensions; link to the Store, never bundle it.
4. **Export defaults:** H.264 + AAC MP4 via MF, and AV1 + Opus via FFmpeg (hardware AV1 where available, else SVT-AV1) as the royalty-free option.
5. **Editing model:** generate short-GOP proxies (1080p/1440p) on import, and keep our own PTS and keyframe index for frame-accurate seeking and VFR.
6. **Packaging:** winapp CLI → MSIX with `runFullTrust`, no `broadFileSystemAccess`, file-type associations, MinVersion Windows 11, and Partner Center identity. Use a self-signed dev cert locally; the Store re-signs.
7. **Store:** publish a privacy policy URL (local-only processing, no telemetry by default), complete IARC, and disclose optional model downloads in the description.
8. **AI:** whisper.cpp + Whisper base/small (MIT), optional distil-large-v3 (English). MediaPipe Selfie / MODNet for live mattes and BiRefNet-lite for stills, all via ONNX Runtime + DirectML with an abstraction for Windows ML later. **Exclude RMBG-1.4/2.0.**
9. **Name:** keep "Editelier" provisionally. The owner runs the IPOS/WIPO/EUIPO searches (classes 9/42) before the Store name reservation. Pick a domain such as editelier.app; .com is taken.
10. **LGPL:** follow the section 7 checklist and ship the source archive, build config and notices with every release.

## 10-line summary
1. Preview: render wgpu (DX12) into a separate child window placed inside the web UI's layout, with no HTML drawn over it. Streaming frames into a `<canvas>` costs about 250 MB/s at 1080p30 and is too slow (one anecdotal measurement put Tauri IPC on Windows at about 50 MB/s).
2. Decode with FFmpeg 8.1 LGPL (BtbN lgpl-shared, pinned tag) through rsmpeg, using d3d11va and zero-copy into wgpu. ffmpeg-next is frozen; ffmpeg-sidecar can't hand GPU textures over.
3. Media Foundation seeks to the previous keyframe rather than the exact frame, and doesn't handle VFR or many container types. Plan on our own frame index plus short-GOP proxies.
4. Patents: H.264 (Via LA; first 100k units/yr royalty-free), HEVC (Access Advance; $1/copy, $25k annual credit) and AAC (Via LA) all license whoever ships a codec. AV1 (AOM) and Opus are royalty-free.
5. Safest option: use Windows' own MF codecs for H.264, HEVC and AAC, build FFmpeg with its software h264/hevc/aac turned off, and offer AV1+Opus as the royalty-free export. HEVC needs the $0.99 Store extension.
6. tauri-bundler still has no MSIX (issue #4818 is open, no plans). Use Microsoft's winapp CLI (init, cert generate/install, pack): a self-signed cert for local testing, and the Store re-signs.
7. Manifest: `runFullTrust` yes, `broadFileSystemAccess` no. Add file associations, the Partner Center identity and a Windows 11 minimum (WebView2 is preinstalled there). Tauri links the VC runtime statically. Store needs a privacy policy URL and the IARC age-rating questionnaire.
8. AI: whisper.cpp, Whisper weights and distil-whisper are MIT; MODNet, MediaPipe Selfie, U²-Net and SAM 2 are Apache; BiRefNet is MIT (verify). Exclude RMBG (non-commercial). ONNX Runtime + DirectML adds about 41 MB, and DirectML is in sustained engineering.
9. Name: no "Editelier" conflict found in the USPTO (0 hits), the Store API, GitHub, npm, PyPI, crates or the web. editelier.com is registered with no site; .app/.dev/.io are free. IPOS, WIPO and EUIPO blocked automated search, so the owner must check them by hand.
10. FFmpeg LGPL: no gpl/nonfree, dynamic DLLs, ship the LGPL text, source and configure line, credit FFmpeg in About and the EULA, allow reverse engineering, and let users swap the DLLs.
