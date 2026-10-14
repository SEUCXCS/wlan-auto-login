# 贡献指南

感谢参与 WLAN Auto Login 的维护。提交问题时请说明操作系统、程序版本、安装范围、复现步骤和预期行为；日志和协议样本请先去除账号、密码、令牌及其他个人信息。

## 开发环境

项目使用 Cargo workspace、Rust 2024 edition 和 resolver 3：

| 目录 | Cargo 包名 | 职责 |
| --- | --- | --- |
| `crates/wlan-api` | `wlanlogin-lib` | 门户请求、JSONP 解析、认证状态和协议错误 |
| `crates/wlan-app` | `wlanlogin-cli` | CLI / TUI、业务操作、daemon 与操作系统集成 |
| `docs` | — | 登录、注销、状态查询的协议依据 |

安装 Rust、Cargo、rustfmt 和 Clippy，并提交应用依赖的 `Cargo.lock`。仓库没有 `rust-toolchain.toml`，也没有声明 MSRV；此前架构重构使用 Rust/Cargo `1.100.0-nightly` 验证，这不是强制 nightly 的声明。若要建立稳定工具链或最低版本要求，需要单独验证整个依赖树并更新文档。

Linux 需要 C 工具链、`pkg-config`、D-Bus 开发库，测试还需要 `dbus-daemon` 及常见 POSIX 工具。Debian / Ubuntu 可安装 `build-essential pkg-config libdbus-1-dev dbus-daemon`。macOS 使用 Xcode Command Line Tools；Windows 使用与 Rust target 对应的 MSVC 或 GNU 构建工具。

## 模块职责

`xtask/` 提供 `cargo xtask bundle macos`，负责调用 Cargo 构建、生成 plist、组装与验证签名应用包。名称复用 `infra/identity.rs`，版本来自 workspace；plist 模板位于 `packaging/macos/`。普通编译使用 Cargo。维护打包流程时验证参数覆盖环境变量、实际构建产物路径以及失败保留旧 bundle 的行为。

`main.rs` 只调用 `app::run()`。不要向入口增加认证、服务管理或平台分支。

`app::run()` 只解析参数并调用编译期选中的平台 `boot()`。平台启动编排归 `app/bootstrap/`，包括 Windows SCM 应用回调和 macOS AppKit 主线程循环的选择；共享的作用域、日志、应用依赖装配和命令分发由 `app/bootstrap.rs` 的 `entry()` 负责。

