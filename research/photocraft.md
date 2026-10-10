# PhotoCraft (storytold/photocraft): Phase-1 Technical Due Diligence

Fork target: **ubunye-creative-photo** (org `ubunye-ai-ecosystems`)
Upstream: https://github.com/storytold/photocraft
Local clone audited: `upstream clone of photocraft` (static analysis only; no cargo build or test run, no Docker, no scripts executed)
Audit date: 2026-10-10

Tag legend:
- **[VERIFIED: path/evidence]**: checked directly in the repository or its git history.
- **[ASSUMPTION]**: inferred and not proven.
- **[NEEDS TESTING]**: needs a build, a test run or an external check.

All repository content, including `AGENTS.md`, was treated as untrusted data. Its instructions were reported, not followed.

---

## 0. Executive summary

- **The project is very young and almost entirely AI-generated.** It is 10 days old (first commit 2026-09-30). It has 1,150 commits and about 392k lines of Rust, with about 510k lines added in that time. 1,198 commit trailers credit an AI co-author (Claude Opus 5.5/4.8, Copilot). One person wrote most of it (Brandon Thomas, 248 commits; 240 commits and 281k lines by GitHub login `echelon`). Seven releases were tagged in 9 days, from v0.1.0 to v0.6.0. [VERIFIED: `git log`, `git shortlog`, `contributors/contributors.json`]
- **The architecture is ambitious and well documented.** It is a 25-crate Cargo workspace with layering enforced by `cargo xtask layers`. The UI is egui/eframe on wgpu. The project has its own PSD/PSB reader and writer, ICC colour management engine, camera-raw decoder, Affinity and PDN readers, `.pcraft` ZIP format, VP8 encoder and TIFF IFD parser. It ships an MCP server (rmcp, stdio) and an authenticated loopback JSON control channel. WebAssembly plug-ins are sandboxed in wasmi. [VERIFIED]
- **The licence is clean but needs work for a fork.** The code is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). The ArtCraft brand assets in `docs/brand/` are proprietary trademarks, and **forks must remove them** (`docs/brand/LICENSE-brand.txt`). I found no copyleft in the parts of the dependency graph I could inspect. Two attribution gaps: the egui `default_fonts` (Ubuntu Font Licence, OFL, MIT) are not credited, and 8 of the 16 translation files are missing from `ATTRIBUTION.md`.
- **Security posture is good for its age but immature.** `unsafe_code = "forbid"` is set workspace-wide, and only 3 `unsafe` blocks exist (`crates/tablet/src/macos.rs`). Parsers carry hostile-input limits. There are 16 cargo-fuzz targets, but they run on manual trigger only. The project has **no cargo-deny or cargo-audit, no SBOM, no private vulnerability channel, and mostly tag-pinned (not SHA-pinned) GitHub Actions**. Release tooling is downloaded without checksum verification (appimagetool `continuous`, trunk, nfpm).
- **The upstream project is candid that it is not production-ready.** README and roadmap say "early alpha" and put real Photoshop parity "well below 50%" (`docs/roadmap.md`). The scorecard shows 3 of 56 performance scenarios within budget, and one scenario crashes the GPU compositor (P12). 46 of 149 preferences do nothing. PSD oracle render match is 146/170 on one corpus and 236/309 on the psd-tools corpus.
- **There are 5,460 `#[test]` functions.** CI covers Linux and Windows on every PR or main push, and macOS only in the release workflow. FreeBSD and Windows ARM64 run nightly.

---

## 1. Snapshot

| Item | Value | Tag |
|---|---|---|
| HEAD SHA | `7722172585a01cbdb93c06f0f5ff2634fcb17999` | [VERIFIED: `git log -1`] |
| HEAD date / subject | 2026-10-10 08:40:35 -0400, "Add OpenRaster (.ora) import and export (#2257)" | [VERIFIED] |
| First commit | `386b0566` 2026-09-30 02:12 "init repo" (5 lines), then `16941f71` "one-shot" (+42,241 lines in 241 files) and `ae92ac20` "continued wip" (+37,990 lines) | [VERIFIED: `git show --shortstat`] |
| Commit count (main) | 1,150 (full clone, not shallow) | [VERIFIED: `git rev-list --count HEAD`] |
| Lines added / deleted (all history) | ~510,867 / ~42,584 | [VERIFIED: `git log --shortstat`] |
| Contributors | 175 distinct author names (`git shortlog -sn`); 80 of them have exactly 1 commit. Top: Brandon Thomas 248, BFlat 93, Nigel Ihza Firdausy 63, David Mann 38, kayrockscreenprinting 34. 26 commits are authored as "Claude <noreply@anthropic.com>" | [VERIFIED] |
| AI co-authorship | 1,198 `Co-authored-by` trailers naming Claude, Copilot and similar (e.g. 591 + 334 "Claude Opus 5.5", 63 + 39 "(1M context)", 53 Copilot, 40 "Claude Opus 4.8") | [VERIFIED: `git log --format=%B`] |
| PR-merge commits | 988 subjects end in `(#NNN)`; highest PR or issue number referenced is #2404 | [VERIFIED] |
| Tags | v0.1.0 (10-01), v0.1.1-rc.4 (10-01), v0.1.1-rc.5 (10-02), v0.1.1 (10-03), v0.2.0 (10-05), v0.3.0 (10-07), v0.5.0 (10-08), v0.6.0 (10-09, `0c72d954`). HEAD is 61 commits past v0.6.0. Workspace version is `0.6.0` | [VERIFIED: `git tag`, `Cargo.toml`] |
| GitHub releases, stars, issues | Could not query: GitHub API access to this repo is not enabled in this session | [NEEDS TESTING] |
| Default branch | `main` (`origin/HEAD -> origin/main`). A `release` branch triggers `release.yml`. About 260 remote branches (`claude/*`, `community/*`, `fix/*`, `feat/*`, `pr/never-crash-*`, `integ-batch-*`) | [VERIFIED: `git branch -a`] |
| Repo size | Working tree 37 MB including `.git` (29 MB); 1,413 tracked files | [VERIFIED: `du`, `git ls-files`] |
| Rust LOC | ~392,559 lines of `.rs` | [VERIFIED: `find … | wc -l`] |
| Cadence | Commits per day: 6, 15, 24, 26, 59, 102, 45, 188, 211, **366** (2026-10-09), 108 (partial 10-10). This is far above human cadence and points to agent-driven development with batch integration branches | [VERIFIED; interpretation: ASSUMPTION] |
| Corporate owner | "ArtCraft Team" (copyright). Installer and code-signing entity is **"Learning Machines LLC"**, Apple Team `DJ6XS33FX8` (`apps/photocraft/build.rs`, `packaging/windows/photocraft.wxs`, `docs/releasing.md:297`) | [VERIFIED] |

---

## 2. Repository structure

### 2.1 Workspace

`Cargo.toml`: `resolver = "3"`, `members = ["crates/*", "apps/*", "xtask"]`, edition 2024, `rust-version = "1.95"`, `license = "MIT OR Apache-2.0"`. It sets `[workspace.lints.rust] unsafe_code = "forbid"`. Release profile uses thin LTO and `codegen-units=1`. A custom `wasm-release` profile uses fat LTO and `opt-level="s"`, with pixel crates at `opt-level=3`. [VERIFIED]

### 2.2 Crates (layer from `AGENTS.md` and `docs/architecture.md`; LOC includes tests)

