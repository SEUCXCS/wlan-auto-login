# 贡献指南

感谢参与 WLAN Auto Login 的开发与维护。提交 Issue 时请提供操作系统、程序版本、安装作用域、复现步骤与预期行为，并对日志及抓包样本中的账号、密码、Cookie 与令牌脱敏。

## 1. 开发环境与仓库结构

项目使用 Rust 2024 edition Cargo workspace（resolver 3），并提交应用依赖的 `Cargo.lock`。仓库未固定 `rust-toolchain.toml` 或 MSRV；验证与构建统一使用 `--locked`。

| 目录 | Cargo 包名 | 职责 |
| --- | --- | --- |
| `crates/wlan-api` | `wlanlogin-lib` | 校园网门户协议、JSONP 解析、认证状态与 HTTP 诊断 |
| `crates/wlan-app` | `wlanlogin-cli` | CLI / TUI、业务控制、后台守护与操作系统集成 |
| `xtask` | `xtask` | macOS / Windows / Debian / RPM 打包与平台源码边界检查 |
| `packaging/windows` | `wlanlogin-installer` | 独立 workspace 的 Windows 安装向导与 MSI 自定义动作 |
| `packaging/{macos,linux}` | — | macOS 应用资源与 man 手册、Linux 发行版打包定义与双语 man 手册 |
| `docs` | — | 门户协议说明与研究记录 |

原生构建依赖：

- **通用**：Rust、Cargo、`rustfmt`、`clippy`、Git。
- **Linux**：C 工具链、`pkg-config`、D-Bus 开发库、`dbus-daemon` 及标准 POSIX 工具（Debian/Ubuntu：`build-essential pkg-config libdbus-1-dev dbus-daemon`；Fedora：`gcc pkgconf-pkg-config dbus-devel dbus-daemon`）。
- **macOS**：Xcode Command Line Tools。
- **Windows**：与目标架构对应的 MSVC 或 GNU 构建工具；打包安装向导另需 .NET 8 SDK 与 WiX 6.0.2（含 `WixToolset.UI.wixext/6.0.2`）。

## 2. 架构与模块职责

### 2.1 应用标识与路径约定

- `APP_NAME` 为 `wlanlogin`（命令名、二进制名、目录名与日志 target 前缀）；`DISPLAY_NAME` 为 `CXCS WLAN Login`（macOS bundle 显示名、Windows 服务/任务描述、Linux systemd 描述）；环境变量前缀为 `WLANLOGIN_`。
- 应用 ID 固定为 `dev.cxcs.tools.wlanlogin`，数据目录采用 `SEUCXCS/wlanlogin` 两级结构。名称常量集中于 `infra/identity.rs`，路径构造集中于 `infra/paths.rs`。
- 用户目录使用 `ProjectDirs::from_path`（不使用 `ProjectDirs::from`，避免 Linux 忽略组织名或 macOS 拼接点分目录）。
- 清理逻辑只能删除应用自身文件与已清空的 `wlanlogin` 目录，必须保留共享父目录 `SEUCXCS` 及同级工具，并校验组织与应用目录的符号链接 / reparse point 边界。
- 图标源文件为 `assets/icon/icon.svg`，预生成图标分别存放于 `packaging/macos/icon/icon.icns` 与 `packaging/windows/icon/icon.ico`（包含 32 / 64 / 128 / 256 像素）。

### 2.2 `crates/wlan-app` 分层

依赖方向严格为 `app → system → infra`（允许 `app → infra` 直接调用）：

- `system` 与 `infra` 不得反向依赖 `app`；`infra` 不得依赖 `system`。
- Clap 与终端输入输出属于应用前端，不得进入 `system` 契约。
- `wlan-api` 不依赖应用层；重构应用架构时不要附带修改协议库。
- `main.rs` 只调用 `app::run()`；`app::run()` 解析参数后进入编译期选中的平台 `boot()`，共享初始化与命令分发由 `app/bootstrap.rs` 的 `entry()` 负责。