| 路径（相对 `crates/wlan-app/src`） | 维护范围 |
| --- | --- |
| `app.rs` | 应用模块声明、Clap 入口、编译期选择平台 bootstrap |
| `app/bootstrap.rs` | 平台模块声明、普通 Tokio runtime 的创建、错误记录与限时关闭；共享 `entry()` 初始化日志、作用域与应用依赖并分发命令 |
| `app/bootstrap/{linux,macos,windows}.rs` | 平台启动编排；Linux 普通入口、macOS AppKit 选择、Windows SCM 应用回调 |
| `app/context.rs` | 门户客户端、所选路径及 Wi-Fi / 凭据适配器的构建 |
| `app/cli.rs` | 参数声明、解析与命令分发；把 CLI 参数转换为 controller 请求 |
| `app/cli/login.rs` | stdin、交互选择、TUI 重试与安装时的凭据输入 |
| `app/cli/output.rs` | controller 结果的终端展示与状态格式化 |
| `app/tui.rs` | 终端生命周期、绘制、键盘事件和登录表单 |
| `app/controller/{login,logout,install,uninstall}.rs` | 业务校验、操作顺序与可复用接口，不读取终端输入 |
| `app/controller/daemon.rs`、`app/controller/daemon/` | `run`、`start`、`stop`、`restart`、`status`、`logs` 操作接口 |
| `app/daemon.rs` | daemon 生命周期、单实例、停机信号和本地控制初始化 |
| `app/daemon/runtime.rs` | 调度与 Wi-Fi / IPC 事件循环 |
| `app/daemon/check.rs` | 单次状态检查与认证，保护暂停状态和 Wi-Fi 会话 |
| `app/daemon/policy.rs` | 退避、轮询期限、认证拒绝与会话重置规则 |
| `app/daemon/credentials.rs` | 有超时限制、至多一个在途任务的后台凭据读取 |
| `infra/error.rs`、`infra/error/` | `thiserror` 错误、英文消息模板、门户错误转换 |
| `infra/logging.rs`、`infra/file_log.rs` | 日志格式与输出、文件日志历史和跟随读取 |
| `infra/config.rs`、`infra/paths.rs`、`infra/scope.rs` | 线路偏好、目录定位、user / system 范围和访问检查 |
| `infra/control.rs`、`infra/control/` | IPC 协议与传输、控制状态、单实例锁和操作锁 |
| `infra/command.rs`、`infra/shutdown.rs` | 可替换的外部命令执行、超时和进程停止信号 |
| `infra/filesystem.rs`、`infra/cleanup.rs` | 私有目录、文件写入及有边界的清理机制 |
| `infra/identity.rs` | 应用 ID、组织目录名、命令名和原生服务／凭据名称 |
| `system.rs` | 编译期选择原生平台，暴露系统能力入口 |
| `system/define.rs`、`system/define/` | credentials、services、wifi、permission 的公共契约与数据模型 |
| `system/shared/credentials.rs`、`system/shared/wifi.rs` | 凭据作用域与编码、共用 keyring 访问、Wi-Fi 查询超时和工作线程 |
| `system/shared/services/` | 安装文件与归属校验、安装回滚、卸载事务和启停控制 |
| `system/{linux,macos,windows}/services/` | 原生服务定义、管理器状态解析、日志来源；Windows 另含 SCM 进程适配 |
| `system/{linux,macos,windows}/credentials/` | 用户与系统凭据后端 |
| `system/{linux,macos,windows}/wifi/` | 平台 SSID 查询、连接标识、网络事件订阅 |
| `system/{linux,macos,windows}/permission.rs` | 平台权限声明、按需授权与状态诊断 |

依赖方向是 `app → system → infra`，`app` 也可直接使用 `infra`。`system` 和 `infra` 不得反向依赖 `app`；`infra` 不依赖 `system`。Clap、终端输入和展示属于应用前端，不能进入系统契约。原生 SDK 调用放在平台适配器内；通用的文件权限、进程身份和信号机制可在 `infra` 使用必要的条件编译。

`system/shared` 用于多个平台真正共用的流程，平台细节仍归对应目录。`wlan-api` 不依赖应用层；仅重构应用架构时不要顺带修改协议库。

## 增加或维护功能

### CLI 与业务操作

在 `app/cli.rs` 声明参数，再将普通请求类型传给 `app/controller`。controller 负责校验和执行，前端负责交互及展示；返回结构化数据，或显式注入输出 writer、取消 future 等接口。服务状态查询不能靠日志文本判断成功与否。TUI 不直接调用门户或服务管理器。

新增 daemon 行为先明确 `policy.rs` 中的状态转换，再接入单次检查或调度循环。保留持久化停止监听器、请求超时和单个后台凭据任务；Linux D-Bus 事件流必须持续排空。`AuthState::Unknown` 不得触发自动登录，手动登录只恢复相同作用域的 daemon。

### 平台适配

先在 `system/define/` 定义必要契约，再在平台目录实现。服务模板、管理器命令及状态解析分别归 `definition.rs`、`manager.rs`；共享安装器负责备份、回滚和停止后的清理顺序。构造服务计划与命令应能在不调用真实服务管理器的情况下测试。

新增平台后更新 `system.rs`、目标依赖、构建检查和 README。将运行验证与交叉编译结果分开记录。

