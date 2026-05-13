# AGENTS.md

## Scope

This repository ships a Homebrew-installable CLI tool.

## AI Collaboration Rules

- Prefer `gh` for GitHub operations. Before using raw `git`, `curl`, or manual browser steps for releases, workflow checks, PRs, issues, or tags, first check whether `gh` is installed and authenticated with `command -v gh` and `gh auth status`.
- Keep `Cargo.toml`, `Cargo.lock`, release tags, packaged Homebrew formula content, and tap formula versions aligned.
- Do not hand-edit Homebrew SHA256 values. Use `scripts/update-homebrew-formula.sh`.
- For local release publishing, prefer `scripts/release-and-upgrade-local.sh`. It pushes the current branch and tag, waits for the GitHub release and `homebrew-tap` update, then runs local `brew update` and `brew upgrade/install`.
- GitHub release automation lives in `.github/workflows/release.yml` and expects the `HOMEBREW_TAP_PUSH_TOKEN` repository secret to update `life2you/homebrew-tap` automatically after a tag release.
- Run the relevant verification steps, at minimum `cargo test`, before shipping a release.
