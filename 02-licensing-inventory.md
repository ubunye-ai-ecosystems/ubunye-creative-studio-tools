# 02 — Licensing and attribution inventory

Snapshot: upstream `main` on 2026-10-10. LightCraft `b70ae17`, PhotoCraft `7722172`, FilmCraft `98ba9ac`.
Tags: **[VERIFIED]** means checked in the repo, **[ASSUMPTION]** means inferred, **[NEEDS TESTING]** means it has to be confirmed by a tool run or legal review.
This is an engineering inventory, not legal advice. Every item marked ⚖️ needs counsel before the first public release.

## 1. Project licences

| | LightCraft | PhotoCraft | FilmCraft |
|---|---|---|---|
| Code licence | MIT OR Apache-2.0 | MIT OR Apache-2.0 | MIT OR Apache-2.0 |
| Exceptions | `crates/segment` and `crates/fetch` are **Apache-2.0 only**. `segment` is a port of HF Transformers SAM 3 | none | none |
| `LICENSE-APACHE` | byte-identical in all three | ← | ← |
| `LICENSE-MIT` copyright | "ArtCraft Team and the LightCraft contributors" | "… PhotoCraft contributors" | "… FilmCraft contributors" |
| `NOTICE` | yes, with a third-party section | yes | yes |
| `ATTRIBUTION.md` | `assets/ATTRIBUTION.md`, checked by `cargo xtask assets` | root `ATTRIBUTION.md` | root `ATTRIBUTION.md`, plus a per-asset `.attribution` sidecar checked by `xtask assets` |
| CLA / DCO | none | none (175 contributors) | none |

All of this is [VERIFIED].

### What we must do as a redistributor

1. **Keep** `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE` and every `ATTRIBUTION.md` unchanged in each fork. Keep the upstream copyright lines. Apache-2.0 §4(d) requires derivative works to carry the NOTICE.
2. **Add our own notice above theirs.** Do not replace it. Proposed text:
   "Ubunye Creative Light, Copyright (c) 2026 Ubunye AI Ecosystems. Based on LightCraft by the ArtCraft team and the LightCraft contributors (https://github.com/storytold/lightcraft), used under MIT OR Apache-2.0."
3. **Mark modified files.** Apache-2.0 §4(b) requires modified files to carry a notice that they were changed. Use a git-trailer convention plus a header comment on files we change substantially. See `REBRANDING.md` in each fork.
4. **Apache-only crates** (`lightcraft-segment`, `lightcraft-fetch`) must stay Apache-2.0. Any binary that contains them is effectively distributed under Apache-2.0 terms, so the MIT-only option is not available for the whole LightCraft binary. ⚖️ [VERIFIED for the crate licences; the effect on the whole binary is an ASSUMPTION]
5. **Ship third-party crate licences with every binary.** None of the three generates a bundled third-party licence file today. Add `cargo about generate` to packaging so it produces `THIRD-PARTY-LICENSES.html` per app. [VERIFIED gap]
6. **Packaging gap.** In PhotoCraft, `packaging/env.sh copy_docs` and Windows `package.ps1` ship LICENSE files but **not NOTICE or ATTRIBUTION.md**. Only FreeBSD and Docker do. Fix this in our packaging for all three apps. [VERIFIED for PhotoCraft; NEEDS TESTING for the other two]

## 2. Trademarks and brand assets

