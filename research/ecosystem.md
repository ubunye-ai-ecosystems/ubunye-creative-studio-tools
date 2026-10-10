# Part B — Existing ecosystem (read-only survey, 2026-10-10)

Access summary:
- GitHub MCP `get_file_contents` / `gh api`: **denied** for both target repos (session is scoped to `leparalamapara/track4world` only). [VERIFIED]
- `search_repositories` works (metadata only). [VERIFIED]
- Fallback `git clone --depth 1`: **succeeded** for `ubunye-ai-ecosystems/ubunye_engine` (public); **failed** for `LeparaLaMapara/thabangvisionaistudios` (asks for credentials; repo is private). [VERIFIED]
- `add_repo` was NOT called (not requested by the user). Granting session access to `thabangvisionaistudios` is required for the detailed inspection. [NEEDS TESTING]

## 1. ubunye-ai-ecosystems/ubunye_engine

Cloned at `2e05292` (2026-09-30), into `a local shallow clone`.

| Item | Finding | Tag |
|---|---|---|
| What it is | "Config-driven Spark framework for data and ML pipelines": YAML config + Python `Task` class, run on pandas, local Spark, K8s, Glue, Dataproc, Azure Container Apps, Databricks with byte-identical outputs (`ubunye prove`). | [VERIFIED] |
| Language | Python ≥3.10, setuptools, PyPI package `ubunye-engine` v0.8.0, CLI `ubunye` (typer). | [VERIFIED] |
| Structure | `ubunye/{adapters,backends,cli,config,core,deploy,interfaces,lineage,llm,models,orchestration,plugins,proving,telemetry,testing}`, `mcp_server.py`, `api.py`; plus `docs/` (mkdocs), `examples/`, `pipelines/`, `benchmarks/`, `tests/`, 14 GitHub workflows (ci, tests, docs, publish_pypip, release_on_version_bump, Databricks deploys, llm_replay…). | [VERIFIED] |
| Licence | MIT, © 2025-2026 Ubunye AI Ecosystems (author Thabang Mashinini-Sekgoto). Compatible with the craft apps' MIT OR Apache-2.0. | [VERIFIED] |
| AI capabilities | **No image/video features**: no photo enhancement, background removal, captioning, super-resolution, Whisper/ONNX vision code found by grep. What exists: an `llm` port (`anthropic`, `openai_compatible` backends; budgets, price estimates, record/replay of calls), ML model registry (sklearn/torch/mlflow, local/S3/GCS/Databricks volume), plugin entry points for backends/secrets/connectors. | [VERIFIED] (grep-based) |
| Integration surfaces | (a) **MCP server** (`ubunye mcp`, `mcp>=2,<3` extra): tools `tasks`, `doctor`, `plan`, `runs`, `record`, `gate`, `focus`, `run` (only with `--allow-run`); read-only by default. (b) Python API (`ubunye/api.py`) and CLI. (c) `rest_api` reader/writer *connectors* (client side, not a server). **No HTTP server/REST API exposed** (no FastAPI/Flask/uvicorn). | [VERIFIED] |
| Fit for Rust desktop apps | Best path: Rust app spawns `ubunye mcp` (stdio) or `ubunye` CLI as a subprocess, or the Rust app's own MCP server is called by the same agent that drives Ubunye. A batch "export → run pipeline (e.g. captioning via LLM port) → import" flow is plausible; real-time photo AI is not what this engine does. Requires Python on the user's machine — conflicts with the craft apps' "pure Rust, no runtime" stance. | [ASSUMPTION] |

Sibling repo `ubunye-ai-ecosystems/flood-extent-mapping` serves a U-Net as ONNX (the only image-ML artefact in the org) — a template if an ONNX-served image model is wanted. [VERIFIED from description only]

## 2. LeparaLaMapara/thabangvisionaistudios

Metadata via `search_repositories` [VERIFIED]:
- **Private** repo, TypeScript, default branch **`main`**, created 2026-03-06, last push 2026-10-10, size ~6 MB, user has admin permissions.
- Description: "ThabangVision: a marketplace to book South African photographers, videographers and production gear. Verified creators, equipment rentals, PayFast and Paystack payments, and Ask Ubunye for creator matching. Live at thabangvision.com." Homepage https://thabangvision.com.

Not inspectable this session (contents access denied) [NEEDS TESTING]:
- Framework: TypeScript; Next.js likely [ASSUMPTION] — not confirmed.
- Hosting: Vercel likely [ASSUMPTION] (a Vercel MCP connector is attached to this session and could confirm via `list_projects` — not done, out of scope for read-only repo survey).
- Routing structure (`app/` vs `pages/`), where `/studio` catalogue + download pages would fit, env/config for R2 / Supabase / Cloudinary (Supabase and Cloudinary connectors are attached to this session, suggesting use, but unverified) [ASSUMPTION], production branch beyond `main`, CI/preview deployments: all [NEEDS TESTING].
- Next step: ask the user to grant access (add_repo for `LeparaLaMapara/thabangvisionaistudios`, read), then inspect `package.json`, `next.config.*`, `app/`, `.env.example`, `vercel.json`, `.github/workflows/`, `list_branches`.

Suggested placement once confirmed [ASSUMPTION]: `app/studio/page.tsx` (catalogue of Light/Photo/Film builds), `app/studio/[app]/page.tsx` (per-app detail + download), download binaries hosted in GitHub Releases of the forks (or R2/Blob), page reading a static `releases.json` manifest; avoid Cloudinary for multi-hundred-MB binaries.

## 3. Do ubunye-creative-light/photo/film/studio repos exist?

`search_repositories org:ubunye-ai-ecosystems` returned 12 repos (visible to this token, incl. private): `ubunye_engine`, `tfilterpy` (archived), `masepala`, `tfilterspy`, `ubunye-infra` (private), `demo-repository` (private), `ubunye_engines` (private), `flood-extent-mapping`, `uaie-discussion-forum`, `.github`, `ubunye_compiler` (private), `ubunye-examples`.
**No `ubunye-creative-*` repositories exist.** [VERIFIED]

## 4. Branding note relevant to forks

The craft repos ship `docs/brand/LICENSE-brand.txt`: ArtCraft name/marks are trademarks, not open source; a distributed modified version **must remove the ArtCraft marks** and must not be presented as ArtCraft. Rebranding to "Ubunye Creative …" is therefore required, not optional. Product names (LightCraft/PhotoCraft/FilmCraft) are not explicitly trademarked in that file, but using new names avoids confusion. [VERIFIED — text of LICENSE-brand.txt]