| 路径（相对 `crates/wlan-app/src`） | 职责 |
| --- | --- |
| `app.rs` | 模块声明、Clap 顶层定义、编译期平台 `boot()` 分发 |
| `app/bootstrap.rs` | Tokio runtime 生命周期、共享 `entry()`（作用域、日志、依赖装配与命令分发） |
| `app/bootstrap/{linux,macos,windows}.rs` | 平台启动编排（Linux 普通入口、macOS AppKit 入口、Windows SCM 回调） |
| `app/bootstrap/<platform>/cli.rs` | 平台 CLI 参数声明、解析与 controller 请求转换 |
| `app/context.rs` | 门户客户端、路径与 Wi-Fi / 凭据适配器装配 |
| `app/cli/`、`app/tui.rs` | 终端交互、stdin 凭据读取、结果展示与 TUI 表单 |
| `app/controller/` | 可复用业务操作（`login`、`logout`、`daemon`、`service`、`config`、`permission`），不直接读取终端 |
| `app/daemon.rs`、`app/daemon/` | daemon 生命周期、调度循环（`runtime.rs`）、单次检查（`check.rs`）、重试策略（`policy.rs`）与后台凭据读取（`credentials.rs`） |
| `infra/error/` | `thiserror` 错误类型、英文诊断模板与门户错误转换 |
| `infra/logging.rs`、`infra/file_log.rs` | 结构化日志格式、文件日志历史与跟随读取 |
| `infra/config.rs`、`infra/paths.rs`、`infra/scope.rs` | 配置持久化、线路偏好、目录定位与作用域校验 |
| `infra/control/` | 本地 IPC 协议与传输、控制状态、单实例锁与操作锁 |
| `infra/command.rs`、`infra/shutdown.rs` | 可替换外部命令执行、超时与停机信号 |
| `infra/filesystem.rs`、`infra/cleanup.rs` | 私有目录、原子写入与有边界清理 |
| `infra/identity.rs` | 应用 ID、目录名、命令名与原生服务/凭据标识 |
| `system.rs` | 编译期选择原生平台，暴露系统能力入口 |
| `system/define/` | `credentials`、`services`、`wifi`、`permission` 的公共契约、数据模型与报告类型 |
| `system/shared/` | 跨平台共用适配器、凭据编码与诊断分类，以及 `system::flow` 导出的统一流程 |
| `system/{linux,macos,windows}/flow/` | 原生步骤、平台 hook 和 Linux 会话通知等平台专属入口 |
| `system/{linux,macos,windows}/services/` | 原生服务模板（`definition.rs`）、管理器命令与状态解析（`manager.rs`）、产物与就绪检查 |
| `system/{linux,macos,windows}/credentials/` | 平台用户与系统凭据后端 |
| `system/{linux,macos,windows}/wifi/` | 平台 SSID 查询、会话标识与网络变更事件 |
| `system/{linux,macos,windows}/permission/` | 平台权限声明、状态诊断与交互授权 |
| `system/{linux,macos,windows}/runtime/` | Windows SCM、macOS AppKit/Wi-Fi 宿主、Linux 会话与凭据事件、平台原生 `infra` 源码 |

### 2.3 平台源码边界（`check-boundaries`）

- 平台相关实现全部位于 `system/linux`、`system/macos` 或 `system/windows`。
- 其余目录中，目标平台相关的 `cfg`、`cfg_attr`、`cfg!` 仅允许出现在受 `cargo xtask check-boundaries` 检查的门面文件中用于模块选择和重导出。测试代码遵守相同约束（普通 `#[cfg(test)]` 允许）。禁止用运行时操作系统名判断替代编译期分支。
- `system/<platform>/runtime/infra` 下的原生基础设施代码仅通过 `infra.rs` 门面编译一次，属于源级平台选择，不构成 `infra → system` 反向依赖；共享状态与协议文件保留在 `infra/control`。
- `shared/flow` 负责步骤顺序、锁、验证和回滚。每个业务步骤提供前后 hook，平台模块只处理原生差异，包括 API、XML、ACL 和任务注册。前置 hook 失败时跳过步骤，后置 hook 接收已执行步骤的结果。回滚失败时必须保留恢复资料。

### 2.4 `xtask` 与 `packaging/windows`

`xtask` 负责应用打包与边界检查（普通编译直接使用 Cargo）：

