#!/usr/bin/env bash
# Property 1: a clean checkout resolves its dependencies with no sibling
# repositories present. Guards against reintroducing `path = "../adk-rust/..."`
# into Cargo.toml, which silently breaks CI and every external contributor.
#
# The check materializes the set of files a commit would carry — tracked files
# plus untracked, non-ignored files — into a scratch directory that has no
# siblings, so it is useful as a pre-commit gate and not only after pushing.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "${SCRATCH}"' EXIT

TARGET="${SCRATCH}/zavora-cli"
mkdir -p "${TARGET}"

echo "==> materializing committable tree into ${TARGET} (no siblings)"
# --exclude-standard honours .gitignore, so the ignored .cargo/config.toml
# override cannot leak in and mask a path dependency.
#
# Deleted-but-still-indexed files are filtered out: `ls-files --cached` lists
# them until the deletion is staged, and tar would abort on the first missing
# path.
git -C "${REPO_ROOT}" ls-files -z --cached --others --exclude-standard \
  | (cd "${REPO_ROOT}" && while IFS= read -r -d '' path; do
      [[ -e "${path}" ]] && printf '%s\0' "${path}"
    done) \
  | tar -C "${REPO_ROOT}" --null -T - -cf - \
  | tar -C "${TARGET}" -xf -

if [[ -f "${TARGET}/.cargo/config.toml" ]]; then
  echo "FAIL: .cargo/config.toml is committable; it must stay gitignored." >&2
  exit 1
fi

if grep -qE '^\s*adk-[a-z-]+\s*=.*\bpath\s*=' "${TARGET}/Cargo.toml"; then
  echo "FAIL: Cargo.toml declares a path dependency on a sibling ADK-Rust crate." >&2
  grep -nE '^\s*adk-[a-z-]+\s*=.*\bpath\s*=' "${TARGET}/Cargo.toml" >&2
  echo "      Use registry versions and 'make local-adk' for local development." >&2
  exit 1
fi

echo "==> resolving dependency graph"
if ! (cd "${TARGET}" && cargo metadata --format-version 1 >/dev/null); then
  echo "FAIL: a clean checkout cannot resolve its dependencies." >&2
  exit 1
fi

echo "OK: clean checkout resolves without sibling repositories."