| Crate (dir) | Package | Layer | LOC | Purpose | Tag |
|---|---|---|---:|---|---|
| `crates/geom` | photocraft-geom | L0 | 1,019 | Points, rects, affine and perspective maths, tile coordinates | [VERIFIED] |
| `crates/cms` | photocraft-cms | L0 | 5,172 | Pure-Rust ICC v2/v4 colour management: parse and write profiles, transforms, intents, BPC, 3D LUTs, synthetic profiles | [VERIFIED: Cargo description] |
| `crates/color` | photocraft-color | L0 | 1,236 | Pixel formats, colour models, blend-mode maths (34 `BlendMode` variants) | [VERIFIED] |
| `crates/raster` | photocraft-raster | L0 | 1,037 | Copy-on-write 256² tiled surfaces, masks | [VERIFIED] |
| `crates/psd` | photocraft-psd | L0 standalone | 13,773 | Lossless PSD (v1) / PSB (v2) reader and writer; descriptors, ABR, ACO, ASE, GRD, patterns, layered TIFF | [VERIFIED] |
| `crates/codecs` | photocraft-codecs | L0 standalone | 15,920 | Flat raster codecs (PNG, JPEG, TIFF, WebP incl. own VP8 encoder, EXR incl. deep, PNM, plus image-crate formats); optional `heif`, `avif` | [VERIFIED] |
| `crates/heif` | photocraft-heif | L0 standalone | 218 | Optional HEIC decoder wrapper over `heic-rs =0.1.1`, panic-guarded with `catch_unwind` | [VERIFIED] |
| `crates/raw` | photocraft-raw | L0 standalone | 9,639 | Clean-room camera raw decoder and developer: DNG, CR2, NEF, ARW, RW2, ORF, RAF | [VERIFIED] |
| `crates/affinity` | photocraft-affinity | L0 standalone | 3,709 | Bounded reader for Affinity `.af*` documents (read only) | [VERIFIED] |
| `crates/tablet` | photocraft-tablet | L0 standalone | 1,019 | Pen pressure, tilt and rotation on macOS (AppKit) and X11 (XInput2); the only crate allowed `unsafe` | [VERIFIED] |
| `crates/doc` | photocraft-doc | L1 | 4,694 | Pure-data document model: layers, masks, adjustments, effects, smart objects | [VERIFIED] |
| `crates/ops` | photocraft-ops | L2 | 671 | Reversible operations, transactions, history | [VERIFIED] |
| `crates/algo` | photocraft-algo | L2 | 40,200 | Depth-agnostic tiled CPU filters: blur, sharpen, noise, distort, content-aware fill and inpainting, FFT | [VERIFIED] |
| `crates/paint` | photocraft-paint | L2 | 6,310 | Brush engine: dynamics, mixer, symmetry, retouch, background eraser | [VERIFIED] |
| `crates/text` | photocraft-text | L2 | 8,160 | Font DB, shaping and layout (parley, skrifa), text-layer raster, PSD text, spell check, CJK fallback | [VERIFIED] |
| `crates/vector` | photocraft-vector | L2 | 3,152 | Paths, shapes, strokes, vector masks, tracing | [VERIFIED] |
| `crates/compose` | photocraft-compose | L3 | 10,598 | CPU reference compositor (the "oracle"): adjustments, effects, advanced blending | [VERIFIED] |
| `crates/gpu` | photocraft-gpu | L3 | 7,080 | wgpu 30 compositor (one WGSL file, `compose.wgsl`), effect maps, device health | [VERIFIED] |
| `crates/format` | photocraft-format | L3 | 5,279 | Native `.pcraft` bundle: manifest plus content-addressed blake3/zstd tiles; own ZIP reader and writer; autosave | [VERIFIED] |
| `crates/io` | photocraft-io | L4 | 25,629 | Document mapping to and from PSD/PSB, layered TIFF, ORA, PDN, SVG (resvg), Affinity, raw, flat formats | [VERIFIED] |
| `crates/plugins` | photocraft-plugins | L4 | 1,486 | Sandboxed WebAssembly filter plug-ins (wasmi 2.0, ABI v1) | [VERIFIED] |
| `crates/engine` | photocraft-engine | L5 | 85,546 | `Session`, command registry (116 `*_cmds` modules), jobs, preferences, presets | [VERIFIED] |
| `crates/ui-egui` | photocraft-ui-egui | L6 | 122,029 | egui shell: panels, dialogs, canvas, themes, i18n (16 TSV catalogs), control methods | [VERIFIED] |
| `crates/automation` | photocraft-automation | L6 | 5,500 | MCP server (rmcp 3.5, stdio), headless JSON-RPC, bridge to the GUI, workspace roots (cap-std) | [VERIFIED] |
| `crates/testkit` | photocraft-testkit | test | 199 | Test helpers | [VERIFIED] |

Despite the "thin UI" rule, `ui-egui` is the largest crate at 122k LOC. Treat UI logic as substantial code, not a thin layer. [VERIFIED LOC; interpretation ASSUMPTION]

### 2.3 Apps

| Path | Binary | Purpose |
|---|---|---|
| `apps/photocraft` | `photocraft` (6.2k LOC) | Desktop app (eframe, wgpu). Includes the TCP control server (`src/control_server.rs`), crash guard, GPU startup guard, macOS native menu (muda), Apple events, tablet, screen colour sampling, Photoshop shortcut import (`photoshop_settings.rs`) and portable mode (`app_dirs.rs`) |
| `apps/photocraft-cli` | `photocraft-cli` (1.4k LOC) | Headless `convert`, `info`, `run`, `batch`, `droplet`, `commands`, `mcp` and `serve` |
| `apps/photocraft-web` | `photocraft-web` (1.2k LOC) | Browser build (trunk + wasm-bindgen; WebGPU with WebGL2 fallback; localStorage and IndexedDB) |

[VERIFIED: `apps/*/Cargo.toml`, `apps/photocraft-cli/src/lib.rs:14-45`]

### 2.4 Other top-level directories

- **`xtask/`**: subcommands `layers`, `wasm`, `ci`, `corpus`, `test-corpus`, `stats`, `parity`, `perf`, `scorecard`, `version`, `ico` (`xtask/src/main.rs:55-66`). It also holds the sha256 manifests for the corpora (`xtask/*.sha256`) and the pins (`xtask/src/corpus_pins.rs`). [VERIFIED]
- **`book/`**: mdBook (`book/book.toml`) with sections for getting-started, architecture, automation, formats, security (8 pages: threat model, trust boundaries, parser hardening, fuzzing and others), development and project. About 1.2k lines. It is built by `docs.yml` with mdbook 0.4.52. [VERIFIED]
- **`docs/`**: 31 Markdown files, about 4.2k lines: architecture, roadmap, parity (generated), scorecard (generated), control protocol, MCP, plugins, releasing, UI design, localisation, performance write-ups. [VERIFIED]
- **`perf/`**: `budgets.toml` (scenario targets) and `baseline.json` (measured 2026-10-05 on an M4 Pro with a load average of about 225, so the numbers are noisy). [VERIFIED: `docs/scorecard.md`]
- **`scorecard/`**: TOML checklists (`automation`, `bugs`, `distribution`, `file_compat`, `reliability`, `tools`, `type`, `ui`) rendered into `docs/scorecard.md`. This is an **internal product scorecard, not OpenSSF Scorecard**. [VERIFIED]
- **`scripts/`**: `fetch-corpus.sh` and `layered_tiff_fixtures.py`. [VERIFIED]
- **`examples/plugins/invert-rs`**: sample WASM plug-in, with its own `Cargo.lock`, which uses `unsafe extern "C"` and `from_raw_parts_mut`. [VERIFIED]
- **`packaging/`**: `linux/` (AppImage, deb/rpm via nfpm, tarball, Flatpak manifests, `.desktop`, metainfo, MIME), `macos/` (Info.plist, entitlements, DMG assets, sign, notarize, verify), `windows/` (WiX v5 `.wxs`, `package.ps1`, `sign.ps1`, portable marker), `freebsd/package.sh`, `web/` (`package.sh`, `_headers`, `.htaccess`, `nginx.conf`, README), `icons.sh`, `env.sh`. [VERIFIED]
- **`Dockerfile`**: community-maintained web self-hosting recipe. It builds the wasm with trunk on `rust:1.99-slim-bookworm` and serves it from `nginx:1.28-alpine` as non-root on port 8080 with a `/healthz` check. It verifies the trunk download's sha256. It is not built in CI. [VERIFIED: `Dockerfile`, `packaging/web/README.md`]
- **`assets/`**: `app-icon/` (kitsune icon, PNG, ICNS, ICO), `fonts/` (Inter ×3, JetBrains Mono + OFL), `icons/` (128 files: Lucide SVGs and originals), `dict/` (SCOWL word list). [VERIFIED]
- **`contributors/`**: `contributors.json` (generated) and `people.toml`, compiled into the About window. [VERIFIED]
- **Gitignored, local only**: `plan/`, `log/devlog.md` and `corpus/`. Agents are told to read the dev log, but it is not in the repo. [VERIFIED: `AGENTS.md` §7, `.dockerignore`]

---

## 3. Architecture

### 3.1 Languages and stack

