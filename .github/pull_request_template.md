## What changed and why

<!-- A few sentences. Link the issue this closes, if any: "Closes #123". -->

## How it was checked

<!-- What you ran or tried. For interface changes, attach a screenshot. -->

## Checklist

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] `CHANGELOG.md` has a line under Unreleased, if a user would notice the change
- [ ] Interface changes follow `docs/DESIGN.md` (tokens only, line icons, no emoji)
- [ ] `THIRD-PARTY-NOTICES` is regenerated, if dependencies changed
- [ ] `PRIVACY.md` is updated, if the app sends anything new over the network
