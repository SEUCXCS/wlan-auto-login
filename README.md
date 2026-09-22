# cxxy-wlanlogin

使用 Cargo workspace 组织的 Student_CX 校园网登录工具。

- `crates/wlan-api`：异步 ePortal API 库，封装状态查询、登录、注销。
- `crates/wlan-app`：基于 Tokio 的 `cxxy-wlanlogin-cli`，包含 CLI、TUI、daemon，以及用户和系统服务管理。

## 构建

```sh
cargo build --release --locked --workspace
```

程序位于 `target/release/cxxy-wlanlogin-cli`，Windows 下带 `.exe` 后缀。普通构建使用当前 Rust 工具链；Windows 7 请使用下方的专用 target。

Linux 构建需要 `pkg-config` 和 D-Bus 开发库，例如 Debian/Ubuntu 的 `libdbus-1-dev`。自动登录需要 NetworkManager；用户安装保存凭据需要提供 Secret Service 接口的密钥管理器，例如 GNOME Keyring 或启用 Secret Service 的 KWallet。

## 登录与注销

```sh
cxxy-wlanlogin-cli login
cxxy-wlanlogin-cli login --remember-me
cxxy-wlanlogin-cli login --interactive --remember-me
cxxy-wlanlogin-cli logout
```

`login` 先检查当前 Wi-Fi，再准备凭据：优先使用参数，其次使用环境变量，然后补充所选安装范围中保存的凭据。账号和密码齐全时直接提交；信息不足且运行于交互终端时打开 TUI。显式切换账号后不会复用另一个账号的密码。

提交前会再次检查 Wi-Fi。只有确认连接到 `Student_CX` 才请求校园网接口；未连接目标 Wi-Fi 或无法识别 SSID 时，**仅将账号和密码保存到所选范围的凭据存储，不发送状态查询或认证请求**，并提示凭据尚未在线验证。因此可以在校外预先配置。离线 TUI 的 Remember me 强制开启，以黄色提示说明原因；输入期间离开目标 Wi-Fi 也会转为仅保存凭据。

`--interactive`（别名 `--tui`）强制打开 TUI，可用于更改已保存的账号、密码或线路。Tab/Shift+Tab 切换控件，方向键选择线路，空格切换 Remember me，点击登录按钮或聚焦按钮后按 Enter 提交；Esc/Ctrl+C 取消。密码以掩码显示。

连接到 `Student_CX` 时，只有显式使用 `--remember-me` 或勾选复选框，才在登录成功后保存账号和密码（安装流程除外）。在线登录未勾选时不会覆盖已有凭据；离线配置始终保存二者。用户密钥库不可用时不回退到明文密码文件。系统凭据的存储方式见下文。

| 参数 | 环境变量 | 说明 |
| --- | --- | --- |
| `--username` / `-u` | `CXXY_WLAN_USERNAME` | 学号，不包含运营商后缀 |
| `--password` / `-p` | `CXXY_WLAN_PASSWORD` | 校园网密码 |
| `--line` | `CXXY_WLAN_LINE` | `free1`、`free2`、`campus` |
| `--password-stdin` | — | 从标准输入读取一行密码 |
| `--remember-me` | — | 登录成功后保存账号、密码 |
| `--non-interactive` | — | 凭据不足时报错，不打开 TUI |

脚本可以通过参数或环境变量提供完整凭据，也可以使用 `--password-stdin`，避免把密码写入命令行参数。不要将真实密码写入仓库或脚本示例。

线路对应关系：`free1` 为临时免费 1（`@dianxin`），`free2` 为免费 2（`@free`），`campus` 为内网（`@xyw`）。未指定时使用上次成功登录或离线保存的线路，首次默认为 `free1`。

`logout` 不需要账号密码，也没有二次确认。如果 daemon 正在运行，会暂停当前 Wi-Fi 连接期间的自动登录；手动 `login` 成功（包括离线保存）或 daemon 观察到断开并重新接入目标 Wi-Fi 后恢复。暂停状态会落盘，同一连接上重启 daemon 不会立即取消暂停。若 daemon 停止期间发生断开重连，部分平台无法恢复这段历史，可手动 `login` 恢复。

## 安装范围与自动启动

```sh
# 普通用户：登录操作系统后启动
cxxy-wlanlogin-cli install
cxxy-wlanlogin-cli uninstall

# Linux / macOS：系统开机后启动，无需用户登录
sudo cxxy-wlanlogin-cli install
sudo cxxy-wlanlogin-cli daemon status
sudo cxxy-wlanlogin-cli uninstall
```

