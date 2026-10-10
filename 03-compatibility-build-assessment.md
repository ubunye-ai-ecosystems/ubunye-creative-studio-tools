# 03 — Technical compatibility and build assessment

## 1. Build evidence collected in this iteration

| Test | Environment | Result |
|---|---|---|
| `cargo build --locked -j2 -p lightcraft-cli` (dev profile) at `b70ae17` | Linux x86_64 container, 4 vCPU / 15 GB RAM, rustc/cargo 1.97.0, no GPU | ✅ **PASS**: 304 crates in 13 min 41 s wall, 23.5 min CPU; debug binary is 770 MB. One warning: no craft-fonts embedded, so CJK text renders as boxes. [VERIFIED] |
| `ring` / `cc` / `aws-lc` compiled? | same build | Not compiled for `lightcraft-cli` default features: "pure Rust" holds for this target. [VERIFIED for the CLI; NEEDS TESTING for the desktop app with all features] |
| Headless smoke test: import a PNG, `develop.set light.exposure=0.7`, save to library, export JPEG, **reopen the library in a new process**, export again | same | ✅ **PASS**: the export after reopen is byte-identical to the edited export and differs from the unedited control export. The library is persisted as `catalog.log` + `catalog.snap` + `catalog.lock`. [VERIFIED] |
| GPU | same | No Vulkan driver (`vkCreateInstance: Found no drivers!`). wgpu fell back and the export still succeeded on the CPU path. GPU paths are **untested**. [VERIFIED] |
| `cargo audit` (RustSec DB 2026-10-09) | all three lockfiles | Light 7 vulns, Photo 0, Film 5. Details in `research/shared.md` §4. [VERIFIED] |
| PhotoCraft and FilmCraft builds; desktop GUI apps; Windows; macOS; wasm | — | **not run in this iteration** [NEEDS TESTING] |

Script for the smoke test (to become a CI job): [`templates/smoke-light.sh`](templates/smoke-light.sh).

## 2. Platform support matrix (upstream claims vs what we verified)

| Platform | LightCraft | PhotoCraft | FilmCraft | Our evidence |
|---|---|---|---|---|
| Linux x86_64 (glibc ≥ 2.35, Ubuntu 22.04 baseline) | release: AppImage, deb, rpm, tar, Flatpak | same | same | Light CLI builds and passes the smoke test ✅; others NEEDS TESTING |
| Linux aarch64 | release | release | release | NEEDS TESTING |
| macOS 11+ universal (arm64 + x86_64) | signed, notarised DMG | same | same | NEEDS TESTING (needs a macOS runner) |
| Windows x64 / x86 / arm64 (MSVC) | MSI + zip | same | same | NEEDS TESTING; upstream calls Windows "thinly tested" |
| FreeBSD 14 | tarball, nightly CI | same | same | out of scope for ThabangVision v1 |
| Web (wasm32) | WebGL2 only; static site | WebGPU, falls back to WebGL2; 24 MiB gate | WebGPU, falls back to WebGL2; WebCodecs, OPFS | NEEDS TESTING in Chrome, Edge, Firefox and Safari |

## 3. Toolchain and dependency compatibility

- **Rust**: MSRV 1.90 (Light) and 1.95 (Photo, Film). Edition 2024. None pins a toolchain.
  **Action:** add `rust-toolchain.toml` with `channel = "1.97.0"` (verified here) plus the `wasm32-unknown-unknown` target to each fork, as an Ubunye-owned file. It does not exist upstream, so it can't conflict.
- **Shared dependency stack is aligned**: wgpu 30.0.1, egui/eframe 0.36.2, winit 0.30.13, image 0.25.10, tokio 1.53.1, rmcp 3.5.0 (Photo, Film). 393 crates are common to all three, and only 52 of them diverge, all transitive. A shared `deny.toml` and a shared version policy are low-cost.
- **Linux system packages** (build): `libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev`, plus `libgtk-3-dev` (Photo) and `libasound2-dev` (Film).
- **Hidden build input**: `storytold/craft-fonts` through `CRAFT_FONTS_DIR`. It is optional, but releases require it (`CRAFT_FONTS_REQUIRED=1`). Mirror it.
- **wasm tooling**: wasm-bindgen-cli **=0.2.129** (Light, Film) and trunk 0.21.14 (Photo). Pin both in CI.

## 4. Hardware requirements (initial; to be measured)

| | Build machine (measured / estimated) | End user (upstream claims and our assumptions) |
|---|---|---|
| CPU | 4 vCPU: 14 min for the Light CLI debug build [VERIFIED]. Release LTO for Photo/Film (343k–392k LOC) is estimated at 30–60 min on 4 cores [ASSUMPTION] | any x86_64 or arm64 from the last 8 years |
| RAM | 15 GB was sufficient for `-j2` [VERIFIED]; recommend ≥16 GB for release LTO | 8 GB minimum, 16 GB recommended for 4K video and large PSDs [ASSUMPTION] |
| Disk | Light debug target dir ≈ several GB; Photo target ≈ 10 GB (upstream note) | 300 MB–1 GB per install [NEEDS TESTING] |
| GPU | not needed to build | Vulkan 1.2, Metal or DX12 for the GPU path. A CPU fallback exists (verified for the Light export) but will be slow for video [ASSUMPTION] |
| Browser | — | Chrome or Edge 113+ for WebGPU; WebGL2 elsewhere; ≤4 GiB wasm memory |

## 5. Cross-app compatibility for an integrated "Studio"

| Integration point | Feasibility | Notes |
|---|---|---|
| Light → Photo ("Edit in Photo") | High | Both read and write TIFF/PSD; PhotoCraft has its own camera-raw path. Hand-off through a temp TIFF plus a CLI command. [ASSUMPTION] |
| Photo/Light stills → Film timeline | High | Film imports PNG and JPEG. [ASSUMPTION] |
| Shared colour management | Medium | All use moxcms, but with different in-house colour crates. Agree on sRGB, Display P3 and Rec.709/2020 tags in exports. |
| One automation surface | High | All three expose CLI + MCP with "every command is a tool". A studio-level agent can drive all three today. Differences: Light has its own JSON-RPC; Photo's control channel has tokens; Film's control port has **no auth** (fix first). |
| Shared installer / launcher | Medium | Keep separate installers (independent releases). A future "Studio launcher" can be a small app that reads the release manifest. |

## 6. Known failures and open issues to track

1. No GPU in our test container. GPU equivalence tests need a self-hosted GPU runner or lavapipe (Mesa software Vulkan).
2. FilmCraft web docs contradict each other on COOP/COEP. We will serve COOP/COEP anyway (harmless, and future-proof for threads).
3. FilmCraft Linux MIME XML appears empty: `.fcproj` won't be associated. NEEDS TESTING.
4. LightCraft macOS `Info.plist` carries PhotoCraft leftovers (`.pcraft`, Photoshop PSB UTI). Fix in our packaging overlay.
5. PhotoCraft upstream scorecard: GPU compositor panics in the 150-layer scenario, and 46 of 149 settings do nothing.

**Readiness statement:** no application is production-ready. One CLI compiled and passed a headless round trip. That shows the toolchain works and the persistence model works for one path. It says nothing about GUI stability, platform coverage, parser robustness or output fidelity.
