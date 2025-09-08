#!/usr/bin/env bash
set -euo pipefail

# Guardrail: forbid stray usages of __init__.py and .__init__ outside centralized modules/tests

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

allow_paths=(
  "crates/cribo/src/python/constants.rs"
  "crates/cribo/src/python/module_path.rs"
  "crates/cribo/src/**/tests/**"
  "crates/cribo/tests/**"
)

cd "$ROOT_DIR"

echo "Running init-usage guardrail..."

violations=0

# Find occurrences excluding allowlist
# Only flag usages inside string literals to avoid comments and docs.
# Forbid occurrences of "__init__.py" and ".__init__" in string literals.
if rg -n '"[^"]*(__init__\\.py|\\.__init__)[^"]*"' crates/cribo/src \
  --glob '!crates/cribo/src/python/constants.rs' \
  --glob '!crates/cribo/src/python/module_path.rs' \
  --glob '!**/*tests*/*' \
  --glob '!crates/cribo/src/**/tests/**' \
  --glob '!crates/cribo/tests/**' ; then
  echo "Error: Found forbidden raw '__init__' usages outside centralized modules/tests." >&2
  violations=1
fi

exit "$violations"