应用 ID 固定为 `dev.cxcs.tools.wlanlogin`，目录采用 `cxcs-tools/wlanlogin` 两级结构；名称常量集中在 `infra/identity.rs`，路径通过 `infra/paths.rs` 构造。不要使用 `ProjectDirs::from` 推导这个布局：Linux 会忽略组织参数，macOS 会拼成点分名称；用户目录使用 `ProjectDirs::from_path`。macOS 的 Keychain 授权和服务安装必须共用包内程序路径。完整包的事务安装位于 `system/macos/services/`；严格限制可清理的包成员，新增打包资源时同步更新成员列表。用户与系统服务通过 `SMAppService` 注册，启动定义位于包内 `Contents/Library/LaunchAgents` / `LaunchDaemons`，使用 `BundleProgram`。定义在 xtask 签名前生成，安装期间不得改写签名包。原生 API 由安装副本中的私有命令调用，检查主 bundle 身份，返回带版本、作用域和应用 ID 的状态；不依赖外部 CLI 的位置。停止先 bootout，再通过带令牌的托管模式 IPC 等待进程退出；停止保留原生注册，卸载及替换签名包前需等待原生注销完成。等待用户后台批准不能报告为运行中，也不能阻塞卸载。保留旧外部 plist 的识别、迁移和失败恢复。手动前台 daemon 不接受托管停止指令；托管启动持锁后再次检查实际 launchd 注册。Windows 服务的创建、查询、启停和 SCM 回调注册使用同一个 ID，用户计划任务追加 SID。

改名不自动迁移旧安装，升级步骤见 README。清理只能删除应用自己的文件和清空后的 `wlanlogin` 目录，必须保留 `cxcs-tools` 父目录和同级工具；验证组织目录与应用目录的符号链接／reparse point 边界。Cargo 包为 `wlanlogin-cli` / `wlanlogin-lib`，可执行文件通过 `[[bin]]` 命名为 `wlanlogin`，日志 target 前缀也为 `wlanlogin`。

### 权限接口

`PermissionId` 使用开放的字符串命名空间；`PermissionDescriptor` 支持名称、可选描述和任意键值元数据。`PermissionProvider::permissions()` 返回动态列表，允许为空，不要固定为两个权限或在公共契约中枚举平台权限种类。

Linux / Windows 返回空列表。macOS 的 `macos.location` 对接签名 Wi-Fi 宿主中的 Core Location；`macos.keychain` 查询本范围的自有条目；查询禁用 UI，申请仅在前台允许 UI。用户与系统 Keychain 共用串行访问器，避免进程级交互开关竞态。诊断区分 Missing / NeedsInteraction / Denied / Unavailable；失败不得删除原条目。未知标识返回错误，保留“不支持 / 未实现”和“已授权”的区别。权限在前台操作需要时自动申请，不增加权限子命令。后台不触发授权弹窗。

包内外 CLI 共用参数解析，无参数不得隐式启动 Wi-Fi 宿主。宿主由前台操作或服务通过明确的内部参数启动。Keychain 保存、删除及 ACL 授权仅查找条目引用，不取密码数据；已有条目直接更新，仅明确不存在时创建，查询或更新失败不得尝试创建替代条目。新条目通过 `SecKeychainItemCreateFromContent` 在创建时传入初始访问控制，包含当前程序及已存在的目标程序。安装已明确要求刷新凭据时，直接进入登录准备，避免预检查重复读取钥匙串。

macOS Wi-Fi 宿主使用有长度限制的 IPC 帧、协议版本和请求 ID；客户端通过 socket 内核审计令牌验证对端签名与 UID，系统范围还检查活动控制台用户。宿主只提供网络状态，不接收校园网凭据。系统宿主在控制台用户改变后退出；启动、请求、空闲存活时间都有界限。交互申请启动不使用 `open -g`；GUI 宿主和用户后台 daemon 保持 AppKit 主线程事件循环，使前台状态能够刷新。Core Location 对象和 delegate 始终位于同一 RunLoop 工作线程；在该线程确认 `NSRunningApplication.isActive` 后才调用授权请求，激活和权限决定分别有超时。已有决定、后台查询及非交互操作均不得申请权限。

安装前后及完整应用包回滚的凭据检查由实际目标程序执行，禁用交互且仅输出带版本的权限状态；仅在回滚时兼容上版检查器以成功退出码证明静默读取、无标准输出的格式。先静默检查；已授权不得改 ACL；锁定时先解锁并重新检查；缺失、不可用、未知或子进程检查失败不得尝试重写。只有确需授权时才复制现有 `SecAccess`，扩展一条仅含 decrypt 的 ACL；无此条目则新增仅含 decrypt 的 ACL，不修改混合权限 ACL、所有者、partition ACL 或其他条目。再次静默检查成功才能继续安装或恢复原服务；未完成回滚保留恢复目录。launchd 定义必须包含 `AssociatedBundleIdentifiers` 与固定应用 ID，关联后台活动归属，继续保持 `open -W` 的生命周期语义。每次重新启动产生新的 Wi-Fi 会话标识，原生查询有超时。

