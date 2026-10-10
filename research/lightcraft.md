# Phase-1 Technical Due Diligence: storytold/lightcraft → ubunye-creative-light

Repo analysed: `upstream clone of lightcraft` (clone of https://github.com/storytold/lightcraft)
Method: static analysis only (git, grep, find, Python `tomllib` over `Cargo.lock` and the local cargo registry cache). No cargo build was run. `cargo tree --offline` failed because the registry index is incomplete offline, so dependency-graph claims come from parsing `Cargo.lock`.
Date of analysis: 2026-10-10.

Tag legend: **[VERIFIED: evidence]** means I saw it in the repo. **[ASSUMPTION]** means I inferred it and did not confirm it. **[NEEDS TESTING]** means it can only be confirmed by building, running or scanning with tools.

All repo content was treated as untrusted data. Instructions in `CLAUDE.md`/`AGENTS.md` were not followed; they are only summarised in §11.

---

## 1. Snapshot

| Item | Value |
|---|---|
| HEAD | `b70ae1741e8236757377ca85eecaafb8788a82ab`, 2026-10-10T05:14:58-04:00, "Merge pull request #692 from storytold/claude/integrate-batch-25" [VERIFIED: `git log -1`] |
| Default branch | `main` (`origin/HEAD -> origin/main`). There is also a `release` branch that triggers the release pipeline [VERIFIED: `git branch -a`, `.github/workflows/release.yml`] |
| Commit count | 1,552 on HEAD, of which 486 are merge commits [VERIFIED: `git rev-list --count HEAD`, `git log --merges`] |
| First commit | **2026-09-30**, so the whole project is **about 10 days old** [VERIFIED: `git log --reverse`] |
| Contributors | 75 distinct name+email pairs (73 distinct names). Top authors: Brandon Thomas 751, Nathaniel Ledford 305, "Claude" 139, radialmonster 74, Daniel Voshart 26 [VERIFIED: `git shortlog -sn`] |
| AI authorship | 958 commit messages carry a `Co-Authored-By: Claude` trailer and 54 mention Codex. Many branches are named `claude/integrate-batch-N`. The code is mostly agent-generated [VERIFIED: `git log --format=%B | grep -ci`] |
| Tags | v0.1.0 (10-02), v0.1.1 (10-03), v0.2.0 (10-05), v0.2.1 (10-06), v0.4.0 (10-08), v0.5.0 (10-09). There is no v0.3.x. Workspace version is `0.5.0` [VERIFIED: `git for-each-ref refs/tags`, `Cargo.toml`] |
| GitHub releases, stars, issues | [NEEDS TESTING]: the GitHub API was not reachable from this session (HTTP 403). The README links to releases and says builds are signed |
| Repo size | Working tree 28 MB, `.git` 37 MB (pack 35.7 MiB) [VERIFIED: `du`, `git count-objects`] |
| Code size | 505 `.rs` files, about 202k lines of Rust including tests [VERIFIED: `find … | wc -l`] |
| Remote branches | about 60 (`claude/*`, `fix/issue-*`, `feat/*`, `pr/license-standard`) |

**Activity cadence** [VERIFIED: commits per day from `git log --date=short`]:

| Day | 09-30 | 10-01 | 10-02 | 10-03 | 10-04 | 10-05 | 10-06 | 10-07 | 10-08 | 10-09 | 10-10 (partial) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Commits | 42 | 151 | 47 | 28 | 104 | 106 | 44 | 135 | 317 | 500 | 78 |

That is 1,297 commits in the last 7 days, about 1,300 per week. This is an extremely high-velocity, agent-driven codebase. Expect upstream churn that is hard to track or rebase onto.

---

## 2. Repository structure

Root [VERIFIED: `ls -la`]: `.cargo/config.toml` (alias `xtask = "run --package xtask --"`), `.github/workflows/` (4 files), `AGENTS.md`, `CLAUDE.md`, `Cargo.toml`/`Cargo.lock`, `LICENSE-APACHE`, `LICENSE-MIT`, `NOTICE`, `README.md` (38 KB), `ROADMAP.md` (29 KB), `apps/`, `assets/`, `clippy.toml`, `contributors/`, `crates/` (24 crates), `docs/`, `flake.nix`/`flake.lock`, `nix/package.nix`, `omastore.toml` (an app-store manifest pointing at the Linux tarballs and icon), `packaging/`, `rustfmt.toml`, `tools/`, `xtask/`.

There is **no `rust-toolchain.toml`**. The toolchain floor is `rust-version = "1.90"` with `edition = "2024"` and `resolver = "3"` [VERIFIED: `Cargo.toml`]. CI uses `dtolnay/rust-toolchain@… # stable`, which is unpinned stable.

Workspace members: `members = ["crates/*", "apps/*", "xtask"]`, a glob. The `[workspace.lints.rust] unsafe_code = "deny"` setting applies to all of them.

### Crates (each line is the crate's own `description` plus layer; LOC includes tests) [VERIFIED: `crates/*/Cargo.toml`]

| Crate (path) | Layer | Purpose | ~LOC |
|---|---|---|---|
| `lightcraft-geom` (crates/geom) | L0 | Geometry primitives (rects, points, orientation) | 1.4k |
| `lightcraft-color` (crates/color) | L0 | Colour maths: CCT, perceptual spaces (OkLCh), transfer functions, splines | 0.9k |
| `lightcraft-raster` (crates/raster) | L0 | Image buffers (Rgb32f, Rgba8), blur | 0.8k |
| `lightcraft-tiff` (crates/tiff) | L0 | **In-house** TIFF/IFD reader and writer, makernotes | 2.0k |
| `lightcraft-sysmem` (crates/sysmem) | L0 | Returns freed allocator memory to the OS. **The only crate allowed `unsafe`** (one FFI call) | 80 |
| `lightcraft-fetch` (crates/fetch) | L0 | Model downloads: mirrors, resume, pinned SHA-256, **hand-written HTTP/1.1 client** over rustls. Licence Apache-2.0 only | 1.4k |
| `lightcraft-raw` (crates/raw) | L1 | **In-house** RAW decoding (DNG, CR2, CR3/CRX, ARW, SR2, NEF, RAF, ORF, PEF, RW2, SRW), lossless JPEG, demosaic (bilinear/PPG/AHD/X-Trans), DNG colour model and DNG writer, opcodes | 22.3k |
| `lightcraft-codecs` (crates/codecs) | L1 | Standard codecs: sniffing, decode to linear light, ICC, encoders (JPEG/PNG/TIFF/WebP/AVIF/JXL/PSD composite), gain maps | 7.3k |
| `lightcraft-meta` (crates/meta) | L1 | **In-house** EXIF, XMP (and XMP merge), IPTC, CR3 container metadata, GPX | 4.3k |
| `lightcraft-develop` (crates/develop) | L1 | Non-destructive edit model, control specs, presets | 2.5k |
| `lightcraft-scenes` (crates/scenes) | L1 | Procedurally generated HDR demo photographs (demos and tests) | 1.0k |
| `lightcraft-heif` (crates/heif) | L1 (opt.) | Optional HEIF/HEIC decoder wrapping `heic-rs` 0.1.1, with a panic guard | 2.1k |
| `lightcraft-denoise-core` (crates/denoise-core) | L1 | Denoise model manifests, Bayer tiling, cache formats (no inference) | 2.2k |
| `lightcraft-denoise` (crates/denoise) | L1 | AI denoise: own ONNX protobuf reader, CPU inference (`faer-core`) | 2.7k |
| `lightcraft-faces` (crates/faces) | L1 | Bounded ONNX CPU inference for face detection and recognition (`ndarray`, `matrixmultiply`) | 5.3k |
| `lightcraft-pipeline` (crates/pipeline) | L2 | Scene-referred float develop pipeline, the CPU reference | 9.8k |
| `lightcraft-gpu` (crates/gpu) | L3 | wgpu compute (WGSL) twin of the CPU pipeline | 5.1k |
| `lightcraft-catalog` (crates/catalog) | L3 | Library model: photos, albums, ratings, search/filter, smart-album rules, **op-log persistence**, library lock | 11.0k |
| `lightcraft-preview` (crates/preview) | L3 | Preview cache: content hashing, memory LRU, disk thumbnails, prioritised job pool | 1.3k |
| `lightcraft-merge` (crates/merge) | L3 | HDR merge (align, deghost) and panorama stitching | 3.8k |
| `lightcraft-segment` (crates/segment) | L3 | SAM 3 segmentation inference on `candle` (Apache-2.0 only, a port of HF Transformers). Includes bin `lightcraft-sam3-worker` | 2.3k |
| `lightcraft-engine` (crates/engine) | L4 | Session, command registry, history/undo, render jobs, view-models, import/export, Lightroom catalog import (own read-only SQLite page reader), presets, fonts | 53.2k |
| `lightcraft-ui-egui` (crates/ui-egui) | L5 | egui Lightroom-style frontend ("swappable"), control-channel methods, i18n (9 locales + Ukrainian) | 46.9k |
| `lightcraft-mcp` (crates/mcp) | L5 | MCP server: JSON-RPC 2.0 over stdio, headless session or connect to a running app | 2.3k |

### Apps

| App | Purpose |
|---|---|
| `apps/lightcraft` | Desktop binary (eframe + wgpu, `muda` native menu on macOS, `rfd` dialogs, control server, logging). `build.rs` embeds the Windows icon and VERSIONINFO |
| `apps/lightcraft-cli` | `lightcraft-cli`: `run`, `mcp`, `render`, `snapshot`, `merge`, `synth-merge`, `commands`, `controls`, `calibrate` |
| `apps/lightcraft-web` | The egui UI compiled to wasm32 (WebGL2 via eframe `glow`), OPFS/IndexedDB backend, Web Worker rendering |

### Tooling and other directories

- **`xtask/`** (2.7k LOC) has these commands: `assets`, `bench`, `ico`, `layers`, `parity [--write]`, `wasm`, `web [--serve] [--dev]`, `ci`, `corpus [--download]`, `stats`, `version` [VERIFIED: `xtask/src/main.rs` USAGE]. `ci` runs fmt-check, clippy `-D warnings`, the heif build, tests, parity refs, layers, assets and the wasm check.
- **`tools/`**: Python and shell helpers: `install-sam3.{sh,ps1}`, `sam3_reference.py`, `spectral_camera_table.py` (generates `crates/raw/src/spectral_table.rs` from rawtoaces-data), and pip requirement files.
- **`packaging/`**: `env.sh` (single version source), `icons.sh`, `linux/` (nfpm deb/rpm, AppImage, Flatpak manifests, desktop/metainfo/mime), `macos/` (Info.plist template, entitlements, DMG background, notarize script), `windows/` (WiX 5 `lightcraft.wxs`, `package.ps1`, `sign.ps1`), `freebsd/`, `web/` (`_headers`, `.htaccess`, hosting README).
- **`docs/`**: 33 Markdown files (about 4.7k lines), including `parity.md` (839 lines, a row-by-row Lightroom parity tracker), `mcp.md`, `control-protocol.md`, `web.md`, `gpu-pipeline.md`, `xmp-interop.md`, `releasing.md`, localisation docs, plus `brand/` (ArtCraft trademarks), `images/` and `showcase/` (scripts that produce the README screenshots through the control channel).
- **`nix/`**: `flake.nix` (systems x86_64-linux, aarch64-linux, aarch64-darwin; inputs are nixpkgs-unstable and craft-fonts pinned by commit) and `nix/package.nix` (builds both binaries and runs `cargo test --workspace` as its check). AGENTS.md calls it "community-maintained and not in CI".
- **`contributors/`**: `contributors.json` (generated) and `people.toml`, compiled into the About window by `crates/ui-egui/build.rs`.
- **`plan/`** is gitignored (local-only). It is referenced heavily by AGENTS.md (`plan/STATUS.md`, `plan/architecture.md`, `plan/lightroom/` screenshots) and **is not in the repo**. Architecture docs that agents rely on are therefore missing for us [VERIFIED: AGENTS.md "`plan/` is gitignored"].

---

## 3. Architecture

- **Languages**: Rust only in the product. Python is used in `tools/` and `docs/showcase/run.py`, plus shell, PowerShell and Nix for packaging. WGSL holds the GPU kernels: 9 `.wgsl` files, about 1.5k lines [VERIFIED].
- **UI**: **egui/eframe 0.36**. Native rendering uses eframe's `wgpu` feature with `accesskit`, `wayland` and `x11`; the web build uses eframe's `glow` (WebGL2) backend [VERIFIED: `apps/lightcraft/Cargo.toml`, `docs/web.md` line 3]. `egui_kittest` is a workspace dependency (one file uses it). There is a CPU-rasterised headless UI snapshot mode (`lightcraft-cli snapshot`). Native macOS menu bar uses `muda` 0.21.
- **GPU pipeline**: `wgpu` 30 with features `metal`, `vulkan`, `dx12` and `wgsl`, and **no GL on native**. Compute kernels mirror the CPU pipeline, and `crates/gpu/tests/equivalence.rs` checks mean |Δ| < 0.5 LSB and max |Δ| ≤ 3 LSB. GPU tests **skip when no adapter exists, so they skip on CI** [VERIFIED: file header]. CPU fallback is used on device limits or errors. Environment knobs: `LIGHTCRAFT_GPU`, `LIGHTCRAFT_GPU_BACKEND`, `LIGHTCRAFT_GPU_LIMITS`, `LIGHTCRAFT_GPU_ADAPTER`.
- **RAW decoding**: **entirely in-house**. `crates/raw/src/vendor/` has `arw.rs`, `arw_lens.rs`, `sr2.rs`, `cr2.rs` (+`cr2/`), `cr3.rs`, `crx.rs`, `crx_wavelet.rs`, `nef.rs`, `nefc.rs`, `orf.rs`, `pef.rs`, `raf.rs`, `rafc.rs`, `rw2.rs` and `srw.rs`, alongside `dng.rs`, `dngwrite.rs`, `ljpeg.rs`, `opcodes.rs`, `gaintable.rs`, `demosaic/{bilinear,ppg,ahd,xtrans}.rs` and a JXL path for JXL-compressed DNG via `jxl-oxide`. The project explicitly forbids rawloader, rawler, LibRaw and similar (clean-room policy) [VERIFIED: tree, AGENTS.md]. The CR3 docs cite lclevy/canon_cr3 (a public format description).
- **Colour management**: Linear Rec.2020 float working space, Bradford adaptation, gamut mapping, filmic shoulder and OkLCh-based colour tools (README) [VERIFIED code location: `crates/color/src/{cct,perceptual,transfer,spline}.rs`]. ICC handling uses **`moxcms` 0.9** (pure-Rust CMS) in `crates/codecs/src/icc.rs`. Camera colour comes from DNG matrices, 4 bundled fitted JSON profiles (`assets/camera-profiles/`), a 52-model spectral matrix table (`crates/raw/src/spectral_table.rs`, derived from rawtoaces-data), "camera-JPEG estimates", or a neutral fallback. Display profiles are covered in `docs/display-profiles.md`.
- **Catalog/database**: **No SQLite in the product.** The catalog is a custom crash-safe **append-only JSON-lines op log plus snapshot**: `catalog.log` lines are `{"seq","crc","op"}` (CRC-32), `catalog.snap` is `{"format":"lightcraft-catalog","version":V,"seq":N,"catalog":{…}}`, with atomic temp+fsync+rename, background compaction, torn-write recovery and `catalog.lock` [VERIFIED: `crates/catalog/src/journal.rs`, `store.rs`, `lock.rs`]. Library directory: `~/Pictures/LightCraft Library/` containing `catalog.snap`, `catalog.log`, `presets.json`, `view.json`, `prefs.json`, `thumbs/` and `Originals/` [VERIFIED: `crates/engine/src/library.rs`]. A **read-only clean-room SQLite page reader** exists only to import Lightroom Classic `.lrcat` catalogs (`crates/engine/src/lightroom_sqlite.rs`, with hard caps on pages, rows and B-tree depth).
- **File formats**: In: JPEG, PNG, TIFF, WebP, PSD composite, JXL, GIF/BMP sniffed, HEIC (feature `heif`) and the RAW list above. Out: JPEG, PNG, TIFF (incl. 32-bit float), WebP, AVIF (incl. PQ HDR), DNG, "original + XMP", ISO 21496-1 gain-map JPEG, contact-sheet PDF. Sidecars: XMP read/write, including reading Adobe `crs:` develop settings. Custom namespace `lc` = `http://ns.lightcraft.app/lc/1.0/` (`crates/meta/src/xmp.rs:18`). Presets: `.lcpreset`, XMP presets, `.lrtemplate`, DNG presets, Luminar looks (`.lmp`/`mplumpack`), `.cube`, zip bundles (`apps/lightcraft/src/main.rs:468`, `crates/engine/src/cmd/preset_files.rs`).
- **Project/catalog format**: see Catalog above. The macOS `Info.plist.in` also declares a document UTI `ai.storyteller.lightcraft.document` with extension **`pcraft`** and MIME `application/x-lightcraft`, and imports a `com.adobe.photoshop-large-image` (`.psb`) UTI. These look like **copy-paste leftovers from PhotoCraft** and do not match LightCraft's directory-based library [VERIFIED: `packaging/macos/Info.plist.in` lines 48-106].
- **Plugin system**: **None.** Extensibility is the command registry: every action is a command with an id and JSON params, reachable from UI, CLI, control channel and MCP. Models (SAM 3, denoise, faces) are opt-in downloadable weights, not plugins [VERIFIED: AGENTS.md "Everything is a command"; no plugin loader found].
- **Threading model**: The egui UI thread plus a background worker pool for loupe, thumbnail and before/after rendering (drafts during drags) in `crates/preview` ("prioritized job pool"). `rayon` is used for per-pixel parallelism (134 rayon call sites) and there are 81 `thread::spawn`/`Builder` sites. Library compaction runs on a background thread. The control server takes TCP connections on threads and answers on the UI thread between frames. Panic hooks and `catch_unwind` (35 sites) guard command dispatch, import/export and heic-rs. Web uses Web Workers running the same wasm module, without shared memory and without wasm threads [VERIFIED: grep counts, `docs/web.md`].
- **Layering is enforced** by `cargo xtask layers` (`xtask/src/layers.rs`): nothing below L5 may depend on egui, eframe, winit or rfd.

### Crate dependency diagram (internal; from each `Cargo.toml`) [VERIFIED]

```
L0   geom   color(geom)   raster(color,geom)   tiff   sysmem   fetch
       \       |             |                  |               |
L1   develop(color,geom)  meta(geom,tiff)  raw(color,geom,meta,raster,tiff)
     codecs(color,geom,heif,raster)  heif  scenes(color,raster)
     denoise-core(raster)  denoise(denoise-core,raster)  faces
       \         |            |             |
L2   pipeline(color,develop,geom,raster,raw,scenes)
       |
L3   gpu(color,[denoise],develop,geom,pipeline,raster,raw,scenes)
     catalog(develop,geom,meta)   preview(raster)
     merge(codecs,color,geom,meta,raster,raw,scenes)   segment(fetch)
       \          |          |         |
L4   engine(catalog,codecs,color,denoise*,denoise-core,develop,faces,fetch,geom,gpu,
            heif,merge,meta,pipeline,preview,raster,raw,scenes,segment*,tiff)
       |                                   \
L5   ui-egui(catalog,codecs,color,denoise,develop,engine,faces,geom,merge,meta,
             pipeline,preview,raster)            mcp(codecs,engine,raster)
       |                     \                       |
apps lightcraft(codecs,engine,raster,sysmem,ui-egui)   lightcraft-cli(codecs,engine,mcp,merge,raster,sysmem,ui-egui)
     lightcraft-web(codecs,engine,preview,raster,ui-egui)
     (* = optional feature: denoise, sam)
```

Note: `lightcraft-cli` depends on `ui-egui` because of headless snapshots and connect-mode UI methods, so the CLI is not UI-free.

---

## 4. Key dependencies

- **Total packages in `Cargo.lock`: 656** (version 4 lockfile). **No git sources** (`grep '^source = "git'` finds nothing). **No `[patch]` or `[replace]`** [VERIFIED].
- Top-level third-party crates from `[workspace.dependencies]` [VERIFIED: `Cargo.toml`]: `egui`/`eframe`/`egui_extras`/`egui_kittest` 0.36, `wgpu` 30, `pollster` 1, `rfd` 0.17, `muda` 0.21, `web-time` 1, `candle-core`/`candle-nn` **=0.9.2** (pinned so newer `tokenizers` does not pull in Oniguruma C), `image` 0.25 (png/jpeg/tiff/webp), `zune-jpeg`/`zune-core` 0.5, `jpeg-decoder` 0.3, `jpeg-encoder` 0.7, `png` 0.18, `tiff` 0.11, `image-webp` 0.2, `moxcms` 0.9, `jxl-oxide` 0.12, `zune-jpegxl` 0.5 (test fixtures), `ravif` 0.13 (no asm), `rav1e` 0.8.1, `avif-serialize` 0.8.6, `ab_glyph` 0.2, `quick-xml` 0.38, `miniz_oxide` 0.8, `serde`/`serde_json` 1, `rayon` 1, `sha2` 0.10, `crc32fast`, `siphasher`, `bytemuck`, `thiserror` 2, `log`, `proptest` 1, `dhat` 0.3 (opt-in), `libc` (bench only), `wasm-bindgen` 0.2.129, `js-sys`/`web-sys` 0.3.
- Crate-local dependencies: `heic-rs` **=0.1.1** (heif), `rustls` 0.23 + **`rustls-rustcrypto` 0.0.2-alpha** + `webpki-roots` 1 (fetch), `ndarray` =0.17.2 and `matrixmultiply` =0.3.11 (faces), `faer-core` =0.17.1 (denoise, feature `runtime`), `harfrust` 0.12 and `unicode-segmentation` (engine, text shaping for watermarks), `winresource` 0.1 (desktop build-dep).
- **Native / C surface** [VERIFIED from lockfile; actual compilation NEEDS TESTING]:
  - `-sys` crates in the lock: `core-foundation-sys`, `objc-sys`, `windows-sys`, `linux-raw-sys`, `wayland-sys`, `renderdoc-sys`, `ndk-sys`, `jni-sys`, `js-sys`, `web-sys`, `libfuzzer-sys`. These are mostly OS-binding crates with no C code of their own. `wayland-sys` uses dlopen, so it needs libwayland at runtime.
  - **`ring` 0.17.14 (C and assembly, built with `cc`) appears in `Cargo.lock`** as a dependency of `rustls-webpki`/`rustls-rustcrypto`. That contradicts the "no C" claim in `crates/fetch/src/http.rs` ("no ring or aws-lc-rs") **unless the edge is feature-gated and never built**. [NEEDS TESTING: `cargo tree -e features -i ring`.]
  - `cc` 1.5.1 is reachable via `ring`, `libfuzzer-sys` (from `rav1e`, probably its non-default `fuzzing` feature, [ASSUMPTION] not built), `wayland-backend` and `android-activity` (Android only).
  - `glow` (OpenGL) is present for the web build and possibly eframe on native [ASSUMPTION: native uses wgpu only].
  - Linux runtime and system packages: CI installs `libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev` (`release.yml`). FreeBSD additionally installs `gtk3`, `vulkan-loader`, `fontconfig` and others.
  - `tokio` 1.53.1 is in the lock via `wasm-bindgen-futures`, `rfd`, `wgpu` and `winit` edges (not used directly by the product) [VERIFIED reverse deps from lock].
- **Build scripts in the workspace** (3): `crates/engine/build.rs` (embeds craft-fonts from `CRAFT_FONTS_DIR`; local directory only, no network), `crates/ui-egui/build.rs` (compiles locale JSON into a `tr_format!` macro and bakes contributor credits), `apps/lightcraft/build.rs` (Windows resources via `winresource`; **panics** if `LIGHTCRAFT_REQUIRE_WINRES` is set and rc fails) [VERIFIED].
- **Vendored third-party code**: none in the usual sense. `crates/raw/src/vendor/` is the project's own per-vendor decoders, not vendored libraries. `crates/segment` is a **port** of HF Transformers SAM 3 (Apache-2.0) [VERIFIED: NOTICE].

---

## 5. Features implemented and limitations

Self-reported, from the README "Feature status" and `ROADMAP.md` "Where we stand", dated 2026-10-08:

- Parity checklist **79.7%** (P0 98.2%, P1 95.6%, P2 41.6%). The authors' own "honest" estimate for replacing Lightroom is **60–70%**. They state that ✅ rows are set by whoever lands a feature and nobody has systematically checked them against Lightroom [VERIFIED: ROADMAP.md lines 7-20].
- Implemented, per the docs: library (albums, folders, smart albums, stacks, virtual copies, ratings, flags, labels, field search), Compare/Survey culling, all Basic/Tone/Color/Effects/Detail panels, tone curves, colour grading, B&W mix, masking (brush, linear, radial, luminance/colour range, boolean ops, "Subject/Sky" as **classical heuristics**), crop/straighten/Upright, heal/clone/red-eye, presets and profiles (own looks), history/versions/sync, export (JPEG/PNG/TIFF/WebP/AVIF/DNG, watermarks), HDR editing and gain-map/PQ export, Photo Merge HDR/panorama, XMP interop, Lightroom catalog import, GPX geotagging, contact-sheet PDF, faces MVP, optional SAM 3 masks and Bayer AI denoise with **user-supplied weights**, 9+ UI languages, web build.
- Limitations the project itself lists: no measured camera calibration database (most non-DNG raws use estimated or neutral matrices); no lens-profile database; RAW coverage about 55% (other CR3/CRX variants, compressed ORF, compressed SRW, Nikon HE NEF, Canon sRAW not decoded; preview fallback only); about 55 corpus files against more than 1,000 camera models; no on-screen HDR display; no video; no Map, Book, Print or Web modules; AI and computational features at about 15–20%, "blocked on a model strategy"; GPU path proven only on Apple; Windows installer UI unverified on Windows hardware; web build experimental (see §6).
- **Internal inconsistency**: the README feature table marks "CR3 … decode ⬜" while the same row and the ROADMAP say CR3 lossless and C-RAW decode exists (`crx.rs`, 1,096 LOC). Treat the docs as drifting quickly [VERIFIED: README line 323 vs ROADMAP RAW row].

**Source cross-check (sample)** [VERIFIED: each command id string found in an engine command module]:

| Claimed feature | Evidence |
|---|---|
| Contact sheet PDF | `"export.contactSheet"` in `crates/engine/src/cmd/export.rs`, tested in `apps/lightcraft-cli/tests/cli.rs`; `crates/engine/src/contact_sheet.rs` |
| Masks | `"mask.add"` in `crates/engine/src/cmd/masks.rs`, SAM in `crates/engine/src/segment/` |
| HDR/Pano merge | `"merge.hdr"`, `"merge.panorama"` in `crates/engine/src/cmd/merge.rs`; `crates/merge/tests/{hdr,pano}.rs` |
| Library flagging | `"photo.flag"`, `"library.selectAll"` in `crates/engine/src/cmd/library.rs` |
| Lightroom catalog import | `crates/engine/src/lightroom_{catalog,sqlite,archive,job}.rs`, `docs/lightroom-catalog-import.md` |
| CR3, compressed RAF | `crates/raw/src/vendor/{cr3,crx,crx_wavelet,rafc}.rs`, `crates/raw/tests/cr3_corpus.rs` (corpus-gated) |
| Fidelity versus Lightroom | Nothing found; the ROADMAP confirms it is not measured [NEEDS TESTING] |

---

## 6. Build

- **Toolchain**: no `rust-toolchain.toml`. Requires Rust ≥ 1.90 (edition 2024). CI uses unpinned **stable**; the local machine has cargo 1.97 [VERIFIED].
- **Desktop (all OSes)**: `cargo run --release -p lightcraft` and `cargo build --release -p lightcraft-cli`. Optional `CRAFT_FONTS_DIR=../craft-fonts` (a clone of `storytold/craft-fonts`, needed for CJK glyphs; `CRAFT_FONTS_REQUIRED=1` makes it mandatory, as in releases). Feature flags: `denoise` (default), `heif`, `rawnind-model` (GPL weights offer), `dhat-heap` (CLI) [VERIFIED: README Quick start, app `Cargo.toml`].
  - **macOS**: `packaging/macos/package.sh --arch universal` builds aarch64 and x86_64, lipo, signs with hardened runtime (empty entitlements, not sandboxed), notarizes and builds a DMG. Minimum macOS 11.0. The `sysmem` FFI call (`malloc_zone_pressure_relief`) is macOS-only.
  - **Windows**: `packaging/windows/package.ps1 -Arch x64|x86|arm64` with MSVC targets and WiX 5.0.2 (`dotnet tool install --global wix`). Signing (`sign.ps1`) uses either a PFX (`WINDOWS_CERTIFICATE`) or **Azure Trusted Signing**. ARM64 is cross-compiled on x64.
  - **Linux**: `packaging/linux/package.sh` builds the AppImage (with embedded zsync update info), `.deb`/`.rpm` via **nfpm 2.47.0** and a tarball, on Ubuntu 22.04 (glibc ≥ 2.35). Flatpak via `flatpak-builder` (`packaging/linux/flatpak/ai.storyteller.lightcraft.yml`). Build deps: libxkbcommon, wayland, X11, randr, xi, GL dev packages.
  - **FreeBSD**: `packaging/freebsd/package.sh` in a VM.
- **WebAssembly** [VERIFIED: `docs/web.md`, `xtask/src/web.rs`, `packaging/web/`]:
  - Setup: `rustup target add wasm32-unknown-unknown` and `cargo install wasm-bindgen-cli --version <lock version>`. Then `cargo xtask web [--serve [port]] [--dev]`, which writes to `<target>/web/` (`index.html`, `worker.js`, `lightcraft_web.js`, `lightcraft_web_bg.wasm`). `wasm-opt` runs if it is on PATH. A custom `[profile.web]` uses fat LTO, opt-level "s" with per-pixel crates at opt-level 3, and `panic=abort`.
  - Output is a static site: about **13.5 MB of wasm, 3.1 MB brotli**. It needs an HTTP(S) origin (OPFS, module workers). Optional COOP/COEP/CORP headers; `_headers`/`.htaccess` samples target Netlify, Cloudflare Pages and Apache; **Cloudflare's 25 MiB per-file limit** drives font splitting. No server-side code. Release artifact: `lightcraft-web-<ver>.zip`.
  - Browser limitations (`docs/web.md` "Not yet"): WebGL2 only (WebGPU 🚧); exports and auto-adjust on the main thread; durability is "a frame later", not fsync; a decoder panic kills the page (`panic=abort`) (#107); no multi-tab lock (last snapshot wins); removed photos' originals are not deleted from storage; backups over 4 GB unsupported (no zip64); **no control channel or MCP** in the browser; AVIF export untested; Safari and Firefox testing 🚧; the browser may evict storage.
- **xtask**: see §2.
- **Nix**: `nix run|build|develop github:storytold/lightcraft`, plus an overlay for NixOS/home-manager. Not in CI.
- **Packaging identifiers** [VERIFIED]:
  - Bundle ID / app ID / Flatpak ID / desktop file / icon name: **`ai.storyteller.lightcraft`** (`apps/lightcraft/src/main.rs:38` `APP_ID`, `Info.plist.in`, `packaging/linux/ai.storyteller.lightcraft.*`, `assets/app-icon/hicolor/*/apps/ai.storyteller.lightcraft.png`).
  - UTIs: `ai.storyteller.lightcraft.document` (`.pcraft`!), `ai.storyteller.lightcraft.qoi`.
  - Windows MSI: `Manufacturer="Learning Machines LLC"`, `UpgradeCode="229BB552-5EFD-479A-BA86-8CB6D926390C"` (**must be regenerated for the fork**), registry key `HKLM\Software\LightCraft`, ARP URLs to getartcraft.com and discord.gg/artcraft (`packaging/windows/lightcraft.wxs`).
  - Linux vendor: `Learning Machines LLC` (`nfpm.yaml`, `metainfo.xml.in`).
  - AppImage update channel: `gh-releases-zsync|storytold|lightcraft|latest|…`.
  - Signing secrets: `APPLE_CERTIFICATE(_PASSWORD)`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, `WINDOWS_CERTIFICATE(_PASSWORD)`, `AZURE_{TENANT_ID,CLIENT_ID,CLIENT_SECRET,SIGNING_ENDPOINT,SIGNING_ACCOUNT,CERT_PROFILE}`. All are optional; unsigned builds produce warnings instead of failures.
- Actually building on each OS: **[NEEDS TESTING]**. No build was run here.

---

## 7. Tests

- **`#[test]` count: 2,576** (with `grep -a`; two files contain literal NUL bytes in byte-string test fixtures, `crates/raw/src/vendor/{cr3,pef}.rs`) [VERIFIED].
  - Per crate: engine 658, ui-egui 594, raw 312, catalog 190, pipeline 143, codecs 88, faces 75, meta 60, apps/lightcraft 47, gpu 40, xtask 38, denoise-core 37, mcp 31, geom 31, develop 26, heif 24, denoise 24, color 21, lightcraft-web 21, merge 20, lightcraft-cli 20, tiff 19, fetch 17, preview 13, segment 10, raster 10, scenes 4, sysmem 3.
- Most tests are inline unit tests (`tests_*.rs` modules in `src/`). There are 28 integration-test files in 12 `tests/` dirs: `codecs/tests/{fuzz,corpus,header,heif,roundtrip,bench}.rs`, `raw/tests/{robust,vendor_robust,dng,corpus,cr3_corpus}.rs` (+ `robust.proptest-regressions`), `meta/tests/robust.rs`, `tiff/tests/roundtrip.rs`, `gpu/tests/{equivalence,fallback,memory,shutdown*}.rs`, `merge/tests/{hdr,pano}.rs`, `mcp/tests/e2e.rs`, `segment/tests/{reference,remote}.rs`, `denoise/tests/real_model.rs`, `engine/tests/denoise_*.rs`, `ui-egui/tests/shutdown.rs`, `apps/lightcraft-cli/tests/cli.rs`.
- **Fuzzing**: **no cargo-fuzz targets.** "Fuzz" here means **proptest-based malformed-input tests** (17 files use `proptest!`), for example `crates/codecs/tests/fuzz.rs` (random bytes behind every magic number, mutated and truncated files, using `decode_unguarded` so panics are not masked), `crates/raw/tests/robust.rs` and `vendor_robust.rs`, and `crates/meta/tests/robust.rs`. `libfuzzer-sys` in the lock comes from `rav1e`, not the project [VERIFIED].
- **Golden-image tests**: none in the classic sense. The closest are the CPU↔GPU equivalence bounds (skipped without a GPU), CR3 "pixel-exact corpus" checks and corpus tests. Corpus tests need `cargo xtask corpus --download` (CC0 raw.pixls.us and PngSuite into a gitignored `corpus/`), `LIGHTCRAFT_CORPUS` or `LIGHTCRAFT_CR3_CORPUS`, so they probably skip in CI [ASSUMPTION]. UI tests run headless egui (`egui_kittest`/headless renderer). The ROADMAP notes "headless UI tests time out under machine load".
- **E2E**: `crates/mcp/tests/e2e.rs` (MCP JSON-RPC), `apps/lightcraft-cli/tests/cli.rs`, and the Windows ARM64 MSI install/run/uninstall job.
- **CI workflows (`.github/workflows/`)** [VERIFIED]:

| Workflow | Trigger | Runner/OS | What it does |
|---|---|---|---|
| `freebsd.yml` | push to main; PRs **only when Cargo manifests/lock change**; manual | ubuntu-latest → FreeBSD 14.3 VM (`vmactions/freebsd-vm`) | `cargo build --workspace --all-targets --locked` and `cargo test --workspace --locked` with craft-fonts. **This is the only workflow that runs the test suite** |
| `packaging-lint.yml` | push main/release, PRs, manual | ubuntu-latest | actionlint, shellcheck, PowerShell parse, XML/desktop/AppStream/Flatpak validation, FreeBSD tarball layout with stub binaries |
| `windows-arm64.yml` | push main, PRs, weekly cron, manual | windows-latest (cross-build, unsigned MSI) → windows-11-arm | Install the MSI, check it, run it, uninstall |
| `release.yml` | push to `release`, manual (version override) | macos-15-xlarge, windows-latest (x64/x86/arm64), ubuntu-22.04 (+arm), Flatpak, FreeBSD VM, web | Build, sign and package all artifacts, compute SHA256SUMS, create or update a **draft** GitHub release. Runs **no tests**; Linux checks `ldd` and `--version` only |

- **There is no CI running `cargo xtask ci` (fmt/clippy/tests/layers/assets/wasm) on Linux, macOS or Windows.** Quality gates rely on agents running `cargo xtask ci` locally before each commit (AGENTS.md). On PRs that don't touch manifests, nothing tests the code in CI. This is a major gap for a fork [VERIFIED: workflow triggers].
- Whether tests pass: **[NEEDS TESTING]**.

---

## 8. Security

- **Media-parsing attack surface**: almost all parsers are **in-house and agent-written**: TIFF/IFD (`crates/tiff`), every RAW decoder (`crates/raw`, 22k LOC), EXIF/XMP/IPTC/makernotes (`crates/meta`; XMP parsed with `quick-xml`), PSD composite (`codecs/src/psd.rs`), the SQLite page reader (`engine/src/lightroom_sqlite.rs`), the ONNX protobuf reader (`denoise/src/onnx_proto.rs`), zip handling for preset bundles and web backups, and `.cube` LUT import. Third-party crate parsers: `zune-jpeg`/`jpeg-decoder` (JPEG), `png`, `tiff`, `image-webp`, `jxl-oxide`, `heic-rs` 0.1.1 (young; the heif README says PhotoCraft's fuzzing found two panics, now wrapped in `catch_unwind`), `moxcms` (ICC).
- Mitigations [VERIFIED: AGENTS.md, `clippy.toml`, crate roots]: a "never crash" policy (`#![deny(clippy::unwrap_used, expect_used, panic, …)]` on every production crate root); `get()` instead of indexing on input-derived offsets; caps on allocations and pixel counts (`DecodeOptions{max_pixels}`); bounded recursion; and `catch_unwind` at dispatch (35 sites). Memory safety comes from Rust. Remaining risks are DoS (huge allocations, slow paths), logic bugs and panics on the web (`panic=abort` kills the tab, #107).
- **`unsafe`**: `unsafe_code = "deny"` workspace-wide and `#![forbid(unsafe_code)]` in 27 files. Exactly **1 production unsafe block**: `crates/sysmem/src/lib.rs:42` (`malloc_zone_pressure_relief`, macOS). The rest are 4 bench/example files (`ui-egui/examples/{grid_bench,thumb_request_bench}.rs` counting allocators; `engine/examples/render_bench.rs` and `raster/examples/blur_bench.rs` calling `clock_gettime`) [VERIFIED]. Dependencies (wgpu, eframe, candle, winit) contain plenty of unsafe, which is normal.
- **Fuzzing**: proptest robustness only; **no coverage-guided fuzzing (cargo-fuzz/AFL) and no OSS-Fuzz** [VERIFIED]. Recommend adding cargo-fuzz targets for tiff, raw vendors, meta/xmp, psd, lightroom_sqlite and onnx_proto.
- **Network access in the app**:
  - `crates/fetch` is a **hand-written HTTP/1.1 client** (550 LOC) on `std::net` with `rustls` and **`rustls-rustcrypto 0.0.2-alpha`**, an alpha, unaudited crypto provider. It is used only for opt-in model downloads. Hosts include `github.com/opencv/opencv_zoo` (YuNet/SFace pinned by commit), `huggingface.co/fal/AuraFace-v1` (pinned revision), `github.com/darktable-org/darktable-ai` (RawNIND, GPL-3.0, feature `rawnind-model`) and SAM 3 mirrors (`LIGHTCRAFT_SAM3_MIRRORS`). SHA-256 is pinned "where pinned", downloads go to `.part` files and are renamed after verification. No proxy support [VERIFIED: `crates/fetch/src/{lib,http}.rs`, `crates/faces/src/known.rs`, `crates/denoise-core/src/known.rs`].
  - Help menu links and OpenStreetMap URLs are opened in the system browser.
  - **No telemetry, analytics, crash upload or in-app update check** was found (grep for telemetry/sentry/analytics/update_check returned nothing) [VERIFIED]. The README claims "no telemetry".
- **Control channel** (`apps/lightcraft/src/control_server.rs`): `--control PORT` or `LIGHTCRAFT_CONTROL_PORT` binds **`127.0.0.1` TCP with no authentication**. It is off by default. It rejects non-JSON lines (to stop HTTP/CSRF smuggling from browsers), limits lines to 4 MiB and allows 16 connections. Risk: **any local process or local user** on a multi-user host can drive the app, including `app.export` and `ui.screenshot {path}` (arbitrary file writes as the user), imports, deletes and clipboard access. Consider a token or Unix-socket transport in the fork.
- **MCP server** (`crates/mcp`): stdio JSON-RPC (2025-06-18), hand-written, no async runtime. The attack surface is whatever the MCP client (an LLM) is induced to call: `import` (move mode deletes source files after verification), `export`/`render_photo` with arbitrary `path`, `run_command` and one `cmd_*` tool per registry command (free-form params, conservatively marked as edits), and in connect mode clicks, keys, clipboard and screenshots. No auth is needed because stdio is local. Prompt-injection risk comes from metadata (XMP titles/keywords returned to the LLM via `query_photos`/`doc_inspect`) [ASSUMPTION based on tool list in `docs/mcp.md`].
- **SAM 3 remote worker** (`crates/segment/src/bin/lightcraft-sam3-worker.rs`): serves Metal inference on `127.0.0.1:8793` for use over an SSH tunnel; it has a `LIGHTCRAFT_SAM3_TOKEN` env var [VERIFIED name; token semantics NEEDS TESTING].
- **Update mechanism**: none in-app. AppImage supports external AppImageUpdate via zsync against GitHub releases (`storytold/lightcraft` hard-coded). Windows ARP "update info" URL points to getartcraft.com.
- **Supply chain**:
  - `Cargo.lock` is committed; CI uses `--locked` in FreeBSD only. No git deps, no patches. Several exact pins (`candle =0.9.2`, `heic-rs =0.1.1`, `ndarray`, `faer-core`).
  - **No `deny.toml`, no `cargo-audit` config, no `cargo-deny`/`audit` CI job** [VERIFIED].
  - GitHub Actions are **pinned by commit SHA** (checkout, upload/download-artifact, rust-cache, freebsd-vm, rust-toolchain), which is good. However, `dtolnay/rust-toolchain` installs whatever `stable` is; FreeBSD uses `curl https://sh.rustup.rs | sh` (unpinned); Linux release downloads nfpm `.deb` with **no checksum**; Windows installs WiX with `dotnet tool` at a pinned version but no hash. Workflow permissions are `contents: read`, except the release job (`contents: write`). Signing jobs use a `release` environment restricted to the `release` branch.
  - Build scripts read only local files (no network) [VERIFIED].
  - `rustls-rustcrypto` alpha and the possible `ring` C build need review [NEEDS TESTING].
  - A codebase this new and this agent-generated (about 60% of commits co-authored by Claude) has had essentially **no independent human security review** [ASSUMPTION].

---

## 9. Licensing

- **Licence**: `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`, workspace `license`) [VERIFIED]. **Exceptions**: `crates/segment` and `crates/fetch` declare `license = "Apache-2.0"` only. NOTICE explains `segment` (a port of HF Transformers SAM 3). Why `fetch` is Apache-only is not documented [VERIFIED: `Cargo.toml`s].
- **NOTICE (quoted in full)** [VERIFIED: `NOTICE`]:

> LightCraft
> Copyright (c) 2026 ArtCraft Team and the LightCraft contributors.
> Licensed under the MIT License or the Apache License, Version 2.0, at your option
> (LICENSE-MIT, LICENSE-APACHE).
>
> The ArtCraft name, wordmark and logos in docs/brand/ are trademarks of the ArtCraft Team. They are
> not open source and are not covered by the license above; their terms are in
> docs/brand/LICENSE-brand.txt.
>
> Third-party material
>
> This product bundles third-party material under its own licenses, listed below. The per-file
> list (author, source, license) is in assets/ATTRIBUTION.md.
>
> Inter (assets/fonts/Inter-*.ttf) — SIL Open Font License 1.1. See assets/fonts/OFL-Inter.txt.
> Showcase photographs by Ansel Adams, Dorothea Lange and NASA (docs/images/) — public domain. See assets/ATTRIBUTION.md.
>
> crates/raw/src/spectral_table.rs (lightcraft-raw) holds camera colour matrices derived from the
> camera spectral sensitivities, training reflectances and model alias list of rawtoaces-data
> (https://github.com/AcademySoftwareFoundation/rawtoaces-data, v1.1.0), an Academy Software
> Foundation project licensed under the Apache License 2.0, and from the CIE 1931 colour-matching
> functions it ships (CIE, CC-BY-4.0). The matrices were fitted by the LightCraft contributors
> (2026) with tools/spectral_camera_table.py. See assets/ATTRIBUTION.md.
>
> crates/segment (lightcraft-segment) ports the SAM 3 model code of Hugging Face Transformers
> (https://github.com/huggingface/transformers, src/transformers/models/sam3 and sam3_tracker),
> Copyright The HuggingFace Team and Meta Platforms, Inc., licensed under the Apache License 2.0.
> The port was modified by the LightCraft contributors (2026): translated from Python/PyTorch to
> Rust on candle and restructured; each ported file says so. Because of this, crates/segment is
> licensed under the Apache License 2.0 only (LICENSE-APACHE), not under the MIT option.
>
> The SAM 3 weights are not part of LightCraft and are never distributed with it. A user downloads
> the facebook/sam3 checkpoint only when they choose to (in the app, or by hand), under Meta's SAM
> License (https://github.com/facebookresearch/sam3/blob/main/LICENSE), a separate, non-OSI
> licence. See docs/ai-masks.md.
>
> AI denoise weights are not part of LightCraft and are never distributed with it.
> The default build offers user-supplied models only. The optional rawnind-model
> feature offers a separately downloaded RawNIND UtNet2 Bayer checkpoint under
> GPL-3.0 (https://www.gnu.org/licenses/gpl-3.0.html), after the user accepts its terms.
> Commercial use is subject to those terms, including applicable source, licence and
> notice obligations on redistribution. Enabling that offer in releases remains a
> maintainer decision. The ONNX reader and CPU/GPU executor are our Rust implementation;
> no darktable, RawSpeed, LibRaw or other GPL decoder/runtime code is included.
>
> Builds made with craft-fonts (the optional CRAFT_FONTS_DIR build input; all official releases)
> also embed the fonts listed in https://github.com/storytold/craft-fonts/blob/main/ATTRIBUTION.md
> (the Japanese fonts BIZ UDPGothic, BIZ UDMincho and Shippori Mincho; the web build embeds only BIZ
> UDPGothic Regular), under the SIL Open Font License 1.1. Release packages include each font's
> licence as OFL-<family>.txt.

- **ATTRIBUTION** (`assets/ATTRIBUTION.md`, enforced by `cargo xtask assets`) [VERIFIED]:
  - Fonts: Inter (OFL 1.1, `assets/fonts/OFL-Inter.txt`). CJK fonts come from craft-fonts at build time: BIZ UDPGothic/UDMincho, Shippori Mincho, and **Noto Sans CJK SC (co-developed by Adobe; allowed as a "maintainer-approved exception")**, all OFL 1.1. `epaint_default_fonts` (from egui) is also bundled: `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`, and **its Ubuntu Font Licence is not mentioned in NOTICE** [VERIFIED from registry Cargo.toml; inclusion via `default_fonts` feature in `apps/lightcraft/Cargo.toml`].
  - Icons: app icon "lynx", the project owner's original work, MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`). UI icons are drawn in code (`crates/ui-egui/src/icons.rs`).
  - Colour data: 4 camera profiles (ILCE-7M4, ILCE-7CR, X-H2S, X-T4) fitted from contributors' photos, MIT OR Apache-2.0. `assets/camera-tone/sony-dro-off.json` is derived from CC0 raw.pixls.us files. `spectral_table.rs` is Apache-2.0 plus CIE CC-BY-4.0. **No DCP/LCP/LUTs** are shipped; `xtask assets` rejects `.dcp`, `.lcp` and `.lrtemplate`.
  - Images: public-domain showcase photos (Adams, Lange, NASA) inside screenshots; the DMG background is original work by @XusBadia.
  - Locale: Ukrainian catalogs "by @dmatviichuk with Codex", reusing labels from PhotoCraft, FilmCraft and VectorCraft (MIT/Apache).
- **Dependency licence scan** [VERIFIED for 345 of 656 locked packages whose sources were in the local cargo registry cache; the remaining 311 (including `candle-*`, `heic-rs`, `eframe`, `accesskit*`, `zbus`) are NEEDS TESTING with `cargo deny check licenses`]: all permissive. Notable entries: BSD-2/3 (rav1e, ravif, avif-serialize, dalek crypto), ISC (ring, rustls-webpki, libloading, untrusted), Zlib (foldhash, zlib-rs), **CDLA-Permissive-2.0 (webpki-roots: Mozilla CA data)**, Unicode-3.0 (unicode-ident), IJG (jpeg-encoder), `self_cell` (Apache-2.0 OR GPL-2.0-only, so Apache can be chosen). **No copyleft-only crate was found in the scanned set.** There is **no `deny.toml`**.
- **Model weights and licences (not bundled)**: SAM 3 (Meta SAM Licence, non-OSI), RawNIND (GPL-3.0, opt-in feature), YuNet/SFace (OpenCV Zoo licences), **AuraFace (fal, licence to be verified)**. [NEEDS TESTING: legal review before offering in-product downloads under the Ubunye name.]
- **Trademarks** [VERIFIED: README lines 555-560, `docs/brand/LICENSE-brand.txt`]:
  - ArtCraft name, wordmark and mark (`docs/brand/*.svg|png`) are **not open source**. The brand licence says: *"If you distribute or publish a modified version of LightCraft… you must remove the ArtCraft Marks… and must not call it ArtCraft or present it as an ArtCraft product. You may say, in plain text, that your work is based on LightCraft by the ArtCraft team."*
  - Adobe disclaimer: "Adobe, Photoshop, … Lightroom … are trademarks … LightCraft is an independent … not affiliated with … Adobe".
  - "LightCraft" itself is not declared a trademark in the repo, but it is clearly the ArtCraft family brand. Renaming it is prudent [ASSUMPTION].
  - "Storytold" is only the GitHub org name. `ai.storyteller.*` IDs and "Learning Machines LLC" (manufacturer/vendor) identify the upstream company.
- **Clean-room claims** [VERIFIED: README line 16 and 433, AGENTS.md "Non-negotiables"]: the project calls itself "a clean-room reimplementation of Adobe Lightroom". Rules: never read or disassemble Adobe binaries; never copy GPL code (darktable, RawTherapee, LibRaw, rawspeed, rawloader, rawler, lensfun, dcraw); work from public specs and **black-box observation of an installed Lightroom** (screenshots kept in the gitignored `plan/lightroom/`). Caveats:
  - The clean-room claim is self-attested, and the process (agents told "never read X") is not auditable from the repo.
  - The product interoperates with Adobe formats (`crs:` XMP, `.lrtemplate`, `.lrcat`, DNG). That is generally fine but should get legal review.
  - The Info.plist registers Adobe's `com.adobe.photoshop-large-image` UTI (leftover).
  - `crates/raw/src/vendor/cr3.rs` cites lclevy/canon_cr3 (public documentation).
  - The darktable-ai model URL is only a download pointer.

---

## 10. Branding surface

Counts across tracked text files (`git grep -I`) [VERIFIED]:

| Token | Occurrences / files |
|---|---|
| `LightCraft` | 1,201 / 238 (316 in `.rs`; **526 in 18 locale JSONs** in `crates/ui-egui/locales/`) |
| `Lightcraft` | 553 / 92 (e.g. type `LightcraftApp`, WiX ids) |
| `lightcraft` (crate names, binaries, paths) | 4,809 / 475 |
| `LIGHTCRAFT` (env vars) | 280 / 77; 32 distinct `LIGHTCRAFT_*` env vars (`LIGHTCRAFT_GPU_BACKEND`, `_LOG`, `_PROFILE`, `_CORPUS`, `_VERSION`, `_SAM3_*`, `_NO_PREFS`, `_LANGUAGE`, `_LIBRARY`, `_CONTROL_PORT`, `_MEMORY_MB`, `_FACE_*`, `_DENOISE_MODELS`, `_CAMERA_PROFILES`, `_BUILD_SHA/DATE`, …) |
| `artcraft` (any case) | 199 / 32 |
| `storytold` | 101 / 33 |
| `ai.storyteller` | 62 / 17 |
| `getartcraft` | 32 / 8 |
| `discord.gg` | 9 / 4 |
| `Learning Machines` | 4 / 3 |
| `craft-fonts` / `craftrules` / `photocraft` | 153/29, 41/13, 34/14 |
| `ns.lightcraft.app` (XMP namespace) | 11 / 8 |

Key structural locations [VERIFIED]:

- **Window title / app ID**: `apps/lightcraft/src/main.rs:38` (`APP_ID = "ai.storyteller.lightcraft"`), `:742` `.with_title("LightCraft")`, `:758` app name; macOS app menu `apps/lightcraft/src/native_menu.rs:230` `Submenu::new("LightCraft")`; web `apps/lightcraft-web/index.html:6` `<title>`; IndexedDB name `"lightcraft"` (`apps/lightcraft-web/src/backend.rs:12`).
- **Links / menus / About**: `crates/ui-egui/src/links.rs` (Discord, getartcraft.com, GitHub repo, issues, docs, "Join the ArtCraft Discord", "ArtCraft Website"); top-bar Discord button `crates/ui-egui/src/panels/topbar.rs:161`; About dialog `crates/ui-egui/src/panels/dialogs.rs:925-937`; credits `crates/ui-egui/src/credits.rs` plus `contributors/`.
- **Icons**: `assets/app-icon/` (`lightcraft.svg`, `-small.svg`, `-1024.png`, `-macos-512.png`, `.icns`, `.ico`, `hicolor/{16..512,scalable}/apps/ai.storyteller.lightcraft.png|svg`), generated by `packaging/icons.sh`. DMG background in `packaging/macos/dmg/` (shows the lynx icon). ArtCraft logos are in `docs/brand/` (README only). There is no splash screen; the first run seeds a demo library [ASSUMPTION: no splash asset found].
- **Installer metadata**: `packaging/macos/Info.plist.in` (CFBundleName/DisplayName, bundle id, UTIs, copyright), `packaging/windows/lightcraft.wxs` (Name, Manufacturer, UpgradeCode, `HKLM\Software\LightCraft`, ARP links), `apps/lightcraft/build.rs` (VERSIONINFO), `packaging/linux/ai.storyteller.lightcraft.{desktop,metainfo.xml.in,mime.xml}`, `flatpak/*.yml`, `nfpm.yaml` (vendor), `omastore.toml`, `flake.nix`/`nix/package.nix`.
- **Config and data dirs**: `crates/engine/src/config.rs` (`~/Library/Application Support/LightCraft`, `%APPDATA%\LightCraft`, `$XDG_CONFIG_HOME/lightcraft`); logs `logs/lightcraft.log` and `lightcraft-panics.log` in temp; default library `~/Pictures/LightCraft Library` (`crates/engine/src/library.rs` `DEFAULT_NAME`); catalog format tag `"lightcraft-catalog"` (`crates/catalog/src/journal.rs:73`; **renaming it would break compatibility with existing libraries, so keep it or add an alias**).
- **File extensions / formats**: `.lcpreset`; XMP `lc:` namespace `http://ns.lightcraft.app/lc/1.0/` (renaming it breaks sidecar interop with upstream); `pcraft` UTI (bogus); MIME `application/x-lightcraft`.
- **Export metadata**: `software: Some("LightCraft")` (`crates/engine/src/export.rs:1309`) is written into exported files' EXIF/XMP.
- **Release URLs**: AppImage zsync `storytold|lightcraft`; README download links; `release.yml` title "LightCraft v…".
- **Telemetry endpoints**: none found.

**Isolation suggestion**: create a `crates/brand` (L0, no deps) crate exposing `const`s: `APP_NAME`, `APP_NAME_LOWER`, `APP_ID` (reverse-DNS), `VENDOR`, `CONFIG_DIR_{MAC,WIN,XDG}`, `DEFAULT_LIBRARY_NAME`, `ENV_PREFIX`, `XMP_NS`/`XMP_PREFIX` (kept as legacy-read alias), `PRESET_EXT`, `EXPORT_SOFTWARE`, `LINKS` (website, community, repo, issues, docs), `UPDATE_CHANNEL`. Then:

1. Replace literals in `apps/*/src/main.rs`, `native_menu.rs`, `crates/engine/src/{config,library,export}.rs`, `crates/ui-egui/src/{links,credits}.rs` and the panels with the brand constants.
2. Make the i18n catalogs use a `{app}` placeholder instead of the literal "LightCraft". There are 526 occurrences, and the `tr_format!` build script already validates placeholders.
3. Generate packaging files (Info.plist, .wxs, .desktop, metainfo, flatpak, nfpm, omastore) from templates fed by one `brand.toml` read by `packaging/env.sh` and `xtask`. Several are already `.in` templates.
4. Rename env vars behind a helper `brand::env("GPU")` that also reads the legacy `LIGHTCRAFT_*` names.
5. Keep crate names (`lightcraft-*`) initially to reduce merge pain with upstream, and rename later if the fork diverges.
6. Delete `docs/brand/` and all ArtCraft/Discord references, as the brand licence requires.

---

## 11. CLI, MCP and agent automation

- **Binaries** [VERIFIED]: `lightcraft` (desktop; flags `--library`, `--memory`, `--control PORT`, `--version`, `--help`), `lightcraft-cli` (`run`, `mcp`, `render`, `snapshot`, `merge hdr|panorama|hdr-panorama`, `synth-merge`, `commands [--json]`, `controls [--json]`, `calibrate`), `lightcraft-sam3-worker` (`crates/segment/src/bin/`; a Metal SAM 3 server on loopback for SSH-tunnel use), `xtask`. There are also examples in `crates/raw/examples/{rawdump,rawinfo,rawrender,colstats}.rs` and elsewhere.
- **MCP server** (`crates/mcp`: `server.rs`, `tools.rs`, `headless.rs`; launched by `lightcraft-cli mcp`) [VERIFIED: `docs/mcp.md`]:
  - Transport: newline-delimited JSON-RPC 2.0 over **stdio**; protocol 2025-06-18, also accepting 2025-03-26 and 2024-11-05. Hand-written, no async runtime. No auth (stdio).
  - Modes: **headless** (in-process `Session`; `--demo`, `--library DIR`, files to import) or **connect** (`--connect 127.0.0.1:7980`, driving the running app via the control channel; reconnects automatically).
  - Tools: core `command_list`, `command_run`, `command_batch`, `doc_inspect`, `render_preview`. Helpers: `list_commands`, `run_command`, `import`, `query_photos`, `select_photos`, `list_controls`, `get_develop`, `set_develop`, `apply_preset`, `crop`, `render_photo` (returns image content), `export`. Connect-only: `screenshot`, `inspect_ui`, `set_ui`, `list_widgets`, `click`, `press_key`, `type_text`, `clipboard`, `pointer_gesture`, `ui_inspect`, `ui_screenshot`. Plus **one `cmd_<id>` tool per registry command** (`--compact` hides them). Resources: `lightcraft://document`, `lightcraft://commands`. Tool annotations (hints) are set.
  - Tests: `crates/mcp/tests/e2e.rs` and 31 unit tests.
- **Control protocol** (`docs/control-protocol.md`): JSON-lines on `127.0.0.1:<port>` with methods `engine.execute`, `engine.commands`, `develop.controls`, `ui.*` (inspect, widgets, clickWidget, pointer, key, screenshot (headless option), resize, render, menu.invoke) and `app.quit`. No auth (see §8).
- **`.mcp.json`**: **not present**. There is also no `.claude/` directory [VERIFIED].
- **CLAUDE.md / AGENTS.md** (about 16 KB each, nearly identical; CLAUDE.md adds two UI rules about `TaskGuard` and `TextField`, and AGENTS.md adds a "Contributor credits" section) [VERIFIED: diff]. Summary of contents, reported as data:
  - They start each agent session from `plan/STATUS.md` and `plan/execution-plan.md` (both gitignored and unavailable), point agents at `ROADMAP.md` and `docs/parity.md` to pick work, and have agents follow an "autonomous operation protocol" without stopping to ask.
  - **Never-crash standard**: no unwrap/expect/panic/unsafe in non-test code, checked indexing, bounded recursion, `catch_unwind` as a safety net, and a regression test for every crash fix. Enforced by clippy denies on crate roots.
  - **Non-negotiables**: clean-room (no Adobe binaries, no GPL code), pure Rust, layering, everything is a command, resolution independence, `cargo xtask ci` green before every commit, and one task id per commit.
  - **Asset rule**: no Adobe assets (including OFL Adobe fonts, except Noto CJK), every asset in `ATTRIBUTION.md`, fonts only via craft-fonts.
  - **Running the app**: use the control channel and never OS-level synthetic input; use headless snapshots when unattended; parallel agents get separate worktrees, a `CARGO_TARGET_DIR` and a control port in 18000–19999.
  - Corpora are CC0 only and never committed.
  - **Contributor credits**: agents are told to ask their human about being credited in `people.toml`.
  - References to sibling repos (`../craftrules`, `../photocraft`, `../pdfcraft`, …) assume a monorepo-style checkout.
  - There are no instructions that would be harmful for a fork. Several are upstream-process-specific and should be rewritten for Ubunye (plan/ paths, craftrules, Discord, attribution trailer).

---

## 12. Readiness assessment

| Dimension | Score (0-5) | Rationale |
|---|---|---|
| Build | **3** | Clean Cargo workspace, `Cargo.lock` committed, no git deps, pure-Rust intent, solid xtask. But no toolchain pin, no CI build on Linux, macOS or Windows (only FreeBSD and release), and builds not verified here [NEEDS TESTING] |
| Features | **3** | Very broad Lightroom-like feature set for 10 days of work. Own estimate is 60–70% of a real replacement. Camera colour, RAW coverage, AI masks and lens profiles are weak; ✅ rows are unaudited |
| Tests | **3** | 2,576 tests, proptest robustness suites, GPU equivalence and MCP e2e. But **CI runs them only on FreeBSD and only on manifest-changing PRs**; GPU and corpus tests skip in CI; no golden-image regression suite; no coverage-guided fuzzing |
| Security | **2.5** | Rust with a single unsafe block, a strong no-panic discipline, no telemetry, SHA-pinned Actions. Against that: a very large in-house, agent-written parser surface with no cargo-fuzz; an unauthenticated loopback control port able to write arbitrary files; a hand-rolled HTTP client on an **alpha** crypto provider; no cargo-deny/audit; and essentially no human security review |
| Docs | **4** | Extensive README, ROADMAP, parity tracker, protocol, MCP, web, release and GPU docs. Weaknesses: docs drift (CR3 contradiction), and the key architecture docs live in gitignored `plan/` |
| Packaging | **4** | Signed and notarized macOS DMG, MSI for x64/x86/arm64 with Azure Trusted Signing, AppImage with zsync, deb, rpm, Flatpak, FreeBSD, web zip, Nix. Leftover PhotoCraft UTIs, an hard-coded UpgradeCode/IDs/vendor that must change, and Windows UI unverified on hardware |

**Top risks**

1. **Upstream velocity and provenance**: about 1,300 commits per week and about 60% agent-co-authored, at 10 days old. Rebasing or tracking upstream will be painful, and quality claims are self-reported. Decide early whether to fork hard or track upstream.
2. **No real CI test gate** on mainstream OSes. Add a Linux/macOS/Windows `cargo xtask ci` workflow (with `--locked`) on day one.
3. **Parser attack surface**: about 30k LOC of in-house RAW, TIFF, metadata, PSD, SQLite and ONNX parsers, never coverage-fuzzed. Add cargo-fuzz targets and OSS-Fuzz-style runs before shipping to users who open untrusted files.
4. **Colour fidelity and RAW coverage** gaps: no camera calibration database, about 55 corpus files. Image quality versus Lightroom is unmeasured.
5. **Branding and trademark obligations**: ArtCraft marks must be removed, and LightCraft, `ai.storyteller.*`, Learning Machines LLC, getartcraft.com, Discord and storytold URLs must be replaced, including 526 locale strings, update channels and the MSI UpgradeCode.
6. **Model licences**: SAM 3 (non-OSI), RawNIND (GPL-3.0), AuraFace and OpenCV Zoo licences need review before the fork offers downloads.
7. **Supply chain**: `rustls-rustcrypto 0.0.2-alpha`, a possible `ring` C build, no `deny.toml`, unhashed tool downloads (rustup curl|sh, nfpm), and unpinned stable toolchain.
8. **Unauthenticated control port** (local privilege boundary on multi-user systems).
9. **External build inputs** from `storytold/craft-fonts` (pinned by commit) and implicit sibling-repo conventions (`craftrules`). These must be mirrored into `ubunye-ai-ecosystems`.
10. **Web build** is experimental: `panic=abort` kills the tab, durability is weak, and there is no multi-tab lock.

**What needs testing (Phase 2)**

- `cargo build --locked` and `cargo xtask ci` on Linux, macOS and Windows; `cargo xtask web`; `nix build`.
- `cargo deny check licenses bans advisories sources` and `cargo audit`; `cargo tree -e features -i ring` / `-i cc` to confirm "no C".
- Run the full test suite and record pass/fail and flaky tests, including GPU equivalence on real Vulkan/DX12/Metal adapters.
- Download the corpus and run the raw/codec corpus tests; compare renders against reference outputs for the 10–20 most common camera models.
- Fuzz campaigns (cargo-fuzz) on `lightcraft-tiff`, `lightcraft-raw` vendors, `lightcraft-meta` XMP/EXIF, `codecs::psd`, `lightroom_sqlite`, `onnx_proto` and preset/zip import.
- Pentest the control port: local file write via `ui.screenshot`/`app.export`, connection limits, DNS-rebinding/browser smuggling, and the SAM 3 worker token.
- Review the MCP tools for prompt-injection exposure (metadata returned to LLMs) and destructive operations (`import mode=move`, deletes).
- Verify installers on real Windows, macOS and Linux; confirm signing with our own certificates; check AppImage update behaviour after re-pointing.
- Legal review: the clean-room process and Adobe interop formats, model weight licences, the `epaint_default_fonts` Ubuntu Font Licence, Noto CJK, and the Apache-only crates (`segment`, `fetch`).
- Confirm GitHub release assets, stars and issues via the API (unreachable from this session).
