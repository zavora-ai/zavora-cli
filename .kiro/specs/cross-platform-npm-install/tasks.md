# Implementation Plan: Cross-Platform npm Install

## Overview

Incrementally add Windows support to the npm distribution channel, starting with the install script internals, then the bin wrapper, package metadata, CI build targets, smoke tests, and documentation. Each step builds on the previous and is validated by tests.

## Tasks

- [x] 1. Add platform resolution and zip extraction to install script
  - [x] 1.1 Extend SUPPORTED_ARTIFACTS map and add resolveArchiveInfo function in `npm/zavora-cli/scripts/install.js`
    - Add `win32: { x64: "windows-x64", arm64: "windows-arm64" }` to SUPPORTED_ARTIFACTS
    - Add `resolveArchiveInfo()` that returns `{ suffix, ext, binaryName }` based on platform
    - Update `main()` to use `resolveArchiveInfo()` for archive URL construction and binary name verification
    - _Requirements: 2.1, 2.2, 2.7, 2.8_

  - [x] 1.2 Implement `unpackZip` function and update `unpackArchive` to dispatch by format
    - Implement zip extraction using Node.js `zlib.inflateRawSync` for deflate and raw copy for stored entries
    - Update `unpackArchive` to accept archive extension and call `unpackZip` for `.zip` files
    - Preserve existing `tar` extraction path for `.tar.gz` files
    - _Requirements: 2.3, 2.4_

  - [ ]* 1.3 Write property test: Platform resolution correctness
    - Install `fast-check` as a dev dependency
    - **Property 1: Platform resolution correctness**
    - **Validates: Requirements 2.2, 2.7, 2.8, 3.1, 3.2**

  - [ ]* 1.4 Write property test: Zip extraction round-trip
    - **Property 2: Zip extraction round-trip**
    - **Validates: Requirements 2.3**

  - [ ]* 1.5 Write property test: Checksum verification correctness
    - **Property 3: Checksum verification correctness**
    - **Validates: Requirements 2.5, 2.6**

  - [ ]* 1.6 Write unit tests for install script
    - Test each supported platform/arch combo returns correct resolution
    - Test unsupported platform returns null
    - Test `parseExpectedChecksum` with various manifest formats
    - Test checksum mismatch triggers failure
    - _Requirements: 2.1, 2.5, 2.6_

- [x] 2. Checkpoint - Verify install script changes
  - Ensure all tests pass, ask the user if questions arise.

- [x] 3. Update bin wrapper for Windows binary resolution
  - [x] 3.1 Update `npm/zavora-cli/bin/zavora-cli.js` to resolve platform-specific binary name
    - Use `process.platform === "win32"` to choose between `zavora-cli.exe` and `zavora-cli`
    - Preserve existing spawn logic and error handling
    - _Requirements: 3.1, 3.2, 3.3, 3.4_

  - [ ]* 3.2 Write unit tests for bin wrapper binary resolution
    - Test binary path resolves to `.exe` on win32
    - Test binary path resolves without extension on unix
    - Test missing binary produces error exit
    - _Requirements: 3.1, 3.2, 3.4_

- [x] 4. Update package.json metadata
  - [x] 4.1 Add `win32` to the `os` field in `npm/zavora-cli/package.json`
    - Change `"os": ["darwin", "linux"]` to `"os": ["darwin", "linux", "win32"]`
    - _Requirements: 4.1, 4.2_

- [x] 5. Add Windows build targets to release workflow
  - [x] 5.1 Add Windows matrix entries to `.github/workflows/release.yml`
    - Add `windows-latest` / `x86_64-pc-windows-msvc` / `windows-x64` entry
    - Add `windows-latest` / `aarch64-pc-windows-msvc` / `windows-arm64` entry (cross-compile with `rustup target add`)
    - _Requirements: 1.1_

  - [x] 5.2 Update packaging step to produce .zip on Windows and .tar.gz on Unix
    - Conditionally use `Compress-Archive` (PowerShell) on Windows runners for zip creation
    - Preserve existing `tar -czf` for Unix runners
    - Ensure binary is named `zavora-cli.exe` in Windows archives
    - _Requirements: 1.2, 1.3_

  - [x] 5.3 Update checksums manifest generation to include .zip files
    - Ensure `checksums.txt` covers both `.tar.gz` and `.zip` artifacts
    - Use `sha256sum` on Linux runner (where manifest is generated)
    - _Requirements: 1.4_

- [x] 6. Add smoke test job to release workflow
  - [x] 6.1 Add `smoke-test` job to `.github/workflows/release.yml`
    - Run on `ubuntu-latest`, `macos-14`, and `windows-latest`
    - Install package globally from local source (`npm install -g ./npm/zavora-cli`)
    - Execute `zavora-cli --version` and verify output contains expected version
    - _Requirements: 5.1, 5.2, 5.3_

  - [x] 6.2 Gate npm publish on smoke test success
    - Add `smoke-test` to `publish-npm` job's `needs` array
    - _Requirements: 5.4_

- [x] 7. Update distribution documentation
  - [x] 7.1 Update `docs/DISTRIBUTION.md` with Windows artifacts and verification steps
    - Add `windows-x64.zip` and `windows-arm64.zip` to artifact list
    - Document archive format difference (tar.gz vs zip)
    - Add Windows to post-release verification steps
    - _Requirements: 6.1, 6.2, 6.3_

- [x] 8. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Property tests use `fast-check` and require minimum 100 iterations each
- Smoke tests run on real CI runners and validate the full install-to-execute flow
- The zip extractor is dependency-free, using only Node.js built-in `zlib`
