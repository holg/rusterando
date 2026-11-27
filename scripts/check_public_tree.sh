#!/usr/bin/env bash
# Fails when the public tree mentions what must stay private to one
# deployment: real shop names, addresses, phone numbers, device ids …
#
# The patterns are private themselves, so they are not in this repo: one
# extended regex per line in site/forbidden-patterns.txt (gitignored, like
# all of site/ except README.md and example/; `#` starts a comment).
#
#   scripts/check_public_tree.sh            tracked + new (not ignored) files
#   scripts/check_public_tree.sh --cached   the index (pre-commit hook)
#
# Install as a hook:  ln -s ../../scripts/check_public_tree.sh .git/hooks/pre-commit
# (as a hook it checks the index; it is called without arguments there).
#
# Migrations named before FROZEN_BEFORE are left as they are: sqlx checksums
# applied migrations, so editing them would stop every existing deployment.
# Code that must match such a migration's literals marks those lines with a
# `frozen-seed` comment; they are skipped too.
set -euo pipefail

FROZEN_BEFORE=20261128

cd "$(git rev-parse --show-toplevel)"
patterns=site/forbidden-patterns.txt
if [[ ! -f "$patterns" ]]; then
    echo "check_public_tree: no $patterns — nothing to check" >&2
    exit 0
fi
regex=$(grep -v -E '^[[:space:]]*(#|$)' "$patterns" | paste -sd '|' -)
[[ -z "$regex" ]] && exit 0

# As the pre-commit hook (or with --cached): what is about to be committed.
mode=--untracked
if [[ "${1:-}" == "--cached" || "$(basename "$0")" == "pre-commit" ]]; then
    mode=--cached
fi

excludes=()
for f in migrations/*.sql; do
    b=$(basename "$f")
    [[ "${b:0:8}" < "$FROZEN_BEFORE" ]] && excludes+=(":(exclude)$f")
done

status=0
# A source line that has to mirror a frozen migration byte for byte (e.g.
# seed ids it recognises) carries the marker `frozen-seed` and is skipped.
if hits=$(git grep -I -n -i -E "$mode" -e "$regex" -- . "${excludes[@]}" | grep -v 'frozen-seed'); then
    echo "$hits"
    status=1
fi

if [[ "$mode" == --cached ]]; then
    names=$(git ls-files --cached)
else
    names=$(git ls-files --cached --others --exclude-standard)
fi
if bad=$(grep -i -E "$regex" <<<"$names"); then
    echo "file names:"
    echo "$bad"
    status=1
fi

if [[ $status -ne 0 ]]; then
    echo "check_public_tree: private names in the public tree (patterns: $patterns)" >&2
fi
exit $status
