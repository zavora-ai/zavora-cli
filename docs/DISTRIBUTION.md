# Distribution Channels

`zavora-cli` supports three installation channels:

1. Cargo: `cargo install zavora-cli`
2. npm: `npm i -g @zavora-ai/zavora-cli`
3. Homebrew: `brew install zavora-ai/tap/zavora-cli`

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

- `NPM_TOKEN`: stage `@zavora-ai/zavora-cli` for a maintainer to approve
- `HOMEBREW_TAP_TOKEN`: update `Formula/zavora-cli.rb` in `zavora-ai/homebrew-tap`
  (a PAT with `contents:write` on that repository; `GITHUB_TOKEN` cannot write to
  another repo)
- `CARGO_REGISTRY_TOKEN`: publish `zavora-cli` to crates.io from CI

Without these secrets, GitHub release artifacts are still produced — but the jobs
that need them now raise a workflow warning and record a line in the run summary
rather than passing quietly. A release that publishes nothing previously reported
those jobs green, which is how v2.0.0 was tagged with no assets attached and no npm
package published.

The two failure modes are not symmetric. A missing `CARGO_REGISTRY_TOKEN` simply
means crates.io is not updated. A published npm wrapper *without* release assets is
worse than no wrapper at all: the package installs, `postinstall` tries to download
`zavora-cli-<tag>-<platform>.tar.gz` from the release, and every install fails. So
`publish-npm` depends on `publish-release`, which depends on every `build` matrix
entry succeeding.

## npm publishing is staged, not direct

npm no longer accepts an unattended `npm publish` for this package. Following the
2026 supply-chain attacks, long-lived tokens that bypass 2FA were withdrawn, and a
trust relationship can be configured stage-only — in which case `npm publish` from a
workflow is rejected and only `npm stage publish` is accepted.

So the release workflow **stages** the wrapper. `npm stage publish` packs the
package and uploads it in a non-public state, and never prompts for 2FA, which is
what makes it usable from CI. The version is not installable at that point. A
maintainer then approves it, and approval *does* require 2FA — that is the whole
point: every release carries proof a human was present.

    npm stage list @zavora-ai/zavora-cli   # find the stage id
    npm stage view <stage-id>              # inspect the metadata
    npm stage download <stage-id>          # inspect the tarball
    npm stage approve <stage-id>           # publish; prompts for 2FA
    npm stage reject <stage-id>            # discard it instead

Approval cannot be automated, and not only by convention: short-lived tokens issued
through a trust relationship are permitted to run `npm stage publish` and
`npm publish`, but not any other `npm stage` subcommand. So `approve` is necessarily
a human action from a maintainer's own machine or npmjs.com.

Two consequences worth remembering. A green `publish-npm` job means *staged*, not
released — the run summary prints the commands to approve it. And `npm stage`
requires npm 11.19.0 or newer, which is why the job installs the CLI explicitly
rather than trusting whichever npm the runner image ships.

## Maintainer Release Steps

1. Keep versions in sync:
   - `Cargo.toml`: `version = "X.Y.Z"`
   - `npm/zavora-cli/package.json`: `"version": "X.Y.Z"`
2. Run checks: `make dist-check`
3. Nothing to do for Homebrew by hand: the release workflow repoints
   `zavora-ai/homebrew-tap` at the new tag. The formula is pinned by git tag and
   revision, so there is no digest to regenerate.
4. Commit, push, and create tag:
   - `git tag -a vX.Y.Z -m "zavora-cli vX.Y.Z"`
   - `git push origin main --tags`
5. Confirm the release workflow attached artifacts and checksums. The npm wrapper
   downloads these, so they must exist before the staged package is approved —
   approving first would publish a wrapper whose every install fails.
6. Approve the staged npm package: `npm stage approve <stage-id>` (requires 2FA).
7. Verify installs in clean environments:
   - `cargo install zavora-cli`
   - `npm i -g @zavora-ai/zavora-cli`
   - `brew install zavora-ai/tap/zavora-cli`
