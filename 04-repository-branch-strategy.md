# 04 — Repository and branch strategy (proposal, awaiting approval)

## 1. Repositories in `ubunye-ai-ecosystems`

None of these exist yet. [VERIFIED: org search on 2026-10-10 returned 12 repos, none `ubunye-creative-*`]

| Repository | Created as | `upstream` remote | Purpose |
|---|---|---|---|
| `ubunye-creative-light` | full-history mirror of `storytold/lightcraft` | `https://github.com/storytold/lightcraft.git` | RAW development app |
| `ubunye-creative-photo` | full-history mirror of `storytold/photocraft` | `https://github.com/storytold/photocraft.git` | image editor |
| `ubunye-creative-film` | full-history mirror of `storytold/filmcraft` | `https://github.com/storytold/filmcraft.git` | video editor |
| `ubunye-creative-studio` | new, empty | — | cross-app architecture, brand kit, reusable CI workflows, `deny.toml`, packaging templates, release manifests, these documents |
| `craft-fonts` (mirror) | full-history mirror of `storytold/craft-fonts` | `storytold/craft-fonts` | pinned font input for release builds |

### Why mirror instead of using GitHub's "Fork" button

- A GitHub fork stays tied to the upstream "fork network". PRs default to targeting upstream, and visibility and settings are constrained. A mirror is an independent repo, so we can enable branch protection, secrets and Actions freely.
- Both preserve full history. A mirror push (`git push --mirror` of a fresh `git clone --bare`) keeps **every commit, author, date and tag** byte-for-byte. Nothing is squashed or rewritten.

### Creation procedure (per app; run only after approval)

```bash
# 1. Create an empty repo ubunye-ai-ecosystems/ubunye-creative-light (no README, no licence).
git clone --bare https://github.com/storytold/lightcraft.git
cd lightcraft.git
git push --mirror https://github.com/ubunye-ai-ecosystems/ubunye-creative-light.git   # keeps all tags
cd .. && rm -rf lightcraft.git

# 2. Working clone with the two required remotes
git clone https://github.com/ubunye-ai-ecosystems/ubunye-creative-light.git
cd ubunye-creative-light
git remote add upstream https://github.com/storytold/lightcraft.git
git remote set-url --push upstream DISABLED      # we never push to upstream by accident
git fetch upstream --tags

# 3. Create the Ubunye branches (see §2) from the imported upstream HEAD
git switch -c upstream-main upstream/main && git push -u origin upstream-main
git switch -c develop main && git push -u origin develop
```

Verification: `git rev-list --count main` must equal the upstream count, and `git log --format=%H upstream/main | head -1` must equal `UPSTREAM_STATE.json.last_integrated.sha` at import time.

**Note:** `--mirror` would also copy upstream's 62, 249 and 11 working branches. Before the first push, prune them to `main` + tags with `git push origin --delete <branch>` on the new repo, or by pushing selected refs: `git push origin 'refs/heads/main' 'refs/tags/*'`. Recommendation: push only `main` and tags.

## 2. Branch model

```
 upstream/main  (storytold, read-only)
      │  daily fetch
      ▼
 upstream-main  ── exact mirror of upstream/main, fast-forward only, bot-updated, never edited
      │  merge (never rebase) via integration PR
      ▼
 integration/upstream  ── rolling; created by the sync bot; CI + conflict report; deleted after merge
      │  PR → develop  (requires human approval)
      ▼
 develop  ── default branch; all Ubunye work lands here via PRs (feature/*, fix/*, brand/*)
      │  release PR
      ▼
 release/X.Y  ── stabilisation; only fixes cherry-picked from develop
      │  tag vX.Y.Z-ubunye.N
      ▼
 main  ── points at the latest stable release tag; what ThabangVision downloads are built from
```

| Branch | Writers | Protection |
|---|---|---|
| `upstream-main` | sync bot only | fast-forward only; force-push forbidden; no direct human commits |
| `integration/upstream` | sync bot, then humans for conflict resolution | deleted after merge |
| `develop` (default) | PRs only | 1+ approving review, required CI (`ci`, `deny`, `smoke`), no force-push, linear history **not** required (merge commits preserve upstream SHAs) |
| `feature/*`, `fix/*`, `brand/*` | contributors | none |
| `release/X.Y` | release managers via PR | review + CI, no force-push |
| `main` | release managers | review + tag signing; only fast-forward to a release tag |

Rules:
1. **Never rebase** Ubunye work onto upstream, and never rewrite `upstream-main`. Upstream is brought in with **merge commits**, so upstream SHAs stay reachable and attributable, and `git log --first-parent develop` shows our integration points.
2. Ubunye patches stay **minimal and isolated** (see `REBRANDING.md`) so integration merges stay cheap.
3. Fixes that are generic (security, crashes) go to upstream as PRs from a personal fork, so our patch set shrinks.
4. Tags: upstream tags stay as they are (`v0.5.0`). Our tags are `vX.Y.Z-ubunye.N`, where X.Y.Z is the upstream base version and N is our build. Example: `v0.5.0-ubunye.1`.

## 3. Repository documentation set (per app repo)

Each file is short, specific to the app, and kept current by PR:

| File | Content |
|---|---|
| `ARCHITECTURE.md` | Ubunye-facing summary of the crate graph (from research/), plus where our patches live. Links to upstream `docs/architecture*` instead of duplicating it. |
| `DEVELOPMENT.md` | toolchain (`rust-toolchain.toml` pin), OS prerequisites, build, test, web build, `cargo xtask ci`, how to run smoke tests |
| `UPSTREAM.md` | remotes, branch model, sync procedure, conflict playbook, list of Ubunye patches against upstream |
| `UPSTREAM_STATE.json` | machine-readable last integrated revision (schema in `templates/UPSTREAM_STATE.json`) |
| `ROADMAP.md` | the Ubunye roadmap only. Upstream's own `ROADMAP.md` stays untouched; ours lives at `ubunye/ROADMAP.md` (see the note below). |
| `SECURITY.md` | private disclosure channel (security@… or GitHub private advisories), supported versions, parser fuzzing status |
| `REBRANDING.md` | inventory of brand touchpoints and how they are overridden |
| `CLAUDE.md` | Ubunye agent rules (no force-push, keep attribution, branding goes through the brand layer, never edit `upstream-main`) |

**Avoiding conflicts with upstream files.** Lightcraft and Filmcraft already have `CLAUDE.md`. All three have `ROADMAP.md` or `AGENTS.md`, and PhotoCraft has `SECURITY.md`. Editing those files in place guarantees a merge conflict on every sync. Proposal:
- Put Ubunye docs under **`ubunye/`** (`ubunye/ARCHITECTURE.md`, `ubunye/ROADMAP.md`, …). Exception: the root `UPSTREAM_STATE.json` and `UPSTREAM.md`, which upstream doesn't have.
- For root `CLAUDE.md` and `SECURITY.md`, which upstream owns: add **one line at the very top** pointing to `ubunye/CLAUDE.md` / `ubunye/SECURITY.md`. A one-line prepend rarely conflicts.

This deviates slightly from "files at the root". It is flagged for approval.