Windows 在普通终端中安装当前用户的登录计划任务，在管理员终端中安装以 LocalSystem 运行的原生 SCM 服务，随开机启动。

默认范围根据实际权限选择：普通用户为“用户”，root / 提权管理员为“系统”。可在任意命令中显式指定全局参数 `--user` 或 `--system`。系统范围的写入、登录、注销和服务控制需要管理员权限；只读的 `daemon --system status` 无需提权。提权终端不能操作用户范围，请回到普通用户终端执行 `--user` 命令。

两个范围的凭据、线路偏好、暂停状态和 IPC 相互独立；系统服务使用管理员为本机保存的一套校园网账号，不读取个人密钥库，也不自动复制原用户密码。安装时会检查另一范围的服务：用户安装检查系统服务，系统安装检查当前调用用户（Unix 下为 sudo 原用户）的服务；发现冲突需先卸载原服务。登录、注销和启动也会拒绝已检测到的范围冲突；请使用已安装服务对应的范围和权限执行。其他用户的自定义安装目录不作全盘扫描，用户 daemon 发现系统服务后会退出。

`install` 先检查 Wi-Fi；不在 `Student_CX` 或无法识别 SSID 时会提示，并继续准备凭据和安装服务，不请求门户。所选范围缺少已保存凭据时先进入登录流程，离线时只保存凭据。**安装流程的 Remember me 强制开启且不能取消，TUI 以黄色提示说明原因。** 取消登录、在线认证失败或凭据保存失败均不会安装服务。`install` 支持 `login` 的参数与环境变量。

程序会复制到稳定安装目录，然后注册并启动服务。重新执行 `install` 可更新副本；安装失败时尝试恢复此前的文件和服务，回滚未完成会明确报错。用户安装不启用 Linux linger。

| 平台 | 用户安装与凭据 | 系统安装与凭据 |
| --- | --- | --- |
| Linux | `systemd --user`，Secret Service | `systemd --system`，`systemd-creds` 加密凭据 |
| macOS | LaunchAgent，用户 Keychain | LaunchDaemon，System Keychain，授权已安装的服务程序读取 |
| Windows | 当前用户登录计划任务，Credential Manager | 原生 SCM 服务，机器 DPAPI 加密，ACL 限制为 SYSTEM 与管理员 |

Linux 系统安装默认需要支持 `systemd-creds` 和 `LoadCredentialEncrypted` 的 systemd。加密数据保存在 `/etc/cxxy-wlanlogin/login.cred`，启动时由 systemd 提供解密后的凭据；更新密码后 daemon 会重新读取，平时缓存凭据，避免每 30 秒重新调用解密工具。

旧 Linux 系统可在首次配置时显式选择：

```sh
sudo cxxy-wlanlogin-cli install --credential-store protected-file
```

这会以明文写入 `/etc/cxxy-wlanlogin/login.json`，目录权限 `0700`、文件 `0600`，仅允许 root 读取；程序会给出警告。加密失败不会自动降级。后续登录和更新安装沿用已有方式；已有凭据时拒绝直接切换后端。如需切换，应先卸载服务，再自行备份并移除旧系统凭据，使用新后端重新安装。

| 平台 | 系统程序与服务文件 | 系统状态、IPC 与凭据 |
| --- | --- | --- |
| Linux | `/usr/local/libexec/cxxy-wlanlogin/`；`/etc/systemd/system/cxxy-wlanlogin.service` | `/var/lib/cxxy-wlanlogin/`；`/run/cxxy-wlanlogin/`；`/etc/cxxy-wlanlogin/` |
| macOS | `/Library/Application Support/cxxy-wlanlogin/bin/`；`/Library/LaunchDaemons/org.seucxcs.cxxy-wlanlogin.plist` | 应用目录下 `state/`；`/var/run/cxxy-wlanlogin/`；System Keychain |
| Windows | Program Files 下 `cxxy-wlanlogin/`；SCM 注册与 ProgramData 下安装记录 | ProgramData 下 `cxxy-wlanlogin/{state,run,secrets}/` |

