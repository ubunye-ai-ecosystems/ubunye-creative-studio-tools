# Ubunye Creative Studio: Phase 1 research and architecture

> **Staging location.** These documents are kept here temporarily because the
> `ubunye-ai-ecosystems/ubunye-creative-studio` repository does not exist yet. Once the
> plan is approved, they move into that repo with history (Milestone 1.5) and this folder is removed.
> Nothing here affects Track4World's code.

Date: 2026-10-10. Scope: research, repository strategy and architecture only.
No repositories were created. No upstream, Ubunye Engine or ThabangVision code was modified, and nothing was deployed.

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
7. **Build verified (partial).** The LightCraft CLI builds on Linux (rustc 1.97, 13 min 41 s on 4 vCPU). It passes a headless **import → edit → save → reopen → export** round trip on the CPU path. Nothing else has been built yet, and **nothing is production-ready.**
8. **Legal review is needed** for codec patents (H.264, HEVC, AAC), Adobe feature names in the UI (Lumetri and others), the provenance of AI-generated code, and the licences of downloadable models (SAM 3 non-OSI, RawNIND GPL-3.0).
9. **Ubunye Engine** is a Python data and ML pipeline framework with an MCP server. It has no image or video AI today. Integration is agent orchestration over MCP, not linking.
10. **The ThabangVision repo is private and could not be inspected** in this session. Deployment routes are therefore proposals.

## Decisions needed

1. Approve the **thin-fork** strategy (track upstream; branding through a brand layer) rather than a hard fork.
2. Approve **mirror-push** repo creation (independent repos, full history, `main` + tags only) instead of GitHub forks.
3. Approve Ubunye docs under `ubunye/`, with root `UPSTREAM.md` and `UPSTREAM_STATE.json`, to avoid conflicts with upstream's `CLAUDE.md`, `ROADMAP.md` and `SECURITY.md`.
4. App identifier namespace: `com.thabangvision.ubunye.*` or `org.ubunye.creative.*`, and the vendor legal name for installers.
5. Sync cadence: daily check plus a rolling PR (proposed), with a weekly merge target.
6. Start legal review (codecs, trademarks, provenance, model licences) and the signing paperwork (Apple Developer, Azure Trusted Signing).
7. Grant this session read access to `LeparaLaMapara/thabangvisionaistudios` so the `/studio` integration can be planned against the real code.
