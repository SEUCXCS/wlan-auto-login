# WLAN Auto Login

面向东南大学成贤学院 `Student_CX` 校园 Wi-Fi 的 Rust 登录工具，支持 CLI、终端交互界面、系统凭据存储和后台自动登录。

- 登录、注销、状态识别与重复登录保护。
- Linux、macOS、Windows 原生 Wi-Fi 事件与服务管理适配。
- 用户与系统两种安装范围，各自保存配置、凭据和运行状态。
- 离线预配置、自动重试、服务日志查看与完整卸载。

本仓库是一个 Cargo workspace：`crates/wlan-api`（包名 `cxxy-wlanlogin-lib`）负责门户协议，`crates/wlan-app`（包名及可执行文件名 `cxxy-wlanlogin-cli`）负责应用与系统集成。

[构建](#构建) · [使用](#使用) · [后台自动登录](#后台自动登录) · [日志与排障](#日志与排障) · [项目结构](#项目结构) · [参与贡献](#参与贡献)

## 平台与依赖

| 平台 | Wi-Fi 识别 | 用户服务 / 系统服务 | 用户凭据 / 系统凭据 |
| --- | --- | --- | --- |
| Linux | NetworkManager、D-Bus | systemd user / systemd system | Secret Service / systemd 加密凭据 |
| macOS | CoreWLAN、SystemConfiguration | LaunchAgent / LaunchDaemon | 用户 Keychain / System Keychain |
| Windows | Native Wi-Fi API | Task Scheduler / Service Control Manager | Credential Manager / 机器 DPAPI |

Linux 构建需要 C 工具链、`pkg-config` 和 D-Bus 开发库；例如 Debian / Ubuntu 的 `build-essential pkg-config libdbus-1-dev`。运行需要 NetworkManager；用户凭据保存需要 Secret Service 实现（如 GNOME Keyring），系统范围默认需要支持 `systemd-creds` 的 systemd。macOS 构建需要 Xcode Command Line Tools；Windows 构建需要与所选 Rust target 匹配的工具链。

macOS 的 SSID 读取和 Keychain 访问受系统权限限制。项目已预留可扩展的 permission 接口，声明 Location 与 Keychain；当前状态查询和申请均返回 `NotImplemented`，尚未接入实际授权流程。Linux、Windows 的权限描述列表为空，不代表绕过已有的系统访问控制。

## 构建

安装 Rust 与 Cargo 后，在仓库根目录构建。项目使用 Rust 2024 edition，未固定工具链，也未声明 MSRV；不要把 edition 的最低版本当作整个依赖树的最低版本。本次重构在 Rust/Cargo `1.100.0-nightly` 上验证。

```sh
git clone https://github.com/SEUCXCS/wlan-auto-login.git
cd wlan-auto-login
cargo build --release --locked --workspace
```

生成的程序为 `target/release/cxxy-wlanlogin-cli`，Windows 下带 `.exe` 后缀。将程序放入 `PATH`，即可使用以下命令。

Linux 已通过本地测试和 release 构建；Windows GNU 与 macOS ARM64 已通过交叉编译检查，包括测试代码。交叉编译结果不代表对应平台的运行验证。

## 使用

```sh
cxxy-wlanlogin-cli login                         # 使用已保存的凭据，或打开交互界面
cxxy-wlanlogin-cli login --remember-me           # 登录并保存账号密码
cxxy-wlanlogin-cli login --interactive           # 修改账号、密码或线路
cxxy-wlanlogin-cli logout                       # 注销，并暂停当前连接的自动登录
cxxy-wlanlogin-cli logout --forget              # 注销、停止后台进程并删除所选范围的凭据
```

交互界面支持输入账号密码、选择线路和勾选 Remember me。Tab 切换控件，方向键选择线路，空格切换复选框，Esc 取消。

当前设备已认证在线时，`login` 只提示已在线，不重复提交凭据，也不声称本次提供的凭据或线路已通过认证。修改账号或线路需要先 `logout` 再登录；`--remember-me` 仍可保存凭据供后续使用。

登录前会检查 Wi-Fi：连接 `Student_CX` 时执行认证；不在该网络或无法识别 SSID 时，仅保存凭据，方便提前配置。用户凭据通过系统密钥管理器保存。

也可以通过参数或环境变量提供凭据，跳过交互界面：

| 参数 | 环境变量 | 说明 |
| --- | --- | --- |
| `--username` / `-u` | `CXXY_WLAN_USERNAME` | 账号 |
| `--password` / `-p` | `CXXY_WLAN_PASSWORD` | 密码；也可用 `--password-stdin` 从标准输入读取 |
| `--line` | `CXXY_WLAN_LINE` | `free1`（免费 1）、`free2`（免费 2）、`campus`（内网） |
| `--non-interactive` | — | 凭据不足时直接报错 |

自动化场景优先使用 `--password-stdin`；直接在命令行传入密码可能进入 shell 历史或进程参数列表。

## 后台自动登录

```sh
cxxy-wlanlogin-cli install          # 安装并启动后台服务
cxxy-wlanlogin-cli daemon status    # 查看安装与运行状态
cxxy-wlanlogin-cli daemon logs      # 查看最近 100 行服务日志
cxxy-wlanlogin-cli daemon logs -f   # 持续查看新日志，Ctrl+C 退出
cxxy-wlanlogin-cli daemon start     # 启动服务
cxxy-wlanlogin-cli daemon stop      # 停止服务，保留自动启动设置
cxxy-wlanlogin-cli daemon restart   # 重启服务
cxxy-wlanlogin-cli uninstall        # 卸载服务，保留已保存的凭据
cxxy-wlanlogin-cli uninstall --purge # 卸载服务并清理所选范围的应用数据和凭据
```

`daemon logs -n 200` 可指定历史日志行数，`-n 0 -f` 只查看新日志。新安装的 Linux 服务、macOS 与 Windows 服务均读取应用配置目录中的 `daemon.log`；旧 Linux 服务仍按已安装的服务定义读取 journal，重新执行 `install` 后改用独立日志文件。按 `--user` / `--system` 选择范围；读取系统日志可能需要 sudo 或管理员权限。未安装时会报错，服务停止后仍可查看已有日志。

`install` 缺少已保存凭据时会先进入登录流程，强制开启 Remember me，并在交互界面显示黄色提示。不在 `Student_CX` 时可先保存凭据并完成安装。

- **普通用户安装**：用户登录操作系统后启动。
- **sudo／管理员安装**：安装系统服务，随开机启动。Linux、macOS 使用 `sudo cxxy-wlanlogin-cli install`，Windows 使用管理员终端。

可用全局参数 `--user` / `--system` 指定范围；系统服务的登录、注销及启停也需要管理员权限。两个范围的凭据独立，切换安装范围前需先卸载原服务。系统凭据在 Linux 使用 `systemd-creds` 加密，在 macOS 使用 System Keychain，在 Windows 使用机器 DPAPI；Linux 系统安装需要支持加密凭据的 systemd。

Linux 旧系统可显式使用 `--credential-store protected-file`，例如 `sudo cxxy-wlanlogin-cli --system install --credential-store protected-file`。此模式以仅管理员可读的文件权限保护**明文**凭据；默认加密模式失败时不会自动降级。该选项仅用于系统范围，macOS / Windows 不支持此明文模式。

后台仅在连接 `Student_CX` 时访问门户，使用 `chkstatus` 的 `result=1` / `0` 判断在线 / 离线，确认离线后登录；未知状态不自动提交登录。确认在线后每 4 分钟检查一次，登录成功后先在 30 秒后确认状态；新 Wi-Fi 会话仍会立即触发检查。首次取得状态前遇到临时网络错误时，按 1、2、4 秒最多快速重试三次，之后进入 30、60、120 秒的常规退避，最多等待 2 分钟；重试到期直接触发，不再等待下一个检查周期。日志标明失败阶段、门户返回码含义和等待时间。手动注销后，手动登录或重新接入目标 Wi-Fi 可恢复自动登录。

直接运行 `cxxy-wlanlogin-cli daemon` 会在当前进程运行后台循环，需要事先在同一范围保存凭据。`start`、`stop`、`restart` 仅管理已安装服务，未安装时会报错。

更多参数见 `cxxy-wlanlogin-cli --help` 和各子命令的 `--help`。

## 日志与排障

应用日志使用英文事件文本、UTC RFC 3339 时间、日志级别、模块路径以及结构化字段，例如：

```text
2026-09-29T08:00:00Z INFO  cxxy_wlanlogin_cli::app::daemon::runtime: Daemon started scope="user" ssid="Student_CX"
```

默认级别为 `info`，可用 `RUST_LOG` 调整；前台运行时日志写入 stderr。以下设置仅作用于本次启动的进程，已安装服务需要在服务环境中配置：

```sh
RUST_LOG=cxxy_wlanlogin_cli=debug cxxy-wlanlogin-cli --user daemon
```

后台日志写入所选范围配置目录下的 `daemon.log`。程序启动时若已有日志超过 2 MiB，会清空该文件；这不是持续轮转或历史归档。门户错误会转换为英文诊断并保留错误分类；可识别的已知返回码保留英文说明，未知、脱敏或截断后无法识别的内容使用通用摘要，不输出原始门户响应或凭据。CLI 帮助和 TUI 保留中文，底层操作系统错误可能使用系统语言。

| 现象 | 检查方式 |
| --- | --- |
| 仅保存凭据，没有认证 | 确认已连接 `Student_CX`，并检查 SSID 读取权限 |
| 密钥管理器不可用 | 解锁用户密钥管理器；后台不会主动弹出交互提示 |
| 已在线，但新账号未生效 | 先注销再登录；已在线时不会重新提交凭据 |
| 服务未安装或安装不完整 | 用同一范围执行 `daemon status`，必要时重新 `install` |
| 显式登录后 daemon 没有恢复 | 确认两者使用相同的 `--user` / `--system` 范围 |
| 自动重试暂停 | 更新被拒绝的凭据：`login --interactive --remember-me` |

报告问题时提供平台、版本、操作范围和已脱敏的日志。

## 完全登出与卸载

`logout --forget` 会暂停自动登录、停止该范围的后台进程，并删除保存的账号密码。未连接 `Student_CX` 时只清理本地凭据；门户请求失败或无法识别 Wi-Fi 时仍尝试删除凭据，但会以非零退出码说明门户注销未完成。凭据删除失败同样会报错，自动登录保持暂停。此选项保留服务安装和线路配置；之后可通过 `login --remember-me` 保存新凭据，再用 `daemon start` 恢复后台运行。

`uninstall --purge` 会移除服务注册、服务定义文件、安装的后台程序副本，以及应用管理的日志、配置、运行文件、凭据和清空后的应用目录。服务文件丢失但注册仍残留时也会尝试卸载；停止或注销服务失败时不会继续删除数据，凭据删除失败会保留剩余文件并报告未完成，可修复原因后重试。

两个选项都只作用于所选的 `--user` 或 `--system` 范围，不跨用户清理。普通终端默认使用用户范围，sudo／管理员终端默认使用系统范围。例如：

```sh
cxxy-wlanlogin-cli --user logout --forget
cxxy-wlanlogin-cli --user uninstall --purge
sudo cxxy-wlanlogin-cli --system uninstall --purge  # Linux / macOS 系统范围
```

Windows 系统范围在管理员终端运行 `cxxy-wlanlogin-cli --system uninstall --purge`。

`--purge` 清理本程序管理的文件，保留原始 CLI 文件和目录中的其他文件。旧 Linux 共享 journal 中的历史日志，以及 Windows/macOS/Linux 的系统服务审计记录，无法作为应用文件单独删除，仍由操作系统的日志保留策略管理；本工具不会清空全系统日志。

## 协议文档

- [状态查询（chkstatus）](docs/eportal-status.md)：设备信息、认证状态、后台检查和重试行为。
- [登录](docs/eportal-login.md)
- [注销](docs/eportal-logout.md)

## 项目结构

```text
crates/
├── wlan-api/                     # 门户协议库
└── wlan-app/
    ├── Cargo.toml
    └── src/
        ├── main.rs              # 最小程序入口
        ├── app.rs               # 应用初始化与依赖装配
        ├── app/
        │   ├── cli.rs           # Clap 参数、命令分发
        │   ├── cli/             # 终端输入与结果展示
        │   ├── tui.rs           # 交互表单
        │   ├── context.rs       # 应用依赖
        │   ├── controller.rs
        │   ├── controller/      # login、logout、install、uninstall、daemon/*
        │   ├── daemon.rs
        │   └── daemon/          # 调度、单次检查、凭据后台读取、重试策略
        ├── infra.rs
        ├── infra/               # 错误、日志、配置、路径、IPC、锁、文件与进程
        ├── system.rs
        └── system/
            ├── define.rs
            ├── define/          # credentials、services、wifi、permission 契约
            ├── shared.rs
            ├── shared/          # 共用适配与服务安装/卸载事务
            ├── linux/           # mod.rs、services/、credentials/、wifi/、permission.rs
            ├── macos/           # 同上
            └── windows/         # 同上；services/runtime.rs 对接 SCM
```

`app` 组织业务操作，`system` 提供平台能力，`infra` 提供通用机制；后两层不依赖 `app`。`wlan-app` 的单元测试均写在所属源文件末尾的 `#[cfg(test)] mod tests` 中。详细职责、扩展方式与验证流程见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 开发验证

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --workspace
```

测试使用 loopback 模拟门户、临时目录、模拟服务命令和隔离 IPC。部分测试需要本地 socket、D-Bus 与子进程权限；Linux 测试环境还需要 `dbus-daemon`。开发验证不连接真实校园门户、不切换 Wi-Fi、不操作已安装服务或真实凭据。

## 参与贡献

问题与建议请提交到 [Issues](https://github.com/SEUCXCS/wlan-auto-login/issues)，代码贡献请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。自动化编码代理的工具链和工作约定见 [AGENT.md](AGENT.md)。每个完整功能单独提交，提交信息遵循 Conventional Commits。

## 许可证

仓库目前未附带 `LICENSE` 文件，尚未声明开源许可证。
