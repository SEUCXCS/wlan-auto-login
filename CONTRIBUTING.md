# 贡献指南

感谢参与 WLAN Auto Login 的维护。提交问题时请说明操作系统、程序版本、安装范围、复现步骤和预期行为；日志和协议样本请先去除账号、密码、令牌及其他个人信息。

## 开发环境

项目使用 Cargo workspace、Rust 2024 edition 和 resolver 3：

| 目录 | Cargo 包名 | 职责 |
| --- | --- | --- |
| `crates/wlan-api` | `cxxy-wlanlogin-lib` | 门户请求、JSONP 解析、认证状态和协议错误 |
| `crates/wlan-app` | `cxxy-wlanlogin-cli` | CLI / TUI、业务操作、daemon 与操作系统集成 |
| `docs` | — | 登录、注销、状态查询的协议依据 |

安装 Rust、Cargo、rustfmt 和 Clippy，并提交应用依赖的 `Cargo.lock`。仓库没有 `rust-toolchain.toml`，也没有声明 MSRV；本次重构使用 Rust/Cargo `1.100.0-nightly` 验证，这不是强制 nightly 的声明。若要建立稳定工具链或最低版本要求，需要单独验证整个依赖树并更新文档。

Linux 需要 C 工具链、`pkg-config`、D-Bus 开发库，测试还需要 `dbus-daemon` 及常见 POSIX 工具。Debian / Ubuntu 可安装 `build-essential pkg-config libdbus-1-dev dbus-daemon`。macOS 使用 Xcode Command Line Tools；Windows 使用与 Rust target 对应的 MSVC 或 GNU 构建工具。

## 模块职责

`main.rs` 只调用 `app::run()`。不要向入口增加认证、服务管理或平台分支。

| 路径（相对 `crates/wlan-app/src`） | 维护范围 |
| --- | --- |
| `app.rs` | Clap 入口、Tokio runtime、日志初始化、作用域与应用依赖装配；Windows 服务回调 |
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
| `system.rs` | 编译期选择原生平台，暴露系统能力入口 |
| `system/define.rs`、`system/define/` | credentials、services、wifi、permission 的公共契约与数据模型 |
| `system/shared/credentials.rs`、`system/shared/wifi.rs` | 凭据作用域与编码、共用 keyring 访问、Wi-Fi 查询超时和工作线程 |
| `system/shared/services/` | 安装文件与归属校验、安装回滚、卸载事务和启停控制 |
| `system/{linux,macos,windows}/services/` | 原生服务定义、管理器状态解析、日志来源；Windows 另含 SCM 进程适配 |
| `system/{linux,macos,windows}/credentials/` | 用户与系统凭据后端 |
| `system/{linux,macos,windows}/wifi/` | 平台 SSID 查询、连接标识、网络事件订阅 |
| `system/{linux,macos,windows}/permission.rs` | 平台权限声明与未来授权适配入口 |

依赖方向是 `app → system → infra`，`app` 也可直接使用 `infra`。`system` 和 `infra` 不得反向依赖 `app`；`infra` 不依赖 `system`。Clap、终端输入和展示属于应用前端，不能进入系统契约。原生 SDK 调用放在平台适配器内；通用的文件权限、进程身份和信号机制可在 `infra` 使用必要的条件编译。

`system/shared` 用于多个平台真正共用的流程，平台细节仍归对应目录。`wlan-api` 不依赖应用层；仅重构应用架构时不要顺带修改协议库。

## 增加或维护功能

### CLI 与业务操作

在 `app/cli.rs` 声明参数，再将普通请求类型传给 `app/controller`。controller 负责校验和执行，前端负责交互及展示；返回结构化数据，或显式注入输出 writer、取消 future 等接口。服务状态查询不能靠日志文本判断成功与否。TUI 不直接调用门户或服务管理器。

