#!/usr/bin/env bash
# Publish a fork prepared by prepare_fork.py to its (empty) GitHub repository.
#
#   scripts/publish_fork.sh <path-to-prepared-fork>        e.g. forks/ubunye-creative-light
#
# Prerequisites (a maintainer of ubunye-ai-ecosystems does these once):
#   1. Create an EMPTY private repo ubunye-ai-ecosystems/<slug> (no README, no licence, no .gitignore):
#        gh repo create ubunye-ai-ecosystems/<slug> --private \
#          --description "<Name>, based on <Upstream> by the ArtCraft team (MIT OR Apache-2.0)"
#   2. Have push access configured for git (gh auth login, or a credential helper).
#
# Safety: plain pushes only (never --force). If the remote already has diverging history,
# git rejects the push and nothing is overwritten.
set -euo pipefail

REPO=${1:?path to prepared fork}
cd "$REPO"
ORIGIN=$(git remote get-url origin)
SLUG=$(basename "$ORIGIN" .git)
echo "Publishing $(pwd) -> $ORIGIN"

# Sanity checks before anything leaves the machine.
test "$(git remote get-url --push upstream)" = "DISABLED" || { echo "upstream push URL must be DISABLED"; exit 1; }
test -f UPSTREAM_STATE.json || { echo "not a prepared fork (no UPSTREAM_STATE.json)"; exit 1; }
git diff --quiet main develop -- LICENSE-MIT LICENSE-APACHE NOTICE || { echo "LICENSE/NOTICE changed on develop; refusing"; exit 1; }
test "$(git rev-parse main)" = "$(git rev-parse upstream-main)" || { echo "main and upstream-main differ"; exit 1; }
test "$(git rev-parse main)" = "$(jq -r .last_integrated.sha UPSTREAM_STATE.json)" || echo "note: upstream moved since prepare; the first sync PR will bring it in"

# Push upstream history first (main + upstream-main + every upstream tag), then our develop branch.
git push origin refs/heads/main:refs/heads/main refs/heads/upstream-main:refs/heads/upstream-main
git push origin --tags
git push -u origin refs/heads/develop:refs/heads/develop

if command -v gh >/dev/null; then
  gh repo edit "ubunye-ai-ecosystems/$SLUG" --default-branch develop --delete-branch-on-merge \
    --enable-auto-merge=false --allow-forking=false || true
  # Branch protection (rulesets): see ubunye-creative-studio/04-repository-branch-strategy.md §2.
  for b in develop main upstream-main; do
    gh api -X PUT "repos/ubunye-ai-ecosystems/$SLUG/branches/$b/protection" --input - <<JSON || echo "warn: protection for $b not applied"
{
  "required_status_checks": $( [ "$b" = develop ] && echo '{"strict": false, "contexts": ["build + test (ubuntu-22.04)"]}' || echo null ),
  "enforce_admins": false,
  "required_pull_request_reviews": $( [ "$b" = upstream-main ] && echo null || echo '{"required_approving_review_count": 1}' ),
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false
}
JSON
  done
fi

echo "Done. Verify:"
echo "  git ls-remote origin | wc -l    # branches + tags"
echo "  gh repo view ubunye-ai-ecosystems/$SLUG --web"