| 路径（相对 `xtask/src`） | 职责 |
| --- | --- |
| `main.rs`、`cli.rs`、`cli/bundle.rs` | 参数解析与顶层/平台子命令定义（`--profile`、`--target`、`--target-dir` 位于平台子命令之后；`--codesign-identity` 仅属 macOS） |
| `commands.rs`、`commands/check_boundaries.rs` | 命令分发与平台源码边界检查 |
| `commands/bundle.rs` | 打包平台分发 |
| `commands/bundle/macos.rs` 及子模块 | macOS 应用构建、组装（`assemble.rs`）、构建号原子分配（`build_number.rs`）、签名验证（`sign.rs`）与原子发布回滚（`publish.rs`） |
| `commands/bundle/windows.rs` | Windows CLI / 安装动作 / 初始化 DLL 构建、MSI 与安装向导打包及 cabinet 逐字节校验 |
| `commands/bundle/deb.rs` | Linux CLI 构建、`dpkg-shlibdeps` 计算共享库依赖并校验 Debian 12 glibc 下限、生成 Debian 变更记录、按 `packaging/linux/distro/debian/` 模板生成元数据并用 `dpkg-deb` 打包 |
| `commands/bundle/rpm.rs` | Linux CLI 构建、按 `packaging/linux/distro/fedora/wlanlogin.spec` 模板用 `rpmbuild` 打包，并按生成的 `GLIBC_*` 依赖校验 Fedora 43 glibc 下限 |
| `commands/bundle/linux.rs` | Debian 与 RPM 共用的工具检查、打包日期格式化与文件暂存 |
| `support/` | Cargo JSON 产物构建（`cargo.rs`）、外部进程（`process.rs`）、工作区资源（`workspace.rs`）与平台工具检查（`platform/`）；不依赖 `cli` 或 `commands`，复用 `infra/identity.rs` |

新增顶层命令需在 `cli.rs` 与 `commands.rs` 注册并实现 `commands/<command>.rs`；新增打包平台需在 `cli/bundle.rs`、`commands/bundle.rs` 注册并实现 `commands/bundle/<platform>.rs`，打包资源存放于 `packaging/<platform>/`。

**Linux 软件包（Debian / Fedora / Arch）**

- **统一安装内容**：三份软件包仅安装 `/usr/bin/wlanlogin`、许可证、README 与 `packaging/linux/man/` 下的中英文 man 手册（Debian / RPM 另附各自格式的变更记录）。systemd unit 与后台副本始终由 `service install/uninstall` 管理，软件包不得放置 unit、启用服务或写入运行数据目录。
- **同步约束**：修改运行依赖或安装文件时同步更新 Debian、RPM 与 Arch 三份定义；修改命令、选项、作用域、配置项、环境变量或路径时同步更新两份 man 手册。
- **Debian（`bundle deb`）**：最低支持 Debian 12 / Ubuntu 24.04（`libc6` 要求高于 2.36 时失败，须在 Debian 12 或更旧系统构建）。版本格式 `<workspace>-<revision>`（预发布 `-` 转 `~`，`--revision` 默认 1）；`Depends` 合并 `dpkg-shlibdeps` 与 `packaging/linux/distro/debian/control`，维护者取自模板并生成单条目 `changelog.Debian.gz`。
- **Fedora（`bundle rpm`）**：最低支持 Fedora 43（最高 `GLIBC_*` 符号版本高于 2.42 或未生成该依赖时失败）。`packaging/linux/distro/fedora/wlanlogin.spec` 由 xtask 填充 `@NAME@` 占位符，`%install` 直接安装 `%{_sourcedir}` 中已编译的 CLI 与文档（兼容 Fedora `rpm-build` 与 Debian/Ubuntu `rpm`）。版本格式 `<workspace>-<release>%{?dist}`（预发布 `-` 转 `~`，`--release` 默认 1）。
- **变更日期与发布说明**：Debian / RPM 变更记录日期按 UTC 格式化，优先读取 `SOURCE_DATE_EPOCH`（非负 Unix 秒级时间戳），否则读取当前 Git 提交时间（无 `.git` 的源码压缩包须显式设置 `SOURCE_DATE_EPOCH`）。面向用户的版本说明通过 GitHub Release 的 **Generate release notes** 生成。
- **Arch Linux（`packaging/linux/distro/archlinux/PKGBUILD`）**：通过 `git+file` 拉取检出的已提交 HEAD，在解析阶段计算 `pkgver`（不使用 `pkgver()`，避免 `makepkg` 改写受控文件）；`check()` 仅执行 `--version` 与 `--help`。

