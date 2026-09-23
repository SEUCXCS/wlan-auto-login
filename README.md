# Wlan Auto Login

面向东大成贤校园 Wi-Fi 的 Rust 登录工具，提供终端登录、账号密码保存和后台自动登录。

本仓库包含两个 workspace:

- `crates/wlan-api`: 登录、注销和状态查询 API
- `crates/wlan-app`: 提供网络登陆注册 CLI、TUI 和 daemon。

## 构建

```sh
cargo build --release --locked --workspace
```

生成的程序为 `target/release/cxxy-wlanlogin-cli`，Windows 下带 `.exe` 后缀。将程序放入 `PATH`，即可使用以下命令。

Linux 构建需要 `pkg-config` 和 D-Bus 开发库（如 `libdbus-1-dev`）；运行需要 NetworkManager，用户凭据保存需要 Secret Service 密钥管理器（如 GNOME Keyring）。

## 使用

```sh
cxxy-wlanlogin-cli login                         # 使用已保存的凭据，或打开交互界面
cxxy-wlanlogin-cli login --remember-me           # 登录并保存账号密码
cxxy-wlanlogin-cli login --interactive           # 修改账号、密码或线路
cxxy-wlanlogin-cli logout                       # 注销，并暂停当前连接的自动登录
cxxy-wlanlogin-cli logout --forget              # 注销、停止后台进程并删除所选范围的凭据
```

交互界面支持输入账号密码、选择线路和勾选 Remember me。Tab 切换控件，方向键选择线路，空格切换复选框，Esc 取消。

登录前会检查 Wi-Fi：连接 `Student_CX` 时执行认证；不在该网络或无法识别 SSID 时，仅保存凭据，方便提前配置。用户凭据通过系统密钥管理器保存。

也可以通过参数或环境变量提供凭据，跳过交互界面：

| 参数 | 环境变量 | 说明 |
| --- | --- | --- |
| `--username` / `-u` | `CXXY_WLAN_USERNAME` | 账号 |
| `--password` / `-p` | `CXXY_WLAN_PASSWORD` | 密码；也可用 `--password-stdin` 从标准输入读取 |
| `--line` | `CXXY_WLAN_LINE` | `free1`（免费 1）、`free2`（免费 2）、`campus`（内网） |
| `--non-interactive` | — | 凭据不足时直接报错 |

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

后台仅在连接 `Student_CX` 时访问门户，接入后尝试登录，每 30 秒检查认证状态，确认掉线后重试。首次取得状态前遇到临时网络错误时，按 1、2、4 秒最多快速重试三次，之后进入 30 秒至 5 分钟的常规退避；重试到期直接触发，不再等待下一个检查周期。日志标明失败请求的阶段和等待时间。手动注销后，手动登录或重新接入目标 Wi-Fi 可恢复自动登录。

直接运行 `cxxy-wlanlogin-cli daemon` 会在当前进程运行后台循环，需要事先在同一范围保存凭据。`start`、`stop`、`restart` 仅管理已安装服务，未安装时会报错。

更多参数见 `cxxy-wlanlogin-cli --help` 和各子命令的 `--help`。

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
