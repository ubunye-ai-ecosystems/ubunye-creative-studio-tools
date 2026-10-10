# Part A — LightCraft / PhotoCraft / FilmCraft: what is shared

Snapshot (local clones in `upstream clone of `, 2026-10-10):

| Repo | HEAD | Commits | Root commit | Version | MSRV (`rust-version`) |
|---|---|---|---|---|---|
| lightcraft | `b70ae17` (2026-10-10) | 1552 | `3f1ffeb` "M0.1/M0.2: workspace, xtask…" 2026-09-30 | 0.5.0 | 1.90 |
| photocraft | `7722172` (2026-10-10) | 1150 | `386b056` "init repo" 2026-09-30 | 0.6.0 | 1.95 |
| filmcraft | `98ba9ac` (2026-10-10) | 1098 | `16489ce` "first commit" 2026-09-30 | 0.5.0 | 1.95 |

All three: edition 2024, `license = "MIT OR Apache-2.0"`, egui/eframe on wgpu, `crates/*` + `apps/{x,x-cli,x-web}` + `xtask` layout. [VERIFIED]

Tools: `analysis scripts locks.py` (tomllib lock parser, run `python3 -I`), `analysis scripts sim.py` (difflib per-file similarity). Raw output: `(raw output not committed)`.

## 1. Cargo.lock comparison [VERIFIED]

| Repo | `[[package]]` entries | external crate names | external name@version | workspace crates | git deps |
|---|---|---|---|---|---|
| lightcraft | 656 | 553 | 628 | 28 | 0 |
| photocraft | 728 | 635 | 699 | 29 | 0 |
| filmcraft | 698 | 577 | 649 | 49 | 0 |

Overlap of external crate names: **393 common to all three**; light∩photo 445, light∩film 472, photo∩film 432. Unique: light 29, photo 151, film 66.

Of the 393 common crates, **only 52 diverge in version set**, and none of the important ones do. The lockfiles have been updated in near lock-step (same patch versions of nearly everything):

| crate | lightcraft | photocraft | filmcraft |
|---|---|---|---|
| wgpu / naga | 30.0.1 | 30.0.1 | 30.0.1 |
| egui / eframe | 0.36.2 | 0.36.2 | 0.36.2 |
| winit | 0.30.13 | 0.30.13 | 0.30.13 |
| accesskit | 0.24.1 | 0.24.1 | 0.24.1 |
| rfd | 0.17.2 | 0.17.2 | 0.17.2 |
| image | 0.25.10 | 0.25.10 | 0.25.10 |
| png | 0.18.1 | 0.17.16 + 0.18.1 | 0.18.1 |
| zune-jpeg | 0.5.15 | 0.4.21 + 0.5.15 | 0.5.15 |
| moxcms | 0.8.1 + 0.9.1 | 0.8.1 | 0.8.1 |
| tokio | 1.53.1 | 1.53.1 | 1.53.1 |
| serde / serde_json | 1.0.229 / 1.0.151 | same | same |
| rayon | 1.12.0 | 1.12.0 | 1.12.0 |
| rmcp | — (hand-rolled) | 3.5.0 | 3.5.0 |
| candle-core | 0.9.2 | — | 0.9.2 |
| rustls / ring | 0.23.45 / 0.17.14 | — | 0.23.45 / 0.17.14 |
| wasm-bindgen | 0.2.129 | 0.2.129 | 0.2.129 |
| rand | 0.8.8 + 0.9.5 | 0.9.5 + 0.10.3 | 0.8.8 + 0.9.5 |
| windows | 0.62.2 | 0.62.2 | 0.54.0 + 0.62.2 |
| toml / toml_edit | 1.1.6 / 0.25.15 | same | + 0.8.2 / 0.19, 0.20 (older build deps) |
| syn | 2 + 3 | 2 + 3 | 1 + 2 + 3 |

Remaining divergences are transitive duplicates (getrandom, hashbrown, itertools, kurbo, zune-core, gif, windows-* 0.42/0.54 in filmcraft, rand_core 0.10 in photocraft). Unification cost is low.

