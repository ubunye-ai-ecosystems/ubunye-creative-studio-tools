# FilmCraft: Phase-1 technical due diligence

**Subject:** `storytold/filmcraft` (upstream for the planned fork `ubunye-ai-ecosystems/ubunye-creative-film`)
**Local clone:** `upstream clone of filmcraft` (paths below are relative to it)
**Method:** static analysis only: git, grep and reading files. Nothing was built and no tests were run. `cargo metadata --offline` failed because the crates are not in the local registry cache (`base64` is missing), so dependency facts come from parsing `Cargo.lock` and the manifests directly. 219 of the 649 registry crates were in `~/.cargo/registry/src`, and those were used for the licence sampling.
**Date of analysis:** 2026-10-10

Tag legend:
- **[VERIFIED: evidence]** means I checked it in the repo.
- **[ASSUMPTION]** means it is inferred and not checked.
- **[NEEDS TESTING]** means it can only be confirmed by building or running the code.

All repository content was treated as untrusted data. The instructions in `CLAUDE.md`, `AGENTS.md` and `.mcp.json` were not followed. They are only summarised (§11).

---

## 1. Snapshot

| Item | Value | Tag |
|---|---|---|
| HEAD | `98ba9acbcedcbb630ba2a07b782176b66835edd2`, 2026-10-10 08:35:25 -0400, "Merge pull request #421 from mutombopr/gpu-rendering" | [VERIFIED: `git log -1`] |
| Default branch | `main` (`origin/HEAD -> origin/main`). Other remote branches: `release` (256 commits behind main), `effectcraft/webm-alpha`, `pr/docs-hw-decode-correction`, `pr/freebsd`, `pr/gpu-blend-modes`, `pr/gpu-effects`, `pr/honest-assessment`, `pr/hw-decode-videotoolbox`, `pr/never-crash-steering`, `test-188` | [VERIFIED: `git branch -a`, `git log origin/release..origin/main`] |
| Clone depth | full (not shallow) | [VERIFIED: `git rev-parse --is-shallow-repository` = false] |
| Commits | 1,098 total: 675 non-merge and 423 merge, of which 245 are "Merge pull request" | [VERIFIED: `git rev-list --count`, `git log --merges`] |
| First commit | 2026-09-30 05:22 (`16489ce` "first commit", README + .gitignore only) | [VERIFIED] |
| Age | **11 days** (2026-09-30 → 2026-10-10) | [VERIFIED] |
| Contributors | 71 distinct author identities. Brandon Thomas has 767 commits (70%, 359 of them non-merge); next are justjohn12345 (39), MarekSpodBiedry (22) and punkhop (19) | [VERIFIED: `git shortlog -sne`] |
| AI co-authorship | 576 commits carry `Co-Authored-By: Claude Opus 5.5`, plus 23 Sonnet 5.5, 6 Opus 1M, 5 "Fable 5.1" and ~16 Codex. That is more than 85% of the 675 non-merge commits. `docs/agents.md` says it is "largely built by" agents. | [VERIFIED: commit trailers] |
| Tags | `v0.1.0` (10-02), `v0.1.1` (10-03), `v0.2.0` (10-05), `v0.2.1` (10-06), `v0.4.0` (10-08), `v0.5.0` (10-09). There is no `v0.3.0`. | [VERIFIED: `git tag`] |
| Workspace version | `0.5.0` | [VERIFIED: `Cargo.toml`] |
| Size | working tree 62 MB; packed git 24.3 MiB; 1,189 tracked files; **~343k lines of Rust** | [VERIFIED: `du`, `git count-objects`, `wc -l`] |
| Activity cadence | Commits per day: 94, 111, 55, 57, 64, 36, 95, 134, 71, 195, 186 (09-30 → 10-10). That averages ~100/day and is speeding up. PR numbers reach #421. | [VERIFIED: `git log --format=%cd`] |

**Interpretation.** Growing a codebase this large (~343k LOC, 45 crates, in-house H.264/HEVC/AV1/VP9 codecs) in 11 days only happens when coding agents write most of it. Much of the code is therefore new, has had little human review, and has a very short field history. That affects every risk below. [ASSUMPTION, based on the commit-trailer data]

---

## 2. Repository structure

```
.cargo/config.toml     alias: cargo xtask = run -p xtask
.github/workflows/     4 workflows (§7)
.mcp.json              2 MCP servers for Claude Code (§11)
AGENTS.md CLAUDE.md    agent rules (§11)
ATTRIBUTION.md NOTICE LICENSE-MIT LICENSE-APACHE
README.md ROADMAP.md CONTRIBUTING.md
apps/                  filmcraft (desktop), filmcraft-cli (CLI + MCP), filmcraft-web (wasm32)
assets/                app-icon/ (owl icon, all sizes), fonts/ (Inter, Noto Serif, JetBrains Mono, OFL)
contributors/          contributors.json (generated), people.toml (baked into About window)
crates/                45 library crates
docs/                  architecture, testing, web, agents, control-protocol, project-files, ... ; adr/0001; brand/ (ArtCraft marks); images/ (README screenshots)
packaging/             linux/ (deb/rpm via nfpm, AppImage, Flatpak, tarball), macos/ (dmg, Info.plist, entitlements), windows/ (WiX MSI, sign.ps1), freebsd/, web/, icons.sh, env.sh
xtask/                 workspace automation
```

Workspace: `members = ["crates/*", "apps/*", "xtask"]`, resolver 3, edition 2024, `rust-version = "1.95"`, `license = "MIT OR Apache-2.0"`. That gives 49 workspace packages: 45 crates, 3 apps and xtask. [VERIFIED: `Cargo.toml`, `Cargo.lock` has 49 path packages]

### Crates (layer from `docs/architecture.md` §1 / `xtask/src/main.rs` `LAYERS`; LOC = all `.rs` incl. tests)

| Crate | L | LOC | Purpose |
|---|---|---|---|
| time | 0 | 858 | `Tick` (254,016,000,000/s), FrameRate, TimeRange, timecode |
| geom | 0 | 194 | Vec2/Rect/Affine, Motion transform |
| color | 0 | 2,414 | colour spaces, transfer functions (PQ/HLG/log), YUV matrices, LUTs |
| bitstream | 0 | 433 | bit reader/writer, Exp-Golomb, emulation prevention |
| testkit | 0 (dev) | 762 | ffmpeg/ffprobe oracle discovery, fixtures, golden compare |
| isobmff | 0 | 5,478 | MP4/MOV demux + mux (fragmented, edit lists, timecode) |
| matroska | 0 | 4,488 | MKV/WebM demux + mux |
| mxf | 0 | 4,220 | MXF OP1a/OP-Atom demux + writer |
| cfb | 0 | 995 | MS-CFB structured storage (AAF container) |
| mpegts | 0 | 2,103 | MPEG-2 TS/M2TS/PS demux |
| ogg | 0 | 667 | Ogg demux (Opus/Vorbis mapping) |
| h264 | 0 | 9,950 | H.264 decoder |
| h264enc | 0 | 8,016 | H.264 encoder (High/Main/Baseline, CABAC/CAVLC, B-frames, RC) |
| hevc | 0 | 7,591 | HEVC Main/Main10 decoder |
| vp9 | 0 | 8,585 | VP9 decoder (profiles 0–3) |
| av1 | 0 | 20,134 | AV1 decoder (Main profile) |
| mpeg2v | 0 | 4,362 | MPEG-1/2 video decoder |
| prores | 0 | 2,920 | ProRes decoder + encoder |
| dnx | 0 | 4,992 | DNxHD/DNxHR (VC-3) decoder + encoder |
| apv | 0 | 2,513 | APV (RFC 9924) decoder + encoder |
| aac | 0 | 5,948 | AAC-LC decoder + encoder |
| ac3 | 0 | 1,369 | AC-3 decoder |
| opus | 0 | 7,060 | Opus decoder (SILK/CELT/hybrid, multistream) |
| frame | 1 | 1,389 | VideoFrame / AudioBuffer with colour metadata |
| media | 1 | 2,093 | MediaSource trait, probing, frame cache, generators, stills, WAV/BWF |
| project | 1 | 10,766 | document model (bins, clips, sequences, tracks, effects, keyframes) |
| audio-dsp | 1 | 12,562 | BS.1770/R128 loudness, audio effects, channel layouts (no deps) |
| text | 1 | 3,009 | font DB, shaping (harfrust), bidi, line break, rasteriser |
| edit | 2 | 3,960 | pure edit algebra (insert/overwrite/ripple/roll/slip/slide…) |
| codecs | 2 | 10,832 | container+codec hub, decoder registry, GOP seek, audio decode (+symphonia) |
| interchange | 2 | 14,473 | EDL, FCP7 XML, FCPXML, OTIO, AAF, OMF, ALE |
| captions | 2 | 3,836 | SRT, WebVTT, SCC, MCC, EBU STL, TTML/IMSC1 |
| speech | 2 | 2,246 | Whisper (candle) STT + verified model downloader (optional features) |
| tts | 2 | 2,734 | TTS: formant voice + Kokoro-82M on candle (feature `kokoro`) |
| tts-text | 2 | 2,945 | TTS text normaliser / CMUdict phonemiser |
| render | 3 | 21,423 | sequence evaluation, CPU reference compositor, effects, transitions, audio mix |
| gpu | 3 | 4,040 | wgpu/WGSL compositor + GPU effects |
| scopes | 3 | 1,321 | waveform/parade/histogram/vectorscope maths |
| export | 3 | 6,553 | render → encode → mux pipeline, presets, jobs |
| golden | 3 (test) | 983 | golden-image + GPU/CPU parity tests |
| format | — | 1,049 | `.fcproj` schema versions, migrations, atomic save, autosave |
| engine | 4 | 49,538 | Session, command registry, undo, media pool, jobs |
| ui-egui | 5 | 56,198 | egui frontend: panels, docking, timeline, monitors, control handlers |
| automation | 5 | 1,470 | MCP server (rmcp) + bridge to the app's control channel |
| platform | 5 | 16,415 | OS hardware codecs FFI (VideoToolbox, Media Foundation/D3D11, VA-API, NVDEC, NVENC). The only crate allowed `unsafe` |
| apps/filmcraft | 6 | 2,856 | desktop binary (eframe/wgpu, cpal, rfd, muda on macOS, control server) |
| apps/filmcraft-cli | 6 | 1,207 | headless CLI + MCP |
| apps/filmcraft-web | 6 | 2,584 | wasm32 app (WebGPU/WebGL2, OPFS, WebAudio, WebCodecs) |
| xtask | — | 759 | layers / wasm / assets / fixtures / ci / bench / web / version / ico |