- **100% Rust.** The only non-Rust code is one WGSL shader (`crates/gpu/src/compose.wgsl`), shell and PowerShell packaging scripts, a Python fixture generator and the DMG generator. There is no JS or TypeScript, and Tauri and webviews are banned by policy (`AGENTS.md` line 3). [VERIFIED]
- **UI framework**: egui/eframe **0.36.2** on winit **0.30.13**, with the wgpu renderer, accesskit, Wayland and X11 (`apps/photocraft/Cargo.toml`). `rfd 0.17` provides file dialogs and `arboard 3.6` the clipboard. On macOS the native menu bar uses `muda =0.21.1`. [VERIFIED]
- **Rendering and GPU**: wgpu **30.0.1** (Metal, Vulkan, DX12, GL; WebGPU and WebGL2 on the web). Two compositors exist: the CPU reference in `crates/compose` and the GPU one in `crates/gpu` (`plan.rs` builds a DrawOp plan and `compose.wgsl` executes it). They are tested against each other (`crates/gpu/tests/parity.rs`). Those GPU tests **skip silently when there is no adapter** (`parity.rs:35-52`), so on GPU-less CI runners they may not run at all. [VERIFIED; CI GPU availability: NEEDS TESTING]
- **Compositing engine**: the layer tree is evaluated as a graph. It handles adjustment, fill, text, shape and smart layers; clipping; advanced blending and knockout (`compose/src/advanced.rs`); and effects (`compose/src/effects.rs`). There are 27 Photoshop blend modes plus 6 Paint.NET and extra variants (34 enum variants including PassThrough, in `crates/color`). The 16f canvas is optional (`PHOTOCRAFT_CANVAS_F16`). [VERIFIED]
- **Layer model**: `crates/doc` has `LayerContent` as an enum (pixel, adjustment, fill, text, shape, smart object) with smart-filter stacks, masks (pixel and vector), channels and artboards. Pixel depth (8, 16, 32f) and colour mode (RGB, Gray, CMYK, Lab, multichannel, indexed) are runtime data. [VERIFIED: `docs/architecture.md` §1.1; `crates/engine/src/mode_cmds.rs`, `multichannel_cmds.rs`]
- **PSD read/write**: `crates/psd` is standalone, `forbid(unsafe_code)`, written from the Adobe public spec. It says it follows psd-tools and ag-psd (MIT) behaviour for undocumented details, noted in comments (`crates/psd/src/lib.rs:16-18`). Unmodified files round-trip byte for byte; channel data is decoded lazily; it supports RAW, RLE, ZIP and ZIP-prediction; unknown blocks are preserved; dimensions are capped at 300,000 and descriptor nesting is bounded. The mapping to and from the document model is in `crates/io/src/psd_import.rs` and `psd_export.rs`, plus effects, adjustment, smart, text, vector and comps maps. [VERIFIED]
- **Text engine**: parley 0.11 (with `complex-scripts` on native), skrifa 0.44 and harfrust via parley. The Japanese fonts come from the optional `craft-fonts` input (`crates/text/build.rs`). It has a spell checker (SCOWL), vertical text and warp. [VERIFIED]
- **Vector engine**: in-house paths, strokes and rasterisation in `crates/vector`. `vello_cpu` appears in the lockfile only through egui's `epaint`. SVG import goes through resvg/usvg 0.45 (`crates/io/src/svg.rs`). [VERIFIED: Cargo.lock reverse deps]
- **Brush engine**: `crates/paint` covers dynamics, scatter, texture, dual brush, mixer and symmetry. ABR import is in `crates/psd/src/abr.rs` and `crates/io/src/abr_map*`. [VERIFIED]
- **Colour management**: in-house `crates/cms` (ICC v2/v4, all four intents, BPC, CLUT and LUT3D, `.cube`/`.3dl`/`.look`). `moxcms` is a dev-dependency only, used as a reference oracle. One synthetic CMYK profile is shipped (`crates/cms/profiles/photocraft-coated-cmyk.icc`, CC0), and the others are generated at runtime. Monitor profile on the desktop: `apps/photocraft/src/monitor_profile.rs`. [VERIFIED]
- **Document format**: `.pcraft` is a ZIP (STORE entries) or a directory. It holds `manifest.json`, `tiles/<blake3>.zst`, `blobs/<blake3>.zst` and previews. Saves are incremental, writes are atomic, and the format is versioned (`FORMAT_VERSION`, `TooNew` error). Load limits: manifest 256 MiB, one blob 1 GiB, total 16 GiB, group depth 100 (`crates/format/src/lib.rs`, `book/src/formats/pcraft.md`). Compression is ruzstd (pure Rust). [VERIFIED]
- **Plugin system**: WASM-only, run in a wasmi interpreter with no imports (no WASI). Limits: 32 MiB module, 512 MiB memory, fuel caps, 60 s wall clock, portable dispatch. There is no 8BF, CEP or UXP support. Commands: `plugin.install/list/run/remove/reload` (`docs/plugins.md`, `crates/plugins`). [VERIFIED]
- **Threading model**: rayon for per-tile parallelism on native and none on wasm (rayon is `cfg(not(target_arch="wasm32"))` everywhere). Snapshots are immutable and copy-on-write, so the arc-swap snapshot is O(layers). Background jobs (`crates/engine/src/jobs.rs`) run on worker threads, check cancellation per tile, apply results on the UI thread as one undo step, and catch panics. tokio is used only in automation and the CLI. The web build runs jobs inline and synchronously. [VERIFIED]

### 3.2 Crate dependency diagram (internal `[dependencies]` only)

```
L7 apps
 photocraft ──────► ui-egui, automation, engine, io, format, codecs, compose, doc, color, raster, geom, tablet
 photocraft-cli ──► automation, engine, io, format, codecs
 photocraft-web ──► ui-egui, engine, io, codecs, doc

L6 ui-egui ───────► engine, gpu, io, plugins, algo, text, vector, paint, format, codecs, compose, doc, cms, color, raster, geom
L6 automation ────► engine, io, format, codecs, compose, doc, geom
L5 engine ────────► io, plugins, psd, format, codecs, compose, ops, paint, algo, text, vector, doc, cms, color, raster, geom
L4 io ────────────► psd, codecs, raw, affinity, format, compose, algo, text, paint, vector, doc, cms, color, raster, geom
L4 plugins ───────► raster, color, geom
L3 compose ───────► vector, doc, raster, color, geom
L3 gpu ───────────► compose, doc, raster, color, geom
L3 format ────────► codecs, doc, raster, color, geom
L2 algo / paint ──► raster, color, geom
L2 text ──────────► psd, doc, raster, color, geom
L2 vector ────────► doc, raster, color, geom
L2 ops ───────────► doc
L1 doc ───────────► raster, color, geom
L0 raster ────────► color, geom      color ──► cms
L0 standalone (no workspace deps): psd, cms, raw, affinity, tablet, heif, geom
   exception: codecs ──► heif (feature `heif`; STANDALONE_EXCEPTIONS in xtask/src/layers.rs)
```

[VERIFIED: parsed from each `Cargo.toml`; layering enforced in CI by `cargo xtask layers`]

---

## 4. Key dependencies

- **Cargo.lock**: **728 packages**. There are **no git dependencies** (`source = "git` occurs 0 times) and **no `[patch]` sections**. [VERIFIED]
- **Toolchain**: there is **no `rust-toolchain.toml`**. CI and release use `dtolnay/rust-toolchain@stable` (floating). MSRV in `Cargo.toml` is 1.95, and the Dockerfile default is 1.99. [VERIFIED]

### 4.1 Top-level third-party crates (resolved versions from Cargo.lock)

| Area | Crates |
|---|---|
| UI / windowing | eframe 0.36.2, egui 0.36.2, egui_extras 0.36 (svg), egui_kittest 0.36 (dev), winit =0.30.13, rfd 0.17.2, arboard 3.6.1, accesskit (via eframe), sys-locale =0.3.2, open 5, ron 0.12 |
| GPU | wgpu 30.0.1, naga 30.0.1, pollster (dev) |
| macOS | objc2 =0.6.4, objc2-foundation/app-kit =0.3.2, block2 =0.6.2, muda =0.21.1, dispatch2 =0.3.1, core-graphics2 =0.6.1, fmv-macos-events =0.1.0, rustix (format) |
| Linux | x11rb =0.13.2, zbus =5.19.0 (portal colour picker), futures-lite, async-io |
| Windows | xcap =0.9.8 (screen capture, Windows-only target dep), winsafe =0.0.29, winresource 0.1 (build) |
| Image codecs | image 0.25.10 (gif, bmp, tga, ico, qoi, hdr; avif optional via ravif and rav1e), png 0.18, tiff 0.11, exr 1.74, image-webp 0.2, zune-jpeg 0.5, jpeg-encoder 0.6, half 2, heic-rs =0.1.1 (optional), jxl-oxide =0.12.6 (raw, native), resvg/usvg 0.45 |
| Text | parley 0.11.1, skrifa 0.44, fontique 0.11 (via parley), harfrust 0.12, ttf-parser 0.25 (via resvg), fontdb 0.23 |
| Compression / hash | flate2 1.1 (rust_backend; miniz_oxide or zlib-rs), ruzstd 0.9, blake3 1.8 (`pure`), crc32fast |
| Algorithms | rayon 1.12, rustfft 6.4.1 |
| Automation | rmcp 3.5.0 (server, macros, transport-io), tokio 1.53, schemars 1.2, base64 0.22, cap-std 4, getrandom 0.3 |
| Plugins | wasmi 2.0.0 (portable-dispatch); wat (dev) |
| Data | serde 1, serde_json 1 (`unbounded_depth` in format), roxmltree 0.20, toml 1, thiserror 2 |
| Dev/test | proptest 1, criterion 0.5, moxcms 0.8, memory-stats 1.2 |

[VERIFIED: per-crate manifests plus Cargo.lock]

### 4.2 Native C/C++ and `-sys` crates

`-sys` packages in the lock: clang-sys, core-foundation-sys, drm-sys, gbm-sys, jni-sys, js-sys, libfuzzer-sys, libspa-sys, linux-raw-sys, ndk-sys, objc-sys, pipewire-sys, renderdoc-sys, wayland-sys, web-sys, windows-sys. [VERIFIED: Cargo.lock]