## 2. In-house crates: same names, different code [VERIFIED]

Package names are always prefixed (`lightcraft-color`, `photocraft-color`, …), so the only identical workspace package name is `xtask`. Directory names do recur: `color`, `geom`, `gpu`, `codecs`, `engine`, `ui-egui` in all three; `raster`, `raw`, `heif` in light+photo; `text`, `testkit`, `automation` (MCP) in photo+film; `mcp` only in lightcraft.

`diff -rq` + per-file line-similarity (difflib ratio; 1.00 = identical):

| crate dir | pairs | identical files | typical `src/lib.rs` similarity | notes |
|---|---|---|---|---|
| color | all | 0 | 0.16–0.19 | different scope (film: 8 files/2.4k LOC; light 0.9k) |
| geom | all | 0 | 0.22–0.27 | Cargo.toml ~0.8 similar (template) |
| gpu | all | 0 | 0.04–0.07 | wholly different pipelines |
| codecs | all | 0 | 0.13–0.18 | |
| ui-egui | all | 0 | 0.06–0.11 (lib/state/menus/theme) | **`credits.rs` 0.87 (L/F), 0.58 (P/F); `build.rs` 0.87 (P/F); `i18n/catalog.rs` 0.81 (P/F)** |
| xtask | L/P | `stats.rs`, `version.rs` **1.00**; `ico.rs` 0.99 | main.rs 0.30 | |
| xtask | L/F | `version.rs` 0.97, `ico.rs` 0.99 (only path line differs) | main.rs 0.03 | |

Conclusion: the repos are **siblings built to a common template/conventions, not a shared codebase**. Copy-paste is limited to small utilities (xtask `version`/`ico`/`stats`, About-window credits, i18n catalog scaffolding, packaging scripts). PhotoCraft's AGENTS.md states the policy explicitly: *"Standards and learnings shared across the crafting apps live in `../craftrules` … Contribute reusable learnings there, never code; repos don't share code."*

Git history [VERIFIED]: three different root commits; `comm` of `git rev-list --all` across each pair = **0 shared commits**; 0 shared tree hashes (light/film). Clones are not shallow. Main authors overlap (Brandon Thomas is top committer in all three; "Claude" has 139 commits in lightcraft).

Publication / shared deps [VERIFIED]: no `git+` sources in any lockfile; none of `lightcraft-*`/`photocraft-*`/`filmcraft-*` crates exist on crates.io (sparse index 404, control `serde` 200). Only 2 crates set `publish = false` (filmcraft golden/testkit) — the rest are simply never published. Shared external storytold resources are **non-Cargo**: `storytold/craft-fonts` (cloned in release CI, embedded via `CRAFT_FONTS_DIR` at build time, never in Cargo.toml) and `storytold/craftrules` (private standards repo referenced from AGENTS/CLAUDE.md).

## 3. Toolchain, CI, xtask, packaging, WASM, MCP, licences, agent docs