[VERIFIED: each `Cargo.toml` `description`; LOC via `wc -l`]

Docs: `docs/` has 15 Markdown guides (4,341 lines including ROADMAP) and one ADR. Packaging layout is covered in §6.

---

## 3. Architecture

### 3.1 Languages and frameworks
- **Rust** (edition 2024, MSRV 1.95) is used for the whole product. Small amounts of JS (`apps/filmcraft-web/web/audio-worklet.js`, `index.html`, the Node smoke test `apps/filmcraft-web/tests/smoke.mjs`), shell/PowerShell (packaging), Python (`packaging/macos/dmg/generate.py`, `packaging/linux/test_install.py`) and WGSL shaders. [VERIFIED]
- **UI:** egui/eframe **0.36.2** on wgpu, with accesskit. The desktop build adds the winit `wayland` and `x11` features. File dialogs use rfd 0.17.2 and the native macOS menu uses muda 0.21 (`apps/filmcraft/Cargo.toml`, macOS target only). The UI is meant to be replaceable: one crate, `ui-egui`, sits over the headless engine. [VERIFIED]
- **Engine-first, commands everywhere.** Every action is a command with an id, label, menu path, shortcut, params, `enabled()` and `run()` (`crates/engine/src/commands.rs`, `cmd!` macro). The UI, CLI, control channel and MCP all dispatch by id. Undo works as a stack of `Arc<Project>` copy-on-write snapshots (`docs/architecture.md` principles 1–4). [VERIFIED: docs + `cmd!("edit.undo", …)` at `commands.rs:1196`]
- **Layering is enforced** by `cargo xtask layers` (`xtask/src/main.rs` `LAYERS`/`SAME_LAYER`): only downward edges, L0 codecs may depend only on `bitstream`, and no egui/eframe/winit/rfd/cpal/muda below L5. [VERIFIED: xtask source and doc. Whether it passes is NEEDS TESTING.] Minor finding: `LAYERS` reserves names for crates that don't exist yet (`riff`, `mjpeg`, `keyframe`, `effects`, `audio`, `playback`). [VERIFIED: `docs/architecture.md` §8]

### 3.2 Render / GPU pipeline
- **CPU reference compositor** (`crates/render`) works in premultiplied linear-light f32. The GPU path (`crates/gpu`, wgpu 30.0.1, WGSL) takes a `render::plan::plan_frame` layer plan and samples YUV straight from the decoder. Anything the shaders don't cover is pre-rendered on the CPU and uploaded as an image. GPU vs CPU parity is tested (p99 ≤ 6, mean < 1.5 per 8-bit level). [VERIFIED: `docs/architecture.md` §5, `docs/testing.md` §3, `crates/golden`]
- About 33 common effects and the blend modes run on the GPU. Lumetri, keys, masks, adjustment layers and **export rendering run on the CPU**. [VERIFIED: README "Status", `ROADMAP.md`]
- wgpu backends: Metal, Vulkan, DX12, GLES/WebGL2, WebGPU. Windows starts only Vulkan+DX12 by default to avoid an AMD GL driver crash; `WGPU_BACKEND` overrides this. `FILMCRAFT_CPU_COMPOSITE=1` forces the CPU path. [VERIFIED: README, architecture]

### 3.3 Timeline model
`Project` → `root: Bin` tree plus a flat `items` map (`Media | Sequence | Subclip | AdjustmentLayer | Graphic`). Each `Sequence` has video, audio and submix tracks, and each `Track` holds sorted, non-overlapping `TrackItem`s plus transitions and a mixer strip. All time is integer `Tick` (254,016,000,000/s, divisible by every broadcast frame rate and audio rate), so there is no f64 edit math. Edit algebra lives in `crates/edit`, separate from the UI. [VERIFIED: `docs/architecture.md` §2–3, `crates/time`, `crates/edit`]

### 3.4 Media decode/encode: pure Rust, no FFmpeg
- **No FFmpeg is linked.** No `ffmpeg-sys`, `ffmpeg-next` or `libav*` appears in `Cargo.lock`. ffmpeg/ffprobe run only as **external test oracles** (`crates/testkit`, oracle tests) and in one ignored dev probe (`crates/dnx/src/ffmpeg_probe.rs`, which runs ffmpeg as a process from `#[ignore]` tests). [VERIFIED: Cargo.lock scan; `Command::new` scan shows product code spawns only `open`/`explorer`/`xdg-open` (`apps/filmcraft/src/main.rs:290-305`) and `gsettings`-style queries (`apps/filmcraft/src/appearance.rs:55`)]
- **In-house codecs:** H.264 dec+enc, HEVC dec, VP9 dec, AV1 dec, MPEG-1/2 dec, ProRes dec+enc, DNxHD/HR dec+enc, APV dec+enc, MJPEG, AAC-LC dec+enc, AC-3 dec, Opus dec, PCM. [VERIFIED: crate list]
- **Third-party codecs:** `symphonia 0.5.5` (MPL-2.0) for MP3, FLAC, ALAC, Vorbis, MP1/2 and WAV/AIFF fallback. Still images use `image 0.25.10` (png, jpeg, gif, webp, tiff, bmp). [VERIFIED: `crates/codecs/Cargo.toml`, `crates/media/Cargo.toml`]
- **OS frameworks** (in `crates/platform`, optional, with transparent fallback to the software decoders):
  - macOS: VideoToolbox decode (H.264/HEVC) and encode (H.264 opt-in, HEVC), via `objc2-video-toolbox 0.3.2`, `objc2-core-media` and `objc2-core-video`.
  - Windows: Media Foundation + D3D11 decode of H.264/HEVC/VP9/AV1 (`windows 0.62`), and **NVENC** H.264/H.265 encode.
  - Linux: **VA-API** (`dlopen("libva.so.2")`, `libva-drm.so.2`), **NVDEC** (`libnvcuvid.so.1`), and **NVENC** (`libnvidia-encode.so.1`, `libcuda.so.1`), all through `libloading`.
  - [VERIFIED: `crates/platform/Cargo.toml`, `crates/platform/src/vaapi/va.rs:64-66`, `nvdec/cuvid.rs:36`, `nvenc/session.rs:69`, `nvenc/device.rs:102`]
- **H.265 export exists only through hardware encoders** (VideoToolbox, or NVENC on Windows). There is no software HEVC encoder and no AV1 encoder. [VERIFIED: README/ROADMAP]
- Web: the browser's WebCodecs `VideoDecoder` is used for H.264/HEVC/VP9/AV1 in MP4/MOV, falling back to the in-house decoders (`apps/filmcraft-web/src/webcodecs*`). [VERIFIED: `docs/web.md`]

### 3.5 Audio engine
cpal 0.16 output on the desktop; WebAudio `AudioWorklet` on the web. **The sound card is the master clock**: played sample counts drive the playhead, with a wall-clock fallback when there is no device. The mixer graph is in `render::mixer`. Clip effects come from the in-house `audio-dsp` crate (EQ, dynamics, true-peak limiter, reverb, DeNoise, DeHummer, pitch shift; loudness to BS.1770 / EBU R128). It supports automation modes (Read/Latch/Touch/Write), sends, submixes and 5.1. Voice-over input is in `apps/filmcraft/src/audio_in.rs`. [VERIFIED: `docs/architecture.md` §5, §5.1; app module list]. Linux ALSA issue #23 is acknowledged in `ROADMAP.md`. [VERIFIED]

### 3.6 Colour pipeline
Source colour comes from Interpret Footage, VUI/`colr` or MKV Colour. It passes through decode tables (sRGB/709, PQ, HLG, camera log), the HLG OOTF, a 3×3 conversion to the working gamut, a BT.2390 tone map and gamut compression. Effects and compositing then run in working linear light, where 1.0 = 203 cd/m². Monitors are SDR (no macOS EDR). HDR export writes PQ/HLG BT.2020 with mdcv/clli/SEI. Working spaces are Rec.709, Rec.2100 PQ and Rec.2100 HLG. LUTs are `.cube`/`.3dl` embedded in the project or code-generated `builtin:` LUTs, with a GPU tetrahedral counterpart. [VERIFIED: `docs/architecture.md` §5.4]
Note: that section still says "there is no `platform` crate yet", but the crate exists. The doc is stale. [VERIFIED]

### 3.7 Project file format
`.fcproj` is UTF-8 compact JSON with an envelope `{"format":"filmcraft.project","schema_version":13,"generator":…,"project":…,"view":…}`. Media is referenced by path and never embedded. A chain of single-step migrations (v1→v13) lives in `filmcraft_format::MIGRATIONS`. Saves are atomic, autosave rotates, and a crash-recovery journal is kept. Graphics templates use `.fcgt`. [VERIFIED: `docs/project-files.md`, `crates/format`, grep for `.fcgt`]

### 3.8 Plugin / effects system
- **No third-party plugin hosting**: no VST3, AU or OpenFX. A grep for `vst3|openfx|ofx` in `.rs` files finds nothing. All effects are built in. The README claims "Premiere 26's full Video Effects bin (93 effects…)", and a test asserts that count (`crates/engine/src/vfx_tests.rs:70`). Effect definitions live in `crates/project`, implementations in `crates/render` (`effects`, `vfx`, `gpufx`) and `crates/gpu`. ROADMAP lists plugin hosting as needing an **owner decision**, because it would require FFI/`unsafe`. [VERIFIED]