`packaging/windows` 使用独立 Cargo workspace 与 `Cargo.lock`：
- `installer.wxs` 定义 WixUI 向导页、文件组件、PATH、升级与动作顺序；`initialize.rs` 准备唯一事务 ID 与一次性 DPAPI 凭据文件。
- `src/transaction.rs` 调用 CLI 完成服务事务，`state.rs`、`credentials.rs`、`services.rs` 负责备份与恢复，`diagnostics.rs` 记录阶段与 CLI 输出，`verify.rs` 负责 MSI / cabinet 校验。

## 3. 子系统设计规范

### 3.1 CLI、业务流与后台守护

- 公开服务命令统一为 `service install/uninstall/start/stop/restart/status/logs`，`daemon` 仅运行当前进程循环；原生服务定义调用隐藏 `service run`。
- 服务 controller 统一调用 `system::flow`，不直接引用 `shared` 或具体原生平台；controller 返回结构化数据或接收显式 writer / 取消信号，不读取终端；TUI 不直接调用门户或服务管理器；服务状态判定不依赖日志文本解析。
- 调整 daemon 行为时，先在 `app/daemon/policy.rs` 定义状态转换，再接入 `check.rs` 或 `runtime.rs`。
- 关键守护边界：
  - `AuthState::Unknown` 不得触发自动登录。
  - 完成门户状态和页面预检后复核凭据许可及 Wi-Fi 会话。最终 Wi-Fi 查询返回后检查本地会话和凭据代次，随即提交认证。
  - 手动登录只恢复相同作用域的 daemon；保持停机监听器存活并持续排空 Linux D-Bus 事件流。
  - 限制原生调用与凭据读取超时，至多保留一个在途后台凭据读取任务。

### 3.2 门户协议

- 协议实现集中于 `crates/wlan-api`，`PortalConfig` 分别配置状态、页面配置、登录与注销端点；页面与方案索引在登录和注销前实时拉取。
- 修改协议时同步更新脱敏 loopback fixtures 与 `docs/eportal-*.md`，测试需分别覆盖无凭据配置请求与凭据提交路径。

### 3.3 权限框架

- `PermissionId` 为开放字符串命名空间，`PermissionDescriptor` 支持名称、可选描述与键值元数据；`PermissionProvider::permissions()` 返回动态列表（可为空，不得在公共契约中硬编码平台枚举）。
- `permission list/check/get` 在所有平台编译暴露，由运行时 provider 决定能力：Linux / Windows 返回空列表，显式传入未知 ID 必须报错。
- `check` 禁用 UI 且不申请授权；`get` 仅在 stdin 与 stderr 均为终端时请求权限；后台与非交互命令不得弹窗。
- 诊断区分 `Missing`、`NeedsInteraction`、`Denied`、`Unavailable`，逐项保留结果，读取失败不得删除已有凭据，也不得将不可用能力报告为已授权。
- JSON 输出使用带版本、作用域与权限 ID 的 `PermissionReport`，安装流程严格校验其全部协议字段。

### 3.4 凭据存储与后台读取时序

- `define/credentials.rs` 只定义契约与状态（`Available` / `Missing` / `Waiting` / `Failed`），不执行 I/O；`shared/credentials/codec.rs` 负责格式编码，`flow/credentials` 为统一入口。
- **延迟读取**：daemon 启动时禁止预读凭据，仅在通过目标 Wi-Fi、暂停状态、策略期限与门户明确 `Offline` 校验后才发起后台读取。
- **凭据新鲜度（`CredentialPermit`）**：读取结果封装于不可打印的 `CredentialPermit`，绑定作用域、配置根目录与 `control.json` 中独立的 `credential_generation`。缺少文件或旧文件缺少字段时按 `0` 读取，随后正常保存时持久化。
  - 常规保存（`save.rs`）与删除（`delete.rs`）在持有 operation lock 下先原子递增并持久化 `credential_generation`，再调用原生后端；持久化失败必须中止变更，原生变更失败不得回退旧代次。
  - 任何完成结果（含超时后保留的任务）在消费时及最终 Wi-Fi `await` 后均须复检许可；超时、注销或失效不得另起第二个原生读任务。
  - `service uninstall --purge` 在确认服务注销、后台停止并持有 daemon / operation lock 后直接删除作用域凭据，不要求解析待删除的 `control.json`。
  - Linux 和 macOS 在持久状态目录保留私有锁文件。停止旧版后台进程时同时检查旧运行目录中的守护进程锁。