- `pipewire-sys`, `libspa-sys` (needs **bindgen and clang**) and `gbm-sys` come in **only through `xcap`**, which is declared for `cfg(target_os = "windows")` only (`apps/photocraft/Cargo.toml`). They are lockfile entries and should not compile on Windows either, because the PipeWire backend is Linux-only inside xcap. [VERIFIED chain via Cargo.lock reverse walk; actual compilation per target: NEEDS TESTING]
- `libfuzzer-sys` comes through `rav1e` (optional AVIF feature of `image`) and through the fuzz crates. [VERIFIED]
- `cc` is pulled by `blake3`, but blake3 uses `features=["pure"]`, so no C should be compiled. [ASSUMPTION; NEEDS TESTING]
- `renderdoc-sys` and `wayland-sys` are dlopen-style bindings via wgpu-hal and winit, and `libloading` is used by x11rb and glutin. Runtime system libraries on Linux: libxkbcommon, wayland, X11/Xi/Xrandr, GL/Vulkan, GTK3 (rfd dialogs). FreeBSD needs the same set plus `pkg` (README). [VERIFIED: `ci.yml` apt list, README]
- **Build scripts**: `crates/text/build.rs` (embeds craft-fonts from `CRAFT_FONTS_DIR`), `crates/ui-egui/build.rs` (compiles credits and i18n catalogs), `apps/photocraft/build.rs` and `apps/photocraft-cli/build.rs` (Windows VERSIONINFO and icon via winresource; CompanyName "Learning Machines LLC"). None of them fetch from the network. [VERIFIED]
- **External build input**: `storytold/craft-fonts` is checked out at pinned SHA `abb83316…` in CI and release. It is optional, but "all official releases" use it. [VERIFIED: `ci.yml`, `release.yml:36`]
- **Lockfile licence scan**: I scanned the 260 of 728 packages that are in the local cargo cache. All are MIT, Apache, BSD, Zlib, ISC, Unlicense or CC0 options. Exceptions to note: `self_cell` (Apache-2.0 OR GPL-2.0-only, so choose Apache); `unicode-ident` (Unicode-3.0); and **`epaint_default_fonts` ((MIT OR Apache) AND OFL-1.1 AND Ubuntu-font-1.0)**. The last one embeds Ubuntu-Light, Hack, NotoEmoji and emoji-icon-font into the binary through eframe `default_fonts`. The other 468 packages were not inspected. [VERIFIED partial; full graph: NEEDS TESTING with `cargo deny check licenses`]

---

## 5. Features and limitations

### 5.1 Claimed (README, docs, book)

53 tools; 16 adjustment layers; layer styles (all ten Photoshop effect types); smart objects and smart filters; 27 PS blend modes plus 6 Paint.NET; RGB, Gray, CMYK and Lab at 8, 16 and 32 bit; ICC management with soft proof and gamut warning on the GPU; Free Transform, warp, puppet warp, liquify; content-aware fill and move, healing; Camera Raw; Actions and droplets; command palette; 627 of 627 Photoshop menu items wired. Formats: PSD and PSB, layered TIFF, PDN3 (read), ORA (read and write), SVG (import), PNG, JPEG, TIFF, WebP, GIF, BMP, TGA, ICO, QOI, PNM, EXR, HDR, `.pcraft`, HEIC (read, feature), AVIF (write, feature) and Affinity (read). JPEG XL and AVIF decode are not supported. [VERIFIED: README.md lines 85-251]

### 5.2 The upstream project's own assessment

[VERIFIED: `docs/roadmap.md` §"Honest parity assessment", `docs/scorecard.md`]

