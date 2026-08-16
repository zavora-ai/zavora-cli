#!/usr/bin/env bash
# Property 4: no unwired modules.
#
# Every module reachable from lib.rs must have at least one non-test caller, be
# behind a feature that is off by default, or not exist. The v2 audit found six
# features that compiled, were unit-tested, and were never invoked at runtime —
# LSP initialization, multi-strategy compaction, the hook executor, and others.
# Unit tests hid the problem: the code was exercised, just never by the product.
#
# The census is deliberately coarse. It counts references from non-test source
# outside the module's own file. A module referenced only by its own tests, or
# only from src/tests.rs, is reported.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

# Modules that are legitimately reachable only through a disabled feature, or
# whose entire purpose is to be invoked by name from configuration. Each needs a
# reason.
declare -a ALLOWED=(
  "tests"          # the test module itself
  "tool_surface_tests" # ditto
  "prompt_surface_tests" # ditto
)

failures=0

# Top-level modules declared in lib.rs.
while IFS= read -r decl; do
  module="$(echo "${decl}" | sed -E 's/^.*pub mod ([a-z_0-9]+);.*$/\1/')"
  [[ -z "${module}" ]] && continue

  skip=0
  for allowed in "${ALLOWED[@]}"; do
    [[ "${module}" == "${allowed}" ]] && skip=1
  done
  [[ "${skip}" == 1 ]] && continue

  # Count references from production source outside the module's own files.
  refs="$(grep -rn --include='*.rs' "\b${module}::" src/ 2>/dev/null \
    | grep -v "^src/${module}\.rs:" \
    | grep -v "^src/${module}/" \
    | grep -v "^src/tests\.rs:" \
    | grep -v "^src/tool_surface_tests\.rs:" \
    | grep -cv '^\s*//' || true)"

  if [[ "${refs}" -eq 0 ]]; then
    echo "FAIL: module '${module}' has no non-test caller." >&2
    failures=$((failures + 1))
  fi
done < <(grep -E '^\s*pub mod [a-z_0-9]+;' src/lib.rs)

# Public functions that exist but are never called outside their own module and
# their own tests. This is the check that would have caught init_manager(),
# auto_compact() and HookExecutor.
declare -a WATCHED=(
  "src/tools/lsp.rs:init_manager"
  "src/compact.rs:auto_compact"
  "src/compact.rs:compact_to_target"
  "src/prompt_surface.rs:PromptSurface"
  "src/tools/secret_policy.rs:scan_command"
  "src/hooks.rs:HookExecutor"
)

for entry in "${WATCHED[@]}"; do
  file="${entry%%:*}"
  symbol="${entry##*:}"
  [[ -f "${file}" ]] || continue

  # References from other production files.
  external="$(grep -rn --include='*.rs' "\b${symbol}\b" src/ 2>/dev/null \
    | grep -v "^${file}:" \
    | grep -v "^src/tests\.rs:" \
    | grep -v "^src/tool_surface_tests\.rs:" \
    | grep -cv '^\s*//' || true)"

  # References from the symbol's own module, excluding its definition and
  # anything below the first #[cfg(test)] marker. A symbol called by reachable
  # code in its own module is wired, so this must count.
  production_part="$(awk '/^#\[cfg\(test\)\]/ { exit } { print }' "${file}")"
  internal="$(printf '%s\n' "${production_part}" \
    | grep -n "\b${symbol}\b" \
    | grep -vE "(pub )?(async )?fn ${symbol}\b|pub struct ${symbol}\b|pub enum ${symbol}\b" \
    | grep -cv '^\s*[0-9]*:\s*//' || true)"

  if [[ "${external}" -eq 0 && "${internal}" -eq 0 ]]; then
    echo "FAIL: '${symbol}' in ${file} is written but never called by the runtime." >&2
    failures=$((failures + 1))
  fi
done

if [[ "${failures}" -gt 0 ]]; then
  echo "" >&2
  echo "${failures} unwired item(s). Wire them, feature-gate them, or delete them." >&2
  exit 1
fi

echo "OK: no unwired modules or watched symbols."