| Mark / identity | Owner | Status | Action |
|---|---|---|---|
| ArtCraft name, wordmark and logos (`docs/brand/*.svg`, `*.png`) | ArtCraft Team | **Proprietary, not covered by the OSS licence** (`docs/brand/LICENSE-brand.txt`). "If you distribute or publish a modified version … you must remove the ArtCraft Marks … and must not call it ArtCraft." | **Remove from every distributed build.** FilmCraft compiles them into the binary (`crates/ui-egui/src/brand.rs`, `include_bytes!`). Plain-text "based on LightCraft by the ArtCraft team" is explicitly allowed. [VERIFIED] |
| LightCraft / PhotoCraft / FilmCraft names | upstream | No trademark claim found [VERIFIED absence of a claim] | Rename anyway so we are not presented as the upstream product. ⚖️ |
| App icons (lynx, kitsune, owl) | project owner | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | Legally reusable, but they are the de facto upstream identity, so **replace them** with Ubunye icons. [VERIFIED] |
| `ai.storyteller.*` reverse-DNS IDs, "Learning Machines LLC" vendor, MSI UpgradeCodes, Apple Team `DJ6XS33FX8` | upstream company | identity, not licensed | **Must change.** Using another vendor's identity on installers is misrepresentation, and a reused UpgradeCode would upgrade over a user's upstream install. [VERIFIED] |
| getartcraft.com, discord.gg/artcraft, `storytold/*` update channels | upstream | links | Repoint to ThabangVision and Ubunye endpoints. [VERIFIED] |
| Adobe, Photoshop, Lightroom, Premiere Pro, Lumetri, Essential Sound, Ultra Key, Warp Stabilizer | Adobe Inc. | third-party marks | Keep the non-affiliation disclaimer. ⚖️ **Rename Adobe feature names used as UI labels.** FilmCraft has "Lumetri" in 136 `.rs` occurrences and a "Premiere Pro Compatible" shortcut preset. Nominative use ("imports Lightroom catalogs") is lower risk than labelling panels with Adobe feature names. [VERIFIED counts; legal position is an ASSUMPTION] |
| Photoshop/Lightroom/Premiere trade dress | Adobe | UI imitation ("If you know Photoshop, you already know PhotoCraft") | ⚖️ Review before marketing. Avoid comparative marketing copy on thabangvision.com that uses Adobe marks beyond plain compatibility statements. |

## 3. Bundled third-party assets

| Asset | App(s) | Licence | Credited? | Action |
|---|---|---|---|---|
| Inter | all | OFL-1.1 | yes | keep the OFL text in packages |
| JetBrains Mono | Photo, Film | OFL-1.1 | yes | keep |
| Noto Serif | Film | OFL-1.1 | yes | keep |
| craft-fonts (BIZ UDPGothic, BIZ UDMincho, Shippori Mincho; Light also has Noto Sans CJK SC) | all official builds, via `CRAFT_FONTS_DIR` from `storytold/craft-fonts` | OFL-1.1 | yes, by external link | **Mirror** `craft-fonts` into `ubunye-ai-ecosystems` at a pinned commit, and copy its ATTRIBUTION into our packages instead of linking. "Noto" is a Google reserved font name: keep the files unmodified. |
| `epaint_default_fonts` (Ubuntu-Light, Hack, NotoEmoji, emoji-icon-font), via egui `default_fonts` | all | OFL-1.1 + **Ubuntu Font Licence 1.0** + MIT | **no**, in all three | Add to NOTICE and ATTRIBUTION; ship the UFL and OFL texts. [VERIFIED gap] |
| Lucide icons (110) and Feather-derived subset | Photo | ISC / MIT | yes | keep |
| `lasso-magnetic.svg` (Noun Project, "Lil' Seal") | Photo | **CC BY 3.0** | yes in ATTRIBUTION; needs **visible** credit | Add it to the About/credits screen, or replace the icon. |
| SCOWL en_US-50 word list | Photo | SCOWL licence (permissive) | yes | keep |
| CMYK ICC profile, runtime LUTs | Photo | CC0 (synthetic) | yes | keep |
| Camera colour matrices from rawtoaces-data, plus CIE 1931 CMFs | Light | Apache-2.0 + **CC-BY-4.0** | yes | keep the credit text |
| Camera profiles and tone curves (contributor-fitted; pixls.us CC0 source) | Light | MIT/Apache, CC0 | yes | keep |
| Showcase photos and frames (Adams, Lange, NASA, public-domain films, Musopen CC0) | all docs | PD / CC0 | yes | docs only; fine |
| DMG background by @XusBadia | all | MIT/Apache | yes | replace for branding |
| Translations | Light (uk), Photo (16 locales), Film (uk) | contributors, MIT/Apache | **8 Photo locales unlisted** (cs, es, fr, id, ja, ru, zh-hans, zh-hant) | Report upstream; credit them in our ATTRIBUTION. |
| NVIDIA `nvEncodeAPI.h` transcription | Film | MIT | yes | keep |

## 4. Runtime-downloaded models (not bundled; still our responsibility if we offer the download)

