# Requirements Document

## Introduction

The `@zavora-ai/zavora-cli` npm package currently supports macOS and Linux only. This feature extends the npm distribution channel to support Windows, making npm the universal cross-platform installation method for zavora-cli. The work spans CI build targets, the postinstall script, the binary wrapper, and production smoke testing across all supported platforms.

## Glossary

- **Install_Script**: The Node.js postinstall script (`npm/zavora-cli/scripts/install.js`) that downloads and extracts the platform-specific prebuilt Rust binary from GitHub Releases during `npm install`.
- **Bin_Wrapper**: The Node.js entry point (`npm/zavora-cli/bin/zavora-cli.js`) that spawns the downloaded native binary, forwarding arguments and exit codes.
- **Release_Workflow**: The GitHub Actions CI pipeline (`.github/workflows/release.yml`) that builds platform-specific binaries, packages them as archives, and publishes them to GitHub Releases.
- **Artifact**: A platform-specific compressed archive containing the prebuilt zavora-cli binary, published to GitHub Releases (tar.gz for Unix, zip for Windows).
- **Checksums_Manifest**: A `checksums.txt` file published alongside release artifacts containing SHA-256 hashes for integrity verification.
- **Smoke_Test**: An automated test that installs the npm package on a target platform and verifies the binary executes successfully.
- **SUPPORTED_ARTIFACTS**: The mapping in the Install_Script that resolves `process.platform` and `process.arch` to the correct artifact suffix for download.

## Requirements

### Requirement 1: Windows Build Targets in CI

**User Story:** As a maintainer, I want the Release_Workflow to produce Windows binaries, so that Windows users can install zavora-cli via npm.

#### Acceptance Criteria

1. WHEN a release tag is pushed, THE Release_Workflow SHALL build binaries for `x86_64-pc-windows-msvc` (windows-x64) and `aarch64-pc-windows-msvc` (windows-arm64) targets in addition to existing Linux and macOS targets.
2. WHEN packaging a Windows artifact, THE Release_Workflow SHALL produce a `.zip` archive containing `zavora-cli.exe`.
3. WHEN packaging a Unix artifact, THE Release_Workflow SHALL continue to produce a `.tar.gz` archive containing `zavora-cli`.
4. WHEN all platform artifacts are built, THE Release_Workflow SHALL generate a single Checksums_Manifest covering all artifacts including Windows `.zip` files.

### Requirement 2: Windows Support in Install Script

**User Story:** As a Windows user, I want `npm i -g @zavora-ai/zavora-cli` to download and install the correct binary, so that I can use zavora-cli without needing Rust or Cargo.

#### Acceptance Criteria

1. THE SUPPORTED_ARTIFACTS map in the Install_Script SHALL include entries for `win32/x64` and `win32/arm64` platform-architecture combinations.
2. WHEN running on Windows, THE Install_Script SHALL download a `.zip` artifact instead of a `.tar.gz` artifact.
3. WHEN extracting a `.zip` artifact on Windows, THE Install_Script SHALL use Node.js built-in APIs or bundled logic instead of relying on the system `tar` command.
4. WHEN extracting a `.tar.gz` artifact on Unix, THE Install_Script SHALL continue to use the system `tar` command.
5. WHEN running on Windows, THE Install_Script SHALL verify the downloaded artifact against the Checksums_Manifest using SHA-256.
6. IF the downloaded artifact fails checksum verification, THEN THE Install_Script SHALL abort installation and display an error message with a fallback suggestion.
7. WHEN extraction completes on Windows, THE Install_Script SHALL verify that `zavora-cli.exe` exists in the vendor directory.
8. WHEN extraction completes on Unix, THE Install_Script SHALL continue to verify that `zavora-cli` (without extension) exists in the vendor directory.

### Requirement 3: Windows Support in Binary Wrapper

**User Story:** As a Windows user, I want the bin wrapper to correctly locate and spawn the Windows executable, so that `zavora-cli` commands work after npm install.

#### Acceptance Criteria

1. WHEN running on Windows, THE Bin_Wrapper SHALL resolve the binary path to `vendor/zavora-cli.exe`.
2. WHEN running on Unix, THE Bin_Wrapper SHALL continue to resolve the binary path to `vendor/zavora-cli`.
3. THE Bin_Wrapper SHALL forward all command-line arguments and inherit stdio streams on all platforms.
4. WHEN the binary is not found, THE Bin_Wrapper SHALL display an error message and exit with a non-zero code on all platforms.

### Requirement 4: Package Metadata Update

**User Story:** As a maintainer, I want the npm package metadata to declare Windows support, so that npm allows installation on Windows systems.

#### Acceptance Criteria

1. THE package.json `os` field SHALL include `win32` in addition to `darwin` and `linux`.
2. THE package.json `cpu` field SHALL continue to include `x64` and `arm64`.

### Requirement 5: Cross-Platform Smoke Tests in CI

**User Story:** As a maintainer, I want automated smoke tests that validate npm installation on each supported platform, so that I can catch platform-specific regressions before publishing.

#### Acceptance Criteria

1. WHEN release artifacts are published, THE Release_Workflow SHALL run Smoke_Tests on Linux x64, macOS arm64, and Windows x64 runners.
2. WHEN a Smoke_Test runs, THE Smoke_Test SHALL perform a global npm install of the package from the built artifacts.
3. WHEN a Smoke_Test runs, THE Smoke_Test SHALL execute `zavora-cli --version` and verify the output contains the expected version string.
4. IF a Smoke_Test fails on any platform, THEN THE Release_Workflow SHALL report the failure and prevent the npm publish step from proceeding.

### Requirement 6: Distribution Documentation Update

**User Story:** As a maintainer, I want the distribution documentation to reflect Windows support, so that contributors and users understand the full set of supported platforms.

#### Acceptance Criteria

1. WHEN Windows support is added, THE DISTRIBUTION.md SHALL list Windows x64 and Windows arm64 artifacts alongside existing Unix artifacts.
2. THE DISTRIBUTION.md SHALL document the archive format difference between Unix (tar.gz) and Windows (zip).
3. THE DISTRIBUTION.md SHALL include Windows in the post-release verification steps.