- **诊断分类与 IPC**：类型化错误定义于 `infra/error/credentials.rs`，由 `shared/credentials/diagnostics.rs` 分类。锁定、前台交互、拒绝访问与缺少 provider 归为 `Waiting`；超时（含 D-Bus `NoReply`）、格式/版本错误、重复条目、解密、校验与任务失败归为 `Failed`；未知错误保持中性类别，禁止误判为权限等待。
- `Waiting` 与 `Failed` 均保持 daemon 存活并保留凭据，不触发门户网络退避，也不解除门户凭据拒绝。状态 IPC 仅接受版本 3 有界 JSON 帧，校验作用域与令牌，不得激活 Keyring 或暴露凭据与原始自由文本。

### 3.5 平台专项规范

#### Linux

- 用户后台读取前依次检查活动 logind 会话、显示 socket 与用户 systemd 环境，同步 D-Bus 激活环境后，再使用官方 `dbus-secret-service`（DH 加密、零 prompt 超时）连接标准 `org.freedesktop.secrets` 并允许标准 D-Bus 激活（包括 provider 已运行的情况）。不 vendor 依赖、不固定 owner、不按桌面/钱包类型分支，`default.target` 保持不变。
- 用户 `CredentialPermit` 额外绑定 logind 会话 ID、`TimestampMonotonic` 与本地会话代次，实时查询前后均做校验；`session-ready` / `session-end` 提示仅存内存并绑定实际会话。
- 通过独立持续排空的 D-Bus 流监听 provider / 解锁事件，断连后自动重建并辅以定时重试。系统作用域完全独立于图形会话。
- 系统凭据每次从当前受保护文件读取或命中与文件标识和代次绑定的缓存。安装回滚按 systemd 原有启用与运行状态恢复。

#### macOS

- **应用包与构建号**：`CFBundleShortVersionString` 对应 Cargo 版本；`CFBundleVersion` 由 `build_number.rs` 在签名前通过 `.bundle-build/build-number` 加锁原子递增，跨 profile/target 共享，不因失败复用也不静默重置。`service install` 安装完整签名包（`flow/install.rs` + `services/payload.rs`），严格校验 xtask 维护的包成员清单，拒绝未知文件与符号链接。
- **原生服务（`SMAppService`）**：启动定义（含 `BundleProgram` 与 `AssociatedBundleIdentifiers`）在 xtask 签名前生成于 `Contents/Library/LaunchAgents` / `LaunchDaemons`，安装期间严禁改写签名包。原生操作由安装副本的隐藏 `service native status/register/unregister` 执行。
- **生命周期与回滚**：停止先 `bootout` 再通过带令牌的托管模式 IPC 等待退出（禁止向未校验 PID 发信号），且保留原生注册；卸载与替换前等待原生注销完成；后台批准状态独立于存活状态且不得阻塞卸载；托管启动持锁后复查实际 launchd 加载状态；安装确认就绪前保留回滚副本，回滚未完成时保留恢复目录。若 `launchd` 报代码签名启动失败且需更新 LWCR，允许完成一次受 `ThrottleInterval` 限制的系统重试；若仍报 `EX_CONFIG` 且需更新 LWCR，使用同一安装副本重新注册一次（调整 `ThrottleInterval` 时同步核对等待时长）。
- **Wi-Fi 宿主与定位**：裸 CLI 不得隐式启动宿主；宿主通过隐藏 `daemon --internal-wifi-host` 运行（系统客户端组合 `--system-helper`），跳过自动登录，源码需保留用途注释。宿主 IPC 校验协议版本、请求 ID、帧长、内核审计令牌签名与 UID（系统模式额外校验活动控制台用户），严禁传输校园网凭据。Core Location 对象与 delegate 固定在同一 RunLoop 线程，每轮清空独立 `autoreleasepool`；交互授权不使用 `open -g`，需在工作线程确认 `NSRunningApplication.isActive` 后再申请。因定位受限时 SSID 与 BSSID 可能同时被隐藏，须通过接口模式识别“已连接但 SSID 不可读”并返回错误。
- **Keychain**：用户与系统访问共用串行访问器保护进程级交互开关；保存、删除与 ACL 授权仅查找条目引用不读取密码；已有条目直接更新，仅明确不存在时用 `SecKeychainItemCreateFromContent` 创建并写入初始 ACL。安装前后与回滚均由实际目标二进制执行 `permission check macos.keychain --json`：已授权不改 ACL，仅确需授权时复制 `SecAccess` 并新增/扩展仅含 decrypt 的 ACL，不改动混合权限、所有者或 partition ACL。

