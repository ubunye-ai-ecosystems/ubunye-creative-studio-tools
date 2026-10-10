# Security policy: {{NAME}}

## Reporting
Please **do not** open public issues for vulnerabilities. Use GitHub's private reporting ("Security" → "Report a vulnerability") on `ubunye-ai-ecosystems/{{SLUG}}`. We acknowledge within 5 working days.
If the issue also affects upstream {{UPNAME}}, we coordinate disclosure with upstream ({{UPURL}}).

## Supported versions
Only the latest `vX.Y.Z-ubunye.N` release. Pre-1.0 builds are previews.

## Threat model summary
- **Untrusted files.** Every importer ({{FORMATS}}) parses attacker-controlled data with in-house Rust parsers. Fuzzing status: {{FUZZ}}.
- **Local automation.** The `--control` port and the MCP server can read and write files. They are off by default; never expose them beyond localhost. {{CONTROL}}
- **Downloads.** Optional AI model downloads are user-initiated and hash-pinned upstream.
- **Supply chain.** `deny.toml` + `cargo audit` in CI (`ubunye-audit.yml`); toolchain pinned; Actions pinned by SHA in Ubunye workflows.

## Known advisories at import (cargo audit, 2026-10-09 DB)
{{ADVISORIES}}
