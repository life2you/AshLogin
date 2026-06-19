# AGENTS.md

## Scope

This repository ships a small Rust CLI that reads a local config file, lets the user pick a host, and hands off to the system `ssh` client.

## AI Collaboration Rules

- Keep the product focused on SSH login only. Do not add file upload, file download, password storage, or a custom SSH transport unless the user explicitly asks for it.
- Prefer calling the system `ssh` binary over embedding SSH protocol logic in Rust.
- Keep the config format human-editable and document every user-facing field in both `README.md` and `config.toml.example`.
- Preserve support for direct login by name and interactive selection when no server name is passed.
- Before finishing a change, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
