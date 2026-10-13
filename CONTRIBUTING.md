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

图标源文件为 `assets/icon/icon.svg`。

`APP_NAME` 是命令、二进制和目录名 `wlanlogin`；`DISPLAY_NAME` 是显示名称 `CXCS WLAN Login`。macOS 的 `CFBundleName` / `CFBundleDisplayName`、Windows 服务显示名及任务描述、Linux 的 systemd 描述使用后者。

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
| `app/controller/{login,logout}.rs` | 业务校验、操作顺序与可复用接口，不读取终端输入 |
| `app/controller/daemon.rs` | 当前进程的自动登录循环、托管服务启动准备 |
| `app/controller/service.rs`、`app/controller/service/` | `install`、`uninstall`、`start`、`stop`、`restart`、`status`、`logs` 服务操作接口 |
| `app/controller/permission.rs`、`app/cli/permission.rs` | 通用权限查询／请求，以及 macOS CLI 输出 |
| `app/daemon.rs` | daemon 生命周期、单实例、停机信号和本地控制初始化 |
| `app/daemon/runtime.rs` | 调度与 Wi-Fi / IPC 事件循环 |
| `app/daemon/check.rs` | 单次状态检查与认证，保护暂停状态和 Wi-Fi 会话 |
| `app/daemon/policy.rs` | 退避、轮询期限、认证拒绝与会话重置规则 |
| `app/daemon/credentials.rs` | 有超时限制、至多一个在途任务的后台凭据读取和内存会话提示 |
| `infra/error.rs`、`infra/error/` | `thiserror` 错误、英文消息模板、门户错误转换 |
| `infra/logging.rs`、`infra/file_log.rs` | 日志格式与输出、文件日志历史和跟随读取 |
| `infra/config.rs`、`infra/paths.rs`、`infra/scope.rs` | 线路偏好、目录定位、user / system 范围和访问检查 |
| `infra/control.rs`、`infra/control/` | IPC 协议与传输、控制状态、单实例锁和操作锁 |
| `infra/command.rs`、`infra/shutdown.rs` | 可替换的外部命令执行、超时和进程停止信号 |
| `infra/filesystem.rs`、`infra/cleanup.rs` | 私有目录、文件写入及有边界的清理机制 |
| `infra/identity.rs` | 应用 ID、组织目录名、命令名和原生服务／凭据名称 |
| `system.rs` | 编译期选择原生平台，暴露系统能力入口 |
| `system/define/` | credentials、services、wifi、permission 的公共契约与数据模型，包括安装报告、文件状态和日志后端接口 |
| `system/shared/credentials/`、`system/shared/wifi/` | 凭据作用域与编码、共用 keyring 访问、Wi-Fi 查询超时和工作线程 |
| `system/shared/services/` | 归属检查实现、共用日志适配、服务步骤、卸载扩展点和回滚停止屏障 |
| `system/{linux,macos,windows}/services/` | 原生服务定义、管理器状态解析、日志来源、签名包和就绪检查 |
| `system/{linux,macos,windows}/credentials/` | 用户与系统凭据后端 |
| `system/{linux,macos,windows}/wifi/` | 平台 SSID 查询、连接标识、网络事件订阅 |
| `system/{linux,macos,windows}/permission/` | 平台权限声明、按需授权与状态诊断 |

| `system/{linux,macos,windows}/flow/` | 各平台 install、uninstall、start、stop、restart、status、logs、run 与 credentials、wifi、permission 操作的统一入口 |
| `system/shared/flow/` | 公共流程；平台没有额外步骤时显式复用，操作不能无条件返回成功 |
| `system/{linux,macos,windows}/runtime/` | Windows SCM、macOS AppKit／认证宿主、Linux 会话就绪／凭据事件 |

依赖方向是 `app → system → infra`，`app` 也可直接使用 `infra`。`system` 和 `infra` 不得反向依赖 `app`；`infra` 不依赖 `system`。Clap、终端输入和展示属于应用前端，不能进入系统契约。原生 SDK 调用放在平台适配器内；通用的文件权限、进程身份和信号机制可在 `infra` 使用必要的条件编译。

`system/shared` 用于多个平台真正共用的流程，平台细节仍归对应目录。`wlan-api` 不依赖应用层；仅重构应用架构时不要顺带修改协议库。

## 增加或维护功能

### CLI 与业务操作