### 错误与日志

应用代码使用 `infra::error::{Error, Result, Context, Message}`。新增稳定诊断文本在 `infra/error/message.rs` 用 `#[derive(thiserror::Error)]` 与 `#[error("...")]` 定义；用有意义的字段携带上下文。`anyhow` 仅在错误边界封装外部错误，不作为各模块的公共错误类型。

通过 `context` / `with_context` 保留错误来源；门户错误由 `infra/error/portal.rs` 转换为英文、安全的摘要，同时保留供重试分类使用的原始错误类型。当前协议库只在本地化错误文本中提供返回码，应用边界仅识别已知的标准说明与返回码组合，并用静态英文模板输出；无法识别时退回通用摘要。维护映射时用真实 API 加 loopback 响应验证，不要依赖任意门户文本。不要把门户原始响应、URL 查询参数或凭据写入日志。

日志使用 `tracing`，事件文本采用英文句式；使用 `error`、`scope`、`retry_after_secs` 等结构化字段。成功事件用 `info`，可恢复故障用 `warn`，终止或需人工处理的故障用 `error`，重复的跳过检查等诊断用 `debug`。daemon 的 `check.rs` 返回检查结果，由 `runtime.rs` 在首次检查、结果变化或错误恢复时用 `info` 输出 `outcome` 和 `next_check_secs`，并记录重试开始，避免静默跳过被误判为定时器失效。macOS 不能用同样受定位权限限制的 BSSID 判断 SSID 不可读时是否断网；已连接但名称不可读必须返回错误，保留重试及凭据保护。时间和格式由 `infra/logging.rs` 统一处理，业务模块不自行拼时间戳。中文 CLI / TUI 文案留在前端。

## 测试与验证

`wlan-app` 的测试一律放在所属文件末尾的 `#[cfg(test)] mod tests` 中；即使测试很长，也不要拆成 `tests.rs`、独立测试目录或 `#[path]` 引入的测试文件。共用假实现可放在相应基础模块的测试模块里，例如 `infra::command::tests`。协议库已有的集成测试和 fixtures 保持其现有组织。

优先覆盖业务约束：未知状态不登录、切换 SSID 后不提交凭据、回滚保留原安装、purge 只处理所选范围、取消能退出日志跟随或 daemon。不要只测试实现的逐行复刻。

开发验证不得访问真实校园门户、切换 Wi-Fi、读取真实密钥管理器或启停、安装、卸载实际服务。使用 loopback HTTP mock、临时目录、假凭据、模拟命令、私有 D-Bus 和隔离子进程。`cargo test` 需要本地 IPC 和子进程权限；沙箱拒绝这些权限时应在允许隔离测试的环境重跑，不能把环境拒绝当作通过，也不要改为调用真实系统服务。

从仓库根目录运行：

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --workspace
git diff --check
```

在已安装对应 target 的环境中进行跨平台静态检查：

```sh
rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin
cargo check -p wlanlogin-cli --locked --target x86_64-pc-windows-gnu
cargo check -p wlanlogin-cli --locked --target aarch64-apple-darwin
cargo check -p wlanlogin-cli --tests --locked --target x86_64-pc-windows-gnu
cargo check -p wlanlogin-cli --tests --locked --target aarch64-apple-darwin
```

这些命令检查编译，不链接或运行对应平台程序。验证 CLI 帮助可执行 `cargo run -p wlanlogin-cli -- --help`，不要将真实登录、注销或服务命令作为 smoke test。信号测试中的 `signal_child` 标记为 ignored，由父测试显式启动，不要用 `--include-ignored` 直接运行整个套件。

## 提交与审查

每完成一个可验证的功能就创建一个提交。使用 Conventional Commits，例如 `refactor(app): separate login controller from terminal input`、`fix(system): preserve service definition on rollback`、`docs: document contributor toolchain`。

提交前检查 staged diff，只包含本次功能；保留其他维护者未提交的修改。PR 描述说明具体行为变化、影响范围和已执行的验证。平台只有编译检查时明确写出，不把它描述为运行测试。命令、目录职责或运行行为变化时同步维护 README、本指南及英文 [AGENT.md](AGENT.md)。
