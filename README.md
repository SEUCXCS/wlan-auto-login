<div align="center">

<img src="assets/icon/icon.svg" alt="WLAN Auto Login" width="96" height="96" />

# WLAN Auto Login

东南大学成贤学院 `Student_CX` 校园 Wi-Fi 自动登录工具

[![Release](https://img.shields.io/github/v/release/SEUCXCS/wlan-auto-login?style=flat-square)](https://github.com/SEUCXCS/wlan-auto-login/releases)
[![Tests](https://img.shields.io/github/actions/workflow/status/SEUCXCS/wlan-auto-login/test.yml?branch=main&label=tests&style=flat-square)](https://github.com/SEUCXCS/wlan-auto-login/actions/workflows/test.yml)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-blue?style=flat-square)](#安装指南)
[![License](https://img.shields.io/github/license/SEUCXCS/wlan-auto-login?style=flat-square)](LICENSE)

[下载与版本更新](https://github.com/SEUCXCS/wlan-auto-login/releases) · [快速开始](#快速开始) · [使用指南](#使用指南) · [协议文档](#相关文档) · [提交问题](https://github.com/SEUCXCS/wlan-auto-login/issues)

</div>

---

## 特性

- **跨平台原生集成**：支持 Linux、macOS（≥ 13.0）与 Windows，使用各平台原生网络状态接口与系统服务机制。
- **后台自动认证**：接入 `Student_CX` 且门户离线时自动登录，支持断线重连与手动注销暂停。
- **原生凭据存储**：通过系统密钥环或受保护存储（Secret Service / `systemd-creds`、Keychain、Credential Manager / DPAPI）保存账号密码。
- **多形态操作**：同时提供命令行（CLI）、终端交互表单（TUI）与非交互脚本参数。

## 快速开始

| 平台 | 推荐安装方式 |
| --- | --- |
| **Windows** | 从 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 下载 `wlanlogin-windows-setup-<arch>.exe`，运行安装向导并填写账号即可。 |
| **Linux** | 从 Releases 安装 `.deb` / `.rpm` / Arch 软件包，或将二进制放入 `~/.local/bin/wlanlogin`，再执行 `wlanlogin --user service install`。 |
| **macOS** | 下载 `wlanlogin.app` 放入 `~/Applications`，完成[本地签名](#macos)并软链至 PATH 后执行 `wlanlogin --user service install`。 |

## 安装指南

### 运行环境与作用域

| 平台 | Wi-Fi 状态 | 后台服务 | 用户模式凭据 | 系统模式凭据 |
| --- | --- | --- | --- | --- |
| Linux | NetworkManager / D-Bus | systemd（用户 / 系统） | Secret Service（GNOME Keyring、KWallet 等） | `systemd-creds` |
| macOS (≥ 13.0) | CoreWLAN + 定位服务 | `SMAppService`（LaunchAgent / LaunchDaemon） | 登录钥匙串 | 系统钥匙串 |
| Windows | Native Wi-Fi API | 计划任务 / Windows 服务 | 凭据管理器 | 机器 DPAPI |

程序支持两种安装作用域，相互独立：

| 作用域 | 参数 | 启动时机 | 权限要求 |
| --- | --- | --- | --- |
| 用户模式 | `--user`（普通终端默认） | 当前用户登录后 | 普通用户 |
| 系统模式 | `--system`（`sudo` / 管理员终端默认） | 随系统开机启动 | 管理员权限 |

`service install` 会引导保存凭据（未连接校园网时亦可提前配置），将后台专用副本安装至系统数据目录并注册服务。切换作用域前请先卸载原服务。

<a id="quick-windows"></a>

### Windows

从 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 下载与架构匹配的 `wlanlogin-windows-setup-<arch>.exe` 并运行：

1. 选择**仅当前用户**（用户模式）或**此计算机上的所有用户**（系统模式，需要管理员权限）。
2. 输入账号、密码与线路，完成安装。

向导会自动安装 `wlanlogin.exe`、配置 PATH 并启动后台服务，重新打开终端即可使用 `wlanlogin`。卸载时在系统**已安装的应用**中移除即可同步清理服务、凭据与数据。

<details>
<summary>使用便携压缩包（ZIP）安装</summary>

1. 下载并解压 `wlanlogin-windows-<arch>.zip`。
2. 在解压目录打开 PowerShell，安装并检查服务：

   ```powershell
   .\wlanlogin.exe --user service install
   .\wlanlogin.exe --user service status
   ```

3. 如需全局调用，将解压目录添加至 PATH 并重新打开终端。系统模式请在管理员 PowerShell 中将 `--user` 替换为 `--system`。

</details>

<a id="quick-linux"></a>

### Linux

1. 从 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 下载对应架构压缩包并解压（或[从源码构建](#build-linux)）。
2. 安装可执行文件并配置 PATH：

   ```sh
   mkdir -p "$HOME/.local/bin"
   install -m 0755 ./wlanlogin "$HOME/.local/bin/wlanlogin"
   export PATH="$HOME/.local/bin:$PATH"
   ```

   将 `export PATH="$HOME/.local/bin:$PATH"` 写入 `~/.bashrc` 或 `~/.zshrc`。

3. 安装并验证后台服务：

   ```sh
   wlanlogin --user service install
   wlanlogin --user service status
   ```

系统模式请使用管理员权限执行：

```sh
sudo "$HOME/.local/bin/wlanlogin" --system service install
sudo "$HOME/.local/bin/wlanlogin" --system service status
```

<details>
<summary>使用 Debian / Ubuntu、Fedora 或 Arch Linux 软件包安装</summary>

从 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 下载软件包（或按[软件包打包](#build-linux-packages)自行生成）后安装：

```sh
# Debian 12+ / Ubuntu 24.04+
sudo apt install ./wlanlogin_<version>-1_amd64.deb

# Fedora 43+
sudo dnf install ./wlanlogin-<version>-<release>.x86_64.rpm

# Arch Linux
sudo pacman -U ./wlanlogin-<pkgver>-<pkgrel>-x86_64.pkg.tar.zst
```

软件包安装 `/usr/bin/wlanlogin` 与中英文 man 手册（`man wlanlogin`，中文环境如 `LANG=zh_CN.UTF-8` 下显示中文版），并自动声明 NetworkManager、systemd 与 D-Bus 依赖。后台服务仍通过上文第 3 步的 `service install` 注册（系统模式使用 `sudo wlanlogin --system service install`）。

升级软件包后，在原作用域重新执行 `service install` 更新后台副本；移除软件包前，先执行同作用域的 `service uninstall`（需清除配置与凭据时加 `--purge`）。

</details>

<details>
<summary>桌面钱包与旧版系统说明</summary>

- **桌面钱包（用户模式）**：后台服务仅在接入 `Student_CX` 且需要认证时读取钱包。若桌面自动登录未解锁钱包，执行一次 `wlanlogin --user login` 或在桌面密钥环中解锁后，后台将自动重试。
- **自定义桌面会话**：未使用标准图形会话目标的桌面环境，可在图形会话就绪后执行 `wlanlogin --user service session-ready` 同步环境，注销前执行 `wlanlogin --user service session-end`。
- **旧版系统明文存储（系统模式）**：缺少 `systemd-creds` 时，可显式指定 `protected-file`（账号密码将以明文保存于仅 root 可读的文件中）：

  ```sh
  sudo "$HOME/.local/bin/wlanlogin" --system service install --credential-store protected-file
  ```

</details>

<a id="quick-macos"></a>

### macOS

macOS 读取 Wi-Fi SSID 需要定位权限，后台服务与钥匙串授权依赖固定代码签名身份，安装前需使用本地代码签名证书对应用包签名。

1. **放置应用包**：从 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 下载并解压 `wlanlogin.app`（或[自行打包](#build-macos)），移入 `~/Applications`：

   ```sh
   export WLANLOGIN_APP="$HOME/Applications/wlanlogin.app"
   ```

2. **对应用包签名**（首次使用请先展开下方步骤创建证书 `wlanlogin-dev`，并确保已安装 Xcode Command Line Tools）：

   ```sh
   codesign --force --sign "wlanlogin-dev" \
     --identifier dev.cxcs.tools.wlanlogin \
     --options runtime --timestamp=none "$WLANLOGIN_APP"

   codesign --verify --deep --strict --verbose=2 "$WLANLOGIN_APP"
   ```

   > [!NOTE]
   > 首次运行若被 Gatekeeper 拦截，在**系统设置 → 隐私与安全性**中选择**仍要打开**即可。保留签名证书与私钥，后续升级使用同一证书重签可延续系统授权。

3. **创建命令入口**（签名后请勿修改 `.app` 内部文件）：

   ```sh
   mkdir -p "$HOME/.local/bin"
   ln -sf "$HOME/Applications/wlanlogin.app/Contents/MacOS/wlanlogin" \
     "$HOME/.local/bin/wlanlogin"
   export PATH="$HOME/.local/bin:$PATH"
   ```

   将 `export PATH="$HOME/.local/bin:$PATH"` 写入 `~/.zshrc`。如需供多用户使用，可将已签名的应用包通过 `sudo ditto` 复制至 `/Applications/wlanlogin.app` 并软链至 `/usr/local/bin/wlanlogin`。

4. **安装服务并完成授权**：

   ```sh
   wlanlogin --user service install
   wlanlogin --user service status
   ```

   按系统提示依次允许**定位服务**（读取 SSID）与**钥匙串访问**（建议选择**始终允许**）。若提示需要批准后台活动，前往**系统设置 → 通用 → 登录项**开启 CXCS WLAN Login，随后执行 `wlanlogin --user service start`。

<a id="macos-signing"></a>

<details>
<summary>创建本地代码签名证书（wlanlogin-dev）</summary>

已有代码签名证书可直接复用，将命令中的 `wlanlogin-dev` 替换为既有证书名。

1. 打开**钥匙串访问**，选中**登录**钥匙串。
2. 选择菜单**钥匙串访问 → 证书助理 → 创建证书**。
3. 名称填写 `wlanlogin-dev`，身份类型选择**自签名根证书**，证书类型选择**代码签名（Code Signing）**并完成创建。
4. 打开新建证书详情，展开**信任**，将**代码签名**设为**始终信任**并保存。
5. 验证签名身份可用：

   ```sh
   security find-identity -v -p codesigning
   ```

</details>

<details>
<summary>macOS 系统模式（--system）注意事项</summary>

```sh
sudo "$HOME/.local/bin/wlanlogin" --system service install
```

- 系统服务读取 Wi-Fi 状态仍依赖当前活动桌面用户的定位授权。无桌面会话安装完成后，需在桌面终端执行一次 `sudo wlanlogin --system login` 完成授权。
- Apple 对包含原生 LaunchDaemon 的应用要求公证，本地自签名包在系统模式下可能被系统拒绝注册。

</details>

### 升级与卸载

**升级**

- **Windows 安装向导**：直接运行新版安装程序，同作用域升级会自动保留已有凭据与配置。
- **Linux / macOS / Windows ZIP**：替换新版程序（macOS 需使用原证书重新签名至同一路径），在原作用域重新执行 `service install` 更新后台副本。

**卸载**

```sh
wlanlogin service uninstall           # 卸载后台服务，保留凭据与配置
wlanlogin service uninstall --purge   # 卸载服务并清除当前作用域的后台副本、配置、日志与凭据
```

系统模式卸载需附加 `--system` 与管理员权限。手工下载的文件、复制到 `Applications` 的 `.app` 与 PATH 软链接需自行移除；macOS 系统定位授权不会随卸载重置。

## 使用指南

### 常用命令

以下示例假设 `wlanlogin` 已配置至 PATH（Windows 未配置时使用 `.\wlanlogin.exe`），各子命令均支持 `--help`。

| 命令 | 说明 |
| --- | --- |
| `login` | 登录校园网，或离线保存凭据 |
| `logout` | 注销当前会话并暂停当前连接自动登录 |
| `service install` / `uninstall` | 安装或卸载后台服务 |
| `service start` / `stop` / `restart` | 启停或重启后台服务 |
| `service status` / `logs` | 查看后台状态或日志 |
| `config get` / `set` / `list` / `remove` | 读写作用域配置（如 `log.level`） |
| `permission list` / `check` / `get` | 查询或申请平台权限（当前支持 macOS） |
| `daemon` | 在当前进程前台运行自动登录循环 |

### 登录、注销与切换账号

```sh
wlanlogin login                              # 使用已保存凭据登录；缺少凭据时打开 TUI
wlanlogin login --interactive --remember-me  # 打开表单并保存新凭据
wlanlogin logout                             # 注销并暂停当前连接自动登录
wlanlogin logout --forget                    # 注销、停止后台并删除已保存凭据
```

- **TUI 按键**：`Tab` 切换焦点，方向键选择线路，空格切换复选框，`Esc` 取消。
- **网络判定**：仅在接入 `Student_CX` 时提交认证；其他网络下可离线保存凭据。当前已在线时 `login` 直接提示已在线。
- **切换账号或线路**：先执行 `wlanlogin logout`，再执行 `wlanlogin login --interactive --remember-me`。
- **自动登录暂停与恢复**：手动 `logout` 会暂停当前连接的自动登录（重启服务亦保持暂停），在同一作用域手动 `login` 或重新接入 Wi-Fi 后恢复。

### 参数与脚本自动化

| 参数 | 环境变量 | 说明 |
| --- | --- | --- |
| `-u`, `--username` | `WLANLOGIN_USERNAME` | 校园网账号 |
| `-p`, `--password` | `WLANLOGIN_PASSWORD` | 密码 |
| `--password-stdin` | — | 从标准输入读取一行密码（与 `-p` 互斥） |
| `--line` | `WLANLOGIN_LINE` | 线路：`free1`（免费 1）、`free2`（免费 2）、`campus`（内网） |
| `--remember-me` | — | 保存凭据 |
| `--interactive` | — | 强制打开 TUI 表单 |
| `--non-interactive` | — | 缺少必要信息时直接报错退出 |
| `--credential-store` | — | Linux 系统模式凭据存储：`native` 或 `protected-file` |

已有凭据时，可用 `wlanlogin service install --line free2` 更新后台使用的线路。

脚本自动化建议使用 `--password-stdin` 与 `--non-interactive`，避免密码进入 shell 历史或进程参数列表：

```sh
printf '%s\n' "$PASSWORD" | wlanlogin login -u 2026000000 --password-stdin --line free1 --non-interactive
```

### 后台服务与日志

```sh
wlanlogin --user service status
wlanlogin --user service logs -n 200
wlanlogin --user service logs -f
```

后台检查策略：接入新 Wi-Fi 立即检查；登录成功后 30 秒复查；在线后每 4 分钟检查一次；首次联网临时错误按 1s / 2s / 4s 快速重试，常规错误按 30s / 60s / 120s 退避。

默认日志级别为 `info`（仅记录失败的门户 HTTP 交互）。排查门户问题时可启用 `debug` 记录完整脱敏请求与响应：

```sh
wlanlogin --user config set log.level debug
wlanlogin --user service restart
wlanlogin --user service logs -f
wlanlogin --user config remove log.level
```

前台运行支持通过 `RUST_LOG` 覆盖日志级别，例如 `RUST_LOG=wlanlogin::portal::page_config=debug wlanlogin --user daemon`。`daemon.log` 超过 2 MiB 时会在后台下次启动时清空。

日志中 `Network check completed` 的常见 `outcome` 字段：

- `online`：已认证在线
- `waiting_for_target_wifi`：未连接至 `Student_CX`
- `automatic_login_paused`：手动注销后已暂停自动登录
- `unknown_portal_state`：门户状态异常或暂无法判定

### 权限诊断（macOS）

```sh
wlanlogin permission list
wlanlogin --user permission check
wlanlogin --user permission check macos.keychain --json
wlanlogin --user permission get macos.location
sudo wlanlogin --system permission get macos.keychain
```

`check [ID]` 仅查询状态，不触发授权弹窗；`get <ID>` 在交互终端中请求指定权限。三个子命令均支持 `--json` 输出。

## 排障

| 现象 | 排查方式 |
| --- | --- |
| 无法识别 `wlanlogin` 命令 | 检查安装路径、符号链接与 PATH 配置，或先使用完整路径执行 |
| 仅保存凭据未触发认证 | 确认已连接 `Student_CX`；macOS 检查定位权限 |
| 凭据存储不可用或提示需要交互 | 解锁系统密钥环 / 钥匙串，在同一作用域执行前台授权 |
| 已在线但新账号未生效 | 先执行 `wlanlogin logout`，再重新登录 |
| 手动登录后后台仍处于暂停 | 确认手动登录与后台服务使用相同作用域（`--user` 或 `--system`） |
| 认证被拒绝后停止自动重试 | 执行 `wlanlogin login --interactive --remember-me` 更新凭据 |
| macOS 已连接 Wi-Fi 但无法读取 SSID | 恢复定位授权后重启或重装服务 |

<details>
<summary>macOS 定位与钥匙串权限恢复</summary>

**定位权限**

1. 打开**系统设置 → 隐私与安全性 → 定位服务**，开启定位总开关与 CXCS WLAN Login。
2. 若列表中未出现该应用，在桌面终端执行 `wlanlogin --user login --interactive` 触发首次授权后重新检查。

**钥匙串权限**

1. 在**钥匙串访问**中解锁**登录**钥匙串，在同一作用域重新执行 `service install` 并允许访问。
2. 若仍提示拒绝，定位服务名 `dev.cxcs.tools.wlanlogin`（系统模式位于**系统**钥匙串，服务名 `dev.cxcs.tools.wlanlogin.system`，账户均为 `saved-login`），在**访问控制**中添加实际安装的后台程序路径（见下方目录参考）。

恢复权限后执行 `service restart` 或重新 `service install`，并通过 `service logs -f` 确认状态。

</details>

<details>
<summary>数据目录、后台副本与系统标识参考</summary>

应用 ID 为 `dev.cxcs.tools.wlanlogin`，数据目录遵循 `SEUCXCS/wlanlogin` 两级结构（Linux 遵循 XDG 规范，Windows 使用系统 Known Folders）。

**配置、日志与运行文件**

| 平台 | 用户模式 | 系统模式 |
| --- | --- | --- |
| Linux | `~/.config/SEUCXCS/wlanlogin/` | `/var/lib/SEUCXCS/wlanlogin/`、`/run/SEUCXCS/wlanlogin/`；凭据位于 `/etc/SEUCXCS/wlanlogin/` |
| macOS | `~/Library/Application Support/SEUCXCS/wlanlogin/` | `/Library/Application Support/SEUCXCS/wlanlogin/state/`、`/var/run/SEUCXCS/wlanlogin/` |
| Windows | `%APPDATA%\SEUCXCS\wlanlogin\config\` | `%ProgramData%\SEUCXCS\wlanlogin\`（`state`、`run`、`secrets`） |

**后台服务副本**

| 平台 | 用户模式 | 系统模式 |
| --- | --- | --- |
| Linux | `~/.local/share/SEUCXCS/wlanlogin/bin/wlanlogin-daemon` | `/usr/local/libexec/SEUCXCS/wlanlogin/wlanlogin-daemon` |
| macOS | `~/Library/Application Support/SEUCXCS/wlanlogin/wlanlogin.app` | `/Library/Application Support/SEUCXCS/wlanlogin/wlanlogin.app` |
| Windows | `%LOCALAPPDATA%\SEUCXCS\wlanlogin\data\bin\wlanlogin-daemon.exe` | `%ProgramFiles%\SEUCXCS\wlanlogin\wlanlogin-daemon.exe` |

> macOS 钥匙串访问控制需授权对应 `.app` 内的 `Contents/MacOS/wlanlogin`。

**服务与凭据标识**

| 项目 | 名称 |
| --- | --- |
| Linux systemd unit | `dev.cxcs.tools.wlanlogin.service` |
| macOS launchd Label / plist | `dev.cxcs.tools.wlanlogin` / `dev.cxcs.tools.wlanlogin.plist` |
| Windows 系统服务 | `dev.cxcs.tools.wlanlogin` |
| Windows 用户计划任务 | `dev.cxcs.tools.wlanlogin-{SID}` |
| 凭据服务名与账户 | `dev.cxcs.tools.wlanlogin`（macOS 系统模式为 `.system`），账户 `saved-login` |

</details>

提交 [Issue](https://github.com/SEUCXCS/wlan-auto-login/issues) 时请附上系统版本、软件版本、安装作用域、复现步骤与脱敏日志。

## 从源码构建

项目采用 Rust 2024 edition Cargo workspace，基于仓库内的 `Cargo.lock` 构建。以下命令均在仓库根目录执行。

<a id="build-linux"></a>

### Linux

安装原生依赖与 [Rust](https://www.rust-lang.org/tools/install)：

```sh
# Debian / Ubuntu
sudo apt update && sudo apt install git build-essential pkg-config libdbus-1-dev

# Fedora
sudo dnf install git gcc pkgconf-pkg-config dbus-devel

# Arch Linux
sudo pacman -S --needed git base-devel rust pkgconf dbus
```

克隆仓库并构建安装：

```sh
git clone https://github.com/SEUCXCS/wlan-auto-login.git
cd wlan-auto-login
cargo build --release --locked -p wlanlogin-cli --bin wlanlogin
install -Dm 0755 target/release/wlanlogin "$HOME/.local/bin/wlanlogin"
```

<a id="build-linux-packages"></a>

<details>
<summary>构建 Debian / Fedora / Arch 软件包</summary>

`.deb` 与 `.rpm` 的共享库依赖均按构建主机生成：`.deb` 最低支持 Debian 12 / Ubuntu 24.04（glibc 2.36），`.rpm` 最低支持 Fedora 43（glibc 2.42），二进制要求的 glibc 高于对应下限时打包会直接报错。两者的变更记录日期默认取自当前 Git 提交；使用不含 `.git` 的源码压缩包构建时，需通过 `SOURCE_DATE_EPOCH` 指定 Unix 秒级时间戳。

**Debian / Ubuntu（`.deb`）**

在 Debian 12（或更旧）主机安装 `dpkg-dev` 后执行：

```sh
cargo xtask bundle deb
```

也可在 Debian 12 容器中构建：

```sh
podman run --rm -v "$PWD":/work -w /work docker.io/library/rust:1-bookworm sh -c '
  apt-get update && apt-get install -y --no-install-recommends libdbus-1-dev pkg-config dpkg-dev &&
  cargo xtask bundle deb --target-dir target/debian12'
```

产物位于 `<target-dir>/release/bundle/wlanlogin_<version>-<revision>_<arch>.deb`（同版本重打可通过 `--revision` 递增修订号）。

**Fedora（`.rpm`）**

在 Fedora 安装 `rpm-build`（或在 Debian/Ubuntu 安装 `rpm`）后执行：

```sh
cargo xtask bundle rpm
```

产物位于 `<target-dir>/release/bundle/wlanlogin-<version>-<release>.<arch>.rpm`（同版本重打可通过 `--release` 递增）。

**Arch Linux（`.pkg.tar.zst`）**

`packaging/linux/distro/archlinux/PKGBUILD` 打包当前检出的已提交 HEAD：

```sh
cd packaging/linux/distro/archlinux
makepkg -s
```

产物为 `packaging/linux/distro/archlinux/wlanlogin-<pkgver>-<pkgrel>-<arch>.pkg.tar.zst`，`pkgver` 形如 `<version>.r<提交数>.g<短哈希>`。

</details>

<a id="build-macos"></a>

### macOS

安装 Xcode Command Line Tools（`xcode-select --install`）与 [Rust](https://www.rust-lang.org/tools/install) 后打包：

```sh
git clone https://github.com/SEUCXCS/wlan-auto-login.git
cd wlan-auto-login
cargo xtask bundle macos --codesign-identity=-
mkdir -p "$HOME/Applications"
ditto "$PWD/target/release/bundle/wlanlogin.app" "$HOME/Applications/wlanlogin.app"
```

随后按 [macOS 安装](#quick-macos)步骤完成签名与服务安装。支持 `--profile`、`--target`（`aarch64-apple-darwin` / `x86_64-apple-darwin`）与 `--codesign-identity`（完整参数见 `cargo xtask bundle macos --help`）。拥有 Developer ID 证书时可直接签名：

```sh
cargo xtask bundle macos --codesign-identity "Developer ID Application: YOUR NAME (TEAMID)"
```

打包时 `CFBundleVersion` 会基于仓库根目录 `.bundle-build/build-number` 自动递增。

<a id="build-windows"></a>

### Windows

安装 Git、[Rust](https://www.rust-lang.org/tools/install) 与 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)（选择“使用 C++ 的桌面开发”；GNU target 请另行安装对应 MinGW 工具链）：

```powershell
git clone https://github.com/SEUCXCS/wlan-auto-login.git
cd wlan-auto-login
cargo build --release --locked -p wlanlogin-cli --bin wlanlogin
```

产物为 `target\release\wlanlogin.exe`。如需构建 `.exe` 安装向导，另需 .NET 8 SDK 与 WiX 6.0.2：

```powershell
dotnet tool install --global wix --version 6.0.2
wix extension add WixToolset.UI.wixext/6.0.2
cargo xtask bundle windows --target x86_64-pc-windows-msvc
```

## 相关文档

- 门户协议说明：[状态查询](docs/eportal-status.md) · [登录接口](docs/eportal-login.md) · [注销接口](docs/eportal-logout.md)
- 贡献与架构说明：[CONTRIBUTING.md](CONTRIBUTING.md) · [AGENT.md](AGENT.md)

## 贡献

欢迎通过 [Issues](https://github.com/SEUCXCS/wlan-auto-login/issues) 反馈问题或提交 Pull Request。提交前请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 并遵循 Conventional Commits 规范。

| 目录 | 说明 |
| --- | --- |
| `crates/wlan-api` | `wlanlogin-lib`，校园网门户协议客户端 |
| `crates/wlan-app` | `wlanlogin-cli`，CLI、TUI、后台服务与平台集成 |
| `xtask` | 应用打包与架构边界检查工具 |
| `packaging/windows` | Windows 安装向导与 MSI 自定义动作 |
| `packaging/{macos,linux}` | macOS 应用资源与 man 手册、Linux 发行版打包定义与双语 man 手册 |
| `docs` | 校园网门户协议说明 |

本地验证命令：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo xtask check-boundaries
```

## 许可证

[MIT License](LICENSE) © 2026 SEUCXCS contributors