在 `app/cli.rs` 声明参数，再将普通请求类型传给 `app/controller`。公开服务管理统一使用 `service install/uninstall/start/stop/restart/status/logs`；`daemon` 只运行当前进程的循环。各平台生成的服务定义使用隐藏 `service run`；Windows 系统范围由 bootstrap 接入 SCM，用户计划任务直接运行，macOS 用户范围保留 AppKit 事件循环。macOS bootstrap 处理 `service native` 和 Wi-Fi 宿主内部入口，共享 bootstrap 在构造门户客户端前处理跨平台权限入口。拒绝旧服务启动参数与旧协议，升级时用旧程序卸载，再安装当前版本。服务 controller 的生产操作统一调用 `system::flow`，不直接引用 shared 或某个原生平台；平台 flow 组合公共流程与原生适配器，前端负责交互及展示；返回结构化数据，或显式注入输出 writer、取消 future 等接口。服务状态查询不能靠日志文本判断成功与否。TUI 不直接调用门户或服务管理器。

新增 daemon 行为先明确 `policy.rs` 中的状态转换，再接入单次检查或调度循环。保留持久化停止监听器、请求超时和单个后台凭据任务；Linux D-Bus 事件流必须持续排空。`AuthState::Unknown` 不得触发自动登录，手动登录只恢复相同作用域的 daemon。

### 平台适配

先在 `system/define/` 定义必要契约，再在平台目录实现。服务模板、管理器命令及状态解析分别归 `definition.rs`、`manager.rs`；共享安装器负责备份、回滚和停止后的清理顺序。构造服务计划与命令应能在不调用真实服务管理器的情况下测试。

新增平台后更新 `system.rs`、目标依赖、构建检查和 README。将运行验证与交叉编译结果分开记录。

应用 ID 固定为 `dev.cxcs.tools.wlanlogin`，目录采用 `SEUCXCS/wlanlogin` 两级结构；名称常量集中在 `infra/identity.rs`，路径通过 `infra/paths.rs` 构造。不要使用 `ProjectDirs::from` 推导这个布局：Linux 会忽略组织参数，macOS 会拼成点分名称；用户目录使用 `ProjectDirs::from_path`。macOS 的 Keychain 授权和服务安装必须共用包内程序路径。完整包安装入口位于 `system/macos/flow/install.rs`，签名负载事务位于 `system/macos/services/payload.rs`；严格限制可清理的包成员，新增打包资源时同步更新成员列表。用户与系统服务通过 `SMAppService` 注册，启动定义位于包内 `Contents/Library/LaunchAgents` / `LaunchDaemons`，使用 `BundleProgram`。定义在 xtask 签名前生成，安装期间不得改写签名包。原生 API 由安装副本中的隐藏 `service native status/register/unregister` 调用，检查主 bundle 身份，返回带版本、作用域和应用 ID 的状态；不依赖外部 CLI 的位置。停止先 bootout，再通过带令牌的托管模式 IPC 等待进程退出；停止保留原生注册，卸载及替换签名包前需等待原生注销完成。等待用户后台批准不能报告为运行中，也不能阻塞卸载。旧外部 plist 和旧签名启动协议必须在修改之前报错，不自动迁移或回退；当前协议之间保留回滚。手动前台 daemon 不接受托管停止指令；托管启动持锁后再次检查实际 launchd 注册。Windows 服务的创建、查询、启停和 SCM 回调注册使用同一个 ID，用户计划任务追加 SID。

改名不自动迁移旧安装，升级步骤见 README。清理只能删除应用自己的文件和清空后的 `wlanlogin` 目录，必须保留 `SEUCXCS` 父目录和同级工具；验证组织目录与应用目录的符号链接／reparse point 边界。Cargo 包为 `wlanlogin-cli` / `wlanlogin-lib`，可执行文件通过 `[[bin]]` 命名为 `wlanlogin`，日志 target 前缀也为 `wlanlogin`。

### 权限接口

`PermissionId` 使用开放的字符串命名空间；`PermissionDescriptor` 支持名称、可选描述和任意键值元数据。`PermissionProvider::permissions()` 返回动态列表，允许为空，不要固定为两个权限或在公共契约中枚举平台权限种类。

Linux / Windows 返回空列表。macOS 的 `macos.location` 对接签名 Wi-Fi 宿主中的 Core Location；`macos.keychain` 查询本范围的自有条目；查询禁用 UI，申请仅在前台允许 UI。用户与系统 Keychain 共用串行访问器，避免进程级交互开关竞态。诊断区分 Missing / NeedsInteraction / Denied / Unavailable；失败不得删除原条目。未知标识返回错误，保留“不支持 / 未实现”和“已授权”的区别。`permission list/check/get` 在所有平台编译并显示，由运行时 provider 决定支持的权限；空实现的列表和全部查询返回空集合，显式 ID 仍需校验。查询不申请授权，`get` 仅在 stdin 和 stderr 均为终端时请求指定权限。逐项保留诊断，某一权限失败不能丢失其他结果。状态 JSON 使用带版本、范围和权限 ID 的 `PermissionReport`，安装端严格检查版本、范围、ID、条目数及错误字段。后台不触发授权弹窗。

