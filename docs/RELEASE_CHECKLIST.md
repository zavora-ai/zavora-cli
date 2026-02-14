# Release Checklist

Run this checklist before creating a `vX.Y.Z` tag.

## Code And Quality

- [ ] Scope matches planned release objective
- [ ] `cargo fmt --all --check`
- [ ] `cargo check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`

## Release Notes

- [ ] `CHANGELOG.md` updated
- [ ] Known risks and mitigations documented
- [ ] Rollback plan documented

## Tag And Publish

- [ ] Tag created: `vX.Y.Z`
- [ ] Tag pushed
- [ ] GitHub release artifacts generated
