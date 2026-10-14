# Agent Guide

## Workspace and toolchain

Work from the repository root. This is a Cargo workspace using Rust edition 2024 and resolver 3.

| Path | Package | Purpose |
| --- | --- | --- |
| `crates/wlan-api` | `wlanlogin-lib` | Async portal protocol library |
| `crates/wlan-app` | `wlanlogin-cli` | CLI, TUI, daemon, and native platform adapters |

The binary is `wlanlogin` (`.exe` on Windows). The release artifact is under `target/release/`. Keep `Cargo.lock` checked in and use `--locked` for validation. Do not update dependencies incidentally during a refactor.

Native identifiers live in `infra/identity.rs`: the application ID is `dev.cxcs.tools.wlanlogin`. Application paths live in `infra/paths.rs` and use the two components `SEUCXCS/wlanlogin` (`ProjectDirs::from_path`, not `ProjectDirs::from`). Cleanup must preserve the shared `SEUCXCS` parent and sibling tools. The rename does not automatically migrate legacy services or credentials; README documents uninstalling with the old binary before installing the new one. The CLI package is `wlanlogin-cli`, its explicit binary target and tracing prefix are `wlanlogin`, and its environment variables use `WLANLOGIN_`.

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

Keep `CFBundleShortVersionString` tied to the Cargo release version. Reserve an independent integer for `CFBundleVersion` before signing through `xtask/src/build_number.rs`. The ignored `.bundle-build/` directory holds the counter and a stable lock file; atomic reservations are shared across profiles, targets, and output paths within one working copy and survive `cargo clean`. Never reuse a reserved number after a packaging failure or silently reset an invalid counter. New working copies start at 1. Verify persistence, concurrent allocation, and the actual signed bundle metadata when changing this behavior.

`APP_NAME` is the command, executable and directory name `wlanlogin`; `DISPLAY_NAME` is `CXCS WLAN Login` and may contain spaces. Keep these roles separate. The icon source is `assets/icon/icon.svg`; the pre-generated `packaging/macos/icon/icon.icns` is copied to `Contents/Resources/icon.icns` before signing. Keep packaged resources in the shared bundle member inventory so installation, rollback and cleanup accept them.

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

- `app::run()` parses arguments and calls the selected platform's `boot()`. `app/bootstrap.rs` owns the shared Tokio runtime lifecycle and the shared asynchronous `entry()` for scope, logging, dependency assembly, and command dispatch; it handles cross-platform permission commands before portal dependency assembly. `app/bootstrap/{linux,macos,windows}.rs` own platform startup orchestration. The Windows bootstrap supplies the application callback to SCM; the macOS bootstrap selects the AppKit main-thread loop.
- `app/cli.rs`, `app/cli/`, and `app/tui.rs` own arguments, terminal input, interaction, and presentation. `app/context.rs` constructs application dependencies.
- `app/controller/` exposes reusable login, logout, daemon, permission, and service operations. `app/controller/daemon.rs` runs the current-process loop; `app/controller/service/` owns install, uninstall, start, stop, restart, status, and logs entry points. Return structured results or accept explicit output/cancellation interfaces; do not read terminal input here.
- `app/daemon.rs` and `app/daemon/` own the daemon lifetime, scheduling, single authentication checks, retry policy, and bounded background credential reads.
- `infra/` owns typed errors, diagnostics, logging, files, configuration, paths, scope, command execution, shutdown, IPC, and locks. It must not depend on `app` or `system`.
- `system/define/` owns credentials, services, Wi-Fi, and permission contracts. `system/shared/` owns reusable adapters and service transactions. `system/{linux,macos,windows}/` owns native services, credentials, Wi-Fi, and permission declarations.

The dependency direction is `app -> system -> infra`, with direct `app -> infra` access allowed. Never make `system` depend on `app`, including through native service callbacks. Keep Clap and terminal dependencies out of system contracts. Do not change `wlan-api` as a side effect of an application-only refactor.