包内外 CLI 共用参数解析，无参数不得隐式启动 Wi-Fi 宿主。macOS 宿主使用隐藏的 `daemon --internal-wifi-host`，仅运行 Wi-Fi／定位宿主，不进入自动登录循环；隐藏的 `--system-helper` 仅能与该模式组合，选择服务于系统客户端的桌面用户宿主。删除旧 `--background` / `--service` 参数，内部字段上方保留用途注释。macOS bootstrap 在构造门户依赖之前处理此模式并保持 AppKit 主线程循环。启动所选包前通过 `--help` 验证当前参数；验证失败时报不支持，实际启动失败不重试另一种协议。Keychain 保存、删除及 ACL 授权仅查找条目引用，不取密码数据；已有条目直接更新，仅明确不存在时创建，查询或更新失败不得尝试创建替代条目。新条目通过 `SecKeychainItemCreateFromContent` 在创建时传入初始访问控制，包含当前程序及已存在的目标程序。安装已明确要求刷新凭据时，直接进入登录准备，避免预检查重复读取钥匙串。

macOS Wi-Fi 宿主使用有长度限制的 IPC 帧、协议版本和请求 ID；客户端通过 socket 内核审计令牌验证对端签名与 UID，系统范围还检查活动控制台用户。宿主只提供网络状态，不接收校园网凭据。系统宿主在控制台用户改变后退出；启动、请求、空闲存活时间都有界限。交互申请启动不使用 `open -g`；GUI 宿主和用户后台 daemon 保持 AppKit 主线程事件循环，使前台状态能够刷新。Core Location 对象和 delegate 始终位于同一 RunLoop 工作线程；在该线程确认 `NSRunningApplication.isActive` 后才调用授权请求，激活和权限决定分别有超时。已有决定、后台查询及非交互操作均不得申请权限。

安装前后及当前协议应用包回滚的凭据检查均由实际目标程序执行 `permission check macos.keychain --json`；只接受当前带版本、范围和权限 ID 的报告，拒绝旧检查命令和空输出成功格式。先静默检查；已授权不得改 ACL；锁定时先解锁并重新检查；缺失、不可用、未知或子进程检查失败不得尝试重写。只有确需授权时才复制现有 `SecAccess`，扩展一条仅含 decrypt 的 ACL；无此条目则新增仅含 decrypt 的 ACL，不修改混合权限 ACL、所有者、partition ACL 或其他条目。再次静默检查成功才能继续安装或恢复原服务；未完成回滚保留恢复目录。launchd 定义必须包含 `AssociatedBundleIdentifiers` 与固定应用 ID，关联后台活动归属，服务生命周期由原生 SMAppService 托管。每次重新启动产生新的 Wi-Fi 会话标识，原生查询有超时。

### Linux 后台凭据时序

`define/credentials.rs` 定义存储契约和 Available / Missing / Waiting / Failed 状态；`shared/credentials/codec.rs` 实现持久化格式，`shared/credentials/mod.rs` 选择作用域后端。`flow/credentials` 是操作入口。查询流程属于 shared/flow，define 不应读取文件、查询网络或启动服务。

daemon 不得在启动时预读。单次检查先验证目标 Wi-Fi、暂停、策略期限及门户 Offline，再读取凭据，提交前再次验证 Wi-Fi 会话。成功读取返回 `BackgroundCredentials`，包含不可输出的 `CredentialPermit`；所有完成结果（包括超时后缓存的结果）在消费时复检，Wi-Fi await 后再次复检，任一检查失败都不得调用门户登录。校验有独立超时，正常等待和校验失败不进入网络失败退避；提供者／解锁事件只唤醒等待，不解除门户凭据拒绝。保留单个在途原生读任务，超时、注销或许可失效不得另开第二个任务。

`CredentialPermit` 绑定 scope、配置目录及 `control.json` 中独立的 `credential_generation`。`flow/credentials/save.rs` 和 `delete.rs` 在调用原生后端之前递增并原子保存代数，调用方继续持有现有 operation lock；写入失效记录失败必须阻止存储操作，原生操作失败也不得恢复旧代数。凭据代数不复用暂停／调度的 revision。缺失字段初始化为零；凭据格式及 IPC v3 不变，没有旧协议回退。

Linux 用户许可另外保存 logind 会话 ID、`TimestampMonotonic` 及内存中的会话提示／代数。每个消费检查点在实时查询前后验证会话代数及完整身份，避免注销、重新登录和提示恢复原值时使用旧结果。Linux 系统范围及 macOS／Windows 不增加图形会话门槛，继续按原生后台访问规则读取，只应用共同的存储新鲜度检查。

