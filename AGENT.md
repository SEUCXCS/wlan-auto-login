# Agent Guide

Execution rules and architectural invariants for coding agents. For detailed background and contributor workflows, see [CONTRIBUTING.md](CONTRIBUTING.md).

## 1. Workspace & Commands

Work from the repository root (Rust 2024 edition, resolver 3, committed `Cargo.lock`, always validate with `--locked`). Do not update dependencies incidentally during refactors.

| Path | Package | Role |
| --- | --- | --- |
| `crates/wlan-api` | `wlanlogin-lib` | Async portal protocol client & HTTP diagnostics |
| `crates/wlan-app` | `wlanlogin-cli` | CLI (`wlanlogin`), TUI, daemon, native platform adapters |
| `xtask` | `xtask` | Packaging (`bundle macos` / `bundle windows` / `bundle deb`) & boundary checks |
| `packaging/windows` | `wlanlogin-installer` | Independent Cargo workspace for Windows installer UI & MSI actions |
| `docs` | — | Portal protocol specifications |

Fixed identifiers (`crates/wlan-app/src/infra/identity.rs`, `infra/paths.rs`):
- `APP_NAME = "wlanlogin"` (binary, command, directory, tracing target prefix); `DISPLAY_NAME = "CXCS WLAN Login"`; env prefix `WLANLOGIN_`.
- App ID: `dev.cxcs.tools.wlanlogin`; directory layout: `SEUCXCS/wlanlogin` via `ProjectDirs::from_path` (never `ProjectDirs::from`).
- Icon source: `assets/icon/icon.svg` (pre-generated `packaging/macos/icon/icon.icns` and `packaging/windows/icon/icon.ico`).

### Validation & Build Commands

```sh
# Main workspace checks
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --workspace
cargo xtask check-boundaries
git diff --check

# Windows installer workspace (when packaging/windows changes)
cargo fmt --manifest-path packaging/windows/Cargo.toml -- --check
cargo clippy --manifest-path packaging/windows/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path packaging/windows/Cargo.toml --locked
rustfmt --edition 2024 --check packaging/windows/initialize.rs

# Cross-target compile checks (compile-only; install targets with rustup only if missing)
cargo check -p wlanlogin-cli --locked --target x86_64-pc-windows-gnu
cargo check -p wlanlogin-cli --locked --target aarch64-apple-darwin
cargo check -p wlanlogin-cli --tests --locked --target x86_64-pc-windows-gnu
cargo check -p wlanlogin-cli --tests --locked --target aarch64-apple-darwin

# Packaging
cargo xtask bundle macos [--codesign-identity <id>]
cargo xtask bundle windows --target x86_64-pc-windows-msvc
cargo xtask bundle deb                      # Debian/Ubuntu host with dpkg-dev
(cd packaging/archlinux && makepkg -s)      # Arch: packages the committed HEAD
```

- Safe CLI smoke checks: use `--help`, `<subcommand> --help`, and `--version` only. Never invoke operational commands without `--help` to probe behavior.
- CI (`.github/workflows/test.yml`) runs native checks on Linux x86_64, Windows x86_64 MSVC, and macOS Apple Silicon (stable Rust + `Cargo.lock`), plus a dedicated format & boundary job.

## 2. Architecture & Source Boundaries

Module roots in `crates/wlan-app/src`: `app.rs`, `infra.rs`, `system.rs`.

- **Dependency direction**: `app -> system -> infra` (`app -> infra` allowed). `system` and `infra` MUST NOT depend on `app`; `infra` MUST NOT depend on `system`. Keep Clap and terminal I/O out of `system` contracts. Do not modify `wlan-api` during app-only refactors.
- **Entry & layers**: `main.rs` -> `app::run()` -> platform `app/bootstrap/<platform>.rs::boot()` -> shared `app/bootstrap.rs::entry()`. Platform CLI definitions live in `app/bootstrap/<platform>/cli.rs`; reusable operations live in `app/controller/` (no terminal reads); service controllers call `system::flow` directly (never `shared` or platform adapters directly).
- **Platform source boundaries** (enforced by `cargo xtask check-boundaries`):
  - Put all target-dependent implementations under `system/{linux,macos,windows}/` (`flow`, `services`, `credentials`, `wifi`, `permission`, `runtime`).
  - Elsewhere, `cfg` / `cfg_attr` / `cfg!` for target OS is permitted ONLY in checked facades for module selection/re-export (same for test code; plain `#[cfg(test)]` is allowed). Never replace compile-time branches with runtime OS-name checks.
  - `system/<platform>/runtime/infra` compiles once via the `infra.rs` facade; shared state/protocol stays in `infra/control`.
  - Keep native API, XML encoding, ACL, and task-registration logic out of `system/shared`. Re-export `system/shared/flow` defaults when a platform has no override; never stub an unimplemented operation with success.