#### Windows

- **路径与服务**：系统路径仅解析 Program Files 与 ProgramData Known Folders（兼容无交互用户配置的 LocalSystem）；`service.lock` 位于 `state`，`daemon.lock` 与 `operation.lock` 位于 `run`。SCM 接入留在 bootstrap（Tokio 启动前）；服务创建、查询、启停与回调共用同一服务 ID；用户计划任务名追加 `{SID}`。
- **任务 XML 与安装验证**：计划任务定义使用匹配的 UTF-16 声明、UTF-16LE 字节与 BOM（读取兼容旧版 UTF-8）。停止旧任务前先以 `TASK_VALIDATE_ONLY` 校验暂存文件；独立快照实际注册 XML；恢复任务仅在原状态为运行或排队时重启；删除失败前通过数值 COM 查询确认任务不存在；回滚未完成时同时保留原生 XML 与文件备份；安装成功要求任务已注册、启用且处于运行或排队状态。
- SCM 回滚恢复原启动类型和运行状态。MSI 恢复凭据前在锁内重新确认代次，并给回滚自身写入的代次记录事务标识，以便识别可重试的恢复。
- **编码、资源与安装器**：`schtasks` / `sc` 输出在原生进程边界按系统 OEM 代码页解码，PowerShell 查询显式输出 UTF-8，不修改全局控制台或系统区域设置；`SHGetKnownFolderPath` 返回内存必须通过作用域守卫在所有路径调用 `CoTaskMemFree`。安装器使用 GUI 子系统与 180 秒 Job Object 超时终止进程树并回滚；日志目录继承引导器私有目录 ACL；通过 `WIX_UPGRADE_DETECTED` 跳过升级账号页并避免清理旧凭据；构建时解包 MSI cabinet 逐字节校验 CLI、`README.md` 与 `LICENSE`。

### 3.6 错误处理与日志

- 业务代码使用 `infra::error::{Error, Result, Context, Message}`，在 `infra/error/message.rs` 与 `credentials.rs` 用 `#[derive(thiserror::Error)]` 定义英文诊断模板；`anyhow` 仅用于边界封装外部错误。
- 门户错误在 `infra/error/portal.rs` 转换为安全英文摘要并保留原始错误供重试分类。
- `crates/wlan-api/src/diagnostics.rs` 在 JSONP 解析前捕获 HTTP 请求行、响应状态行、头信息与脱敏正文（凭据、Cookie、会话与设备标识统一脱敏，换行转义）；成功记录为 `debug`，失败记录为 `warn`。
- `infra/logging.rs` 统一负责 UTC RFC 3339 时间与格式；`infra/config.rs` 从作用域 `config.json` 读取 `log.level`（默认 `info`，`RUST_LOG` 覆盖）。daemon `runtime.rs` 在首次检查、结果变化、错误恢复与重试启动时用 `info` 输出 `outcome` 与 `next_check_secs`。中文 CLI / TUI 文案仅保留在前端。

## 4. 测试与验证

### 4.1 CI 与组织规则

