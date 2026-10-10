# 07 — Prioritised implementation plan, risks and acceptance criteria

Every milestone ends with a review gate. Nothing marked 🔒 starts without explicit approval.

## Milestone 0 — Approve this research (now)
Decisions needed are listed in [README.md](README.md#decisions-needed).

## Milestone 1 — Repositories and history import 🔒
| # | Task | Acceptance criteria |
|---|---|---|
| 1.1 | Create the 4 repos (plus a `craft-fonts` mirror) in `ubunye-ai-ecosystems` | Repos exist, private until M4; default branch `develop` |
| 1.2 | Mirror-push upstream `main` + all tags (procedure in 04 §1) | `git rev-list --count` and the HEAD SHA equal upstream; all tags present; authors and dates unchanged |
| 1.3 | Branches `upstream-main`, `develop`, `main`; rulesets per 04 §2 | Force-push blocked; `develop` requires a PR, 1 approval and checks; the bot can write only `upstream-main` and `integration/**` |
| 1.4 | Add Ubunye docs (`UPSTREAM.md`, `UPSTREAM_STATE.json`, `ubunye/{ARCHITECTURE,DEVELOPMENT,ROADMAP,SECURITY,REBRANDING,CLAUDE}.md`) and `rust-toolchain.toml` | Docs merged by PR; LICENSE, NOTICE and ATTRIBUTION byte-identical to upstream |
| 1.5 | `ubunye-creative-studio` repo: move these documents in; add shared `deny.toml`, reusable workflows | This staging folder is removed from Track4World |

## Milestone 2 — CI and a quality baseline (before any feature work)
| # | Task | Acceptance criteria |
|---|---|---|
| 2.1 | `ubunye-ci.yml`: `cargo xtask ci` (or fmt, clippy, test) `--locked` on ubuntu-22.04, macos-14 and windows-2022 | Runs on every PR to `develop`; baseline pass/fail/skip counts recorded in `ubunye/DEVELOPMENT.md` |
| 2.2 | `cargo deny` (licences, bans, sources, advisories) + `cargo audit` daily | Fails on new advisories or disallowed licences; known advisories listed with an owner and an expiry |
| 2.3 | Fix supply-chain findings: replace `rustls-rustcrypto` alpha (Light, Film); bump quick-xml to 0.41 (Light); SHA-pin Actions (Photo); checksum-verify nfpm, appimagetool and trunk downloads | `cargo audit` reports 0 vulnerabilities in all three; offer the fixes upstream |
| 2.4 | Smoke tests per app: import, edit, save, reopen, export (Light script in `templates/`; Photo with `.pcraft`/PSD; Film with `.fcproj` + MP4) | Green on all 3 OSes in CI |
| 2.5 | Fuzzing: run existing PhotoCraft targets nightly; add cargo-fuzz targets for Light (raw, tiff, xmp) and Film (isobmff, matroska, h264, aac) | ≥1 CPU-hour per target nightly; crashes filed and fixed or reported upstream |
| 2.6 | SBOM (CycloneDX) + `THIRD-PARTY-LICENSES` (cargo-about) + NOTICE/ATTRIBUTION in every package | Present in every artifact; checked by a packaging test |
| 2.7 | Lock down the local control port: off by default; token auth (Light, Film) like PhotoCraft | Unauthenticated requests rejected; offered upstream |

## Milestone 3 — Upstream sync automation 🔒
| # | Task | Acceptance criteria |
|---|---|---|
| 3.1 | Install `upstream-sync.yml` (draft in `templates/`) with a GitHub App token | Runs daily; with no changes it updates only `last_checked_at` on `upstream-main`'s state (via the PR only) |
| 3.2 | Report generator (classification per 05 §3) | PR body lists features, fixes, breaking and security changes, deps delta, audit delta and conflict hotspots |
| 3.3 | Dry run: integrate one week of upstream into a test branch | Report reviewed; merge produced without force-push; `UPSTREAM_STATE.json` updated only by the merged PR |

## Milestone 4 — Branding 🔒 (requires approved brand kit)
| # | Task | Acceptance criteria |
|---|---|---|
| 4.1 | Brand kit in `ubunye-creative-studio/brand/`: names, icons (macOS icns, Windows ico, hicolor PNGs, favicon), colours, splash, DMG background | Original or properly licensed; licence file included |
| 4.2 | Brand layer per app: `ubunye-brand` crate/consts + packaging overlay + i18n overlay; remove ArtCraft marks; new app IDs, UpgradeCodes and vendor; repoint links and update channels | Brand-leak check: zero `ArtCraft`, `getartcraft`, `ai.storyteller`, `Learning Machines` or `storytold` outside the allow-list (LICENSE/NOTICE/ATTRIBUTION/credits/UPSTREAM). "Based on X by the ArtCraft team" credit visible in About. Integration diff stays small (< ~40 files per app as the target) |
| 4.3 | Rename Adobe feature labels (Lumetri and others) after legal review | Legal sign-off recorded |

## Milestone 5 — Release builds
| # | Task | Acceptance criteria |
|---|---|---|
| 5.1 | `ubunye-release.yml` on `vX.Y.Z-ubunye.N` tags: matrix build, sign, notarise, checksums, SBOM, upload to R2 `beta` | Reproducible: rebuilding the same tag gives the same `SHA256SUMS` for Linux tarballs (byte-for-byte target); others documented |
| 5.2 | Install/uninstall tests on clean VMs (Win 11, macOS 14, Ubuntu 22.04/24.04, Fedora) | Checklist pass with screenshots; Gatekeeper and SmartScreen behaviour recorded |
| 5.3 | Web builds to Cloudflare Pages previews | Loads in Chrome, Edge, Firefox and Safari; smoke flow works; size gates pass |
| 5.4 | Hardware and requirements doc | Measured on 3 reference machines |

## Milestone 6 — ThabangVision distribution 🔒
| # | Task | Acceptance criteria |
|---|---|---|
| 6.1 | Inspect the ThabangVision repo (needs read access) | Framework, hosting, routes and preview setup confirmed; 06 §7 updated |
| 6.2 | R2 bucket, custom domain `downloads.thabangvision.com`, manifests | Download works without a GitHub account; checksums match; rollback drill (flip `stable.json`) succeeds |
| 6.3 | `/studio` pages on a feature branch with preview deployment | Reviewed on preview; **production merge only after approval** |

## Milestone 7 — AI integration research (no dependencies added before this)
- Candidates: background removal and segmentation (already partly in LightCraft through SAM 3, but under a non-OSI licence; look for alternatives such as BiRefNet or RMBG under permissive licences), denoise and enhancement, auto colour, captions (FilmCraft Whisper exists), TTS (Kokoro exists).
- Ubunye Engine today is a Python data/ML pipeline framework with an LLM port and an MCP server. It has **no image or video models**. Integration path: agent orchestration over the apps' MCP servers plus Ubunye Engine's MCP; batch pipelines (export → process → import). In-app real-time AI should use Rust-native ONNX/candle runtimes, which the apps already have.
- Deliverable: an evaluation of model licences, size, quality and runtime for each feature.

## Risk register

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Upstream velocity (100–370 commits/day) makes sync costly | High | High | Thin fork, brand layer, upstreamed hooks, a single rolling integration PR, and decide on a sync cadence (daily check, weekly merge) |
| R2 | AI-generated code: provenance (possible GPL reproduction) and review debt | Medium | High | ScanCode plus similarity scans; human review of parsers and codecs; legal sign-off |
| R3 | Codec patents (H.264/HEVC/AAC) | Medium | High | Legal review; royalty-free build profile; OS codec delegation |
| R4 | Hostile-file parser vulnerabilities | High | High | Fuzzing in CI, size and allocation caps, SECURITY.md disclosure channel |
| R5 | Trademark (ArtCraft marks; Adobe trade dress and feature names) | Medium (if unaddressed) | High | M4 brand-leak gate; legal review; neutral marketing copy |
| R6 | Upstream abandonment or a licence change | Low–Medium | Medium | Full history mirrored; MIT/Apache grants are irrevocable for code already published |
| R7 | Signing identity costs and eligibility (Apple, Azure) | Medium | Medium | Start the paperwork in M1; unsigned beta with clear instructions in the meantime |
| R8 | Unauthenticated local control port | Medium | Medium | M2.7 |
| R9 | Hidden upstream infra (craft-fonts, craftrules, corpora) | Medium | Low | Mirror and pin |
| R10 | Overclaiming readiness on the public site | Medium | High (reputation) | "Preview" badge until the acceptance criteria are met: CI green on 3 OSes, smoke tests, 0 known critical vulnerabilities, fuzzing in place, installs signed |

## Definition of "production-ready" (for any app)
CI green on Windows, macOS and Linux for 2 consecutive weeks. Smoke tests (import → edit → save → reopen → export) on all platforms. Zero open critical or high vulnerabilities. Fuzzing running nightly without unresolved crashes. Signed and notarised installers. A manual QA checklist for core workflows. Crash-free sessions measured in beta. Legal sign-off on licensing, trademarks and codecs. **Compiling successfully is not sufficient.**
