#!/bin/sh
# Fails when tracked files mention private project names. Run before publishing.
# The term list is private too: docs/private/private-terms.txt (ignored by git),
# or the file named by PRIVACY_TERMS_FILE. One case-insensitive regex per line.
# Without a term list the check is skipped (e.g. in the public repo's CI).
set -eu
cd "$(dirname "$0")/.."
terms="${PRIVACY_TERMS_FILE:-docs/private/private-terms.txt}"
if [ ! -f "$terms" ]; then
  echo "privacy check skipped: no term list at $terms"
  exit 0
fi
pattern=$(grep -v '^#' "$terms" | grep -v '^$' | paste -sd'|' -)
if git grep -inE "$pattern" -- ':!CLAUDE.local.md' ':!docs/private' ':!scripts/check-privacy.sh'; then
  echo "privacy check failed: private names above must not be published" >&2
  exit 1
fi
echo "privacy check ok"