用户安装使用平台的用户配置和数据目录。Unix 使用权限 `0600` 的 Unix socket，系统 socket 位于仅 root 可访问的运行目录；Windows 使用拒绝远程客户端的命名管道，用户范围允许当前用户与 LocalSystem，系统范围允许提权管理员与 LocalSystem。两者均验证随机控制令牌，不通过 IPC 传输凭据，不监听 TCP 控制端口。

`uninstall` 停止所选范围的 daemon，移除自动启动注册和受管理的程序副本，保留已保存凭据、线路偏好和普通 CLI 程序，不执行校园网注销。

macOS 和部分 Windows 版本可能要求位置权限才能读取 SSID；系统服务的权限与交互式用户不同，需要平台实测。无法识别 SSID 时 `login` 和 `install` 提示并采用离线保存，daemon 记录错误并等待重试，不向门户发送凭据。用户密钥库尚未解锁时用户 daemon 等待重试。

## daemon 行为

```sh
cxxy-wlanlogin-cli daemon
cxxy-wlanlogin-cli daemon start
cxxy-wlanlogin-cli daemon stop
cxxy-wlanlogin-cli daemon restart
cxxy-wlanlogin-cli daemon status
```

不带子命令的 `daemon` 仍在当前进程运行后台循环，可用于查看日志；首次运行前必须在同一范围通过 `login --remember-me` 或离线 `login` 保存过凭据。安装服务会自动使用此模式。

`daemon start`、`stop`、`restart` 分别启动、停止和重启已安装的系统服务。**未执行 `install`、安装文件不完整或系统注册丢失时，三者均报错，不会隐式安装。** 已停止的服务可以直接 `restart`。它们只调用系统服务管理器，不读取密码、不发送门户请求，也不通过 IPC 操作单独启动的前台 daemon。启动后的服务按下方自动登录规则运行。

`stop` 保留自动启动设置：用户服务下次登录系统仍会启动，系统服务下次开机仍会启动；永久移除使用 `uninstall`。Linux 使用所选范围的 `systemctl`；macOS 使用相应的 `gui/<uid>` 或 `system` launchd 域；Windows 用户服务使用 Task Scheduler，系统服务使用 SCM。Windows 重启会等待旧服务停止后再启动，停止失败则不继续启动。

`daemon status` 只读查询实际安装。普通用户未指定范围时查找用户和系统安装；提权默认查询系统范围，显式 `--user` / `--system` 只查询所选范围。未发现任何安装文件时只输出：

```text
🔴 安装状态：未安装
```

不会显示推测的范围、安装位置或已安装版本。已确认安装时，前两行固定为：

```text
🟢 安装状态：已安装
安装范围：系统
```

用户安装第二行为 `安装范围：用户`。后续显示服务名称、实际存在的安装文件、已安装副本版本及运行状态；版本通过副本的 `--version` 读取。安装残缺或注册丢失显示“安装不完整”；管理器不可用显示“无法确认”及“运行状态：未知”，并返回错误。单独运行的前台 `daemon` 不计入系统服务运行状态。

- 启动时检查当前 Wi-Fi，并订阅系统网络事件：Linux 使用 NetworkManager D-Bus，Windows 使用 Native WLAN，macOS 使用 CoreWLAN 和 SystemConfiguration。
- 只在 SSID 精确等于 `Student_CX` 时查询认证接口。离开目标网络时停止门户轮询。
- 初次接入立即检查，之后每 30 秒检查一次；重复网络事件合并处理，不主动扫描周围 Wi-Fi。
- 已认证时不提交登录；明确未认证时自动登录。首次接入时即使认证标志未知，也会尝试登录，失败后按退避重试；首次登录成功后，周期检查的未知状态不会直接触发重新登录。
- 请求有超时；失败采用最长 5 分钟的退避。明确的密码错误暂停自动尝试，更新保存的凭据后恢复。
- CLI 与 daemon 共用操作锁，避免并发登录、注销。每个安装范围只允许一个 daemon 实例。

30 秒轻量内网检查的开销通常较低，但仍会产生定时唤醒和无线通信，不能视为零耗电。系统显示“可以联网”不等于门户已认证，VPN 或其他网络接口也可能影响系统判断。

CLI 和 daemon 业务消息统一使用 tracing，格式为 `[<LEVEL> <本地时间>] <内容>`，例如 `[INFO 2026-09-22 10:30:00] 已登录`；TUI 恢复终端后再输出结果。状态表与命令帮助保持普通文本。

