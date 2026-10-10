# Rebranding {{UPNAME}} → {{NAME}}

Status: **inventory only.** Branding changes start after the brand kit is approved (Milestone 4).

## Obligations
- The ArtCraft name, wordmark and logos (`docs/brand/`) are proprietary. Upstream's `docs/brand/LICENSE-brand.txt` requires them to be **removed** from any distributed modified version.
- We may say "Based on {{UPNAME}} by the ArtCraft team" in plain text. That credit appears in About, the README and the website.
- LICENSE-MIT, LICENSE-APACHE, NOTICE and ATTRIBUTION stay unchanged. Our notice is added alongside them, not in place of them.

## Target identity (from `brand.toml`)
| Item | Upstream | Ubunye |
|---|---|---|
| Product name | {{UPNAME}} | {{NAME}} |
| App / bundle / Flatpak id | `ai.storyteller.{{UP}}` | `{{APPID}}` |
| Vendor / manufacturer | Learning Machines LLC | Ubunye AI Ecosystems |
| Windows MSI UpgradeCode | upstream GUID | `{{UPGRADE}}` |
| Env var prefix | `{{UPENV}}_` | `{{ENVPFX}}_` (keep reading `{{UPENV}}_` as a fallback) |
| Links | getartcraft.com, discord.gg/artcraft, storytold | thabangvision.com/studio, ubunye-ai-ecosystems |
| AppImage update info | `gh-releases-zsync\|storytold\|{{UP}}\|…` | downloads.thabangvision.com zsync |

## Touchpoints (from the Phase-1 inventory)
{{TOUCHPOINTS}}

## How (keeps upstream merges cheap)
1. **Brand layer.** One module (`crates/ui-egui/src/brand.rs` or a new `ubunye-brand` crate) exposes the name, ids, links and logos, generated from `ubunye/brand.toml` by `build.rs`. Upstream call sites get a one-line indirection. Offer that hook upstream.
2. **Locale overlay.** Load `ubunye/i18n/<lang>.*` after upstream catalogs to override product-name strings, instead of editing upstream locale files.
3. **Packaging overlay.** `ubunye/packaging/` holds templated Info.plist, WiX, nfpm and Flatpak files rendered from `brand.toml`. Our release workflow uses these; upstream `packaging/` stays untouched.
4. **Brand-leak gate.** CI greps for `ArtCraft|getartcraft|ai\.storyteller|Learning Machines|storytold` outside the allow-list (LICENSE*, NOTICE, ATTRIBUTION*, UPSTREAM*, credits). It is informational until M4, then blocking.