新增 daemon 行为先明确 `policy.rs` 中的状态转换，再接入单次检查或调度循环。保留持久化停止监听器、请求超时和单个后台凭据任务；Linux D-Bus 事件流必须持续排空。`AuthState::Unknown` 不得触发自动登录，手动登录只恢复相同作用域的 daemon。

### 平台适配

先在 `system/define/` 定义必要契约，再在平台目录实现。服务模板、管理器命令及状态解析分别归 `definition.rs`、`manager.rs`；共享安装器负责备份、回滚和停止后的清理顺序。构造服务计划与命令应能在不调用真实服务管理器的情况下测试。

新增平台后更新 `system.rs`、目标依赖、构建检查和 README。将运行验证与交叉编译结果分开记录。

### 权限接口

`PermissionId` 使用开放的字符串命名空间；`PermissionDescriptor` 支持名称、可选描述和任意键值元数据。`PermissionProvider::permissions()` 返回动态列表，允许为空，不要固定为两个权限或在公共契约中枚举平台权限种类。

当前 Linux / Windows 返回空列表，macOS 仅声明 `macos.location` 与 `macos.keychain`。已声明权限的 `status()` / `request()` 返回 `NotImplemented`，未知标识返回错误。占位实现不请求授权、不报告已授权，也未接入登录或安装流程。实际授权能力应作为独立功能实现，并保留“不支持 / 未实现”和“已授权”的区别。

### 错误与日志

应用代码使用 `infra::error::{Error, Result, Context, Message}`。新增稳定诊断文本在 `infra/error/message.rs` 用 `#[derive(thiserror::Error)]` 与 `#[error("...")]` 定义；用有意义的字段携带上下文。`anyhow` 仅在错误边界封装外部错误，不作为各模块的公共错误类型。

通过 `context` / `with_context` 保留错误来源；门户错误由 `infra/error/portal.rs` 转换为英文、安全的摘要，同时保留供重试分类使用的原始错误类型。当前协议库只在本地化错误文本中提供返回码，应用边界仅识别已知的标准说明与返回码组合，并用静态英文模板输出；无法识别时退回通用摘要。维护映射时用真实 API 加 loopback 响应验证，不要依赖任意门户文本。不要把门户原始响应、URL 查询参数或凭据写入日志。

日志使用 `tracing`，事件文本采用英文句式；使用 `error`、`scope`、`retry_after_secs` 等结构化字段。成功事件用 `info`，可恢复故障用 `warn`，终止或需人工处理的故障用 `error`，跳过检查等诊断用 `debug`。时间和格式由 `infra/logging.rs` 统一处理，业务模块不自行拼时间戳。中文 CLI / TUI 文案留在前端。

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
cargo check -p cxxy-wlanlogin-cli --locked --target x86_64-pc-windows-gnu
cargo check -p cxxy-wlanlogin-cli --locked --target aarch64-apple-darwin
cargo check -p cxxy-wlanlogin-cli --tests --locked --target x86_64-pc-windows-gnu
cargo check -p cxxy-wlanlogin-cli --tests --locked --target aarch64-apple-darwin
```

这些命令检查编译，不链接或运行对应平台程序。验证 CLI 帮助可执行 `cargo run -p cxxy-wlanlogin-cli -- --help`，不要将真实登录、注销或服务命令作为 smoke test。信号测试中的 `signal_child` 标记为 ignored，由父测试显式启动，不要用 `--include-ignored` 直接运行整个套件。

## 提交与审查

每完成一个可验证的功能就创建一个提交。使用 Conventional Commits，例如 `refactor(app): separate login controller from terminal input`、`fix(system): preserve service definition on rollback`、`docs: document contributor toolchain`。

提交前检查 staged diff，只包含本次功能；保留其他维护者未提交的修改。PR 描述说明具体行为变化、影响范围和已执行的验证。平台只有编译检查时明确写出，不把它描述为运行测试。命令、目录职责或运行行为变化时同步维护 README、本指南及英文 [AGENT.md](AGENT.md)。