- `.github/workflows/test.yml` 在推送或 PR 至 `main` 以及手动触发时运行（纯 `docs/**`、`CONTRIBUTING.md`、`AGENT.md` 改动通过 `paths-ignore` 跳过）。先执行独立的格式与平台边界检查门禁（`quality`），通过后再并行运行 **Linux x86_64**、**Windows x86_64 MSVC**、**macOS Apple Silicon** 原生测试矩阵（均使用 stable Rust 与 `Cargo.lock`）及 `archlinux` 容器构建：Linux 以 dev profile 验证 `bundle deb` 与 `bundle rpm`，macOS 以 dev profile 验证 ad-hoc `bundle macos`，Windows 验证安装器测试与 dev profile `bundle windows`，`archlinux` 在 `archlinux:base-devel` 容器中以非 root 用户运行 `makepkg` 构建并检查 PKGBUILD。
- `.github/workflows/release.yml` 在发布时于 Ubuntu 22.04（glibc 2.35，低于 Debian 12 与 Fedora 43 下限）由同一 release 二进制构建 Linux 压缩包、`.deb` 与 `.rpm`，在同样的 Arch Linux 容器中构建 `.pkg.tar.zst`，与其他平台产物一同上传并生成 `SHA256SUMS`。
- `crates/wlan-app` 的单元测试统一放在所属源码文件末尾的 `#[cfg(test)] mod tests`，不拆分为 `tests.rs`、独立测试目录或 `#[path]` 引入。共用假实现可放在基础模块测试中（如 `infra::command::tests`）。`crates/wlan-api` 保持现有集成测试与 fixtures 结构。
- 跨平台行为只维护一份表驱动测试套件，平台目录仅保留原生特例：

| 公共契约位置 | 覆盖范围 | 平台特例 |
| --- | --- | --- |
| `app.rs` | 服务命令、作用域、清理选项、帮助、权限命令、交互冲突 | macOS 内部宿主与原生服务入口 |
| `infra.rs` | 日志脱敏、作用域、锁竞争、IPC 认证、目录保护与重定向拒绝 | Unix 信号/权限；Windows named pipe、ACL、OEM 编码 |
| `system/define/credentials.rs` | 错误分类与脱敏、范围/目录绑定、代次失效与写入失败拦截 | Linux 会话门槛；macOS Keychain；Windows DPAPI |
| `system/shared/flow/mod.rs` | 启停失败、回滚、先停后删、普通卸载与 purge 幂等性 | Linux systemd 链接；macOS 签名包；Windows 任务 XML 与快照 |

目录重定向测试使用平台测试辅助函数（Unix 符号链接或 Windows junction），不得因权限原因在 Windows 上跳过公共保护测试。

### 4.2 隔离要求与检查命令

- 修改公共流程前先确认现有回归测试覆盖顺序与失败恢复。Smoke test 只验证入口能走通，IPC 和 shutdown 的共用断言在各自的原生传输或信号实现上运行。

- 严禁在开发测试中访问真实校园网门户、切换 Wi-Fi、读取真实密钥环或操作真实系统服务；一律使用 loopback mock、临时目录、假凭据、模拟命令、私有 `dbus-daemon` 与隔离子进程。
- `signal_child`、`provider_child`、`read_child` 为父测试显式调用的 `#[ignore]` 辅助用例，禁止对整个套件使用 `--include-ignored`。
- Windows 本地 Temp 目录若被外部工具写入非信任 ACL，请指定仅当前用户与 SYSTEM 可写的隔离临时目录运行测试，不得放宽产品存储校验。

仓库根目录完整检查：

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --workspace
cargo xtask check-boundaries
git diff --check
```

修改 `packaging/windows` 时额外运行：

```sh
cargo fmt --manifest-path packaging/windows/Cargo.toml -- --check
cargo clippy --manifest-path packaging/windows/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path packaging/windows/Cargo.toml --locked
rustfmt --edition 2024 --check packaging/windows/initialize.rs
```

跨平台静态检查（仅验证编译，不代表链接或运行验证）：

```sh
rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin
cargo check -p wlanlogin-cli --locked --target x86_64-pc-windows-gnu
cargo check -p wlanlogin-cli --locked --target aarch64-apple-darwin
cargo check -p wlanlogin-cli --tests --locked --target x86_64-pc-windows-gnu
cargo check -p wlanlogin-cli --tests --locked --target aarch64-apple-darwin
```

安全 CLI smoke 检查仅使用 `--help` 与 `--version`。

## 5. 提交与 PR 规范

- 每个独立可验证的功能单独提交，提交信息遵循 Conventional Commits（如 `fix(portal): accept padded JSONP callbacks`、`refactor(app): separate login controller from terminal input`）。
- 提交前核对 staged diff，仅包含本次改动，保留他人未提交工作。
- PR 描述需列明行为变化、影响范围及实际执行的验证项（将编译检查与运行测试分开说明）。
- 命令、行为、目录职责或维护规则变更时，同步更新 [README.md](README.md)、本指南与 [AGENT.md](AGENT.md)。
