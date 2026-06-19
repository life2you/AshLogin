# AshLogin

`AshLogin` is a small Rust CLI for one job: choose a server from a local config file and hand off to the system `ssh` client.

It is intentionally narrow:

- no file upload or download
- no built-in SSH implementation
- no TUI config editor

`AshLogin` keeps host metadata in `TOML`, supports direct login by server name, and falls back to an interactive picker when you run it without arguments.

## Why this shape

This project is the open-source version of a private server login helper. The public version keeps only the safe, reusable part: local host selection plus a normal `ssh` handoff.

## Install

Build from source:

```bash
cargo build --release
./target/release/ashlogin --help
```

Install with Cargo:

```bash
cargo install --path .
```

Install with Homebrew:

```bash
brew tap life2you/tap
brew install ashlogin
```

## Requirements

- Rust toolchain for building from source
- `ssh` available in `PATH`
- `sshpass` available in `PATH` if you use password-based servers

By default, `AshLogin` works well with standard SSH keys and `ssh-agent`. If a server entry includes a `password`, `AshLogin` uses `sshpass -e ssh`.

## Config

AshLogin resolves config in this order:

1. `--config /path/to/config.toml`
2. `ASHLOGIN_CONFIG`
3. `~/.config/ashlogin/config.toml`

If the default config file does not exist, `ashlogin` creates `~/.config/ashlogin/config.toml` automatically on first launch, prints the path, and exits so you can edit it safely.

You can also copy [`config.toml.example`](config.toml.example) manually if you prefer.

Example:

```toml
[[servers]]
name = "prod"
aliases = ["p"]
host = "203.0.113.10"
user = "deploy"
port = 22
description = "Main production host"
password = "replace-me"
identity_file = "~/.ssh/id_ed25519"
ssh_options = ["IdentitiesOnly=yes"]
```

Supported fields per server:

- `name`: required unique display name
- `aliases`: optional alternate names
- `host`: required hostname or IP
- `user`: required SSH username
- `port`: optional, defaults to `22`
- `description`: optional text shown in the list
- `password`: optional plain-text password; when present, AshLogin launches `sshpass -e ssh`
- `identity_file`: optional path passed to `ssh -i`
- `ssh_options`: optional list of values passed as repeated `ssh -o ...`

If `password` is present, `sshpass` must be installed. `--dry-run` hides the password and prints `SSHPASS=*** sshpass -e ssh ...`.

This field is convenient but not ideal for shared configs. Do not commit real passwords into a public repository.

If a password-based server is not present in `~/.ssh/known_hosts`, AshLogin will first ask whether it should fetch and save that host key before logging in.

## Usage

Interactive picker:

```bash
ashlogin
```

On first run, if the default config file is missing, AshLogin creates it and exits:

```bash
ashlogin
Created a default config at /Users/you/.config/ashlogin/config.toml.
Edit that file with your servers, then run ashlogin again.
```

Direct login:

```bash
ashlogin prod
ashlogin p
```

List configured servers:

```bash
ashlogin --list
```

Preview the final SSH command:

```bash
ashlogin --dry-run prod
```

Use a specific config file:

```bash
ashlogin --config ~/.config/ashlogin/config.toml prod
```

## Example SSH handoff

Given this config:

```toml
[[servers]]
name = "staging"
host = "203.0.113.20"
user = "developer"
port = 2222
ssh_options = ["ServerAliveInterval=30"]
```

AshLogin will execute the equivalent of:

```bash
ssh -p 2222 -o ServerAliveInterval=30 developer@203.0.113.20
```

With password auth:

```toml
[[servers]]
name = "legacy"
host = "203.0.113.30"
user = "root"
password = "replace-me"
```

AshLogin will execute the equivalent of:

```bash
SSHPASS=*** sshpass -e ssh root@203.0.113.30
```

## Development

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```
