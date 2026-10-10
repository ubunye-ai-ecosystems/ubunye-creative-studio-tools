# 01 — Architecture reports (summary)

The full per-application due-diligence reports are in [`research/`](research/):

- [`research/lightcraft.md`](research/lightcraft.md) — LightCraft becomes **ubunye-creative-light**
- [`research/photocraft.md`](research/photocraft.md) — PhotoCraft becomes **ubunye-creative-photo**
- [`research/filmcraft.md`](research/filmcraft.md) — FilmCraft becomes **ubunye-creative-film**
- [`research/shared.md`](research/shared.md) — cross-repo dependency, code-sharing and vulnerability comparison
- [`research/ecosystem.md`](research/ecosystem.md) — Ubunye Engine, ThabangVision and the existing org repos

Each report has 12 sections: snapshot, structure, architecture, dependencies, features, build, tests, security, licensing, branding surface, CLI/MCP, and readiness. Every claim is tagged [VERIFIED] / [ASSUMPTION] / [NEEDS TESTING].

Method: full git clones of each upstream on 2026-10-10. Static analysis, `Cargo.lock` parsing and `cargo audit` against the RustSec DB of 2026-10-09. One real build was attempted (see 03).

## At a glance

| | LightCraft | PhotoCraft | FilmCraft |
|---|---|---|---|
| Equivalent of | Lightroom (RAW development and library) | Photoshop (layered editing) | Premiere Pro (NLE) |
| HEAD (2026-10-10) | `b70ae17` | `7722172` | `98ba9ac` |
| Repo age / commits | 10 days / 1,552 | 10 days / 1,150 | 11 days / 1,098 |
| Contributors | 75 | 175 | 71 |
| AI co-authored commits | ~60% | ~1,200 trailers | >85% |
| Version / tags | 0.5.0 (v0.1.0–v0.5.0) | 0.6.0 (8 tags) | 0.5.0 (v0.1.0–v0.5.0) |
| Rust LOC | ~202k | ~392k | ~343k |
| Workspace | 24 crates + 3 apps + xtask | 25 crates + 3 apps + xtask | 45 crates + 3 apps + xtask |
| MSRV / toolchain pin | 1.90 / none | 1.95 / none | 1.95 / none |
| UI / GPU | egui/eframe 0.36 on wgpu 30 | same | same |
| Reference path | CPU pipeline checks GPU compute | CPU and GPU compositors cross-checked | CPU compositor checks GPU |
| Core format work | in-house RAW decoders (DNG, CR2, CR3, ARW, NEF, RAF, ORF, PEF, RW2, SRW), XMP, Lightroom `.lrcat` reader | in-house PSD/PSB, PDN, Affinity, ORA, camera raw, `.pcraft` (ZIP) | in-house H.264, HEVC, VP9, AV1, ProRes, DNx, MPEG-2, AAC, Opus, AC-3, plus MP4, MKV, MXF, TS, AAF and OMF; symphonia for MP3/FLAC; HW decode and encode through VideoToolbox, Media Foundation, VA-API and NVDEC |
| Native C | none intended (`ring` is in the lock: NEEDS TESTING) | none | none (HW FFI in `crates/platform`) |
| `unsafe` | 1 block (`crates/sysmem`) | 3 blocks (`crates/tablet`, macOS) | 197 blocks, all in `crates/platform` |
| Data / project | JSON-lines catalog log + snapshot; no SQLite | `.pcraft` ZIP document | `.fcproj`, integer-tick timeline |
| Plugins | none (command system) | sandboxed WASM plugins (wasmi) | none (no VST3, AU or OpenFX) |
| Web build | wasm-bindgen; WebGL2; ~13.5 MB wasm (3.1 MB br) | trunk; WebGPU, falls back to WebGL2; ~19 MiB (5.6 MiB br); 24 MiB gate | wasm-bindgen; WebGPU, falls back to WebGL2; WebCodecs + OPFS; ~13 MB |
| CLI | `lightcraft-cli` | `photocraft-cli` | `filmcraft-cli` (650+ commands claimed) |
| MCP | hand-written JSON-RPC over stdio (`lightcraft-cli mcp`), one tool per command | rmcp 3.5, 21 tools, stdio; token-protected localhost control channel | rmcp 3.5, stdio; **unauthenticated** localhost control port |
| Tests | 2,576 (proptest; GPU equivalence; MCP e2e) | 5,460 (pinned real-file corpora; 16 fuzz targets, manual only) | 2,526 (oracle, golden, kittest UI, mutation) |
| CI that runs tests | FreeBSD only, manifest-changing PRs only | `ci.yml` (path-filtered); no macOS in PR CI | **none** |
| `cargo audit` | 7 vulns (rsa, rustls-webpki 0.102 via `rustls-rustcrypto` alpha; quick-xml 0.38) | **0** | 5 (same `rustls-rustcrypto` chain) |
| Upstream self-assessment | 60–70% of a Lightroom replacement | "early alpha", <50% of Photoshop parity | 50–60% "ready for real work" |
| Our readiness score (build/feat/test/sec/docs/pkg, 0–5) | 3 / 3 / 3 / 2.5 / 4 / 4 | 3 / 3 / 3 / 2.5 / 4 / 3.5 | 3 / 3 / 2 / 2 / 4 / 3 |

## Shared architecture pattern (all three) [VERIFIED]

```
apps/<x>        desktop shell (eframe)        apps/<x>-cli   CLI + MCP     apps/<x>-web   wasm shell
      \                    |                          /                         /
       └──────────── crates/ui-egui (panels, menus, theme, brand, i18n) ───────┘
                                   |
               crates/engine — headless document/timeline + command registry
          (every action is a command: reachable from the UI, CLI, control channel and MCP)
                                   |
   crates/{color, geom, gpu, codecs, raster|raw|heif|text|…}   — layering enforced by `cargo xtask layers`
                                   |
                    wgpu 30 · egui 0.36 · winit 0.30 · image 0.25 · rayon · tokio
```

Key architectural observations:

1. **Command-centric engines.** Every UI action is a typed command, so headless automation (CLI, MCP, control port) comes for free. This is the strongest basis for future agent-driven workflows with Ubunye Engine.
2. **Siblings, not a shared codebase.** Same layout and dependency versions (393 external crates in common; wgpu, egui and winit match exactly), but in-house crates with the same names (`color`, `gpu`, `engine`, `ui-egui`) are only 4–27% similar. Only xtask helpers and the credits screen are near-identical. Upstream's policy is "repos don't share code".
3. **Pure-Rust, in-house parsers.** No FFmpeg, LibRaw or ImageMagick. That avoids GPL and LGPL, and simplifies wasm and cross-compiling, but it means very large, young, mostly agent-written parser code (tens of thousands of lines per app) handles untrusted media.
4. **Branding is spread out, not centralised.** Names, IDs and links appear 1,200 (Light), over 12,000 (Photo) and hundreds (Film) of times, including locale files. FilmCraft has a `brand.rs`, but IDs live in `main.rs`, packaging templates and locale catalogs.