| Area | lightcraft | photocraft | filmcraft | Tag |
|---|---|---|---|---|
| `rust-toolchain.toml` | none | none | none | [VERIFIED] |
| CI toolchain | `dtolnay/rust-toolchain@686976e…` (SHA-pinned, `# stable`) | `@stable` (unpinned tag) except windows-arm64 `@89b1218…`; `@nightly` in one ci.yml job | `@686976e…` (same SHA as light) | [VERIFIED] |
| MSRV | 1.90 | 1.95 | 1.95 | [VERIFIED] |
| Workflows | release, freebsd, packaging-lint, windows-arm64 (no general PR `ci.yml`) | **ci.yml** (path-filtered), docs, perf-nightly, release, freebsd, packaging-lint, windows-arm64 | same 4 as light | [VERIFIED] |
| release.yml jobs | version, macos, windows, linux, flatpak, freebsd, web, release (identical job set in all three; light↔film 175 diff lines, light↔photo 302) | ← | ← | [VERIFIED] |
| xtask commands | ico, version, layers, assets, bench, parity, wasm, web, ci, corpus, stats | layers, wasm, ci, corpus, test-corpus, stats, parity, perf, scorecard, version, ico | layers, wasm, assets, fixtures, ci, bench, bench-playback, web, version, ico | [VERIFIED] |
| Packaging | `packaging/{macos(dmg, Info.plist, entitlements, cert import), windows(WiX .wxs, sign.ps1), linux(nfpm .deb/.rpm, flatpak + bundle, .desktop/metainfo/mime), freebsd, web}`, app id `ai.storyteller.<app>`; plus Nix flake | same + AppImage `AppRun`, Dockerfile, nginx.conf for web | same + `install.sh` | [VERIFIED] |
| WASM | `apps/lightcraft-web` + wasm-bindgen CLI + `worker.js`, `packaging/web/_headers` | **trunk** (`Trunk.toml`) | wasm-bindgen CLI | [VERIFIED] |
| MCP | `crates/mcp`: hand-rolled JSON-RPC 2.0 over stdio (serde_json, no rmcp) | `crates/automation` on **rmcp 3.5** (client+server, transport-io) | `crates/automation` on rmcp 3.5 (server) | [VERIFIED] |
| Control channel | JSON-lines `--control <port>` (film documented; others similar) | | | [ASSUMPTION] for light/photo |
| LICENSE-APACHE | byte-identical ×3 | | | [VERIFIED] |
| LICENSE-MIT | differs only in product name in copyright line ("ArtCraft Team and the XCraft contributors") | | | [VERIFIED] |
| NOTICE | same skeleton; app-specific third-party section. **lightcraft `crates/segment` is Apache-2.0-only** (SAM 3 port from HF Transformers); rawtoaces-data matrices (Apache-2.0, CIE CC-BY-4.0). Film/photo list OFL fonts + craft-fonts. | | | [VERIFIED] |
| Brand | `docs/brand/LICENSE-brand.txt` (ArtCraft marks are trademarks, not OSS; **forks must remove the marks** and not present as ArtCraft) — same text modulo product name | | | [VERIFIED] |
| Agent docs | CLAUDE.md 136 lines + AGENTS.md 147 | AGENTS.md 151 only (no CLAUDE.md) | CLAUDE.md 28 + AGENTS.md 162 (AGENTS wins on conflict) | [VERIFIED] |
| Shared conventions | "Never crash" (no unwrap/expect/panic outside tests, cites `craftrules/standards/never-crash.md`), clean-room vs Adobe, pure Rust (no Tauri/webview), layering enforced by `xtask layers`, every command agent-drivable, licensed+attributed assets (`xtask assets`), maintainer-local `plan/` dir, one task id per commit, contributor credits in About window | | | [VERIFIED] |

## 4. Vulnerability audit

`cargo-audit` was not installed; `cargo install cargo-audit --locked` succeeded in under 10 min (v0.22.2). Ran `cargo audit --file <repo>/Cargo.lock --db a local advisory-db --json` (advisory DB updated 2026-10-09). Raw JSON: `(raw JSON not committed)`. No cargo build was run. [VERIFIED]

