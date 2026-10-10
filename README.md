# Ubunye Creative Studio Tools

Free, open source creative tools from Ubunye AI Ecosystems, distributed through ThabangVision.
This repository coordinates the suite: architecture, upstream sync, branding and releases.

| App | Repository | Based on |
|---|---|---|
| Ubunye Creative Light (RAW photo development) | [ubunye-creative-light](https://github.com/ubunye-ai-ecosystems/ubunye-creative-light) | LightCraft by the ArtCraft team |
| Ubunye Creative Photo (image editing) | [ubunye-creative-photo](https://github.com/ubunye-ai-ecosystems/ubunye-creative-photo) | PhotoCraft by the ArtCraft team |
| Ubunye Creative Film (video editing) | [ubunye-creative-film](https://github.com/ubunye-ai-ecosystems/ubunye-creative-film) | FilmCraft by the ArtCraft team |

The Phase 1 documents were first written in a staging folder on `LeparaLaMapara/Track4World`
(branch `ccr-2c411eb0-0d701l`). They were moved here with their commit history on 2026-10-10.

Date: 2026-10-10. Scope: research, repository strategy and architecture only.
At the time of writing no repositories had been created (see Milestone 1 status for what exists now). No upstream, Ubunye Engine or ThabangVision code was modified, and nothing was deployed.

## Deliverables

| # | Document | Contents |
|---|---|---|
| 1 | [01-architecture-reports.md](01-architecture-reports.md) and [research/](research/) | Architecture report for each upstream app, the cross-repo comparison, and the ecosystem survey |
| 2 | [02-licensing-inventory.md](02-licensing-inventory.md) | Licences, NOTICE obligations, trademarks, assets, models, dependency licences, codec patents, provenance |
| 3 | [03-compatibility-build-assessment.md](03-compatibility-build-assessment.md) | Build evidence, platform matrix, toolchain, hardware, cross-app compatibility |
| 4 | [04-repository-branch-strategy.md](04-repository-branch-strategy.md) | Repos, history-preserving import, remotes, branch model, doc set |
| 5 | [05-upstream-sync-design.md](05-upstream-sync-design.md) | Daily monitoring, classification, integration PRs, safety rules |
| 6 | [06-deployment-architecture.md](06-deployment-architecture.md) | R2 downloads, manifests, signing, updates, rollback, browser hosting, `/studio` pages |
| 7 | [07-implementation-plan.md](07-implementation-plan.md) | Milestones, acceptance criteria, risk register, production-ready definition |
| — | [templates/](templates/) | `UPSTREAM_STATE.json`, draft `upstream-sync.yml` (inactive), `release-manifest.json`, `smoke-light.sh` (verified) |

## Key findings

1. **The upstreams are 10 days old and move extremely fast.** They were created on 2026-09-30 and already have 1,098–1,552 commits each, about 100–370 commits a day, versions 0.5–0.6, and are largely AI-agent co-authored. This one fact shapes the whole strategy: **thin forks, an isolated brand layer, a single rolling integration PR**.
2. **All three are pure Rust** (egui 0.36 / wgpu 30), with in-house RAW, PSD and video codec and container implementations. There is no FFmpeg, LibRaw or C dependencies in the paths checked. They share a layout and dependency versions but **no code**: same-named crates are only 4–27% similar. A shared Studio *code* core is not advisable yet. Share brand, CI, packaging and policy instead.
3. **Licences are permissive (MIT OR Apache-2.0), but rebranding is mandatory.** The ArtCraft brand licence requires forks to remove the ArtCraft marks. Upstream installer identities (`ai.storyteller.*`, "Learning Machines LLC", MSI UpgradeCodes) must be replaced. LightCraft's `segment` and `fetch` crates are Apache-2.0 only.
4. **Attribution gaps already exist upstream.** These include the egui default fonts (Ubuntu Font Licence), Kokoro and CMUdict, 8 PhotoCraft translations, and NOTICE files missing from most packages. We must fix these in our packages.
5. **Security.** `cargo audit`: PhotoCraft 0 vulnerabilities; LightCraft 7 and FilmCraft 5, mostly through one alpha TLS crate. Tens of thousands of lines of young, unfuzzed parsers handle untrusted media. FilmCraft's and LightCraft's local control ports have no authentication.
6. **Testing gap.** FilmCraft runs no tests in CI, and LightCraft runs them only on FreeBSD. Our first engineering milestone is a 3-OS CI gate.
7. **Builds verified (partial).** All three CLIs build on Linux with rustc 1.97 (Light 13 min 41 s, Photo 8 min 44 s, Film 7 min 18 s on 4 vCPU) and pass a headless **import → edit → save → reopen → export** round trip on the CPU path. GUI, Windows, macOS and web are not yet built, and **nothing is production-ready.**
8. **Legal review is needed** for codec patents (H.264, HEVC, AAC), Adobe feature names in the UI (Lumetri and others), the provenance of AI-generated code, and the licences of downloadable models (SAM 3 non-OSI, RawNIND GPL-3.0).
9. **Ubunye Engine** is a Python data and ML pipeline framework with an MCP server. It has no image or video AI today. Integration is agent orchestration over MCP, not linking.
10. **ThabangVision repo inspected (2026-10-10, from the dev box).** It is Next.js 16 on Vercel. **`/studio` is already in use** by the live photography and video productions portfolio (`app/(platform)/studio`), and `/ubunye-ai-studio` also exists. The software catalogue therefore lives at **`/tools`** (see decision 8 and doc 06).

## Decisions (2026-10-10)

| # | Decision | Status |
|---|---|---|
| 1 | **Thin fork**: track upstream; branding goes through a brand layer | ✅ approved |
| 2 | **Mirror-push** repo creation (independent repos, full history, `main` + tags) | ✅ approved |
| 3 | Ubunye docs under `ubunye/`, plus root `UPSTREAM.md` and `UPSTREAM_STATE.json`; one-line pointers prepended to upstream `AGENTS.md`/`CLAUDE.md`/`SECURITY.md` | ✅ approved |
| 4 | Identifier namespace **`org.ubunye.creative.*`**; vendor "Ubunye AI Ecosystems". Canonical IDs and Windows UpgradeCodes are in [`brand/identities.toml`](brand/identities.toml). (Spoken as "org.ubuntu.creative"; we use **ubunye**, since "Ubuntu" is a Canonical trademark.) | ✅ approved |
| 5 | Sync cadence: **daily check** with one rolling integration PR per app | ✅ approved |
| 6 | Legal review (codecs, trademarks, provenance, model licences) and signing paperwork | ⏸ deferred by the owner until after the build-out |
| 7 | Read access to `LeparaLaMapara/thabangvisionaistudios` | ✅ done from the dev box |
| 8 | Suite name **Ubunye Creative Studio Tools**; coordination repo `ubunye-creative-studio-tools`; catalogue at `thabangvision.com/tools` (`/studio` is taken) | ✅ decided 2026-10-10 |
| 9 | Release storage: a **new dedicated Cloudflare R2 bucket**, not the existing `edu-papers` bucket and not Supabase Storage (egress is billed there; installers are 75 to 95 MB each, about 23 files per app per release) | ✅ decided 2026-10-10 |
| 10 | Repos start **private with GitHub Actions disabled**. Upstream's own workflows (CI, nightly perf, FreeBSD, release) would otherwise run on our private minutes, and two of our workflows use actions the org allow list blocks. Go public, then enable Actions, after the ArtCraft marks are removed (the brand licence requires removal before publishing a modified version) | ✅ decided 2026-10-10 |

## Milestone 1 status

| Step | Status |
|---|---|
| Fork preparation tooling: [`scripts/prepare_fork.py`](scripts/prepare_fork.py), [`fork-overlay/`](fork-overlay/), [`scripts/publish_fork.sh`](scripts/publish_fork.sh) | ✅ built and tested (Linux and Windows; the script now reads and writes UTF-8 explicitly, which Windows needs) |
| Repos created in `ubunye-ai-ecosystems`: light, photo, film, studio-tools (private, Actions off) | ✅ 2026-10-10 |
| Forks published with full history: Light 1,552 commits / 6 tags (main `b70ae174`); Photo 1,167 / 8 (main `5bb768b8`); Film 1,098 / 6 (main `98ba9acb`). Branches `main` = `upstream-main` = upstream HEAD, `develop` = plus one scaffolding commit; default branch `develop`. Remote refs verified equal to local | ✅ 2026-10-10 |
| Per-app smoke tests (import, edit, save, reopen, export) for the three CLIs on Linux | ✅ all pass (cloud session) |
| `cargo deny check` with the Ubunye `deny.toml` | ✅ all three pass |
| Daily sync workflow simulated against real upstream history | ✅ passes ([details](05-upstream-sync-design.md#7-verification)) |
| Branch protection | ⛔ not available for private repos on the GitHub Free plan. Applied when the repos go public |
| Windows builds | ⏳ in progress on the dev box |

## How to finish Milestone 1 (once the repo-creation block is fixed)

Run this on any machine with `git`, `python3` (3.11+), `jq` and an authenticated `gh` that has admin rights on `ubunye-ai-ecosystems`:

```bash
for app in light photo film studio; do
  gh repo create ubunye-ai-ecosystems/ubunye-creative-$app --private \
    --description "Ubunye Creative Studio: $app"
done
for app in light photo film; do
  python3 -I scripts/prepare_fork.py $app ./forks       # full-history clone + Ubunye scaffolding on develop
  scripts/publish_fork.sh ./forks/ubunye-creative-$app  # plain pushes: main, upstream-main, tags, develop
done
```

Or grant the Claude GitHub App repository-administration access on the org, and Claude will run these steps.
After publishing, set the repo variable `UBUNYE_SYNC_APP_ID` and the secret `UBUNYE_SYNC_APP_KEY` (a GitHub App with contents and pull-request write access) so the daily sync PRs trigger CI. Without them, the sync falls back to `GITHUB_TOKEN`.
