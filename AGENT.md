# Agent Guide

## Workspace and toolchain

Work from the repository root. This is a Cargo workspace using Rust edition 2024 and resolver 3.

| Path | Package | Purpose |
| --- | --- | --- |
| `crates/wlan-api` | `wlanlogin-lib` | Async portal protocol library |
| `crates/wlan-app` | `wlanlogin-cli` | CLI, TUI, daemon, and native platform adapters |

The binary is `wlanlogin` (`.exe` on Windows). The release artifact is under `target/release/`. Keep `Cargo.lock` checked in and use `--locked` for validation. Do not update dependencies incidentally during a refactor.

Native identifiers live in `infra/identity.rs`: the application ID is `dev.cxcs.tools.wlanlogin`. Application paths live in `infra/paths.rs` and use the two components `cxcs-tools/wlanlogin` (`ProjectDirs::from_path`, not `ProjectDirs::from`). Cleanup must preserve the shared `cxcs-tools` parent and sibling tools. The rename does not automatically migrate legacy services or credentials; README documents uninstalling with the old binary before installing the new one. The CLI package is `wlanlogin-cli`, its explicit binary target and tracing prefix are `wlanlogin`, and its environment variables use `WLANLOGIN_`.

There is no pinned `rust-toolchain.toml` or declared MSRV. The architecture refactor was validated with Rust/Cargo `1.100.0-nightly`; this is an observed toolchain, not a nightly requirement or a minimum supported version. Check `rustc --version`, `cargo --version`, and installed targets before making toolchain claims. Rust edition support alone does not establish the minimum version of all dependencies.

Required tools:

- Rust, Cargo, rustfmt, Clippy, and Git.
- Linux: a C toolchain, `pkg-config`, D-Bus development libraries, `dbus-daemon`, and standard POSIX test tools. Debian / Ubuntu packages include `build-essential pkg-config libdbus-1-dev dbus-daemon`.
- macOS: Xcode Command Line Tools and the native Rust toolchain.
- Windows: a Rust toolchain and compatible MSVC or GNU build tools.
- Optional compile targets: `x86_64-pc-windows-gnu` and `aarch64-apple-darwin`.

NetworkManager, Secret Service, and native service managers are runtime dependencies, not substitutes for test doubles.

## Commands

macOS bundles are built with `cargo xtask bundle macos`, optionally using `--codesign-identity` or `CODESIGN_IDENTITY` (CLI takes precedence; default is ad-hoc `-`). `xtask/` shares `infra/identity.rs` and renders `packaging/macos/Info.plist`; it builds the explicit `wlanlogin` binary, validates the bundle, and publishes it only after successful signing. Ordinary compilation uses Cargo, not an xtask build subcommand. Bundle generation does not install or launch services.

