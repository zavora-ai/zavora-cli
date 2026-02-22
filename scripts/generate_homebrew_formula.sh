#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FORMULA_PATH="${ROOT_DIR}/Formula/zavora-cli.rb"

VERSION_INPUT="${1:-}"
if [[ -z "${VERSION_INPUT}" ]]; then
  VERSION_INPUT="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT_DIR}/Cargo.toml" | head -n 1)"
fi

if [[ -z "${VERSION_INPUT}" ]]; then
  echo "Could not determine version from Cargo.toml" >&2
  exit 1
fi

if [[ "${VERSION_INPUT}" == v* ]]; then
  TAG="${VERSION_INPUT}"
else
  TAG="v${VERSION_INPUT}"
fi

SRC_URL="https://github.com/zavora-ai/zavora-cli/archive/refs/tags/${TAG}.tar.gz"
TMP_ARCHIVE="$(mktemp)"
trap 'rm -f "${TMP_ARCHIVE}"' EXIT

curl -fsSL "${SRC_URL}" -o "${TMP_ARCHIVE}"
SHA256="$(shasum -a 256 "${TMP_ARCHIVE}" | awk '{print $1}')"

cat > "${FORMULA_PATH}" <<FORMULA
class ZavoraCli < Formula
  desc "Rust CLI agent shell built on ADK-Rust"
  homepage "https://github.com/zavora-ai/zavora-cli"
  url "${SRC_URL}"
  sha256 "${SHA256}"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", *std_cargo_args(path: ".")
  end

  test do
    assert_match "Usage:", shell_output("#{bin}/zavora-cli --help")
  end
end
FORMULA

echo "Updated ${FORMULA_PATH} for ${TAG}" >&2
