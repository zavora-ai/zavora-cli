#!/usr/bin/env bash
# Property 12: every documented example works as written.
#
# The v2 audit found the README's flagship profile example was not loadable:
# `auto_compact_enabled` lives on RuntimeConfig, not ProfileConfig, and
# ProfileConfig sets deny_unknown_fields, so copying the block verbatim produced
# "invalid profile configuration". A human reading the file could not tell.
# This makes documentation executable rather than aspirational.
#
# Two checks:
#   1. Every fenced `toml` block that looks like profile configuration must load.
#   2. Every `zavora-cli <subcommand>` in a fenced `bash` block must name a real
#      subcommand.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

DOCS=(README.md QUICKSTART.md)
# docs/history holds superseded v1-era documents kept for provenance. They are
# explicitly not current, so their examples are not held to Property 12.
while IFS= read -r doc; do DOCS+=("${doc}"); done < <(find docs -name '*.md' -not -path 'docs/history/*' | sort)

BIN="target/debug/zavora-cli"
if [[ ! -x "${BIN}" ]]; then
  echo "==> building ${BIN}"
  cargo build --quiet
fi

SCRATCH="$(mktemp -d)"
trap 'rm -rf "${SCRATCH}"' EXIT

failures=0

# ---------------------------------------------------------------------------
# 1. Configuration examples must load.
# ---------------------------------------------------------------------------
extract_toml_blocks() {
  # Emits: <file>\t<start-line>\t<base64 block>
  python3 - "$@" <<'PY'
import base64, re, sys
for path in sys.argv[1:]:
    try:
        lines = open(path, encoding="utf-8").read().splitlines()
    except OSError:
        continue
    inside, start, buf = False, 0, []
    for n, line in enumerate(lines, 1):
        if not inside and re.match(r"^\s*```toml\s*$", line):
            inside, start, buf = True, n, []
            continue
        if inside and re.match(r"^\s*```\s*$", line):
            block = "\n".join(buf)
            # Only blocks that declare a profile are loadable configuration.
            if "[profiles." in block:
                print(f"{path}\t{start}\t{base64.b64encode(block.encode()).decode()}")
            inside = False
            continue
        if inside:
            buf.append(line)
PY
}

while IFS=$'\t' read -r file line block; do
  [[ -z "${file}" ]] && continue
  workdir="${SCRATCH}/cfg"
  rm -rf "${workdir}"
  mkdir -p "${workdir}/.zavora"
  printf '%s' "${block}" | base64 --decode > "${workdir}/.zavora/config.toml"

  if ! output="$(cd "${workdir}" && NO_COLOR=1 "${REPO_ROOT}/${BIN}" profiles show 2>&1)"; then
    echo "FAIL: ${file}:${line} configuration example did not load." >&2
    echo "${output}" | head -3 >&2
    failures=$((failures + 1))
  elif grep -qi "invalid profile configuration\|unknown field" <<<"${output}"; then
    echo "FAIL: ${file}:${line} configuration example is rejected by the loader." >&2
    echo "${output}" | head -3 >&2
    failures=$((failures + 1))
  fi
done < <(extract_toml_blocks "${DOCS[@]}")

# ---------------------------------------------------------------------------
# 2. Documented subcommands must exist.
# ---------------------------------------------------------------------------
KNOWN="$(NO_COLOR=1 "${BIN}" --help 2>/dev/null \
  | awk '/^Commands:/{flag=1;next}/^Options:/{flag=0}flag' \
  | awk '{print $1}' | grep -E '^[a-z][a-z-]*$' || true)"

documented_subcommands() {
  python3 - "$@" <<'PY'
import re, sys
for path in sys.argv[1:]:
    try:
        lines = open(path, encoding="utf-8").read().splitlines()
    except OSError:
        continue
    inside = False
    for n, line in enumerate(lines, 1):
        if not inside and re.match(r"^\s*```(bash|sh|shell|console)\s*$", line):
            inside = True
            continue
        if inside and re.match(r"^\s*```\s*$", line):
            inside = False
            continue
        if not inside:
            continue
        stripped = line.strip().lstrip("$ ").strip()
        m = re.match(r"^(?:[A-Z_][A-Z_0-9]*=\S*\s+)*zavora-cli\s+(.*)$", stripped)
        if not m:
            continue
        for token in m.group(1).split():
            # Flags, comments, and shell line-continuations are not subcommands.
            if token.startswith(("-", "#", "\\")):
                break
            if not re.fullmatch(r"[a-z][a-z-]*", token):
                break
            print(f"{path}\t{n}\t{token}")
            break
PY
}

while IFS=$'\t' read -r file line subcommand; do
  [[ -z "${subcommand}" ]] && continue
  if ! grep -qx "${subcommand}" <<<"${KNOWN}"; then
    echo "FAIL: ${file}:${line} documents 'zavora-cli ${subcommand}', which is not a subcommand." >&2
    failures=$((failures + 1))
  fi
done < <(documented_subcommands "${DOCS[@]}")

if [[ "${failures}" -gt 0 ]]; then
  echo "" >&2
  echo "${failures} documentation example(s) do not work as written." >&2
  exit 1
fi

echo "OK: documented configuration and command examples work as written."