Linux 用户日志使用 `journalctl --user -u cxxy-wlanlogin.service`，系统日志使用 `sudo journalctl -u cxxy-wlanlogin.service`。macOS 和 Windows 的服务日志写入所选范围配置目录中的 `daemon.log`；前台与普通 CLI 写入标准错误。`RUST_LOG` 可调整日志级别，例如 `cxxy_wlanlogin_cli=debug`。

## 协议确认范围

请求参数以 [登录协议](docs/eportal-login.md) 和 [注销协议](docs/eportal-logout.md) 为基准。**本次开发未访问真实校园网门户，未执行真实登录、注销或自动登录。** 测试使用本地模拟响应，不能证明线上协议已经完整匹配。

状态解析目前兼容 `v46ip`、`v4ip`、`userip`、`user_ip`、`wlan_user_ip` 中的有效 IPv4；认证状态只识别 `online` 或 `is_login` 的布尔值、0/1 数字或字符串。它们是待现场验证的兼容字段，不是已经确认的本校响应格式。未知字段返回 `Unknown`，没有有效设备 IP 则拒绝提交认证操作；不会将 `result=1` 或网络超时直接解释为登录状态。

因此，**周期掉线重连仍依赖现场响应是否包含受支持的认证标志**。若实际返回格式不同，需要先在获准的环境采集脱敏样例，再调整解析器。登录/注销动作暂按 `result` 为 `1`、`"1"` 或 `"ok"` 判断成功；真实错误码和会话行为也尚未实测。

## 库调用

```rust,no_run
use cxxy_wlanlogin_lib::{Client, Credentials, Line, PortalConfig};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = Client::new(PortalConfig::default())?;
let credentials = Credentials {
    username: std::env::var("CXXY_WLAN_USERNAME")?,
    password: std::env::var("CXXY_WLAN_PASSWORD")?,
};
client.login(&credentials, Line::Free1).await?;
// 按需调用 client.status().await 或 client.logout().await。
# Ok(())
# }
```

库只封装 API，不检查 Wi-Fi、不保存凭据、不读环境变量、不启动后台任务。Wi-Fi 检查及离线保存由应用层完成，库调用者需自行决定何时发送请求。示例中的环境变量读取由调用方完成。`PortalConfig` 可注入模拟 HTTP 端点；库禁用系统代理和重定向，错误输出移除请求 URL，避免暴露协议查询参数中的密码。

## Windows 7

通信使用 Windows 7 已有的命名管道，不依赖 Windows 的 AF_UNIX 支持；计划任务使用 1.2 schema 和当前用户的 InteractiveToken。系统安装使用 Win7 已有的 SCM 与机器 DPAPI 接口。服务状态通过系统 PowerShell 调用 Task Scheduler COM API 或读取 SCM 数字状态，兼容 PowerShell 2 的语法，不依赖本地化状态文本。

普通 `x86_64-pc-windows-*` 构建不能作为 Win7 兼容产物。应使用 Rust 的专用 `x86_64-win7-windows-*` target 和重建的标准库，例如在相应编译工具链环境中：

```sh
rustup component add rust-src --toolchain nightly
cargo +nightly build -Z build-std=std,panic_abort \
  --target x86_64-win7-windows-gnu --release --locked
```

GNU 构建需要兼容的 MinGW 链接器；MSVC 可改用 `x86_64-win7-windows-msvc` 并准备 Windows SDK / C++ 构建工具。Win7 target 会让 `getrandom` 选择旧版 Windows 随机数接口。

已在本地对 `x86_64-win7-windows-gnu` 完成包含标准库重建的 `cargo check`。这不是最终可执行文件链接验证，也不是 Windows 7 实机验证。

## 开发验证

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo check --workspace --target x86_64-pc-windows-gnu --locked
cargo check --workspace --target aarch64-apple-darwin --locked
```

自动化测试只使用临时目录、Unix socket、模拟密钥库与 Wi-Fi 状态、模拟命令以及 `127.0.0.1` HTTP 服务，不使用真实凭据、校园门户或用户服务管理器。覆盖参数编码、响应解析、密码脱敏、账号切换、强制保存、离线保存无请求、提交前 Wi-Fi 变化、TUI 渲染、后台状态机、IPC 验证、用户/系统服务模板、SCM 停止等待、安装状态与失败回滚，以及系统凭据权限和显式后端选择。

原生密钥库访问、平台网络事件、用户登录/开机时服务启动和真实认证仍需各平台实机验收；交叉编译通过不代表这些行为已实测。