Run relevant checks after each logical change and the complete set before delivery of an architectural change:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --workspace
git diff --check
```

Check other platforms when native code, shared interfaces, or conditional compilation change:

```sh
cargo check -p wlanlogin-cli --locked --target x86_64-pc-windows-gnu
cargo check -p wlanlogin-cli --locked --target aarch64-apple-darwin
cargo check -p wlanlogin-cli --tests --locked --target x86_64-pc-windows-gnu
cargo check -p wlanlogin-cli --tests --locked --target aarch64-apple-darwin
```

Use `rustup target add` only when a required target is missing. A successful cross-target check establishes compilation only; it does not establish linking, OS compatibility, or runtime behavior. Report unavailable checks explicitly.

For safe CLI smoke checks, use only `--help`, subcommand `--help`, and `--version`. Do not invoke a command without its help flag to discover behavior.

## Source ownership

`main.rs` delegates to `app::run()`. The module roots are `app.rs`, `infra.rs`, and `system.rs`.

- `app/cli.rs`, `app/cli/`, and `app/tui.rs` own arguments, terminal input, interaction, and presentation. `app/context.rs` constructs application dependencies.
- `app/controller/` exposes reusable login, logout, install, uninstall, and daemon operations. `app/controller/daemon/` contains separate run, start, stop, restart, status, and logs entry points. Return structured results or accept explicit output/cancellation interfaces; do not read terminal input here.
- `app/daemon.rs` and `app/daemon/` own the daemon lifetime, scheduling, single authentication checks, retry policy, and bounded background credential reads.
- `infra/` owns typed errors, diagnostics, logging, files, configuration, paths, scope, command execution, shutdown, IPC, and locks. It must not depend on `app` or `system`.
- `system/define/` owns credentials, services, Wi-Fi, and permission contracts. `system/shared/` owns reusable adapters and service transactions. `system/{linux,macos,windows}/` owns native services, credentials, Wi-Fi, and permission declarations.

The dependency direction is `app -> system -> infra`, with direct `app -> infra` access allowed. Never make `system` depend on `app`, including through native service callbacks. Keep Clap and terminal dependencies out of system contracts. Do not change `wlan-api` as a side effect of an application-only refactor.

Permission identifiers are open strings and descriptors have optional descriptions and extensible metadata. Collections may be empty. Linux and Windows currently expose none; macOS declares Location and Keychain. Location uses the signed GUI-session host and requests authorization on demand during interactive foreground operations. Background and non-interactive operations never request permission. Keychain diagnostics are not yet implemented. Never report an unavailable capability as granted, and reject unknown identifiers. Host IPC must validate protocol versions, response IDs, frame sizes and the live peer's code identity; system clients also validate the active console user. Never transmit portal credentials through the Wi-Fi host.

## Errors, logs, and tests

Use `infra::error::{Error, Result, Context, Message}`. Define application error and event templates with `thiserror` in `infra/error/`; keep their text in English. Preserve source errors and portal error classifications. `anyhow` is confined to the error boundary for external errors.

Use `tracing` with structured fields such as `error`, `scope`, and `retry_after_secs`. The shared formatter writes UTC RFC 3339 timestamps, severity, target, and fields. The default filter is `info`; `RUST_LOG` overrides it. CLI help and TUI labels may remain Chinese. Never log secrets, authentication query strings, or raw portal payloads.

All `wlan-app` tests belong at the bottom of the owning source file in `#[cfg(test)] mod tests`, even when long. Do not extract `tests.rs`, separate test directories, or tests via `#[path]`. Shared mocks may live in a relevant module's inline test module, such as `infra::command::tests`. Preserve the existing API protocol fixtures and integration tests.

Use loopback mock portals, temporary directories, fake credentials and service commands, private D-Bus connections, and isolated signal child processes. Never access the real campus portal, change Wi-Fi, read actual saved credentials, or install, stop, or remove real services during development verification. `cargo test` needs local socket and subprocess permissions. If a sandbox rejects these, rerun only the isolated tests in an appropriately permitted environment and distinguish environment failures from product failures.

The ignored `signal_child` helper is launched explicitly by a parent test. Do not run the whole suite with `--include-ignored`.

Preserve these behavioral boundaries:

- An unknown portal state must never trigger automatic login.
- Recheck the Wi-Fi session before submitting credentials after an await.
- Keep shutdown listeners alive and continuously drain NetworkManager D-Bus events.
- Bound native calls and credential reads; keep at most one background credential read in flight.
- Keep user and system scope separate, including credential cleanup and daemon wake/reset.
- Retain operation locks, service transaction rollback, and stop-before-cleanup ordering.

## Delivery

Inspect `git status` and existing changes before editing. Prefer `rg` for source searches. Preserve unrelated work. Commit each completed, validated feature using Conventional Commits when requested; review the staged diff and do not bypass hooks or rewrite existing commits without explicit direction.

Update [README.md](README.md) for user behavior and [CONTRIBUTING.md](CONTRIBUTING.md) for maintenance responsibilities. Report the checks actually run, compile-only platform coverage, commit hashes, and the final worktree state. Do not claim real portal or platform runtime validation from mocks or cross-compilation.