### 3.9 Threading / playback model
- Desktop: `crates/ui-egui/src/frames.rs` runs a worker pool with prioritised jobs (on-screen frame first, then prefetch, then thumbnails). The UI thread never decodes. It prerolls up to 0.5 s and cancels late jobs; the GOP cache skips non-reference pictures for late frames. Draft decoding is optional. Export runs as a background job using rayon. Panics are contained with `catch_unwind` in the UI pass and workers, plus a panic hook that writes crash logs (`crates/ui-egui/src/crash.rs`). [VERIFIED: `docs/architecture.md` §5, `apps/filmcraft/src/main.rs:115-117`]
- Web: **single-threaded**. Frame jobs, decode, mix and export run cooperatively on the UI thread within a time budget. Render previews, proxies, Project Manager and mask tracking report errors on the web. [VERIFIED: `docs/web.md` "Threads"]

### 3.10 Crate dependency diagram (normal deps; simplified)

```
apps/filmcraft ──► ui-egui ──► engine ──► export ──► render ──► project ──► media ──► frame ──► color, time
     │               │           │          │           │          │                     └──► (rayon, serde)
     │               │           │          │           ├──► audio-dsp, captions, text(harfrust, skrifa)
     │               │           │          ├──► h264enc, prores, dnx, apv, aac, isobmff, mxf (L0 encoders/muxers)
     │               │           │          └──► image
     │               │           ├──► codecs ──► isobmff, matroska, mxf, mpegts, ogg, mpeg2v, ac3,
     │               │           │              h264, hevc, vp9, av1, prores, dnx, apv, aac, opus ──► bitstream
     │               │           │              └──► symphonia (MPL-2.0), image
     │               │           ├──► edit, interchange ──► cfb, roxmltree
     │               │           ├──► format, scopes, captions
     │               │           ├──► speech ──► [opt] candle, ureq+rustls(rustcrypto), sha2
     │               │           └──► tts ──► tts-text, [opt] candle, zip
     │               └──► eframe/egui (wgpu, accesskit), image, toml
     ├──► gpu ──► render, wgpu 30
     ├──► platform ──► codecs, export, h264, hevc, isobmff  + objc2-* (mac) | windows 0.62 (win) | libloading (linux)
     └──► cpal, rfd, chrono, sys-locale, muda/objc2/winit (mac), zbus (linux), winresource (win build)

apps/filmcraft-cli ──► engine, automation ──► rmcp 3.5 (stdio), tokio, schemars, base64, image ; platform, export, ...
apps/filmcraft-web ──► ui-egui, engine, ... + wasm-bindgen =0.2.129, web-sys, js-sys
```
[VERIFIED: the `[dependencies]` of every manifest. The exact edges of the apps' internal deps are abbreviated.]

---

## 4. Key dependencies

### 4.1 Lock-file totals
- **698 packages** in `Cargo.lock`: 649 from crates.io and 49 workspace path packages. **No git dependencies. No `[patch]` sections.** [VERIFIED: parsed `Cargo.lock`; `grep '\[patch'`]
- Version pins: `wasm-bindgen = "=0.2.129"` (exact, so it matches the CLI); everything else uses semver ranges, locked by `Cargo.lock`. [VERIFIED]

### 4.2 Top-level third-party crates (locked versions)
| Area | Crates |
|---|---|
| UI/windowing | egui/eframe/egui_kittest 0.36.2, winit 0.30.13, accesskit 0.24.1, rfd 0.17.2, muda 0.21.0 (mac) |
| GPU | wgpu 30.0.1 (naga 30.0.1, ash 0.38, gpu-allocator 0.28, glow 0.17) |
| Audio | cpal 0.16.0 (alsa-sys on Linux), symphonia 0.5.5 |
| Images/text | image 0.25.10, png 0.18.1, harfrust 0.12.0, skrifa 0.44.0, unicode-bidi/linebreak/segmentation |
| Parse/serde | serde 1.0.229, serde_json 1.0.151, roxmltree 0.21.1, toml 1.1.6 / 0.8.2, schemars 1.2.2, zip 7.2.0 |
| Concurrency | rayon 1.12.0, tokio 1.53.1 (automation/cli only), pollster 0.4 |
| MCP | rmcp 3.5.0 (`server`, `macros`, `transport-io`) |
| ML (optional) | candle-core / candle-nn 0.9.2 (pulls gemm, safetensors, half) |
| Network (optional, `download` feature) | ureq 3.4.2, rustls 0.23.45, **rustls-rustcrypto 0.0.2-alpha** (alpha crypto provider), sha2 0.10.9. ring 0.17.14 also comes in through rustls-webpki |
| OS FFI | objc2 0.5/0.6, objc2-* 0.3.2, windows 0.62.2 (+0.54 transitive), libloading 0.8.9, zbus 5.19 (Linux) |
| Misc | thiserror 2 (+1.0.69 transitive), web-time 1, chrono 0.4, sys-locale 0.3, cpu-time 1 (Windows) |
[VERIFIED: `Cargo.lock`]

### 4.3 Native C/C++ and `-sys` crates
The `-sys` crates in the lock are alsa-sys, atk-sys, core-foundation-sys, gdk-sys, gdk-pixbuf-sys, gio-sys, glib-sys, gobject-sys, gtk-sys, pango-sys, cairo-sys-rs, jni-sys, ndk-sys, objc-sys, renderdoc-sys, security-framework-sys, wayland-sys, windows-sys, js-sys/web-sys, and linux-raw-sys. [VERIFIED]
- The **GTK stack** (`gtk`/`gtk-sys`, LGPL system libraries) comes only from `muda`, which is a **macOS-only** dependency of `apps/filmcraft`. On macOS muda does not use GTK, so the lock entries are inert in practice. The Linux release job does not install `libgtk-3-dev`. However, `docs/contributing.md` still tells Linux developers to install GTK 3 for rfd, which looks stale. [VERIFIED: reverse-dependency lookup `gtk <- muda`; `apps/filmcraft/Cargo.toml` target table; `.github/workflows/release.yml:196-199`]. That GTK is never compiled on Linux is [NEEDS TESTING] (`cargo tree --target x86_64-unknown-linux-gnu -i gtk`).
- **alsa-sys** links system libasound (LGPL-2.1, dynamic, system library) on Linux through cpal. [VERIFIED: lock; ASSUMPTION: the usual dynamic linking]
- **renderdoc-sys** comes through wgpu-hal: a header-only/dynamic debug-capture API. [VERIFIED: reverse dependency]
- **Proprietary runtime libraries are loaded with dlopen** on Linux: libva, NVIDIA libnvcuvid, libnvidia-encode and libcuda. They are not linked at build time, so there is no redistribution, but using the NVIDIA libraries is subject to NVIDIA's driver licence. [VERIFIED: `crates/platform/src/...`]
- **`crates/platform/src/nvenc/ffi.rs`** is a Rust transcription of NVIDIA `nvEncodeAPI.h` (MIT, NVIDIA notice kept in the file). [VERIFIED: `ATTRIBUTION.md` "Third-party source code"]
- **build.rs scripts (3):**
  - `apps/filmcraft/build.rs` embeds the Windows icon and VERSIONINFO (`winresource`) and brands ProductName "FilmCraft".
  - `crates/ui-egui/build.rs` bakes `contributors/*.json|toml` into tables.
  - `crates/text/build.rs` embeds fonts from the optional `CRAFT_FONTS_DIR`.
  - None of them download anything or compile C. [VERIFIED: reading the build.rs files]
- **FFmpeg linkage: none.** LGPL/GPL exposure through FFmpeg is nil. [VERIFIED]

### 4.4 Licences of dependencies (sample of 219 / 649 cached crates)
- 120 `MIT OR Apache-2.0`, 42 `Apache-2.0 OR MIT`, 15 MIT, plus BSD-2/3, ISC, Zlib, Unicode-3.0, and `epaint_default_fonts` (OFL-1.1 + Ubuntu-font-1.0). [VERIFIED: registry Cargo.toml `license` fields]
- Licences to note:
  - **symphonia** is MPL-2.0. That is file-level weak copyleft and fine when used unmodified, which the README says is the case.
  - **self_cell** is `Apache-2.0 OR GPL-2.0-only`; choose Apache.
  - **webpki-root-certs** is CDLA-Permissive-2.0 [ASSUMPTION: not in the cache].
  - **r-efi** is `MIT OR Apache-2.0 OR LGPL-2.1+` [ASSUMPTION].
  - No GPL-only or AGPL crate was found in the sample. The other 430 crates were not checked. **Run `cargo deny check licenses` [NEEDS TESTING].**
- **There is no `deny.toml`, no `cargo audit` config, no dependabot or renovate.** [VERIFIED: file search]

### 4.5 Codec patents (licensing, not copyright)
Implementing codecs in pure Rust removes copyleft risk but **not patent risk**. [ASSUMPTION: general industry knowledge; legal review required]
- **H.264/AVC decode and encode** (encoder ships in software): AVC patent pool (Via LA, formerly MPEG LA). Distributing encoders/decoders above the royalty-free thresholds may need a licence.
- **HEVC decode** (software) plus HEVC encode through OS/NVENC: several pools (Access Advance, Via LA) plus independent holders. This is the highest-risk area. Delegating encode to the OS or NVENC shifts some of the encode exposure; the software decoder still ships.
- **AAC-LC decode and encode**: Via LA AAC pool. Many core AAC-LC patents have expired, but licence terms still apply to products. [ASSUMPTION]
- **AC-3**: core patents expired around 2017. **MPEG-2 video**: expired around 2018. MP3 has expired. [ASSUMPTION]
- **VP9 and AV1**: royalty-free under AOMedia/Google patent licences, though third-party pool claims exist (Sisvel). **Opus**: royalty-free. [ASSUMPTION]
- **ProRes and DNxHD**: Apple and Avid trademarks. ProRes encoders are officially "authorized" by Apple through a licence programme. A clean-room ProRes encoder named "Apple ProRes" in the UI raises trademark and licensing questions. [ASSUMPTION]
- The repo has no patent statement, so the fork would need a policy. Options include feature-gating encoders, relying on OS codecs where they exist, and region or distribution decisions. [VERIFIED: grep shows no patent notice in NOTICE/README]

---

## 5. Features implemented and limitations

### 5.1 What upstream says
- README: the Premiere-like workflow (Source/Program monitors, bins, three-point editing, every trim tool, multicam, nesting, markers, unlimited undo, keyboard shortcut editor with Premiere/FCP/Avid presets, autosave and crash-recovery journal), a Lumetri-style grading panel, 93 effects plus legacy ones, ~30 transitions, keyframes with value/velocity graphs, a GPU compositor, an audio mixer with automation, loudness meters, a text engine with titles, captions (608/708/Teletext; SRT/VTT/SCC), interchange (FCP7 XML, FCPXML 1.9–1.11, OTIO, EDL, AAF, OMF), exports (H.264+AAC MP4, ProRes 422 HQ, MJPEG, DNxHR, MXF OP1a/OP-Atom, PNG sequences, GIF, WAV) and 650+ engine commands. [VERIFIED: as claims in `README.md`]
- `ROADMAP.md` "Honest assessment" (2026-10-05):
  - Feature checklist is ~87%, and **ready for real work is ~50–60%**.
  - Correctness ~60–70%, codecs ~75%, export ~70%, performance ~35–40%, plugins ~0–10%, AI 25–35%.
  - **"macOS solid; Windows and Linux thin"**; "the web build is a compile target; the browser app shell isn't finished".
  - Stability is "improving, unproven".
  - Parity percentages are **partly agent-reported**, and "No automated parity check runs in CI".
  - [VERIFIED: `ROADMAP.md:14-84`]
- Known gaps: no plugin hosting, no AV1 export, H.265 export only through hardware, no camera RAW, no E-AC-3, CPU-only export rendering, no macOS EDR, real-world camera/phone media poorly tested, AAF/OMF never validated in Avid or Pro Tools, Linux audio issue #23. [VERIFIED: README "Status", ROADMAP]
- ROADMAP's header says "35 crates, 1605+ tests". It is stale: the repo has 45 crates and about 2,526 test attributes. [VERIFIED]

### 5.2 Spot checks against source
| Claim | Evidence | Tag |
|---|---|---|
| H.264 encoder with CABAC/CAVLC, B-frames, rate control, lookahead | `crates/h264enc/src/{cabac.rs,cabac_mb.rs,cavlc.rs,ratecontrol.rs,lookahead.rs}` | [VERIFIED: files exist] / quality [NEEDS TESTING] |
| AAF / OMF / FCPXML / OTIO / EDL / ALE interchange | `crates/interchange/src/{aaf/,omf/,fcpxml.rs,fcp7.rs,otio.rs,edl.rs,ale.rs}`, `crates/engine/src/aaf_omf.rs` | [VERIFIED: files exist] |
| 93 Premiere video effects | test `assert_eq!(n, 93, …)` at `crates/engine/src/vfx_tests.rs:70` | [VERIFIED: the test asserts it] / rendering fidelity [NEEDS TESTING] |
| 650+ engine commands | ~197 literal `cmd!(…)` ids in `crates/engine/src` and ~201 `uic!` UI ids. The rest are presumably generated (per-effect etc.) | [NEEDS TESTING: `filmcraft-cli commands --json \| jq length`] |
| No plugin hosting | no VST3/OFX references | [VERIFIED] |
| Bit-exact decoders on conformance streams | conformance tests exist (`crates/{h264,hevc,av1,vp9}/tests/conformance.rs`), but the vectors are generated or fetched at test time and are not in the repo | [NEEDS TESTING] |
| Speech-to-text off by default | `crates/speech` features `default = []`; `whisper` feature is opt-in in the apps | [VERIFIED] |
| Web uses WebCodecs + OPFS + AudioWorklet | `apps/filmcraft-web/src/{webcodecs.rs,opfs.rs,audio.rs}`, `web/audio-worklet.js` | [VERIFIED: files exist] |
| Hardware decode on Linux VA-API/NVDEC | `crates/platform/src/vaapi/`, `nvdec/` | [VERIFIED: code exists] / runtime [NEEDS TESTING] |
| Localisation | `crates/ui-egui/src/i18n/{es,ja,pt-br,uk,zh-cn}.tsv` | [VERIFIED] |

---

## 6. Build

### 6.1 Desktop toolchains
- **Toolchain:** there is no `rust-toolchain.toml`. The workspace declares `rust-version = "1.95"` and edition 2024. CI uses `dtolnay/rust-toolchain@686976e1…` with the `stable` channel, so the action is pinned by SHA but the compiler version floats. [VERIFIED]
- **macOS:** Xcode CLT. Release builds a universal binary (`aarch64-apple-darwin` + `x86_64-apple-darwin`) on `macos-15-xlarge`. [VERIFIED: `docs/contributing.md:18`, `release.yml:68,82`]
- **Windows:** MSVC. Release targets x64, x86 and arm64 (`x86_64/i686/aarch64-pc-windows-msvc`) and cross-compiles arm64 on `windows-latest`. [VERIFIED: `release.yml:120-126`]
- **Linux:** the release job installs `libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libasound2-dev`, building on ubuntu-22.04 for x86_64 and `ubuntu-22.04-arm` for aarch64 (an older glibc baseline). [VERIFIED: `release.yml:181-199`]
- **FreeBSD 14.3** builds in a VM (`vmactions/freebsd-vm`) with `CARGO_BUILD_JOBS=4`. [VERIFIED]
- Optional build input: `CRAFT_FONTS_DIR` (the storytold/craft-fonts checkout, pinned to `abb83316…` in CI) for Japanese fonts. Builds work without it. [VERIFIED: `release.yml:77-78`, `crates/text/build.rs`]
- Optional features: `whisper` (STT plus downloads) and `neural-voices` (Kokoro). Both are off by default. [VERIFIED: `apps/filmcraft/Cargo.toml`]
- Profiles: dev `opt-level=1`, dependencies `opt-level=2`, codec crates `opt-level=3`; release uses `lto="thin"` and `codegen-units=1`. Clean builds will be slow; release LTO across 343k LOC plus wgpu/egui is heavy. [VERIFIED: `Cargo.toml`] Actual build times and RAM use are [NEEDS TESTING].

### 6.2 WebAssembly
- Target `wasm32-unknown-unknown`, eframe web runner, **WebGPU with WebGL2 fallback**. On WebGL2 the compositor runs on the CPU. [VERIFIED: `docs/web.md`]
- Tooling: `wasm-bindgen-cli` exactly **0.2.129**, matching the crate's `=0.2.129` pin (`xtask/src/main.rs:269`), plus optional `wasm-opt`. Commands are `cargo xtask web [--dev] [--serve PORT]` and `cargo xtask wasm`, which runs `cargo check` on L0–L4 + ui-egui + web for wasm32. [VERIFIED]
- Output: a static site (`index.html`, `filmcraft_web.js`, `filmcraft_web_bg.wasm` at ~13 MB or ~5 MB compressed, `audio-worklet.js`, favicon). Hosting samples are in `packaging/web/{_headers,.htaccess,README.md}` for Netlify/Cloudflare Pages/Apache/nginx. No server code; media never leaves the machine. [VERIFIED]
- **WebCodecs:** used for H.264/HEVC/VP9/AV1 in MP4/MOV when `VideoDecoder.isConfigSupported` says yes, with fallback to the in-house decoders. Disable it with `?nowebcodecs`. Media is read through a 384 MiB LRU of 1 MiB Blob chunks. **OPFS** holds recovery snapshots and copies of media up to 4 GiB. [VERIFIED: `docs/web.md`]
- **SharedArrayBuffer and COOP/COEP:** the build is single-threaded and **does not use SharedArrayBuffer**, so production hosting **does not need** COOP/COEP (`packaging/web/README.md`). The dev server `cargo xtask web --serve` nevertheless sends `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp` (`xtask/src/main.rs:384`), and `docs/web.md` says "like a production deployment should be". **The docs contradict each other.** A threaded build would need nightly `build-std` with atomics plus cross-origin isolation. [VERIFIED]
- Browser limits: WebGPU only works in a secure context. Audio starts only after a user gesture. Desktop features that need threads (render previews, proxies, Project Manager, mask tracking) report errors. Matroska files larger than the chunk cache import slowly. A picked file is readable only while the page is open, unless it has an OPFS copy. `std::time::Instant` and `thread::spawn` panic on wasm32 and must be avoided. [VERIFIED: `docs/web.md`]
- Browser test: `apps/filmcraft-web/tests/smoke.mjs` drives headless Chrome over CDP and is run by hand. [VERIFIED] It is not in CI. [VERIFIED: no workflow runs it]

### 6.3 xtask commands
`layers`, `wasm`, `assets` (every asset has a `.attribution` sidecar plus an ATTRIBUTION.md row), `fixtures [crate…]` (pre-generates ffmpeg fixtures), `ci` (fmt --check, clippy -D warnings, test, layers, assets, wasm), `bench`, `bench-playback`, `web`, `version [set X]`, `ico`. [VERIFIED: `xtask/src/main.rs:1-15,478-490`]

### 6.4 Packaging, IDs, signing
| Platform | Artefacts | IDs / metadata | Signing |
|---|---|---|---|
| macOS | `filmcraft-<v>-macos-universal.dmg`, `filmcraft-cli-<v>-macos-universal.zip` | `CFBundleIdentifier ai.storyteller.filmcraft`, UTType `ai.storyteller.filmcraft.project` (`.fcproj`), Copyright "the FilmCraft authors" (`packaging/macos/Info.plist.in`); entitlements empty, hardened runtime, not sandboxed (`entitlements.plist`) | Developer ID + notarisation; secrets `APPLE_CERTIFICATE`, `APPLE_ID`, `APPLE_TEAM_ID`, … (`release.yml:91-99`) |
| Windows | MSI (WiX `packaging/windows/filmcraft.wxs`) + portable zip, for x64, x86 and arm64 | `Manufacturer="Learning Machines LLC"`, `UpgradeCode=31E6FB14-9DBC-41DD-B24B-26B71FB31B62`; VERSIONINFO from `apps/filmcraft/build.rs` | Azure Trusted/Artifact Signing (`AZURE_*` secrets) or PFX (`WINDOWS_CERTIFICATE`); `packaging/windows/sign.ps1` |
| Linux | AppImage (+`.zsync`), deb, rpm (nfpm 2.47.0), tar.gz with `install.sh`, Flatpak | app id `ai.storyteller.filmcraft`; `.desktop`, metainfo (`<developer><name>Learning Machines LLC`), mime XML, nfpm `vendor: Learning Machines LLC`; Flatpak runtime `org.freedesktop.Platform 25.08` | none (not signed) |
| FreeBSD | tar.gz | — | none |
| Web | `filmcraft-web-<v>.zip` | — | none |

[VERIFIED: files listed; `release.yml`]. Missing signing secrets produce unsigned artefacts with a warning rather than a failure. The release is a draft that a maintainer publishes by hand. [VERIFIED: `release.yml:1-9`, `docs/releasing.md`]
Note: `packaging/linux/ai.storyteller.filmcraft.mime.xml` appears to have an empty `<mime-info>` body (no glob for `.fcproj`). [VERIFIED: file content as printed; might be filled at package time, NEEDS TESTING]

---

## 7. Tests

| Metric | Value | Tag |
|---|---|---|
| `#[test]` / `#[tokio::test]` attributes | **2,526** (40 `#[ignore]`) | [VERIFIED: grep] |
| By crate (top) | engine 497, ui-egui 345, render 172, platform 145, codecs 128, audio-dsp 122, export 73, interchange 66, edit 63, project 62, h264 51, captions 49, apps/filmcraft 49 | [VERIFIED] |
| Integration test dirs | 28 crate/app `tests/` directories | [VERIFIED] |
| Property tests | proptest in 9 files (bitstream, time, geom, color, edit, project, captions, interchange, isobmff) | [VERIFIED] |
| UI tests | `egui_kittest` in 53 files (headless UI driving, snapshot dirs via `FILMCRAFT_UI_SNAPSHOT_DIR`) | [VERIFIED] |
| Golden images | 27 PNGs in `crates/golden/goldens/` plus a GPU-vs-CPU parity test (skips without a GPU adapter); re-bless with `FILMCRAFT_BLESS=1` | [VERIFIED] |
| Oracle tests | compare against ffmpeg/ffprobe, which are **skipped (and pass)** when ffmpeg is missing unless `FILMCRAFT_REQUIRE_ORACLES=1`. On Windows they look only in Unix paths, so most skip | [VERIFIED: `docs/testing.md:4-9,52-54`, `docs/contributing.md:20`] |
| Fuzzing | **No cargo-fuzz/libFuzzer/AFL harnesses.** Deterministic in-test mutation fuzzing (xorshift bit flips and truncation under `catch_unwind`) exists in `crates/isobmff/tests/fuzz.rs`, `crates/prores/tests/fuzz.rs` and `crates/apv/tests/fuzz.rs`, plus `robustness.rs` in aac, h264, hevc, opus and vp9, and `ui-egui/tests/robustness_ui.rs`. There are about 62 functions named mutat*/corrupt*/truncat*/hostile*. Matroska, MXF, MPEG-TS, Ogg, CFB/AAF, AV1 and MPEG-2 have no dedicated mutation test file | [VERIFIED] |
| Browser e2e | `apps/filmcraft-web/tests/smoke.mjs` (manual) | [VERIFIED] |

### CI workflows (`.github/workflows/`)
| Workflow | Triggers | Runners / OS | What it does |
|---|---|---|---|
| `release.yml` | push to `release`, manual dispatch | ubuntu-latest (version, FreeBSD VM, web, publish), macos-15-xlarge, windows-latest (×3 arch), ubuntu-22.04 + 22.04-arm (Linux), ubuntu-24.04 + 24.04-arm (Flatpak) | builds, signs and packages every platform; creates a draft GH Release with SHA256SUMS |
| `packaging-lint.yml` | push main/release, PRs | ubuntu-latest | lints packaging scripts/metadata |
| `freebsd.yml` | nightly cron, PRs (path-filtered), dispatch | ubuntu-latest + FreeBSD VM | FreeBSD build |
| `windows-arm64.yml` | push main, PRs, weekly cron | windows-latest (package) + windows-11-arm (install, run CLI, uninstall) | ARM64 MSI install smoke test |

[VERIFIED]

**This is the critical finding for tests: no workflow runs `cargo test`, `cargo clippy`, `cargo fmt` or `cargo xtask ci`.** The quality gates exist only as a local convention for agents and contributors (`CLAUDE.md` "Quality gates"). ROADMAP item 5 asks for "CI that runs the test suite … on both [Windows and Linux]". So the 2,526 tests are **not enforced on PRs**, and whether the suite passes on any OS is unknown. [VERIFIED: grep for `cargo test|clippy` in workflows matches only a comment at `windows-arm64.yml:3`] Whether the suite passes is [NEEDS TESTING].

---

## 8. Security

### 8.1 Parsing attack surface
Almost every container and codec parser is **in-house, new code with a few days of history**:
- MP4/MOV (`isobmff`), MKV/WebM (`matroska`), MXF (`mxf`), MPEG-TS/PS (`mpegts`), Ogg, CFB/AAF and OMF (`cfb`, `interchange/aaf`, `interchange/omf`).
- WAV/BWF/AIFF (`media`, `interchange/wav.rs`).
- Every video and audio bitstream decoder listed in §3.4.
- Subtitle formats SRT/VTT/SCC/MCC/STL/TTML (`captions`), EDL/ALE text, `.cube`/`.3dl` LUTs, `.fcproj`/`.fcgt` JSON, presets.

Third-party parsers: `image` (PNG, JPEG, GIF, WebP, TIFF, BMP), `symphonia` (MP3, FLAC, ALAC, Vorbis, and WAV/AIFF/ISO-MP4 audio), `roxmltree` (FCP7 XML, FCPXML, TTML, metainfo), `serde_json`, `toml`, `skrifa`/`harfrust` (fonts, including system and user fonts), `zip` (Kokoro package), and candle's PyTorch checkpoint reader (`kokoro-v1_0.pth`, a zip/pickle format; `crates/tts/src/kokoro/weights.rs`). [VERIFIED: manifests and crate list]
Mitigations: workspace `unsafe_code = "forbid"`, so memory-safety bugs are limited to platform FFI and dependencies. The AGENTS.md "never crash / hostile input" rules apply. Clean crates carry `#![deny(clippy::unwrap_used, expect_used, panic, …)]` (45 files). My heuristic scan found ~49 panic-style calls in non-test code, almost all in test-utility modules (`crates/dnx/src/ffmpeg_probe.rs`, `crates/engine/src/media_test_util.rs`). [VERIFIED: scripted scan; heuristic, ASSUMPTION about precision]. The remaining risks are DoS (huge allocations, infinite loops, quadratic paths) and logic bugs, not RCE. [ASSUMPTION]

### 8.2 `unsafe`
- **197 `unsafe` blocks/fns, all in `crates/platform`**: `videotoolbox_encode.rs` 39, `vaapi/va.rs` 34, `nvdec/cuvid.rs` 32, `media_foundation/mft.rs` 31, `videotoolbox.rs` 21, `nvenc/session.rs` 17, `media_foundation/gpu.rs` 16, `nvenc/device.rs` 6, `cursor.rs` 1. There are 203 `SAFETY:` comments. [VERIFIED]
- `platform` opts out of the workspace lints and sets `unsafe_code = "deny"` with module-level `#[allow(unsafe_code)]` and `clippy::undocumented_unsafe_blocks = "deny"` (ADR 0001). Every other crate uses the workspace lints (`unsafe_code = "forbid"`). `xtask` has no `[lints]` table, but it is not shipped. [VERIFIED: `grep -L '^\[lints\]'` lists only `crates/platform/Cargo.toml` and `xtask/Cargo.toml`]
- Note: `crates/platform/src/cursor.rs` uses `unsafe`, yet ADR 0001 says the crate is for "OS media FFI only". This is a small scope creep. [VERIFIED: file exists with `#[allow(unsafe_code)]`]
- The hardware paths (DXVA, VA-API, NVDEC, NVENC and VideoToolbox callbacks) run hand-written FFI over C structs transcribed by agents. This is the highest memory-safety risk in the codebase and is mostly exercised on maintainer hardware (tests skip without a GPU). [VERIFIED: tests skip; NEEDS TESTING on real hardware]

### 8.3 Network access
- **No telemetry, analytics or crash upload.** A grep for telemetry/analytics/sentry/update-check finds nothing relevant. Crash logs stay local (`<data dir>/Logs/`). [VERIFIED]
- Outbound HTTP exists only in optional features. These are the `speech` `download` feature (Whisper weights from huggingface.co, pinned revisions and SHA-256, `crates/speech/src/models.rs`) and `tts` Kokoro-82M (huggingface `hexgrad/Kokoro-82M` at a pinned commit, SHA-256) plus CMUdict from raw.githubusercontent.com at a pinned commit with SHA-256 (`crates/tts/src/catalog.rs:44-51`). Downloads happen only after a confirmation dialog, per ATTRIBUTION.md. TLS is ureq + rustls with **`rustls-rustcrypto 0.0.2-alpha`** as the crypto provider (an alpha-quality, not formally audited provider) and the platform verifier. [VERIFIED]. I did not review the confirmation-dialog gating in code [NEEDS TESTING].
- Other URLs in code are Help-menu links only (Discord, getartcraft.com, GitHub) (`crates/ui-egui/src/links.rs`). [VERIFIED]
- The web build has no external scripts or CDNs in `index.html`. [VERIFIED]

### 8.4 Local control surface and MCP
- **Desktop control server** (`apps/filmcraft/src/control_server.rs`): TCP on `127.0.0.1:<port>`, **off unless `--control` or `FILMCRAFT_CONTROL_PORT` is set**, **no authentication**. It accepts only JSON lines (an HTTP request line is rejected and the connection closed, which blocks browser cross-origin `fetch`), with a 4 MiB line cap, at most 16 connections and a 60 s reply timeout. [VERIFIED]
  - Risk: any local process or user on the machine gets full control. Commands include `media_import` of arbitrary absolute paths, `file.exportMedia`/`ui.screenshot` writing to **arbitrary paths**, synthetic UI input and project save. So it is a local arbitrary-file-write and file-read primitive while enabled. The docs say "The port has no authentication, so only enable it while you use it". [VERIFIED: `docs/control-protocol.md:16`]. For the fork, consider a token, a Unix socket or named pipe, or restricting write paths.
- **MCP** (`crates/automation`, rmcp 3.5 `transport-io`): **stdio only**, so the trust boundary is the MCP client. The same file read/write powers apply. Bridge mode connects to the unauthenticated TCP port. [VERIFIED]
- **Web:** `window.filmcraft` is a JS API that exposes execute, readFile, importUrl and so on to any script on the page. That matters if the app is embedded alongside third-party scripts. [VERIFIED: `docs/web.md`]

### 8.5 Update mechanism
There is no in-app updater. The AppImage embeds `gh-releases-zsync|storytold|filmcraft|latest|…` update info for AppImageUpdate (`packaging/linux/package.sh:126`), and **the fork must change it, or users will "update" to upstream builds**. Other platforms update by reinstalling. [VERIFIED]

### 8.6 Supply chain
- GitHub Actions are **all pinned by commit SHA** (checkout v7.0.1, upload/download-artifact, dtolnay/rust-toolchain, Swatinem/rust-cache, vmactions/freebsd-vm). The top-level `permissions: contents: read`; only the publish job has `contents: write`; signing secrets sit in a `release` environment. [VERIFIED: `release.yml:22-23,371-372`]
- Weak points:
  - `appimagetool` is downloaded from the **`continuous`** release with no checksum (`packaging/linux/package.sh:110`).
  - nfpm `.deb` comes from GitHub with no checksum (`release.yml:200-202`).
  - `cargo install wasm-bindgen-cli --locked` is version-pinned but not hash-pinned.
  - The Rust toolchain is the floating `stable`.
  - craft-fonts is checked out at a pinned SHA (good).
  - [VERIFIED]
- Missing: `deny.toml`/cargo-deny, cargo-audit, dependabot/renovate, `SECURITY.md`, CODEOWNERS, SBOM. [VERIFIED: absent]
- No git dependencies or `[patch]`. All 3 build scripts are local and benign. [VERIFIED]

---

## 9. Licensing

- **Project licence:** dual `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`, workspace `license`). The MIT copyright line is "Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors". [VERIFIED]
- **NOTICE** (quoted):
  > FilmCraft
  > Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors.
  > Licensed under the MIT License or the Apache License, Version 2.0, at your option (LICENSE-MIT, LICENSE-APACHE).
  >
  > The ArtCraft name, wordmark and logos in docs/brand/ are trademarks of the ArtCraft Team. They are not open source and are not covered by the license above; their terms are in docs/brand/LICENSE-brand.txt.
  >
  > Third-party material … Inter font (assets/fonts/Inter-*.ttf) — OFL-1.1 … Noto Serif … OFL-1.1 … JetBrains Mono … OFL-1.1 … Builds made with craft-fonts (CRAFT_FONTS_DIR; all official releases) also embed … BIZ UDPGothic, Shippori Mincho, BIZ UDMincho — OFL-1.1 … Public-domain film and NASA frames and CC0 music shown in README screenshots (docs/images/) — public domain / CC0-1.0.

  Apache-2.0 §4(d) requires derivative works to carry this NOTICE. [VERIFIED: `NOTICE`]
