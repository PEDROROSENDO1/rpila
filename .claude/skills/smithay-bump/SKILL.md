---
name: smithay-bump
description: How to move the smithay dependency's rev forward in Cargo.toml, including Pila's fork (igalmarino/smithay, branch pila) and its rev-<sha> tags. Use when bumping Smithay, rebasing the fork or dropping a fork fix that landed upstream.
---

# Bumping Smithay

A bump is a change of its own: read Smithay's `CHANGELOG.md` and log since the old one, run the whole suite, `scripts/check --nested` and
`scripts/apps`, and have the user check bare metal (`udev/`). While the dependency is Pila's
fork (`Cargo.toml` says which rev of Smithay's it is and what it adds), a bump tags the rev
`main` pinned (`rev-<sha>`, so the releases that pin it keep building) and rebases its
`pila` branch on the new rev, or goes back to Smithay's own once it has the fixes: each has a
test that fails without it. The branch is moved through the GitHub API (no git network here),
at the user's word.