Linux 用户后台先检查 logind 活动会话、显示 socket 和用户 systemd 环境，再同步真正的 D-Bus 激活环境，最后使用官方 `dbus-secret-service` 的 DH、零 prompt 超时连接标准 `org.freedesktop.secrets`。禁止启动时预读；已经运行的 provider 也经过同样门槛。就绪后允许标准 D-Bus 激活，不修改依赖源码、不固定唯一 owner、不猜测桌面或钱包实现。`default.target` 保持不变。session-ready/end 是自定义桌面的通用可选入口，提示只保存在内存并绑定实际会话；跨越注销／重新登录的读取结果不得用于提交。系统存储不依赖图形会话。

凭据事件通过独立持续排空的 D-Bus 流合并，断开后重建，定时查询作为后备。控制 IPC 只接受版本 3 的有界 JSON 帧，验证范围与令牌并确认通知；状态包含安全的诊断类别、阶段、数值原生码和已知 D-Bus 错误码，不能包含凭据、原始错误自由文本或触发 Keyring。状态查询失败明确显示未知，不伪装成未读取。旧版本不回退。

类型化凭据错误定义于 `infra/error/credentials.rs`，保留错误链并使用安全的 Display/Debug；`shared/credentials/diagnostics.rs` 负责分类。正常锁定／前台交互／拒绝访问／缺少 provider 使用 Waiting，超时、格式、版本、重复条目、解密、存储校验及任务失败使用 Failed；未知错误保持中性类别。D-Bus NoReply 属于超时，会话查询失败保留 Session 阶段。仅在诊断变化或恢复时输出日志；凭据等待和失败都保持正常轮询，不进入门户网络失败退避或解除真实的凭据拒绝。

私有 D-Bus 回归用隔离子进程为官方库设置环境，禁止并行测试修改当前进程的环境。`provider_child` / `read_child` 是 ignored 辅助用例，由父测试显式调用并验证地址属于临时总线；不得直接运行整个套件的 ignored 用例。

### 错误与日志

应用代码使用 `infra::error::{Error, Result, Context, Message}`。通用稳定诊断文本在 `infra/error/message.rs`、凭据诊断模板在 `infra/error/credentials.rs` 用 `#[derive(thiserror::Error)]` 与 `#[error("...")]` 定义；用有意义的字段携带上下文。`anyhow` 仅在错误边界封装外部错误，不作为各模块的公共错误类型。

通过 `context` / `with_context` 保留错误来源；门户错误由 `infra/error/portal.rs` 转换为英文、安全的摘要，同时保留供重试分类使用的原始错误类型。当前协议库只在本地化错误文本中提供返回码，应用边界仅识别已知的标准说明与返回码组合，并用静态英文模板输出；无法识别时退回通用摘要。维护映射时用真实 API 加 loopback 响应验证，不要依赖任意门户文本。不要把门户原始响应、URL 查询参数或凭据写入日志。

日志使用 `tracing`，事件文本采用英文句式；使用 `error`、`scope`、`retry_after_secs` 等结构化字段。成功事件用 `info`，可恢复故障用 `warn`，终止或需人工处理的故障用 `error`，重复的跳过检查等诊断用 `debug`。daemon 的 `check.rs` 返回检查结果，由 `runtime.rs` 在首次检查、结果变化或错误恢复时用 `info` 输出 `outcome` 和 `next_check_secs`，并记录重试开始，避免静默跳过被误判为定时器失效。macOS 不能用同样受定位权限限制的 BSSID 判断 SSID 不可读时是否断网；已连接但名称不可读必须返回错误，保留重试及凭据保护。时间和格式由 `infra/logging.rs` 统一处理，业务模块不自行拼时间戳。中文 CLI / TUI 文案留在前端。

## 测试与验证

`wlan-app` 的测试一律放在所属文件末尾的 `#[cfg(test)] mod tests` 中；即使测试很长，也不要拆成 `tests.rs`、独立测试目录或 `#[path]` 引入的测试文件。共用假实现可放在相应基础模块的测试模块里，例如 `infra::command::tests`。协议库已有的集成测试和 fixtures 保持其现有组织。

优先覆盖业务约束：未知状态不登录、切换 SSID 后不提交凭据、回滚保留原安装、purge 只处理所选范围、取消能退出日志跟随或 daemon。不要只测试实现的逐行复刻。

后台结果有效性测试必须覆盖：读取超时后完成再注销、保持单任务、会话 ID 复用／提示 ABA、保存删除的预先失效和失败路径、Wi-Fi await 后的最后校验。Linux 的就绪及身份查询通过私有 `dbus-daemon` 上的 systemd/logind 假服务验证，不访问宿主总线。编译检查不等于 Windows/macOS 原生测试；私有 D-Bus 测试也不能替代真实桌面登录竞争测试。

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