- **ATTRIBUTION.md summary:**
  - Bundled fonts: Inter ×5, Noto Serif and JetBrains Mono, all OFL-1.1 with their licence texts.
  - The app icon (an owl drawn "in ArtCraft" by Brandon Thomas) and all derived icns/ico/hicolor/favicon files are MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`).
  - The DMG background is by @XusBadia, MIT/Apache.
  - README screenshots (`docs/images/*.png`) are original, showing public-domain films (Night of the Living Dead, Carnival of Souls, Charade), NASA footage and CC0 Musopen Chopin. The media itself is not in the repo.
  - 27 golden PNGs are original.
  - Code-drawn assets are listed (icons in `crates/ui-egui/src/icons.rs`, procedural demo footage, built-in graphics templates, Lumetri looks and presets written as code, **not LUT files**).
  - The Ukrainian translation is original.
  - Third-party source: the NVIDIA `nvEncodeAPI.h` transcription (MIT).
  - Downloaded at runtime: Whisper weights (MIT; the HF conversion is labelled Apache-2.0).
  - craft-fonts (OFL-1.1).
  - [VERIFIED]
- **Gaps in attribution:**
  - Kokoro-82M (Apache-2.0) and CMUdict (BSD-style), both downloaded at runtime, are **not listed in ATTRIBUTION.md**, although `crates/tts/src/catalog.rs:29` states their licences.
  - symphonia's MPL-2.0 is not in NOTICE (MPL needs source availability for MPL files, which crates.io provides).
  - `epaint_default_fonts` (OFL + Ubuntu Font Licence) is embedded through the eframe `default_fonts` feature and not mentioned. Its OFL/UFL texts must ship.
  - No `THIRD-PARTY-LICENSES` bundle is generated for Rust crates (MIT/Apache/BSD require the notices to ship with binaries). [VERIFIED: absent; NEEDS TESTING whether packaging includes them; consider `cargo-about`]
- **Copyleft deps:** no GPL/AGPL crates found in the sampled 34% of crates. MPL-2.0 (symphonia; possibly webpki-root-certs/option-ext transitively) is unmodified. LGPL system libraries (libasound, and GTK only if compiled) are dynamic. [VERIFIED for the sample / ASSUMPTION for the rest]
- **Trademarks:**
  - **ArtCraft** marks in `docs/brand/` are proprietary (`docs/brand/LICENSE-brand.txt`): "If you distribute or publish a modified version … you must remove the ArtCraft Marks … and must not call it ArtCraft." They are **compiled into the binary** (`crates/ui-egui/src/brand.rs` `include_bytes!` of `artcraft-logo*.png`). [VERIFIED]
  - "FilmCraft" itself is not claimed as a trademark anywhere. [VERIFIED: no claim] Rebranding is still prudent. [ASSUMPTION]
  - Adobe disclaimer: README footer lists Adobe, Photoshop, Illustrator, Premiere Pro, Lightroom, Acrobat, After Effects and InDesign as Adobe trademarks, and states no affiliation. [VERIFIED]
  - Code and UI use Adobe and other product names heavily. Counts in `.rs` / all files:
    - "Premiere" 280 / 346
    - "Lumetri" 136 / 207 (UI panel names like "Lumetri Color")
    - "Essential Sound" 28 / 45
    - "Ultra Key" 8 / 14
    - "Warp Stabilizer" 7 / 13
    - "Avid" 38 / 77
    - "Final Cut" 15 / 22
    - Plus a shortcut preset named "Premiere Pro Compatible" (`crates/engine/src/shortcuts.rs:60`).
    - [VERIFIED: grep]
    - Using Adobe *feature* names (Lumetri, Essential Sound, Ultra Key, Warp Stabilizer) as UI labels is a trademark and trade-dress risk beyond the nominative "compatible with" use. [ASSUMPTION; legal review]
  - Storytold: GitHub org in URLs (§10).
- **Clean-room claims:**
  - AGENTS.md §2 says: implement only from specs; never read the source of reference decoders, even permissive ones; never copy GPL code; ffmpeg as an oracle only; no Adobe bundle disassembly; Premiere reference screenshots stay in the gitignored `plan/premiere/`. [VERIFIED]
  - These are process claims and cannot be verified from the repo. Agents wrote the code, and **LLM output may reproduce training-data code** (FFmpeg/libavcodec, x264, dav1d, libopus are all heavily represented). [ASSUMPTION] I recommend a code-similarity scan (ScanCode, or a fingerprint comparison against libavcodec/x264/dav1d/libvpx) before relying on "no GPL code".
  - The DNx crate documents black-box probing of ffmpeg's VC-3 decoder behaviour (`crates/dnx/src/ffmpeg_probe.rs`). That is observation, not code copying. [VERIFIED]

---

## 10. Branding surface

### 10.1 Counts
| Term | Occurrences | Files | Tag |
|---|---|---|---|
| `filmcraft` (case-insensitive) | 7,945 | 830 | [VERIFIED: `git grep -i -c`] |
| ↳ in `.rs` | 6,014 (275 are capitalised "FilmCraft", i.e. mostly user-facing) | — | [VERIFIED] |
| file paths containing "filmcraft" | 94 | — | [VERIFIED] |
| `artcraft` | 211 | 53 | [VERIFIED] |
| `storytold` | 54 | 20 | [VERIFIED] |
| `storyteller` (app-id prefix `ai.storyteller.`) | 82 | 29 | [VERIFIED] |
| `getartcraft` | 36 | 13 | [VERIFIED] |
| `discord.gg` | 13 | 8 | [VERIFIED] |
| `.fcproj` | 153 | 63 | [VERIFIED] |
| "Learning Machines LLC" | 4 lines in 3 files | — | [VERIFIED] |

### 10.2 Structural locations
- **Crate and package names:** every crate is `filmcraft-*` (workspace deps in the root `Cargo.toml`), and Rust paths use `filmcraft_*`. The binaries are `filmcraft`, `filmcraft-cli` and `filmcraft-web`, and the wasm artefacts are `filmcraft_web*.wasm/js`. [VERIFIED]
- **Window title and dialogs:** `apps/filmcraft/src/main.rs:124` `.with_title("FilmCraft")`, and the error dialogs at `:74` and `:281`. In the web app, `<title>FilmCraft</title>` and "FilmCraft stopped working" are in `apps/filmcraft-web/web/index.html`. [VERIFIED]
- **App ID / bundle ID:**
  - `APP_ID = "ai.storyteller.filmcraft"` at `apps/filmcraft/src/main.rs:49` (Wayland app_id).
  - macOS `CFBundleIdentifier` and the UTType `ai.storyteller.filmcraft.project`.
  - Linux `.desktop`/metainfo/mime/Flatpak file names and ids under `packaging/linux/ai.storyteller.filmcraft.*` and `packaging/linux/flatpak/ai.storyteller.filmcraft*.yml`.
  - hicolor icon names `assets/app-icon/hicolor/*/apps/ai.storyteller.filmcraft.png`.
  - The dev desktop entry is in `apps/filmcraft/src/dev_icon.rs`.
  - [VERIFIED]
- **Windows:** the WiX `filmcraft.wxs` holds Name "FilmCraft", Manufacturer "Learning Machines LLC" and the **UpgradeCode GUID**, which must be regenerated or the fork's MSI will upgrade over upstream installs. `apps/filmcraft/build.rs` sets ProductName and LegalCopyright. [VERIFIED]
- **Icons:** `assets/app-icon/*` (owl; MIT/Apache, so it can legally be kept, but it is upstream's identity) and `apps/filmcraft-web/web/favicon.png`. [VERIFIED]
- **ArtCraft marks compiled in:** `crates/ui-egui/src/brand.rs` (About dialog and Home screen wordmark), `crates/ui-egui/src/header.rs:127-128` (header Discord button, "Join the ArtCraft Discord"), `crates/ui-egui/src/menus.rs:129-131` and `apps/filmcraft/src/native_menu.rs:42` (Help menu: ArtCraft Discord, ArtCraft Website, "FilmCraft on getartcraft.com"), `crates/ui-egui/src/links.rs` (all URLs, plus a unit test asserting the `storytold`/getartcraft URL scheme), `crates/ui-egui/src/panels/{dialogs.rs,import_mode.rs}`, and i18n TSVs. [VERIFIED]
- **Menus and help URLs:** `crates/ui-egui/src/panels/keyboard.rs:136` `HELP_URL = https://github.com/storytold/filmcraft/tree/main/docs`. [VERIFIED]
- **Splash / Home screen:** the ArtCraft wordmark through `brand.rs` (no separate splash asset). [VERIFIED]
- **Installer metadata:**
  - `packaging/linux/nfpm.yaml` (`vendor: Learning Machines LLC`)
  - `ai.storyteller.filmcraft.metainfo.xml.in` (developer name)
  - `packaging/macos/Info.plist.in`
  - `packaging/macos/dmg/background.*` (derived from the owl)
  - `packaging/env.sh`
  - [VERIFIED]
- **Config / data directory:** `crates/engine/src/autosave.rs:494-509`. It resolves `FILMCRAFT_DATA_DIR`, then a portable dir next to the exe, then `~/Library/Application Support/FilmCraft`, `%APPDATA%\FilmCraft` or `$XDG_DATA_HOME/filmcraft`. Logs go to `filmcraft.log`; crash logs go to `<data>/Logs`. [VERIFIED]
- **Environment variables:** 24 `FILMCRAFT_*` variables, for example `FILMCRAFT_CONTROL_PORT`, `FILMCRAFT_DATA_DIR`, `FILMCRAFT_MODELS_DIR`, `FILMCRAFT_CPU_COMPOSITE`, `FILMCRAFT_REQUIRE_ORACLES` and `FILMCRAFT_BLESS`. [VERIFIED]
- **File formats and protocol identifiers:**
  - `.fcproj` with `"format": "filmcraft.project"`
  - `.fcgt` graphics templates
  - `"filmcraft.effect-presets"` and `"filmcraft.export-presets"` preset formats
  - OTIO `metadata.filmcraft`
  - MCP resources `filmcraft://document` and `filmcraft://commands`
  - the JS global `window.filmcraft` / `filmcraftLoad`
  - `.mcp.json` server names
  - [VERIFIED]
  - These are **compatibility-sensitive**. Renaming them breaks the ability to open upstream projects and presets unless migrations or aliases are added.
- **Update channel:** AppImage `gh-releases-zsync|storytold|filmcraft|…`, derived from `REPO` in `packaging/linux/package.sh:126`. [VERIFIED]
- **Repository links:** `Cargo.toml` `repository = "https://github.com/storytold/filmcraft"`, plus README, docs and contributing links to `storytold/craftrules`, `storytold/craft-fonts` and `storytold/photocraft-corpus`. [VERIFIED]
- **Credits:** `contributors/contributors.json` and `people.toml` are baked into the About window and tied to upstream's craftrules scripts. [VERIFIED]
- **Telemetry:** none to rebrand. [VERIFIED]

### 10.3 How to isolate the branding (recommendation)
1. **Mandatory before any public fork release:** remove `docs/brand/*` and `brand.rs`'s `include_bytes!`, the ArtCraft Discord, website and Help-menu entries, and the ArtCraft README header and footer (required by `LICENSE-brand.txt`). Replace the manufacturer/vendor "Learning Machines LLC", regenerate the WiX UpgradeCode, and change the AppImage zsync repo. [VERIFIED requirement]
2. Introduce one **`brand` module or crate** (for example `crates/brand` at L0 with consts, or a `brand.toml` read by `build.rs` and the packaging scripts). It would hold `PRODUCT_NAME`, `APP_ID` (`ai.ubunye.creativefilm` or similar), `VENDOR`, `DATA_DIR_NAME`, `LOG_NAME`, `URLS` (website, docs, issues, community), `ICON_PATHS` and `UPDATE_REPO`. Then refactor `links.rs`, `brand.rs`, `main.rs` (APP_ID/title), `autosave.rs` (data dir), `build.rs` (VERSIONINFO) and the `packaging/*` templates (`Info.plist.in`, `.metainfo.xml.in`, `.desktop`, `nfpm.yaml`, `.wxs`, Flatpak) to read from it. Templates already use `*.in` substitution for some files.
3. Rename user-visible strings ("FilmCraft" appears in 275 `.rs` string sites plus the i18n TSVs) through the brand const and i18n. **Leave the crate names (`filmcraft-*`) alone at first**, so upstream merges stay cheap (~6k internal identifier occurrences). Rename crates only after deciding to diverge permanently.
4. For file formats, keep reading `filmcraft.project`/`.fcproj`, `.fcgt` and the preset formats, and write a new format id only alongside a migration step. Add the new extension to the UTType and mime files and keep the old one as an import type.
5. Data dir: migrate or read the old `FilmCraft` dir on first run if desired. Environment variables: accept both `FILMCRAFT_*` and new prefixes for a transition period.
6. Replace the owl icon, although it is legally reusable under MIT/Apache, and regenerate with `packaging/icons.sh`. Replace the DMG background.
7. Review Adobe feature-name labels (Lumetri etc.) with counsel. Consider generic names (e.g. "Color Panel") with the "Premiere-compatible" wording kept in docs only.
8. Strip or replace `contributors/*`, the craftrules references and the `storytold/*` links in docs.

---

## 11. CLI, MCP and agent automation

### 11.1 CLI (`apps/filmcraft-cli`, binary `filmcraft-cli`)
Subcommands: `exec <id> [k=v…|json]`, `run <script.jsonl|->`, `commands [filter] [--json]`, `describe <id>`, `inspect [project|sequence]`, `import`, `export <out> [--preset|--format h264|hevc|prores|dnxhr|apv|mjpeg|mxf-op1a|mxf-opatom|png|tiff|bmp|gif|wav|aiff] [--range] [--settings json] [--gpu-rendering] [--scale] [--quality] [--no-audio] [--queue]`, `export --list-presets`, `render --seconds S --out f.png`, `probe <media>`, `bench-decode`, `mcp`, `help`, `--version`.
Options: `--project`, `--demo`, `--save`, `--save-as`, `--bridge 127.0.0.1:PORT`, `--data-dir`, `--keep-going`, `--compact`. [VERIFIED: `apps/filmcraft-cli/src/main.rs:63-110`]
The desktop binary `filmcraft` takes `--control <port>`, `--demo|--empty`, `--recover|--no-recover`, `--data-dir` and files. [VERIFIED: `apps/filmcraft/src/main.rs:1-20`]

### 11.2 MCP server (`crates/automation`, rmcp 3.5)
- Transport: **stdio**. Two modes: headless (an in-process engine; `--demo` / `--project`) and **bridge** (forwards to the app's TCP control channel). [VERIFIED: `docs/agents.md` §1]
- 19 `#[tool]` attributes. [VERIFIED: grep]
  - Tools: `command_list`, `command_run`, `command_batch`, `doc_inspect`, `render_preview`, `project_inspect`, `sequence_inspect`, `media_import`, `render_frame`.
  - Bridge-only tools: `ui_inspect`, `ui_elements`, `ui_click`, `ui_drag`, `ui_key`, `ui_type`, `ui_screenshot`, `ui_control`.
  - Resources: `filmcraft://document`, `filmcraft://commands`.
  - The server sends MCP tool annotations (read-only/destructive hints), strict-argument errors (-32602), progress notifications and cancellation for long exports. [VERIFIED: `docs/agents.md`]
- **Auth: none.** stdio trusts the client. Bridge mode uses the unauthenticated loopback TCP channel (§8.4). [VERIFIED]

### 11.3 Control channel
JSON-lines over loopback TCP. Methods include `engine.execute`, `engine.commands`, `ui.inspect`, `ui.elements`, `ui.set` (very broad UI-state setter), `ui.click`, `ui.move`, `ui.drag`, `ui.scroll`, `ui.key`, `ui.type`, `ui.timeline.hit`, `ui.playback`, `ui.screenshot` (writes to a path) and `ui.panel.show`. Every widget registers an automation id. [VERIFIED: `docs/control-protocol.md`, `crates/ui-egui/src/control.rs`]

### 11.4 `.mcp.json` (content)
```json
{"mcpServers":{
  "filmcraft":{"command":"target/release/filmcraft-cli","args":["mcp","--bridge","127.0.0.1:9876"]},
  "filmcraft-headless":{"command":"target/release/filmcraft-cli","args":["mcp","--demo"]}}}
```
These are relative binary paths that Claude Code offers to register when the repo is opened. Anyone who opens the fork in Claude Code will be prompted to run a locally built binary. That is harmless, but keep it in mind. [VERIFIED]

### 11.5 `AGENTS.md` / `CLAUDE.md` (summary only; not followed)
- **AGENTS.md:**
  - §0 "Never crash" outranks feature work: Result everywhere, no unwrap/expect/panic outside tests, no `unsafe` except `crates/platform`, all inputs hostile, bounded recursion, `catch_unwind` for jobs, mutation-fuzz parsers, regression tests for every crash fix.
  - §1 Assets: no Adobe artwork in any form, open licences only, an `.attribution` sidecar plus an ATTRIBUTION.md row, the ArtCraft marks exception (forks must remove them), fonts only through craft-fonts.
  - §2 Clean-room: specs and conformance vectors only, no reading of reference decoder source, no GPL code, ffmpeg as oracle only, permissive deps (MPL-2.0 unmodified allowed).
  - §3 refers to CLAUDE.md.
  - It ends with a contributor-credits process: agents ask their human once about a `people.toml` entry. [VERIFIED]
- **CLAUDE.md:**
  - Describes FilmCraft as a "clean-room … pure-Rust NLE targeting Adobe Premiere Pro parity", sibling of photocraft/pdfcraft/vectorcraft.
  - Tells agents to read the maintainer-local, gitignored `plan/STATUS.md`, `plan/execution-plan.md` and `plan/premiere/` reference screenshots.
  - Says to follow an "autonomous operation protocol" ("Don't stop to ask unless it lists the decision as the user's").
  - Repeats the non-negotiables (never crash, assets, fonts, clean room, pure Rust, layering, exact Tick time, everything is a command and agent-drivable).
  - Gives quality gates (`cargo xtask ci`), the one-task-id-per-commit convention, how to run the app with `--control 9876`, and parallel agent worktrees with separate `CARGO_TARGET_DIR`. [VERIFIED]
  - The development process depends on **private planning files (`plan/`) that are not in the repo**. A fork inherits the code but not the plan or the reference material. [VERIFIED: `.gitignore` has `plan/`]

---

## 12. Readiness assessment

### 12.1 Maturity scores (0 = absent, 5 = production-grade)

| Area | Score | Rationale |
|---|---|---|
| **Build** | **3** | Clean Cargo workspace with no git deps and no C toolchain beyond system headers. A release pipeline exists for 6 OS/arch families plus web. The compiler toolchain is unpinned, LTO builds of 343k LOC will be heavy, and no CI build runs on PRs except FreeBSD, ARM64 packaging and the packaging lint. Building successfully is [NEEDS TESTING]. |
| **Features** | **3** | Exceptionally broad on paper (Premiere-like UI, many codecs, interchange, colour, audio). Upstream rates "ready for real work" at 50–60% and macOS-centric. No plugins, CPU-bound export, hardware-only HEVC encode. Depth and fidelity are unproven and partly agent-reported. |
| **Tests** | **2** | 2,526 tests including oracle, golden, property, kittest UI and mutation tests, which is a good design. But **nothing runs in CI**, oracle tests silently skip without ffmpeg, there is no coverage data, no cargo-fuzz, GPU and hardware tests skip on CI-like machines, and there is no real-world media corpus. |
| **Security** | **2** | Strong base: forbid-unsafe outside one FFI crate, a never-crash discipline, no telemetry, pinned Actions, hash-pinned model downloads. Weak points: a very large new in-house parser surface written mostly by agents in days, no real fuzzing, an unauthenticated control port with arbitrary file write while enabled, an alpha rustls crypto provider, no cargo-deny/audit, no SECURITY.md, unverified tool downloads in packaging, and hand-written GPU-driver FFI. |
| **Docs** | **4** | Thorough architecture, testing, web, control-protocol, project-file and release docs, plus an honest ROADMAP. Some drift: architecture says "no platform crate yet", ROADMAP says "35 crates / 1605 tests", the web COOP/COEP guidance contradicts itself, and Linux GTK instructions look stale. |
| **Packaging** | **3** | MSI (3 arch), signed and notarised DMG, AppImage with zsync, deb, rpm, Flatpak manifest, FreeBSD, web zip, and a SHA256SUMS draft release. Installer smoke tests cover only Windows ARM64. Linux artefacts are unsigned. Every ID and vendor field is upstream-branded and must be changed. |

### 12.2 Top risks
1. **Code provenance and review debt.** About 343k LOC in 11 days, more than 85% of commits co-authored by AI agents, and one dominant maintainer. "Clean-room" is a process claim that can't be audited. LLM output may reproduce GPL code (FFmpeg/x264/dav1d). Mitigation: a code-similarity scan, a human review plan for codecs and parsers, and a provenance policy. [ASSUMPTION based on VERIFIED history]
2. **Codec patent exposure.** Software H.264 encode and decode, HEVC decode and AAC encode and decode ship by default. Pure Rust does not remove patent licensing obligations. Mitigation: legal review, feature flags, or OS codec delegation by region or distribution. [ASSUMPTION]
3. **No CI test gate.** The test suite's status on any OS is unknown, and regressions can merge freely. This is the first thing the fork should fix: a matrix of `cargo xtask ci` with `FILMCRAFT_REQUIRE_ORACLES=1` on Linux, macOS and Windows. [VERIFIED]
4. **Untested platforms.** Upstream says Windows and Linux are "thin" and the web shell is "not finished". Hardware FFI paths (DXVA, VA-API, NVDEC, NVENC) are the riskiest code and the least tested. [VERIFIED claim]
5. **Hostile-media robustness.** The in-house demuxers and decoders face untrusted files and have only deterministic in-test mutation tests. Mitigation: cargo-fuzz harnesses per parser in CI or OSS-Fuzz, plus allocation caps review. [VERIFIED gap]
6. **Trademark and branding obligations.** The ArtCraft marks are compiled in and must be removed. Adobe feature names (Lumetri, Essential Sound, Ultra Key, Warp Stabilizer) appear in the UI. The bundle IDs, MSI UpgradeCode, vendor and AppImage update channel point to upstream. [VERIFIED]
7. **Upstream velocity.** About 100 commits a day means a fork drifts quickly. Merge cost rises with every renamed identifier, so isolate branding behind consts rather than global renames. [VERIFIED cadence]
8. **Local automation surface.** The unauthenticated control port and MCP have arbitrary path read and write. It is fine for development, but risky if users enable it on shared machines. [VERIFIED]
9. **Licence compliance in binaries.** No bundled third-party licence file for ~650 crates. epaint fonts (OFL/UFL) and Kokoro/CMUdict are not attributed. MPL symphonia is not listed in NOTICE. [VERIFIED gap]
10. **Bus factor and private planning.** One maintainer owns 70% of commits, and the roadmap and reference material are in a private `plan/` folder. [VERIFIED]

### 12.3 What needs testing (Phase 2 plan)
- [ ] `cargo build --release -p filmcraft -p filmcraft-cli` on Linux x86_64, macOS arm64 and Windows x64, recording time and peak RAM. Also `cargo xtask web`.
- [ ] `cargo xtask ci` on each OS with `FILMCRAFT_REQUIRE_ORACLES=1` and ffmpeg (libx264/libx265) installed, recording pass/fail/skip counts. Run with and without `CRAFT_FONTS_DIR`.
- [ ] `cargo deny check licenses advisories bans sources` and `cargo audit`. Generate an SBOM and a third-party licence bundle (`cargo about`).
- [ ] `cargo tree -i gtk --target x86_64-unknown-linux-gnu` (confirm GTK is not built on Linux) and `cargo tree -e features` for the default app.
- [ ] `filmcraft-cli commands --json | jq length` to verify the "650+ commands" claim.
- [ ] Real-world media round trips (iPhone HEVC/VFR, Android H.264, GoPro, Sony/Canon XAVC, DJI, screen recordings, ProRes from Resolve, MXF from Avid, damaged files): import, edit, export, then verify with ffprobe/ffmpeg `-xerror`.
- [ ] Cargo-fuzz harnesses for isobmff, matroska, mxf, mpegts, ogg, cfb/aaf, omf, h264, hevc, vp9, av1, aac, opus, captions and LUT parsing, run for at least 24 CPU-hours each.
- [ ] Playback performance (`cargo xtask bench-playback`) at 1080p and 4K on non-Mac hardware. Hardware decode and encode on Intel, AMD and NVIDIA (VA-API, NVDEC, NVENC, Media Foundation).
- [ ] Interchange validation: export FCP7 XML and FCPXML into Premiere, Resolve and FCP, and AAF/OMF into Avid and Pro Tools.
- [ ] Web build in Chrome, Edge, Firefox and Safari: the WebGPU/WebGL2 fallback, WebCodecs, OPFS, large files, and memory limits on a 13 MB wasm.
- [ ] Security review of `crates/platform` FFI and of control-channel and MCP path handling. Check the model-download confirmation gating and the candle `.pth` loader against malicious input.
- [ ] Packaging: install and uninstall each artefact on clean VMs. Verify signatures. Check that the Linux mime XML registers `.fcproj`.
- [ ] Code-provenance scan (ScanCode Toolkit and similarity against FFmpeg/x264/dav1d/libvpx/libopus sources).
