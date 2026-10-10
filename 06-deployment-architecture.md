# 06 — Deployment architecture: ThabangVision downloads and browser apps

Status: **proposal**. Nothing has been created in Cloudflare, Vercel or the ThabangVision repo.
The ThabangVision repository (`LeparaLaMapara/thabangvisionaistudios`) is private and was not readable in this session. Its framework, hosting and routes are therefore [ASSUMPTION] (TypeScript, likely Next.js on Vercel) until the repo is inspected.

## 1. Overview

```
                  GitHub (ubunye-ai-ecosystems/ubunye-creative-*)
                  tag vX.Y.Z-ubunye.N on release/*  ──►  ubunye-release.yml
                           │ build matrix: macOS universal · Windows x64/arm64 · Linux x64/arm64 · web
                           │ sign (Apple Developer ID + notarise; Windows Azure Trusted Signing; Linux minisign/GPG)
                           │ SHA256SUMS + SBOM (CycloneDX) + THIRD-PARTY-LICENSES + provenance attestation
                           ▼
                 ┌──────────────── Cloudflare R2 bucket: ubunye-releases ────────────────┐
                 │  /light/0.5.0-ubunye.1/ubunye-creative-light-0.5.0-ubunye.1-macos-universal.dmg  │
                 │  /light/0.5.0-ubunye.1/SHA256SUMS, SHA256SUMS.sig, sbom.cdx.json                  │
                 │  /manifests/light/stable.json   /manifests/catalog.json  (small, short cache)    │
                 └─────────────────────────────────────────────────────────────────────────────────┘
                           │ custom domain                                   │
            downloads.thabangvision.com (R2 public bucket via CF custom domain)
                           │                                                  │
   thabangvision.com/studio (existing site, reads catalog.json at build/ISR)  │
            "Download for macOS" ─────────────────────────────────────────────┘
   light.studio.thabangvision.com / photo.… / film.…  ← Cloudflare Pages (web builds, static wasm)
```

Visitors never see GitHub. Every download link points at `downloads.thabangvision.com`.

## 2. Release storage: Cloudflare R2 (recommended)

| Criterion | R2 | GitHub Releases | Vercel Blob | Cloudinary |
|---|---|---|---|---|
| Egress cost for 100–500 MB installers | **none** (zero egress fees) | free, but github.com URLs and a GitHub-branded page | billed per GB | not designed for binaries; size limits |
| Custom domain | yes (`downloads.thabangvision.com`) | no | limited | n/a |
| S3 API for CI uploads | yes | n/a | no | n/a |
| Immutable versioned paths + rollback | yes (our layout) | yes | yes | no |

Decision: **R2** is primary storage. GitHub Releases on the Ubunye repos is an optional secondary mirror for developers. Cloudinary is not used for binaries.

Bucket layout and rules:
- Versioned objects are **immutable**: `/<app>/<version>/<file>`, `Cache-Control: public, max-age=31536000, immutable`.
- Mutable pointers are small JSON manifests: `/manifests/<app>/<channel>.json` and `/manifests/catalog.json`, `Cache-Control: max-age=300`.
- CI writes through an R2 API token scoped to the bucket, stored as a GitHub environment secret (`release` environment, required reviewers). It is not readable by PR workflows.
- Lifecycle: keep every stable release; delete `nightly/` builds after 30 days.

## 3. Release manifest and versioning

- Version: `X.Y.Z-ubunye.N` (upstream base plus our build). Channels: `stable` and `beta` (and `nightly` internally).
- Manifest per app and channel: see [`templates/release-manifest.json`](templates/release-manifest.json). It lists the version, release date, minimum OS, and per-platform `url`, `size`, `sha256` and `signature`, plus release notes, `upstream_sha`, and the licence/attribution URLs.
- `catalog.json` aggregates all apps for the `/studio` page: name, tagline, icon, screenshots, stable version, platforms, web app URL, maturity badge ("Preview", never "Production" until the acceptance criteria in 07 are met).
- The manifest is signed with **minisign** (ed25519). The private key lives in the release environment secret. The public key is embedded in the apps (for updates) and published on the site.

## 4. Signing