- **`xtask` layout**: `main.rs` delegates to `commands::run`; `cli.rs` + `cli/bundle.rs` declare commands (build flags after platform subcommand; `--codesign-identity` macOS-only); `commands/<cmd>.rs` and `commands/bundle/<platform>.rs` implement tasks; `support/` must not depend on `cli` or `commands` and locates binaries via Cargo JSON output.
- **Linux packages**: `.deb` (`packaging/debian/` templates) and Arch (`packaging/archlinux/PKGBUILD`) ship only `/usr/bin/wlanlogin`, license, and README. Never ship a systemd unit, enable services, or write runtime data from a package; `service install/uninstall` owns units and background copies. Keep runtime dependencies in sync between both package definitions.
- **Cleanup boundary**: Delete only app-owned files and empty `wlanlogin` directories. Preserve the shared `SEUCXCS` parent and sibling tools, and validate symlink / reparse-point boundaries.

## 3. Behavioral & Platform Invariants

### Daemon & Portal

- `AuthState::Unknown` MUST NEVER trigger automatic login.
- Always recheck the Wi-Fi session after any `await` before submitting portal credentials.
- Keep shutdown listeners alive and continuously drain NetworkManager / credential D-Bus event streams.
- Bound native calls and credential reads; keep at most one background credential read task in flight (timeout, logout, or invalidation must not spawn a second task).
- Keep `--user` and `--system` scopes strictly isolated (credentials, state, cleanup, wake/reset).
- `PortalConfig` fetches `loadConfig` before login/logout; when changing protocol code, update redacted loopback fixtures and `docs/eportal-*.md`.

### Credentials & Permissions

- **No startup pre-read**: Read credentials only after target Wi-Fi, pause, policy, and confirmed portal `Offline` guards pass.
- **Credential freshness (`CredentialPermit`)**: Bind reads to scope, config root, and persisted `credential_generation` in `control.json` (independent of pause/schedule revision; defaults to `0` if missing). `save.rs` and `delete.rs` MUST atomically increment and persist `credential_generation` under the operation lock before native mutation; persistence failure aborts mutation, and failed mutation never restores old generations. Revalidate permits on task completion (including post-timeout cached results) and after the final Wi-Fi await.
- **Purge**: `service uninstall --purge` deletes scoped credentials after verified stop/unregistration while holding daemon and operation locks, without requiring `control.json` to parse.
- **Error classification & IPC**: Typed errors live in `infra/error/credentials.rs`, classified in `shared/credentials/diagnostics.rs`. Keep daemon alive and preserve credentials on both `Waiting` and `Failed`. Never misclassify timeout (`NoReply`), format/version, duplicate, decryption, validation, or task errors as permission `Waiting`; keep unknown errors neutral. Status IPC is version 3 only (bounded JSON, validates scope/token, never triggers Keyring or leaks credentials/raw native messages).
- **Permissions**: Open string `PermissionId`, dynamic `PermissionProvider::permissions()` (empty on Linux/Windows; `macos.location` & `macos.keychain` on macOS). `check` never triggers UI; `get` requires interactive stdin + stderr; background/non-interactive calls never prompt. Validate every field of `PermissionReport` during install. Reject unknown IDs, never report unavailable as granted, never delete credentials on failure.

### Platform-Specific Rules

- **Linux**: User reads require a live logind graphical session, display socket, and synchronized D-Bus activation environment (even if a provider is already running) before calling official `dbus-secret-service` (DH encryption, zero prompt timeout, standard `org.freedesktop.secrets`, standard D-Bus activation). No vendoring, owner pinning, or desktop sniffing; keep `default.target` and system scope independent of GUI. Bind Linux user permits to logind session ID, `TimestampMonotonic`, and local session generation (verified before and after live queries); keep `session-ready`/`session-end` hints in memory only.
- **macOS**:
  - `CFBundleShortVersionString` = Cargo version; `CFBundleVersion` = atomic integer in `.bundle-build/build-number` (shared across profiles/targets, survives `cargo clean`, never reused on failure or silently reset).
  - Install whole signed `.app` bundles (`flow/install.rs` + `services/payload.rs`); validate exact xtask bundle member inventory and reject symlinks/unknown files.
  - `SMAppService` definitions (`BundleProgram`, `AssociatedBundleIdentifiers`) are generated pre-signing and MUST NOT be mutated during install. Native register/unregister/status run via hidden `service native` in the installed copy.
  - Stop uses `bootout` + authenticated managed-stop IPC (never signal unverified PIDs) and retains native registration; replace/uninstall wait for native unregistration. Background approval is independent of liveness and must not block uninstall. Managed startup guard checks actual `launchd` loading. Retain rollback backups until ready (including one `ThrottleInterval` retry + single re-register on `EX_CONFIG` with LWCR update flag) and preserve recovery dirs on incomplete rollback.
  - Bare CLI must not implicitly launch the Wi-Fi host; host uses hidden `daemon --internal-wifi-host` (`--system-helper` for system scope) with a purpose comment. Host IPC validates protocol version, request ID, frame size, live peer audit-token code identity + UID (+ active console user for system) and never carries credentials.
  - Core Location + delegate stay on one RunLoop thread with a per-iteration `autoreleasepool`; interactive requests check `NSRunningApplication.isActive` and never use `open -g`. Detect connected interfaces with redacted SSID/BSSID via interface mode and return an error.
  - Serialize Keychain operations around the process-wide UI switch; lookup ref-only for save/delete/ACL; create new items via `SecKeychainItemCreateFromContent` with initial ACL. Run `permission check macos.keychain --json` in the target installed binary before/after install and rollback; only extend a decrypt-only ACL when needed, preserving mixed/owner/partition ACLs.