| Model | App | Licence | Risk |
|---|---|---|---|
| SAM 3 (facebook/sam3) | Light (AI masks) | Meta SAM Licence (**non-OSI**, usage restrictions) | ⚖️ Do not enable the in-app download under the Ubunye name until it is reviewed. |
| RawNIND UtNet2 | Light, optional `rawnind-model` feature | **GPL-3.0** weights | Keep the feature **off** in our releases. [VERIFIED: off by default] |
| AuraFace (fal) | Light (faces) | needs verifying | ⚖️ |
| YuNet / SFace (OpenCV Zoo) | Light | per-model (Apache-2.0 / MIT) | low |
| Whisper | Film, `whisper` feature | MIT (HF conversion Apache-2.0) | low |
| Kokoro-82M | Film, `neural-voices` feature | Apache-2.0, **not in ATTRIBUTION** | add credit |
| CMUdict | Film (TTS) | BSD-style, **not in ATTRIBUTION** | add credit |

## 5. Rust dependency licences

| | LightCraft | PhotoCraft | FilmCraft |
|---|---|---|---|
| Locked packages | 656 | 728 | 698 |
| Licence fields scanned (local registry cache only) | 345 | 260 | ~34% |
| Copyleft-only found | none | none | none |
| Notable | BSD-2/3, ISC, Zlib, CDLA-Permissive-2.0 (webpki-roots), Unicode-3.0, IJG; `self_cell` Apache-2.0 OR GPL-2.0 (we choose Apache) | same family | **MPL-2.0 symphonia** (unmodified; file-level copyleft, satisfied by crates.io source availability; add it to NOTICE) |
| `deny.toml` | none | none | none |

**[NEEDS TESTING]:** run `cargo deny check licenses` over the full graph of each repo. Proposed allow-list for our shared `deny.toml`: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, CDLA-Permissive-2.0, OFL-1.1, Ubuntu-font-1.0, BSL-1.0, CC0-1.0, IJG. MPL-2.0 goes on an explicit per-crate exception list. GPL, LGPL and AGPL are denied.

## 6. Codec patents (FilmCraft, and HEIF in Light/Photo) ⚖️

FilmCraft ships **software H.264 encode and decode, HEVC decode and AAC encode and decode** by default, and HEVC encode through hardware APIs. LightCraft and PhotoCraft read HEIF/HEIC (HEVC-based).

Pure Rust removes the copyleft risk but **not patent exposure**. Distributing free binaries from thabangvision.com to users (including in South Africa) needs a legal position on:
- which patent pools (Via LA / Access Advance / MPEG LA AVC) apply to free distribution;
- whether to ship H.264/HEVC/AAC software codecs, or delegate to the OS (Media Foundation, VideoToolbox, VA-API) where those are licensed.

Engineering mitigation: the codecs are already feature- and crate-separated, so a "royalty-free build profile" (AV1, VP9, Opus, ProRes/DNx decode) plus OS-delegated H.264/HEVC is feasible. [NEEDS TESTING]

## 7. Code provenance (clean-room claims) ⚖️

All three upstreams describe themselves as "clean-room reimplementations" and forbid reading GPL code (FFmpeg, LibRaw, darktable, dcraw and so on) or Adobe binaries. These are process rules applied to AI agents. About 60–85% of the commits are co-authored by Claude, Codex or Copilot, so **the claim cannot be audited from the repo.** LLM output can reproduce training-data code.

Recommendation before 1.0: run a **ScanCode Toolkit** licence and copyright scan, plus a code-similarity check against FFmpeg/libavcodec, x264, dav1d, libvpx, libopus, LibRaw and rawspeed for the codec and RAW crates. Record the results in each fork's `SECURITY.md` / `UPSTREAM.md`. [NEEDS TESTING]

## 8. Upstream infrastructure we depend on

| Dependency | Used for | Action |
|---|---|---|
| `storytold/craft-fonts` | CJK fonts in release builds | mirror to `ubunye-ai-ecosystems/craft-fonts` at a pinned SHA |
| `storytold/craftrules` (private) | coding standards referenced by AGENTS.md / CLAUDE.md | not accessible; write our own `CLAUDE.md` standards |
| `photocraft-corpus` and other test-corpus repos | oracle tests | mirror or pin with sha256 |
| Gitignored `plan/` folders | upstream roadmap and reference screenshots | not available; our ROADMAP.md is independent |
| Learning Machines LLC signing identities | macOS and Windows signing | replace with Ubunye certificates (see 06) |
