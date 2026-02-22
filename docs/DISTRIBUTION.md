# Distribution Channels

`zavora-cli` supports three installation channels:

1. Cargo: `cargo install zavora-cli`
2. npm: `npm i -g @zavora-ai/zavora-cli`
3. Homebrew: `brew install --formula https://raw.githubusercontent.com/zavora-ai/zavora-cli/main/Formula/zavora-cli.rb`

## Release Artifacts

Tag pushes (`vX.Y.Z`) trigger `.github/workflows/release.yml` to build and upload:

- `zavora-cli-vX.Y.Z-linux-x64.tar.gz`
- `zavora-cli-vX.Y.Z-linux-arm64.tar.gz`
- `zavora-cli-vX.Y.Z-darwin-x64.tar.gz`
- `zavora-cli-vX.Y.Z-darwin-arm64.tar.gz`
- `checksums.txt`

The npm package downloads these artifacts during `postinstall`.

## Required Secrets

Optional publish steps run only when secrets are configured:

- `NPM_TOKEN`: publish `@zavora-ai/zavora-cli`
- `CARGO_REGISTRY_TOKEN`: publish `zavora-cli` to crates.io from CI

Without these secrets, GitHub release artifacts are still produced.

## Maintainer Release Steps

1. Keep versions in sync:
   - `Cargo.toml`: `version = "X.Y.Z"`
   - `npm/zavora-cli/package.json`: `"version": "X.Y.Z"`
2. Run checks: `make dist-check`
3. Refresh Homebrew formula for the target tag:
   - `./scripts/generate_homebrew_formula.sh vX.Y.Z`
4. Commit, push, and create tag:
   - `git tag -a vX.Y.Z -m "zavora-cli vX.Y.Z"`
   - `git push origin main --tags`
5. Confirm release workflow published artifacts and checksums.
6. Verify installs in clean environments:
   - `cargo install zavora-cli`
   - `npm i -g @zavora-ai/zavora-cli`
   - `brew install --formula https://raw.githubusercontent.com/zavora-ai/zavora-cli/main/Formula/zavora-cli.rb`
