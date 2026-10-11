#!/usr/bin/env bash
# upload_web.sh <dir> <app>: put the top-level files of a web build at web/<app>/ in R2.
set -euo pipefail
dir=$1; app=$2
cd "$dir"
for f in *; do
  [ -f "$f" ] || continue
  case "$f" in *.br|*.gz|.cargo*) continue ;; esac
  npx -y wrangler@4 r2 object put "ubunye-creative-tools/web/$app/$f" --file "$f" --remote 2>&1 | grep -E "complete|rror" || true
  echo "  $app/$f"
done