Public service management uses `service install/uninstall/start/stop/restart/status/logs`. Native engines launch the hidden `service run`; Windows system scope enters SCM, user tasks run directly, and macOS user services retain the AppKit loop. macOS bootstrap handles hidden `service native status/register/unregister` and `daemon --internal-wifi-host` before portal dependency assembly. The host mode has a source comment explaining its purpose and skips automatic login; hidden `--system-helper` requires host mode and selects a desktop-user host for system clients. Probe the selected bundle with current help before launching. Reject old startup flags, old host/service/check command protocols and old control frames; never downgrade or retry another protocol. Unsupported macOS signed or external definitions must fail before mutation and direct users to uninstall with their old binary. Keep rollback between supported current-protocol installations.

Permission identifiers are open strings and descriptors have optional descriptions and extensible metadata. Collections may be empty. Linux and Windows currently expose none; macOS declares Location and Keychain. Location uses the signed GUI-session host and requests authorization on demand during interactive foreground operations. Background and non-interactive operations never request permission. `permission list/check/get` is exposed on every platform; the runtime provider supplies capabilities, with empty list/check reports on Linux and Windows and explicit IDs validated normally. Check never requests authorization; get requires interactive stdin and stderr and reports the actual result. The versioned `PermissionReport` includes scope, permission IDs, statuses, and errors. Installed checks validate every protocol field. Keychain diagnostics inspect only the selected scope’s own item without UI and distinguish missing, needs-interaction, denied and unavailable. Serialize native Keychain operations around the process-wide UI switch; failed reads must retain credentials. Never report an unavailable capability as granted, and reject unknown identifiers. Host IPC must validate protocol versions, response IDs, frame sizes and the live peer's code identity; system clients also validate the active console user. Never transmit portal credentials through the Wi-Fi host.

All native platform directories use `flow`, `services`, `credentials`, `wifi`, `permission`, `runtime`. Operation entrypoints live in `system/<platform>/flow/<operation>.rs` and domain flow directories. Re-export shared defaults when no native difference exists; never stub a real operation with success. Explain semantic steps with comments and separate steps by blank lines. `define` holds contracts/data; `shared` holds implementations. Shared uninstall ordering uses native hooks and both installers use shared rollback stop barriers. Windows SCM stays in bootstrap before Tokio; macOS GUI loop implementation lives in runtime/gui.

Service controllers call `system::flow` for every operation. Shared service modules must not re-export native operation entrypoints; native flow composes common orchestration and backend hooks. Log controllers do not select or execute native journal readers. Installation reports and artifact data belong in define; inspection, manager/IPC/version queries and complete log execution belong in system workflows.

Background startup must not pre-read credentials. Read only after target Wi-Fi, pause, policy and confirmed portal Offline guards. Linux user reads first require a live graphical session and synchronized actual activation environment, including when a provider already runs. Then use the official dbus-secret-service crate, DH encryption, zero prompt timeout and standard org.freedesktop.secrets; allow normal D-Bus activation at that point. No vendoring, dependency patches, owner pinning or desktop/provider-specific selection. Attach a result permit carrying scope, config root and an independent persisted credential generation; Linux user reads also carry logind session ID, TimestampMonotonic and local session generation. Revalidate after the native read, when consuming any completed task (including one retained after timeout), and after the final Wi-Fi await before invoking login. Check local session generation before and after awaited live queries, and discard stale credentials. Saving/deleting must advance the credential generation atomically before native mutation under the existing operation lock; failure to persist invalidation prevents mutation, and failed mutation never restores old permits. Timeout, logout or invalidation must not create a second native read while the old task remains in flight. macOS/Windows retain native session behavior and apply only common storage freshness; system scope stays independent of the GUI. Keep default.target and system storage independent of GUI readiness. Session hints are ephemeral/live-validated; continuously drain provider/lock signals with timer fallback.