| Platform | Mechanism | Prerequisite (owner action) |
|---|---|---|
| macOS | Developer ID Application certificate, hardened runtime, `notarytool`, stapled DMG | Apple Developer Program membership for the Ubunye or ThabangVision legal entity (≈ USD 99/yr) |
| Windows | Azure Trusted Signing (upstream scripts already support it), or an OV/EV code-signing certificate | Azure subscription plus identity validation; check eligibility for a South African entity [NEEDS TESTING] |
| Linux | minisign/GPG signature over `SHA256SUMS`; Flatpak repo GPG key; AppImage embedded signature | key generation ceremony; offline backup |
| Web | served over HTTPS; Subresource Integrity on the JS loader | — |

New identities (replacing upstream; approved 2026-10-10): bundle and app IDs `org.ubunye.creative.light` / `.photo` / `.film`, **new MSI UpgradeCodes** (one stable GUID per app, never reused from upstream; see `brand/identities.toml`), and vendor/manufacturer "Ubunye AI Ecosystems".

## 5. Update delivery and rollback

- Upstream uses AppImage zsync pointing at `storytold` GitHub releases. **Repoint** it to `zsync|https://downloads.thabangvision.com/light/stable/…zsync` or disable it. There is no in-app updater for macOS or Windows upstream [VERIFIED for Light/Photo/Film packaging]. v1 proposal: an in-app "check for updates" that reads the signed `stable.json` manifest and opens the download page. No silent self-replacement until signing is in place.
- **Rollback**: point `stable.json` back at the previous version (one JSON write). The binaries remain in place because paths are immutable. The site and apps follow the manifest within the 5-minute cache.
- Staged rollout (later): add `rollout_percent` to the manifest; the app hashes its install ID into a bucket.

## 6. Browser-hosted apps

| | Light | Photo | Film |
|---|---|---|---|
| Output | static site, ~13.5 MB wasm | static site, ~19 MiB wasm (24 MiB gate) | static site, ~13 MB wasm |
| Headers | COOP/COEP optional | none required | serve `COOP: same-origin` + `COEP: require-corp` (docs conflict; harmless) |
| Storage | OPFS | localStorage + IndexedDB | OPFS (media copies ≤4 GiB) |

**Hosting: Cloudflare Pages** (one project per app). Reasons: upstream already ships `_headers` for Pages and sizes files for its 25 MiB limit; free bandwidth; preview deployments per branch; same account as R2.
Domains: `light.studio.thabangvision.com`, `photo.studio.thabangvision.com`, `film.studio.thabangvision.com`. Using separate origins isolates OPFS/IndexedDB per app and keeps the main site's cookies out of the wasm apps.
Alternative: Vercel, if the main site is already there. Vercel's static asset limits and bandwidth pricing for 13–19 MB wasm per visit make Pages preferable [ASSUMPTION; compare once the plan is known].
Each web app is labelled **"Preview — runs entirely in your browser; files never leave your device"**, which is accurate per upstream docs.

## 7. ThabangVision site changes (feature branch + preview only)

All changes happen on a branch such as `feature/studio-catalogue` in `thabangvisionaistudios`, reviewed through Vercel (or equivalent) **preview deployments**. Production (`main`) is untouched until explicit approval.

Proposed routes [ASSUMPTION: Next.js app router]:
- `/studio`: catalogue of the three apps (from `catalog.json`): description, screenshots, "Download" (OS auto-detected), "Open in browser", system requirements, licence and credits ("Based on LightCraft by the ArtCraft team, MIT/Apache-2.0").
- `/studio/[app]`: detail page, all platforms, checksums, release notes, previous versions.
- `/studio/[app]/download/[platform]`: optional redirect endpoint (302 to R2) for download analytics without third-party trackers.
- `/studio/licenses`: aggregated attribution, the Adobe non-affiliation disclaimer, codec notices.

Data flow: the site fetches `https://downloads.thabangvision.com/manifests/catalog.json` at build time with ISR (revalidate every 300 s). No GitHub API calls and no GitHub account needed by visitors.

## 8. Environments

| Env | R2 prefix | Pages branch | Site |
|---|---|---|---|
| nightly / dev | `nightly/` | `develop` preview | preview deployments only |
| beta | `/<app>/<ver>` + `beta.json` | `release/*` | preview deployment |
| stable | `/<app>/<ver>` + `stable.json` | `main` | production, **after approval** |
