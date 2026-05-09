# AshLogin

`AshLogin` is a terminal-first SSH account manager and login launcher for macOS.

## Current shape

- `ashlogin`: list configured accounts in the terminal, choose one, and log in directly
- `ashlogin <name>`: log in directly by account name
- `ashlogin --conf`: open the TUI configuration manager
- `password` auth reads passwords from macOS Keychain
- `ssh_key` auth is supported through `ssh2`

## Install

Build from source:

```bash
cargo build --release
./target/release/ashlogin --help
```

Homebrew tap install:

```bash
brew tap life2you/tap
brew install ashlogin
```

## Release

Maintainer release steps live in [RELEASING.md](/Users/life2you/vibeCodes/github/AshLogin/RELEASING.md).

## Config

AshLogin looks for config in this order:

1. `ASHLOGIN_CONFIG`
2. `./config.toml`
3. `~/.config/ashlogin/config.toml`

If no file exists, AshLogin auto-creates `~/.config/ashlogin/config.toml`.

Example:

```toml
[[servers]]
name = "prod"
host = "192.168.1.10"
port = 22
username = "deploy"
auth_type = "password"
keychain_service = "AshLogin"
keychain_account = "deploy@prod"
```

When `auth_type = "password"`, AshLogin reads the password from macOS Keychain using:

- service: `keychain_service` or `AshLogin`
- account: `keychain_account` or `{username}@{name}`

Example Keychain write:

```bash
security add-generic-password -U -a deploy@prod -s AshLogin -w
```

## Usage

```bash
ashlogin
ashlogin prod
ashlogin --conf
```

With `cargo run` in development:

```bash
cargo run
cargo run -- prod
cargo run -- --conf
cargo run -- --help
```

## Config TUI shortcuts

- `a`: add account
- `d`: delete selected account
- `Tab`: move between form fields while adding
- `Enter`: next field or submit the add form
- `Esc`: cancel the add form
- `q`: quit the config TUI