- **Windows**:
  - System paths resolve Program Files / ProgramData Known Folders only (LocalSystem-compatible). `service.lock` in `state`; `daemon.lock` and `operation.lock` in `run`.
  - Keep SCM entry in bootstrap before Tokio; use one consistent service ID; append `{SID}` for user tasks.
  - Task XML requires matching UTF-16 declaration, UTF-16LE bytes, and BOM (readers accept legacy UTF-8). Validate staged XML with `TASK_VALIDATE_ONLY` before stopping; snapshot live registration XML independently of disk leftovers; restart restored tasks only if previously running/queued; confirm absence via numeric COM query before treating failed deletion as complete; retain XML + file backups after incomplete rollback.
  - Decode `schtasks`/`sc` via system OEM code page; PowerShell queries emit UTF-8 explicitly; never change global console/system locale. Free `SHGetKnownFolderPath` outputs via a scoped guard calling `CoTaskMemFree` on all paths.
  - Installer builds (`xtask bundle windows`) require WiX 6.0.2 + `WixToolset.UI.wixext/6.0.2`, verify MSI cabinet payloads byte-for-byte (CLI, `README.md`, `LICENSE`), use a 180s Job Object timeout with rollback, inherit ACLs on diagnostic logs, skip account page & credential purge on `WIX_UPGRADE_DETECTED`, and keep credential generations monotonic on restore.

### Errors & Logging

- Use `infra::error::{Error, Result, Context, Message}` with English `thiserror` templates in `infra/error/`; confine `anyhow` to external error boundaries.
- Use `tracing` with UTC RFC 3339 timestamps and structured fields (`error`, `scope`, `retry_after_secs`, `outcome`, `next_check_secs`). Load `log.level` from scoped `config.json` (default `info`; `RUST_LOG` overrides).
- Portal diagnostics (`crates/wlan-api/src/diagnostics.rs`) capture HTTP body before JSONP parsing, redact credentials/cookies/session/device IDs, escape newlines, and log `debug` on success / `warn` on failure.
- Daemon runtime logs first check, state changes, error recovery, and retry starts at `info`. Keep Chinese copy in CLI/TUI frontend only.

## 4. Testing Rules

- Keep all `wlan-app` tests inline at the bottom of their owning file (`#[cfg(test)] mod tests`). Never split into `tests.rs`, separate test directories, or `#[path]` includes. Preserve `wlan-api` integration tests and fixtures.
- Maintain one table-driven cross-platform suite per shared contract: `app.rs` (CLI), `infra.rs` (infrastructure), `system/define/credentials.rs` (credentials), `system/shared/flow/mod.rs` (uninstall transaction). Keep only documented native exceptions in platform modules. Use native symlink/junction helpers for directory-redirection tests.
- Never hit the real campus portal, switch Wi-Fi, read real keyrings, or mutate real OS services. Use loopback HTTP mocks, temp dirs, fake credentials/commands, private `dbus-daemon`, and isolated child processes.
- Never run the whole suite with `--include-ignored` (`signal_child`, `provider_child`, `read_child` are invoked explicitly by parent tests).
- If a sandbox blocks local sockets or subprocesses, rerun isolated tests in a permitted environment and report environment vs product failures clearly.
- On Windows, if local Temp ACL allows untrusted writers, use an isolated temp dir writable only by the current user and SYSTEM—never weaken product storage validation.

## 5. Delivery Checklist

1. Inspect `git status` before editing; prefer `rg` for searches; preserve unrelated worktree changes.
2. Run targeted tests during iteration and the full applicable check suite before finishing.
3. Sync docs when behavior or rules change: [README.md](README.md) (user-facing), [CONTRIBUTING.md](CONTRIBUTING.md) (contributor architecture/rules), and this file (agent invariants).
4. When asked to commit, verify the staged diff contains only intended files, use Conventional Commits, and do not bypass hooks or rewrite history unless explicitly instructed.
5. Report exact checks run, compile-only vs runtime coverage, commit hashes, and final worktree state—never claim real portal or native runtime validation from mocks or cross-compilation.