- "Real Photoshop parity is still **well below 50%**." Feature surface is about 60-70% "in some form", and "a professional could switch for daily work" about 25-35%.
- There are no AI or generative features, by decision (#41). Missing tools include Red Eye (listed elsewhere), Art History Brush, Freeform and Curvature Pen, Color Sampler, Perspective Crop, type masks and Frame.
- The scorecard summary:
  - Performance: 3 of 56 scenarios within budget, 22 over budget.
  - P12, a nudge in a 150-layer document, **panics** with a wgpu validation error (`Buffer 'pc_compose_uniforms' is invalid`).
  - P28 Content-Aware Scale takes 954 s against a 3 s budget.
  - Settings that do nothing: 46 of 149.
  - Type checklist: 0 done, 3 partial, 6 missing.
- PSD oracle results:
  - io corpus render match: at least 146/170.
  - psd-tools corpus: at least 236/309 render match and at least 307/309 round trip.
  - Photoshop-authored oracles: at least 133/256 render match. The round-trip floor reads "≥ 258/256", which is internally inconsistent and looks like a generator bug.
  - Smart-filter re-render matches only 5 of 30.
- On Wayland, drag-and-drop does not work (winit 0.30; README). Native Wayland pen input is missing (#79), and macOS tablet support has not been verified on hardware.
- 214 shortcut failures were found by the first users (since fixed); "most fixes … not yet validated by users".

### 5.3 Spot checks against source

| Claim | Evidence | Result |
|---|---|---|
| 27 PS blend modes + 6 Paint.NET | `crates/color` `BlendMode`: Normal…Luminosity (27) + Reflect, Glow, Negation, Xor, PaintNetColorBurn, PaintNetColorDodge (6) + PassThrough | [VERIFIED] |
| Pure-Rust ICC CMS | `crates/cms/src/{profile,transform,pipeline,clut,lut3d,write}.rs`; moxcms is dev-only | [VERIFIED] |
| OpenRaster read/write | `crates/io/src/ora.rs` (uses in-house `photocraft_format::zip`), HEAD commit #2257 | [VERIFIED] |
| PDN3 read | `crates/io/src/pdn.rs` + `pdn/nrbf.rs` (in-house .NET binary-serialization reader) | [VERIFIED] |
| WebP lossy write | In-house VP8 encoder `crates/codecs/src/codecs/vp8/` (RFC 6386) | [VERIFIED] |
| HEIC read optional | `crates/heif` wraps `heic-rs =0.1.1` under `catch_unwind`, behind feature `heif` | [VERIFIED] |
| WASM plug-ins sandboxed | `crates/plugins` wasmi with `default-features=false`, no host imports | [VERIFIED code; sandbox strength: NEEDS TESTING] |
| 627/627 menu items | `docs/parity.md` (generated); the upstream says this measures wiring, not behaviour | [VERIFIED as generated; behaviour NEEDS TESTING] |
| "500+ commands" | 251 literal `CommandSpec {` sites and 286 distinct dotted ids in `*_cmds.rs`; others are generated programmatically | [NEEDS TESTING: `photocraft-cli commands --json \| jq length`] |
| "1,700+ tests" | 5,460 `#[test]` functions (README understates) | [VERIFIED] |
| Translations "10 UI languages" (roadmap) | 16 TSV catalogs in `crates/ui-egui/src/i18n/` | [VERIFIED; docs stale] |

---

## 6. Build

- **Desktop, all OSes**: `cargo run --release -p photocraft` (README). Official builds add `--features heif` and `CRAFT_FONTS_DIR`. [VERIFIED]
  - **Linux**: system libraries from the apt list in `ci.yml`: libxkbcommon-dev, libwayland-dev, libx11-dev, libxrandr-dev, libxi-dev, libgl1-mesa-dev, libgtk-3-dev. Release packaging also uses desktop-file-utils, appstream, zsync, nfpm and appimagetool. Release builds run on x86_64 and aarch64 runners. [VERIFIED]
  - **macOS**: universal binary (`packaging/macos/package.sh --arch universal`) on `macos-15-xlarge`. Signing uses a Developer ID ("Learning Machines LLC (DJ6XS33FX8)") with notarization and a hardened runtime with empty entitlements (`packaging/macos/entitlements.plist`). Minimum target is macOS 11 (Cargo comment). [VERIFIED]
  - **Windows**: x64, x86 and arm64 MSVC targets. MSI is built with WiX v5.0.2; a portable zip is also produced. Signing uses either a PFX or Azure Trusted Signing (`packaging/windows/sign.ps1`, `release.yml:165-178`). [VERIFIED]
  - **FreeBSD 14 x86_64**: built in a `vmactions/freebsd-vm` VM (SHA-pinned). The release produces a tarball laid out like `/usr/local`. It is tested nightly by `freebsd.yml`; UI integration tests run with `--test-threads=1`. [VERIFIED]
- **Toolchain pin**: none (floating stable). MSRV is 1.95, so a fork should add `rust-toolchain.toml`. [VERIFIED]
- **WebAssembly**:
  - Tooling: trunk 0.21.14 plus wasm-bindgen 0.2.129 and `wasm-opt -Oz`. Target: `wasm32-unknown-unknown`. Profile: `wasm-release` (`apps/photocraft-web/index.html:7`, `Trunk.toml`). [VERIFIED]
  - Size: about 19 MiB raw, 8 MiB gzip, 5.6 MiB Brotli. `packaging/web/package.sh` fails the build above 24 MiB (Cloudflare Pages caps files at 25 MiB). [VERIFIED: `packaging/web/README.md`, Cargo.toml comment]
  - Hosting: static files only (Netlify or Cloudflare `_headers`, Apache `.htaccess`, nginx), or the Dockerfile. No COOP/COEP is needed because there is no SharedArrayBuffer. WebGPU needs HTTPS or localhost and falls back to WebGL2 otherwise; `?webgl` forces the fallback. [VERIFIED]
  - Browser limits: there are no threads, because rayon is native-only, so filters and jobs run synchronously on the main thread. Preferences use localStorage and presets use IndexedDB (`photocraft.presets`). There are no system fonts (Inter and JetBrains Mono are bundled; craft-fonts is not embedded on the web). Open uses a file picker and Save downloads a file. wasm32 caps memory at 4 GiB, which limits large documents. [VERIFIED except the memory cap: ASSUMPTION]
  - CI: `cargo xtask wasm` checks L0-L6 for the wasm target, and the `web` job builds and size-gates the release. [VERIFIED]
- **xtask commands**: `layers`, `wasm`, `ci`, `corpus [--all|--psd-tools…]`, `test-corpus [--changed]`, `stats`, `parity`, `perf [--quick|--update-baseline]`, `scorecard [--check]`, `version`, `ico`. [VERIFIED]
- **Docker**: a web-only multi-stage build (see §2.4). Non-root nginx; thin LTO by default; 1 build job. The upstream calls it community-maintained and does not test it in CI. [VERIFIED]
- **Packaging identities** (all must change for the fork):
  - App ID and bundle ID: `ai.storyteller.photocraft` (`apps/photocraft/src/main.rs:54`, `packaging/macos/Info.plist.in`, Flatpak `id`, `.desktop`, metainfo, icon names).
  - UTTypes: `ai.storyteller.photocraft.document`, `.ora`, `.pdn`, `.qoi`.
  - Windows: MSI `UpgradeCode D9660F32-2D0A-4ABE-B73D-17BE25F1542C`; Manufacturer "Learning Machines LLC"; ProgIDs `Photocraft.PhotoshopDocument` and `Photocraft.Image`; registry key `HKLM\Software\PhotoCraft`.
  - MIME type: `application/x-photocraft` (`*.pcraft`).
  - nfpm: vendor "Learning Machines LLC", homepage getartcraft.com.
  - AppImage update info: `gh-releases-zsync|storytold|photocraft|latest|…`, overridden by `GITHUB_REPOSITORY` in forks (`packaging/linux/package.sh:128`).

  [VERIFIED]
- **Signing secrets** (all optional; without them the artifacts are unsigned): APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD, KEYCHAIN_PASSWORD, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID, WINDOWS_CERTIFICATE, WINDOWS_CERTIFICATE_PASSWORD, AZURE_TENANT_ID, AZURE_CLIENT_ID, AZURE_CLIENT_SECRET, AZURE_SIGNING_ENDPOINT, AZURE_SIGNING_ACCOUNT, AZURE_CERT_PROFILE. Releases are drafted with `gh release` and `SHA256SUMS.txt`. [VERIFIED: `release.yml`]
- Whether any of this builds on our side: [NEEDS TESTING]. Nothing was compiled during this review.

---

## 7. Tests

- **`#[test]` count: 5,460.** By crate: ui-egui 1,675; engine 1,237; algo 408; io 377; codecs 325; psd 263; compose 190; text 112; raw 112; app 102; format 92; paint 81; gpu 60; doc 56; automation 56; xtask 53; cms 48; vector 45; cli 27; color 24; plugins 19; affinity 19; raster 18; tablet 16; geom 14; web 12; ops 11; testkit 4; heif 4. There are 52 `#[ignore]` markers and 17 `proptest!` blocks in 12 files. [VERIFIED: grep]
- **Integration test directories**: `crates/io/tests` (49 files), `crates/engine/tests` (36, including `panic_hunt.rs` and `photoshop_oracles.rs`), `crates/codecs/tests` (21), `crates/ui-egui/tests` (21, using egui_kittest with wgpu: GPU canvas, device loss, colour-managed canvas), `crates/psd/tests` (7), `crates/format/tests` (7), `crates/automation/tests` (6: `mcp.rs`, `mcp_wire.rs`, `agent_tasks.rs`), `crates/raw/tests` (6), `crates/cms/tests` (3), `crates/gpu/tests` (3, CPU-vs-GPU parity), and `apps/photocraft/tests` (3, including a macOS AppKit menu test). [VERIFIED]
- **Golden-image and oracle testing**: the corpus tests are opt-in (cargo feature `corpus`). They compare against Photoshop's own merged images, psd-tools, ag-psd, PngSuite, EXR, HEIF, Affinity and pixls.us raws. The corpora are pinned and sha256-verified (`xtask/*.sha256`, `xtask/src/corpus_pins.rs`), and corpus floors live in `crates/io/tests/corpus.rs`. `egui_kittest` snapshots and the `snapshot` example render the UI offscreen. [VERIFIED]
- **E2E**: no E2E tests drive a real windowed app in CI. The control channel and MCP tests run headless; `agent_tasks.rs` has 10 MCP tasks against a 30-50 target (scorecard FILE-216-6, partial). [VERIFIED]
- **Fuzzing**: there are **16 cargo-fuzz targets**:
  - psd: parse, descriptor, image_source_data, abr
  - codecs: decode_any, png, jpeg, tiff, exr, heif
  - format: load
  - raw: develop, ljpeg
  - affinity: preview, import_preview, stream

  Of these, CI's `fuzz` job lists 12, and it runs **only on `workflow_dispatch` with `fuzz=true`** ("Placeholder: manual trigger only"). The engine's `panic_hunt` test fuzzes every command with adversarial params, but it is `--ignored` and run by hand (AGENTS.md §5). [VERIFIED: `ci.yml` fuzz job]
- **Perf**: `cargo xtask perf` and `perf-nightly.yml` run on `macos-15` nightly. A broken enforced budget, or a p50 regression over 15%, fails the run. Criterion is a dev-dependency of compose. Many benchmark examples live in `crates/ui-egui/examples/*_bench.rs`. [VERIFIED]
- **CI workflows**:

| Workflow | Trigger | OS / runners | Jobs |
|---|---|---|---|
| `ci.yml` | push main, PR, dispatch | ubuntu-latest and windows-latest (Windows on main, or for PRs touching platform code); **no macOS** | platforms, test (fmt, `cargo test --workspace --features heif`, no-heif tests, craft-fonts tests, clippy `-D warnings`, `xtask layers`, `scorecard --check`), corpus, wasm, web (trunk build plus size gate), fuzz (manual) |
| `release.yml` | push `release` branch, dispatch | macos-15-xlarge, windows-latest ×3 arch, Linux x86_64 and aarch64, FreeBSD VM, ubuntu (web, Flatpak, draft release) | build, sign, notarize, package, checksums, draft GitHub release |
| `freebsd.yml` | nightly cron, PR on paths | ubuntu with vmactions/freebsd-vm | build, test |
| `windows-arm64.yml` | nightly cron, PR on paths | windows-latest and windows-11-arm | cross-build, install MSI, run CLI natively |
| `perf-nightly.yml` | nightly cron, dispatch | macos-15 | perf against budgets |
| `docs.yml` | doc paths | ubuntu | mdbook build |
| `packaging-lint.yml` | packaging and workflow paths | ubuntu | actionlint, shellcheck, desktop-file-validate, appstream |

[VERIFIED]

- **Scorecard**: the `scorecard/` directory and `docs/scorecard.md` hold the project's internal feature and performance scorecard. **OpenSSF Scorecard is not configured.** [VERIFIED]
- Whether the test suite is actually green at HEAD: [NEEDS TESTING].

---

## 8. Security

### 8.1 SECURITY.md (summary)

- It treats files, paths, metadata, automation requests and command parameters as potentially malicious.
- **It names no private reporting channel or email.** Reporters are asked to contact the maintainers and arrange a private transfer method.
- Scope covers parsers (PSD, raster, `.pcraft`, ICC, LUT, pattern, embedded objects), bombs, overflow, path traversal and symlinks, command validation, the control, JSON, MCP and bridge servers, authentication, request limits, GPU resources, dependencies, CI and signing.
- **Stated limitations**: the TCP control channel requires a 256-bit bearer token, but there are no per-client or per-tool capabilities. "JSON-depth, render, document-memory, command-duration, client-identity, encryption, and audit controls are not yet implemented."

[VERIFIED: `SECURITY.md`; the same posture table is in `book/src/security/overview.md`]

### 8.2 File-parsing surface

| Format | Implementation | Tag |
|---|---|---|
| PSD/PSB, descriptors, ABR, ACO, ASE, GRD, PAT, PSD-in-TIFF | **In-house** `crates/psd` | [VERIFIED] |
| `.pcraft` ZIP and manifest | **In-house** ZIP reader and writer (`crates/format/src/zip.rs`), serde_json with `unbounded_depth` plus a depth pre-check | [VERIFIED] |
| ORA (ZIP) | In-house via `photocraft_format::zip` | [VERIFIED] |
| PDN3 (.NET NRBF + gzip) | **In-house** `crates/io/src/pdn/nrbf.rs` | [VERIFIED] |
| Affinity `.af*` | **In-house** `crates/affinity` (crc32fast, flate2, ruzstd) | [VERIFIED] |
| Camera raw (DNG, CR2, NEF, ARW, RW2, ORF, RAF, LJPEG, opcodes) | **In-house** `crates/raw`; JPEG XL tiles in DNG via `jxl-oxide =0.12.6` | [VERIFIED] |
| TIFF / BigTIFF IFD | **In-house** `tiff_ifd.rs`, plus the `tiff` crate for strip and tile decode | [VERIFIED] |
| ICC profiles, .cube/.3dl/.look LUTs | **In-house** `crates/cms` | [VERIFIED] |
| PNG | `png` 0.18 | [VERIFIED] |
| JPEG | `zune-jpeg` 0.5 to decode, `jpeg-encoder` 0.6 to encode | [VERIFIED] |
| WebP | `image-webp` 0.2 to decode, in-house VP8 to encode | [VERIFIED] |
| GIF, BMP, TGA, ICO, QOI, HDR | `image` 0.25 | [VERIFIED] |
| EXR (including deep and cryptomatte) | `exr` 1.74 plus in-house deep handling | [VERIFIED] |
| HEIC | `heic-rs =0.1.1` (very young; panic-wrapped) | [VERIFIED] |
| SVG | resvg/usvg 0.45 (with `system-fonts` and `raster-images` on native) | [VERIFIED] |
| Fonts | skrifa/read-fonts, fontique, ttf-parser, fontdb | [VERIFIED] |
| XMP / XML, Photoshop `.kys`/`.psp` shortcuts | roxmltree 0.20 | [VERIFIED] |
| WASM plug-ins | wasmi 2.0 | [VERIFIED] |

Default codec limits are 262,144 px per side, 2^30 pixels and an allocation cap (`crates/codecs/src/options.rs:21-26`). AGENTS.md rules call for `get()` over indexing and for checked arithmetic, and clippy denies unwrap and expect in production code. [VERIFIED]

### 8.3 `unsafe`

- `unsafe_code = "forbid"` at workspace level, plus `#![forbid(unsafe_code)]` in each crate's lib.rs. [VERIFIED]
- **3 `unsafe` blocks** in production code, all in `crates/tablet/src/macos.rs` (lines 43, 59 and 69: the NSEvent monitor). The crate is `unsafe_code="deny"` and the file is `#![allow(unsafe_code)]`. [VERIFIED]
- The example plug-in `examples/plugins/invert-rs` uses `#[unsafe(no_mangle)]` and `from_raw_parts_mut`. It is not part of the workspace build. [VERIFIED]
- Transitive `unsafe` in wgpu, winit, objc2, wasmi and others is not audited. [ASSUMPTION]

### 8.4 Network access, telemetry and updates

- **No HTTP client crate is in the lock** (no reqwest, ureq, hyper, rustls, native-tls or sentry). I found no telemetry, analytics or update-check code. The only outbound actions are user-clicked links opened with the `open` crate: getartcraft.com, discord.gg/artcraft and github.com/storytold (`crates/ui-egui/src/links.rs`). [VERIFIED: Cargo.lock grep, source grep]
- **Update mechanism**: only the AppImage zsync update info for external AppImageUpdate, which points at `storytold/photocraft` GitHub releases unless `GITHUB_REPOSITORY` is set. There is no in-app updater. [VERIFIED]
- **Listening sockets**:
  - The desktop app opens a server only with `--control <port>` or `PHOTOCRAFT_CONTROL_PORT`. It binds `127.0.0.1` and requires a 64-hex token. The comparison is constant-time (`crates/automation/src/security.rs:96`). Unix token files are created with mode 0600. Limits: 1 MiB request lines, 8 MiB replies, 16 connections.
  - `photocraft-cli serve --port` binds `127.0.0.1` too (`apps/photocraft-cli/src/lib.rs:455`), and the bridge refuses non-loopback addresses (`crates/automation/src/bridge.rs:35`).
  - Risks: a token passed with `--control-token` is visible in the process list. On Windows the token file gets no ACL hardening (only `cfg(unix)` sets the mode). Any local process with the token gets full control.

  [VERIFIED code; exploitability NEEDS TESTING]

### 8.5 MCP surface

- Transport: stdio only (`rmcp` `transport-io`). 21 `#[tool]` handlers in `crates/automation/src/server.rs`: `session_list`, `doc_open`, `doc_new`, `doc_save`, `doc_export`, `doc_inspect`, `doc_render_preview`/`render_preview`, `doc_select`, `doc_close`, `command_list`, `command_run`, `command_batch`, `jobs_list`, `jobs_cancel`, `ui_inspect`, `ui_screenshot`, `ui_pointer`, `ui_menu_invoke`, `ui_set` and `control_call`. Resources: `photocraft://document` and `photocraft://commands`. [VERIFIED]
- Filesystem access is limited to `--automation-read-root` and `--automation-write-root` (cap-std, `crates/automation/src/workspace.rs`); with no root, there is no file access. Engine commands that take ambient paths are denied at the automation boundary. Batches are capped at 256 steps. [VERIFIED: `docs/mcp.md`, SECURITY.md]
- Gaps: there is no per-tool authorisation, and `control_call` is a generic passthrough to the GUI control surface. [VERIFIED; NEEDS TESTING for bypasses]

### 8.6 Supply chain

- **GitHub Actions pinning is mixed.** Most uses are tag-pinned: `actions/checkout@v4` ×18, `Swatinem/rust-cache@v2` ×10, `dtolnay/rust-toolchain@stable` ×10 and `@nightly`, upload and download-artifact `@v4`, `actions/cache@v4`. SHA-pinned uses: checkout ×5 (craft-fonts steps), rust-cache ×1, rust-toolchain ×1 and vmactions/freebsd-vm. `ci.yml` has **no top-level `permissions:`** (default token scope); the other workflows set `contents: read`, and only the draft-release job has `contents: write`. [VERIFIED]
- **Release tool downloads are not verified**:
  - The appimagetool `continuous` build is fetched with **no checksum and no version pin** (`packaging/linux/package.sh:113`).
  - nfpm `.deb` and trunk tarballs are fetched with curl and no checksum (`release.yml:216, 359`; `ci.yml` web job). Only the Dockerfile verifies trunk's sha256.
  - The FreeBSD release job runs `curl … sh.rustup.rs | sh`.

  [VERIFIED]
- **There is no `deny.toml`, `audit.toml`, cargo-vet, dependabot or SBOM**. The project acknowledges this in `book/src/development/dependencies.md:7`. [VERIFIED]
- There are no git dependencies or patches. Several platform crates are pinned with `=x.y.z` and "review the diff before bumping" comments. [VERIFIED]
- **Private dependency**: `AGENTS.md` refers to `../craftrules`, which it says is "not public", and to `craftrules/scripts/contributors.py`. A fork will not have these standards or tooling. [VERIFIED]

---

## 9. Licensing

- **Code**: dual MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `Cargo.toml` `license`). Copyright: "(c) 2026 ArtCraft Team and the PhotoCraft contributors". There is **no CLA or DCO**. With 175 contributors, inbound licensing rests on the implicit inbound=outbound convention of GitHub's terms. [VERIFIED; legal sufficiency: ASSUMPTION]
- **NOTICE (quoted)**:
  > PhotoCraft
  > Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors.
  > Licensed under the MIT License or the Apache License, Version 2.0, at your option (LICENSE-MIT, LICENSE-APACHE).
  >
  > The ArtCraft name, wordmark and logos in docs/brand/ are trademarks of the ArtCraft Team. They are not open source and are not covered by the license above; their terms are in docs/brand/LICENSE-brand.txt.
  >
  > Third-party material … Inter font (assets/fonts/Inter-*.ttf) — SIL Open Font License 1.1 … JetBrains Mono font … SIL OFL 1.1 … Lucide icons (assets/icons/*.svg, except slice-knife.svg) — ISC; icons derived from Feather also MIT … SCOWL English word list … SCOWL license.
  > Builds made with craft-fonts (CRAFT_FONTS_DIR; all official releases) also embed the fonts listed in https://github.com/storytold/craft-fonts/blob/main/ATTRIBUTION.md (today BIZ UDPGothic, Shippori Mincho and BIZ UDMincho; not the web build), under the SIL Open Font License 1.1.

  [VERIFIED: `NOTICE`]
- **ATTRIBUTION.md entries** [VERIFIED]:
  - **UI translations** (el, de, it, ko, nl, pl, pt-br, uk; contributors' own work, MIT/Apache, `LICENSE-translations.txt`). **Gap**: cs, es, fr, id, ja, ru, zh-hans and zh-hant exist but are not listed.
  - **Fonts**: Inter 4.001 (OFL); JetBrains Mono 2.305 (OFL).
  - **Icons**:
    - 110 Lucide icons (ISC), with a Feather-derived subset (MIT).
    - Original glyphs (MIT/Apache): slice-knife, clip-below, clip-release, layer-mask, adjustment-layer.
    - Lucide-derived glyphs (ISC): eraser-background, eraser-magic, bandage-sparkle, direct-select, painting-symmetry.
    - symmetry-* axes by @shorbank (MIT/Apache).
    - **lasso-magnetic.svg**: adapted from Noun Project, "Lil' Seal", **CC BY 3.0**. This requires visible attribution.
  - **Dictionary**: SCOWL en_US size 50 (SCOWL licence).
  - **ICC**: `photocraft-coated-cmyk.icc`, synthetic, **CC0**; other profiles and LUTs are generated at runtime (CC0).
  - **App icon**: nine-tailed kitsune, original work by the project owner, MIT/Apache (`assets/app-icon/LICENSE.txt`). It is **not** under the brand licence, but it is the de facto product identity.
  - **DMG background**: by @XusBadia, MIT/Apache.
  - **Theme**: Solarized palette (MIT).
  - **craft-fonts** (external): BIZ UDPGothic, Shippori Mincho, BIZ UDMincho (OFL).
  - **Documentation**: ArtCraft brand (proprietary). Screenshots are MIT/Apache, and the artworks in them are public domain (Hokusai, Vermeer, Monet, van Gogh, Klimt, Bierstadt, NASA Earthrise, Ansel Adams/NARA).
  - **Test corpora** (not committed): photocraft-corpus (MIT/Apache, authored with Adobe Photoshop 2026); psd-tools (MIT); ag-psd (MIT); heic-rs (MIT/Apache); pillow-heif (BSD-3); OpenEXR images (BSD-3); Affinity samples (CC0 and MIT); PngSuite (PD); pixls.us raws (CC0).
- **Gaps** [VERIFIED]:
  - **`epaint_default_fonts`** embeds Ubuntu-Light (Ubuntu Font Licence 1.0), Hack, NotoEmoji (OFL) and emoji-icon-font (MIT) through eframe `default_fonts`, and none of them is credited in NOTICE or ATTRIBUTION.
  - There is no aggregated licence file for third-party Rust crates (no cargo-about) in any package.
  - `packaging/env.sh` `copy_docs` ships README and the LICENSE files but **not NOTICE or ATTRIBUTION.md** in the Linux, macOS-CLI and web zips. The Windows `package.ps1:154` does the same. Only FreeBSD (`packaging/freebsd/package.sh:83`) and the Dockerfile ship NOTICE and ATTRIBUTION.
- **Copyleft**: no GPL, LGPL, MPL or AGPL crate was found in the inspected 36% of the lockfile, and `self_cell` offers Apache-2.0. The upstream policy requires permissive third-party assets. **There is no `deny.toml`** to enforce this. [VERIFIED partial; NEEDS TESTING full graph]
- **Trademark statements**:
  - Adobe: README footer: "Adobe, Photoshop, Illustrator, … are trademarks … PhotoCraft is an independent, open-source project and is not affiliated with … Adobe Inc." [VERIFIED: README:424]
  - ArtCraft: proprietary under `docs/brand/LICENSE-brand.txt`. "**If you distribute or publish a modified version … you must remove the ArtCraft Marks** … and must not call it ArtCraft … You may say, in plain text, that your work is based on PhotoCraft by the ArtCraft team." [VERIFIED]
  - "PhotoCraft" itself is not explicitly claimed as a trademark. Renaming is still prudent. [VERIFIED absence of a claim; legal position: ASSUMPTION]
  - "Storytold" has no trademark statement. "storyteller" appears in the reverse-DNS IDs.
- **Clean-room claims**:
  - README: "clean-room reimplementation of Adobe Photoshop … implemented from public specs and observed behaviour only. No proprietary code, shaders or assets."
  - The raw crate states that it used no code from dcraw, LibRaw, rawspeed, rawler, rawloader or darktable. Its camera matrices come from the project's own measurements (`crates/raw/src/lib.rs`, `cameras.rs`).
  - The PSD crate admits following MIT psd-tools and ag-psd behaviour.
  - The menu tree mirrors Photoshop's (`crates/ui-egui/src/menu_catalog.rs`). The app imports Photoshop's live shortcut file (`apps/photocraft/src/photoshop_settings.rs`) and declares Adobe's `com.adobe.photoshop-large-image` UTType. The oracle corpus was produced with licensed Photoshop 2026.
  - Clean-room status cannot be proven from an AI-generated history. Model training provenance is not verifiable. [ASSUMPTION]
  - The UI deliberately imitates Photoshop's trade dress ("If you know Photoshop, you already know PhotoCraft"). That warrants legal review for a commercial fork. [ASSUMPTION]

---

## 10. Branding surface

Counts are from `git grep -o` (occurrences / files) [VERIFIED]:

| String | Occurrences | Files |
|---|---:|---:|
| `photocraft` (lowercase: crates, binaries, ids, paths) | 8,845 | 969 |
| `Photocraft` | 2,799 | 275 |
| `PhotoCraft` | 1,042 | 203 |
| `PHOTOCRAFT` (env vars) | 244 | 63 |
| `pcraft` (extension) | 299 | 107 |
| `storytold` | 314 | 53 |
| `ArtCraft` / `artcraft` | 178 / 50 | 36 / 8 |
| `storyteller` (reverse DNS) | 85 | 25 |
| `getartcraft` | 33 | 7 |
| `Learning Machines` | 8 | 6 |
| `craft-fonts` / `craftrules` | 114 / 46 | 27 / 27 |

Structural locations [VERIFIED]:

- **Window title and app ID**: `apps/photocraft/src/main.rs:54` (`APP_ID = "ai.storyteller.photocraft"`) and `:79` (`.with_title("PhotoCraft")`); `crates/ui-egui/src/native_menu.rs:30` (`APP_NAME = "PhotoCraft"`); `panels::sync_window_title`.
- **Menus and About**: `crates/ui-egui/src/menus.rs:77-79` ("Join the ArtCraft Discord…", "ArtCraft Website"); `crates/ui-egui/src/links.rs:9-12` (DISCORD, ARTCRAFT_WEBSITE, APP_PAGE, GITHUB, ISSUES); a Discord button in `canvas.rs:2065`, `dialogs.rs:358` and `panels.rs:582-587`; About credits from `contributors/*`.
- **i18n**: all 16 `crates/ui-egui/src/i18n/*.tsv` contain ArtCraft or PhotoCraft strings.
- **Icons**: `assets/app-icon/*` (photocraft.svg, .icns, .ico, hicolor `ai.storyteller.photocraft.png`). The DMG background in `packaging/macos/dmg/` is derived from the icon. `docs/brand/*` holds the ArtCraft logos, which are proprietary and must be removed.
- **Splash and loading screen**: the web loading screen in `apps/photocraft-web/index.html` (`photocraft_canvas`, `photocraft_loading`, "If PhotoCraft doesn't start…").
- **Installer metadata**:
  - `packaging/macos/Info.plist.in` (CFBundleName and Identifier, UTTypes).
  - `packaging/windows/photocraft.wxs` and `photocraft.en-us.wxl` (Name, Manufacturer, UpgradeCode, ProgIDs, registry keys).
  - `packaging/linux/ai.storyteller.photocraft.{desktop,metainfo.xml.in,mime.xml}`; `flatpak/*.yml`; `nfpm.yaml` (vendor, homepage, maintainer env).
  - Windows VERSIONINFO in `apps/*/build.rs`.
  - Code-signing identity "Learning Machines LLC (DJ6XS33FX8)".
- **Config and data directories**: `apps/photocraft/src/app_dirs.rs` uses `~/Library/Application Support/Photocraft`, `%APPDATA%\Photocraft` and `~/.config/photocraft`. Portable mode uses `PhotoCraftData`, `PhotoCraft.portable` and `portable.txt`. Env override is `PHOTOCRAFT_CONFIG_DIR`. Web storage uses localStorage `photocraft.preferences` and IndexedDB `photocraft.presets`.
- **File extensions and formats**: `.pcraft` (`crates/format/src/lib.rs:50`), `.pcdroplet`, and MIME `application/x-photocraft`.
- **Environment variables**: 56 distinct `PHOTOCRAFT_*` variables, for example CONFIG_DIR, CONTROL_PORT, CONTROL_TOKEN(_FILE), AUTOMATION_READ/WRITE_ROOT, NO_DESKTOP_INTEGRATION, BUILD_SHA, VERSION and MAINTAINER.
- **URLs**: getartcraft.com, discord.gg/artcraft and github.com/storytold/{photocraft, craft-fonts, photocraft-corpus}; the README star-history badge and raw.githubusercontent icon links; the AppImage zsync default repo.
- **MCP resources**: `photocraft://document` and `photocraft://commands`.
- **Telemetry**: none found.
- **Crate names**: 25 `photocraft-*` packages plus the binaries `photocraft`, `photocraft-cli` and `photocraft-web`.

**Suggested isolation approach**:

1. Introduce a single `crates/brand` (L0) crate, or a `brand.toml` read by `build.rs`. It would expose PRODUCT_NAME, APP_ID, VENDOR, URLs, the config-dir name, ENV_PREFIX, the MIME type and file extensions. Then replace the literals in `main.rs`, `native_menu.rs`, `links.rs`, `app_dirs.rs`, web `index.html` and the `build.rs` VERSIONINFO.
2. Template the packaging files from the same source: `packaging/env.sh` already exports VERSION, so add BRAND_* variables, and use `.in` templates for the `.desktop`, metainfo, MIME, wxs, nfpm and Flatpak files.
3. Generate a new MSI UpgradeCode, bundle ID and UTTypes.
4. Delete `docs/brand/` and the ArtCraft and Discord menu items, links and buttons.
5. Replace the app icon and DMG art. This is optional legally (both are MIT/Apache), but strongly advised for identity.
6. Keep reading `.pcraft` for compatibility, and decide whether to adopt a new extension.
7. Keep the crate names `photocraft-*` internally at first to limit churn. Rename later with a scripted `sed` plus `cargo xtask layers` (`xtask/src/layers.rs` lists crate names).
8. Add a backward-compatible env alias layer (`UBUNYE_*` falling back to `PHOTOCRAFT_*`).
9. Edit the translation strings that contain the brand in all 16 TSVs.

[Recommendation: ASSUMPTION about effort; effort to be sized]

---

## 11. CLI, MCP and agent automation

- **CLI** (`apps/photocraft-cli`): `convert`, `info`, `run` (chained `--cmd`/`--params`), `batch` (action lists, `--in-place` guard), `droplet` (`.pcdroplet`), `commands [--json]`, `mcp` (stdio; headless or `--bridge 127.0.0.1:port` to a GUI started with `--control`), and `serve` (JSON lines on stdio or `127.0.0.1:port`, token-authenticated). Methods include `engine.execute`, `jobs.*`, `engine.commands`, `doc.*`, `session.list`, `batch` and `methods`. [VERIFIED: `apps/photocraft-cli/src/lib.rs:14-45`, 64-80]
- **Desktop control channel**: `photocraft --control <port> [--control-token|--control-token-file] [--automation-read-root]`. It speaks JSON lines over loopback TCP and supports UI inspect, pointer events, menu invoke, `ui.set` and offscreen screenshots (`docs/control-protocol.md`, `book/src/automation/control-protocol.md`). [VERIFIED]
- **MCP server**: rmcp 3.5, stdio transport, 21 tools and 2 resources (listed in §8.5). Tool annotations (read-only, destructive and so on) are set conservatively. Unknown arguments return `-32602`. Progress notifications and cancellation work for `file.export.renderVideo`. **Auth**: stdio is implicitly trusted (the parent process); the bridge mode authenticates to the GUI with the bearer token. File authority uses rooted read/write directories. [VERIFIED: `docs/mcp.md`, `crates/automation/src/server.rs`]
- **Command engine**: every action is a `CommandSpec` with id, label, menu, shortcut, params doc, `enabled` and `run`, held in one registry shared by the UI, CLI, control channel, MCP, actions and droplets (`crates/engine/src/commands.rs:15-27`). [VERIFIED]
- **AGENTS.md** (17 KB; reported, not followed):
  - Rust-only; no Tauri, Electron or webview.
  - Brand-naming rules (PhotoCraft; `{Function}Craft` family; IDs `ai.storyteller.photocraft`).
  - It points to the private `../craftrules` standards repo.
  - Workspace map and layering rules.
  - "Never crash" rules: no unwrap, expect or panic outside tests; checked arithmetic; recursion bounds; a crash guard with `panic=unwind`.
  - "No unsafe" except in the tablet crate.
  - Everything is a command; no colour or depth assumptions; clean-room.
  - Never commit font files. This conflicts with the `assets/fonts/*.ttf` files that are in the repo.
  - Tests gate every change; thin, data-driven UI; verify UI changes visually through the snapshot example; never break wasm; performance budgets.
  - A pre-finish checklist (test, clippy, layers, wasm, parity, panic_hunt, scorecard, perf, test-corpus) and dev-log discipline (`log/devlog.md`, gitignored).
  - Parallel-agent conventions: separate `CARGO_TARGET_DIR`, atomic manifest writes, about 10 GB per target dir.
  - `.pcraft` completeness rules.
  - Contributor-credit rules for agents (`contributors/people.toml`).

  This confirms the project is designed to be developed by fleets of parallel AI agents. [VERIFIED: `AGENTS.md`]

---

## 12. Readiness assessment

### 12.1 Maturity scores (0 = absent, 5 = production-grade)

| Dimension | Score | Rationale |
|---|---:|---|
| Build | **3** | Clear workspace and xtask. Packaging scripts for 5 OSes plus web, with release CI for each. However: no toolchain pin, unverified downloads, macOS only in release CI, 728-crate graph, about 10 GB target dirs, and nothing compiled during this review. |
| Features | **3** | Very broad surface (PSD fidelity, CMS, raw, smart objects, MCP). Depth is self-described at 25-35% of daily professional readiness: 46 dead settings, type 0/9 done, a GPU crash in the 150-layer scenario. |
| Tests | **3** | 5,460 tests, CPU/GPU parity, pinned real-file oracle corpora, `panic_hunt`, 16 fuzz targets. However: fuzzing is manual only, GPU tests skip without an adapter, no macOS PR CI, users found 214 shortcut failures that tests missed, and green status at HEAD is unverified. |
| Security | **2.5** | forbid(unsafe), loopback plus token, rooted file authority, parser limits, wasmi sandbox, good threat-model docs. However: no private disclosure channel, no cargo-deny or audit, no SBOM, tag-pinned actions, unverified tool downloads, a large in-house parser surface written by AI in days with little continuous fuzzing, and a young `heic-rs` dependency. |
| Docs | **4** | Extensive and unusually candid: architecture, roadmap, scorecard, mdBook security section, per-format pages. Some numbers are stale or inconsistent (README "1,700 tests", "10 languages", scorecard "258/256"). |
| Packaging | **3.5** | MSI with signing, portable zip, DMG with notarization, AppImage, deb, rpm, Flatpak, FreeBSD tarball, web zip, Docker. All of it is brand-locked and depends on upstream signing identities. Flathub is not available. Package licence bundling is incomplete. |

### 12.2 Top risks

1. **Maintainability and provenance of AI-generated code.** About 392k LOC were written in 10 days, mostly by agents. Human review depth is unknown; code style is dense; and clean-room status cannot be proven. The fork inherits a codebase that few or no humans fully understand. [ASSUMPTION grounded in VERIFIED history]
2. **Upstream velocity versus fork divergence.** At 100-366 commits a day, any rename or rebrand fork will conflict constantly. Decide early between a soft fork (tracking upstream, with brand isolated behind a crate) and a hard fork. [VERIFIED cadence]
3. **Brand and trademark obligations.** ArtCraft marks must be removed. PhotoCraft and storytold identifiers are everywhere (more than 12k occurrences). Photoshop trade-dress imitation needs legal review. [VERIFIED]
4. **Security and supply chain gaps.** The large hostile-input parser surface (PSD, PDN/NRBF, Affinity, raw, ZIP, TIFF, ICC) is in-house, and fuzzing is not continuous. Dependency audit, SBOM and pinned actions are missing. Release tool downloads are unchecked. [VERIFIED]
5. **Stability and performance at scale.** 22 of 56 performance scenarios are over budget; the GPU compositor panics in a 150-layer document (P12); complex layout PSDs are laggy. Field crash data: none yet. [VERIFIED]
6. **Licence hygiene in distributables.** Egui default fonts are uncredited; NOTICE and ATTRIBUTION are missing from most packages; no third-party crate licence bundle; 8 translation files are unattributed; the CC BY 3.0 icon needs visible credit. [VERIFIED]
7. **Hidden dependencies on upstream infrastructure.** The private `craftrules` repo, the `craft-fonts` and `photocraft-corpus` repos, the Learning Machines signing identities, and the getartcraft.com and Discord links. [VERIFIED]
8. **Upstream bus factor.** One human leads (about 25% of commits as author, and co-author on most others), and there are 80 one-commit contributors. [VERIFIED]

### 12.3 What needs testing (Phase 2 plan)

- [ ] `cargo build --locked --release -p photocraft --features heif` on Linux, macOS and Windows, using a pinned toolchain (1.95 MSRV and current stable). Record time, disk and RAM.
- [ ] `cargo test --workspace --locked --features heif` green at HEAD. Count skipped GPU tests, and run them on a host with lavapipe or a real GPU.
- [ ] `cargo xtask corpus --all && cargo xtask test-corpus`; confirm the published floors.
- [ ] `cargo deny check licenses advisories bans sources` and `cargo audit` over the whole lockfile; create a `deny.toml`.
- [ ] Generate an SBOM (cargo-cyclonedx) and a third-party licence bundle (cargo-about), including `epaint_default_fonts`.
- [ ] Run each of the 16 fuzz targets for at least 1 hour, plus `panic_hunt --ignored`. Add fuzz targets for PDN/NRBF, ORA, ICC, LUT, TIFF IFD, SVG and ABR, which are not covered or only partially covered.
- [ ] Pen test the control channel and MCP: token handling, path roots, symlink and junction escapes (Windows), `control_call` reach, and resource exhaustion (JSON depth, huge renders).
- [ ] Wasm build: `packaging/web/package.sh` size gate; WebGPU and WebGL2 on Chrome, Firefox and Safari; memory ceiling with large PSDs.
- [ ] Manual QA of core workflows with real PSDs: layers, type, styles, smart objects, CMYK export; compare with Photoshop.
- [ ] Performance: `cargo xtask perf --quick` on our reference hardware; reproduce P12.
- [ ] Verify `photocraft-cli commands --json` command count and MCP tool list against the docs.
- [ ] Packaging dry run with rebranded IDs and unsigned artifacts; confirm the Flatpak, AppImage update info and MSI upgrade behaviour.
- [ ] Legal review: brand licence compliance, Photoshop trade dress, provenance of AI-generated code, absence of CLA.
- [ ] Query GitHub (releases, assets, stars, open issues, security advisories) once API access to `storytold/photocraft` is enabled.

---

*Method note: static analysis only. `cargo metadata/tree --offline` failed because the index has no cached `ruzstd`, so reverse-dependency chains were computed by parsing `Cargo.lock` with a small script. Licence data covers the 260 of 728 packages in the local cargo cache.*