Waiting and failed reads both keep the daemon alive and preserve credentials. Typed errors retain source chains but expose only safe diagnostic kinds, stages, numeric native codes and recognized bus codes. Timeouts, format/version/encoding/incomplete data, duplicates, decryption/storage failures and task failures must not be mislabeled as permission waiting. Unknown failures remain unknown. Log diagnostic changes and recovery without entering portal backoff or clearing actual credential rejection. Status IPC is version 3 only; it must not activate providers, serialize credentials or expose raw native messages. Private-bus tests cover actual systemd/logind readiness and identity queries with mocks, never the real buses. Secret Service tests use isolated child environments; provider_child/read_child are ignored helpers invoked by their parent only, never run the entire suite with --include-ignored.

## Errors, logs, and tests

Use `infra::error::{Error, Result, Context, Message}`. Define application error and event templates with `thiserror` in `infra/error/`; keep their text in English. Preserve source errors and portal error classifications. `anyhow` is confined to the error boundary for external errors.

Use `tracing` with structured fields such as `error`, `scope`, and `retry_after_secs`. The shared formatter writes UTC RFC 3339 timestamps, severity, target, and fields. The default filter is `info`; `RUST_LOG` overrides it. CLI help and TUI labels may remain Chinese. Never log secrets, authentication query strings, or raw portal payloads.

Daemon checks return outcomes to the runtime, which logs the first result, changes and recovery after failures at `info`, with `outcome` and `next_check_secs`. Log when a retry starts without repeating unchanged waiting states on every poll. On macOS, SSID and BSSID can both be redacted by Location access controls; use interface mode to detect a connected interface with an unreadable name and return an error so retries remain visible and credentials stay protected.

All `wlan-app` tests belong at the bottom of the owning source file in `#[cfg(test)] mod tests`, even when long. Do not extract `tests.rs`, separate test directories, or tests via `#[path]`. Shared mocks may live in a relevant module's inline test module, such as `infra::command::tests`. Preserve the existing API protocol fixtures and integration tests.

Use loopback mock portals, temporary directories, fake credentials and service commands, private D-Bus connections, and isolated signal child processes. Never access the real campus portal, change Wi-Fi, read actual saved credentials, or install, stop, or remove real services during development verification. `cargo test` needs local socket and subprocess permissions. If a sandbox rejects these, rerun only the isolated tests in an appropriately permitted environment and distinguish environment failures from product failures.

The ignored `signal_child` helper is launched explicitly by a parent test. Do not run the whole suite with `--include-ignored`.

Preserve these behavioral boundaries:

- An unknown portal state must never trigger automatic login.
- Recheck the Wi-Fi session before submitting credentials after an await.
- Keep shutdown listeners alive and continuously drain NetworkManager D-Bus events.
- Bound native calls and credential reads; keep at most one background credential read in flight.
- Keep user and system scope separate, including credential cleanup and daemon wake/reset.
- Retain operation locks, service transaction rollback, and stop-before-cleanup ordering. macOS installs whole signed bundles, validates installed credential access with `permission check macos.keychain --json` in the installed process, rejects unsupported signed job definitions before mutation, and retains recovery directories after incomplete rollback. Native jobs use SMAppService with signed, bundle-contained definitions and BundleProgram; never rewrite these definitions during installation. Stop preserves native registration; replacement and uninstall wait for native unregistration completion before removing the bundle. Background approval is separate from liveness and must not block uninstall. Preserve current-protocol rollback: bootout before authenticated managed-stop IPC; never signal an unverified PID. The managed startup guard checks actual launchd loading, not merely persistent SMAppService registration. Bundle cleanup accepts only the exact xtask member inventory and rejects symlinks or unknown files.

## Delivery

Inspect `git status` and existing changes before editing. Prefer `rg` for source searches. Preserve unrelated work. Commit each completed, validated feature using Conventional Commits when requested; review the staged diff and do not bypass hooks or rewrite existing commits without explicit direction.

Update [README.md](README.md) for user behavior and [CONTRIBUTING.md](CONTRIBUTING.md) for maintenance responsibilities. Report the checks actually run, compile-only platform coverage, commit hashes, and the final worktree state. Do not claim real portal or platform runtime validation from mocks or cross-compilation.