| Advisory | Crate@version | light | photo | film | Pulled in by | Fix |
|---|---|---|---|---|---|---|
| RUSTSEC-2023-0071 (Marvin timing side-channel) | rsa 0.9.10 | yes | – | yes | `rustls-rustcrypto 0.0.2-alpha` (light `crates/fetch`; film `crates/speech`, optional `download` feature) | no patched rsa; replace rustls-rustcrypto with `rustls` + aws-lc-rs/ring provider |
| RUSTSEC-2026-0049 / -0098 / -0099 / -0104 (webpki CRL & name-constraint bugs, CRL-parse panic) | rustls-webpki 0.102.8 | yes | – | yes | same `rustls-rustcrypto` alpha | same; webpki >=0.103.13 |
| RUSTSEC-2026-0194 / -0195 (quick-xml quadratic time / NsReader memory exhaustion) | quick-xml 0.38.4 | yes | – | – | `lightcraft-meta` (workspace pin `quick-xml = "0.38"`, XMP parsing) | bump to 0.41 (already in the lock for other deps) |
| Unmaintained: paste (RUSTSEC-2024-0436) | 1.0.15 | warn | warn | warn | transitive | informational |
| Unmaintained: ttf-parser (RUSTSEC-2026-0192) | 0.25.1 | warn | warn | – | egui/font stack | informational |
| Unmaintained: rustybuzz (RUSTSEC-2026-0206) | 0.20.1 | – | warn | – | text shaping | informational |
| Unmaintained: proc-macro-error (RUSTSEC-2024-0370) | 1.0.4 | – | – | warn | gtk-rs 0.18 macros | informational |
| Unsound: glib (RUSTSEC-2024-0429) | 0.18.5 | – | – | warn | `gtk 0.18` via `muda` (menus) on Linux | upstream muda/gtk upgrade |
| Yanked | yoke-derive 0.8.3 | – | warn | – | transitive | `cargo update -p yoke-derive` |

Summary: **photocraft has 0 vulnerabilities** (4 warnings); **lightcraft 7** and **filmcraft 5** vulnerabilities, almost all from one alpha crate (`rustls-rustcrypto`) used for model/asset downloads. Exploitability is limited to HTTPS download paths (TLS cert validation, RSA timing) and, for quick-xml, to parsing hostile XMP sidecars. [VERIFIED for presence; exploitability ASSUMPTION]. A shared `deny.toml` in the studio core would catch these in CI.

## 5. Recommendation: a shared "ubunye-creative-studio" core?

**What is realistically extractable** (small, low-churn, already near-identical):
1. `xtask` helpers: `version`, `ico`, `stats`, layer checker, asset/attribution checker → one `ubunye-xtask-common` lib (or keep as copied files with a sync script).
2. Packaging templates: per-OS scripts, nfpm/flatpak/WiX/DMG templates parameterised by app id/name; a reusable GitHub Actions workflow (`workflow_call`) for the identical release job matrix; one pinned toolchain SHA (+ add `rust-toolchain.toml` for reproducibility).
3. Branding layer: a single `ubunye-brand` crate/assets dir (icons, theme colours, About/credits, links) — needed anyway because the ArtCraft marks must be removed from each fork.
4. A unified `[workspace.dependencies]` version policy (wgpu 30 / egui 0.36 / winit 0.30 / image 0.25 / tokio 1.53 / rmcp 3.5) enforced by a shared `deny.toml`.
5. Possibly an MCP shim: photo+film already use rmcp 3.5; light's hand-rolled server could stay or adopt rmcp.

**What should NOT be extracted now**: `color`, `geom`, `gpu`, `codecs`, `engine`, `ui-egui` — same names but 4–27 % line similarity; merging means rewriting two of three apps' internals and diverging permanently from upstream.

**Risks of a shared core**
- Upstream compatibility: storytold explicitly does not share code across apps; every upstream merge would conflict with a refactored core. Each upstream repo has ~1–1.5k commits in 10 days — very high churn; a deep fork rots quickly.
- Lock-step dependency upgrades become a cross-repo coordination problem (today each repo bumps wgpu/egui independently, though they happen to be in sync).
- Licence hygiene: lightcraft's Apache-2.0-only `segment` crate and per-repo NOTICE/ATTRIBUTION must be preserved per binary; a merged core must carry the union of attributions.
- MSRV differs (1.90 vs 1.95) — core must target the max.

**Recommended approach** [ASSUMPTION]: keep three thin forks that track upstream `main` with minimal patches (branding, app ids, release targets), and put sharing in a separate `ubunye-creative-studio` repo that holds *non-invasive* artefacts: brand assets, reusable CI workflows, packaging templates, `deny.toml`/version policy, a catalogue manifest for the website, and docs. Revisit code-level sharing only if upstream itself starts sharing crates.
