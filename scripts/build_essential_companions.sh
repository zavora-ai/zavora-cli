#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <rust-target> <staging-directory>" >&2
  exit 2
fi

rust_target="$1"
staging_directory="$2"
libexec_directory="${staging_directory}/libexec/zavora-cli"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "${temporary_directory}"' EXIT

mkdir -p "${libexec_directory}"

# Pin the exact audited v1.7.0 commit. Building the local-system backend into
# the release payload makes device inspection available without requiring an
# end user to have Cargo or to compile third-party code during first use.
git clone --quiet https://github.com/zavora-ai/mcp-device-management.git \
  "${temporary_directory}/mcp-device-management"
git -C "${temporary_directory}/mcp-device-management" checkout --quiet \
  2efb99b15723b81dd51b0912db27abd84edf3fb7
cargo build \
  --manifest-path "${temporary_directory}/mcp-device-management/Cargo.toml" \
  --release \
  --target "${rust_target}" \
  --all-features

device_binary="${temporary_directory}/mcp-device-management/target/${rust_target}/release/mcp-device-management"
if [[ "${rust_target}" == *windows* ]]; then
  device_binary="${device_binary}.exe"
fi
cp "${device_binary}" "${libexec_directory}/"

# computer-use-mcp ships signed native modules for each supported platform in
# its npm artifact. It remains a separate governed runtime because desktop
# permission, lease, approval, and physical-user interruption are authoritative
# there rather than in the model process.
npm install \
  --ignore-scripts \
  --omit=dev \
  --prefix "${libexec_directory}" \
  @zavora-ai/computer-use-mcp@7.1.0
