# CXXY WLAN Auto Login

面向东南大学成贤学院 `Student_CX` 校园 Wi-Fi 的自动登录工具，支持 Linux、macOS 和 Windows。可通过命令行或终端交互界面登录、注销、保存凭据，并安装后台服务，在连接校园 Wi-Fi 后自动检查和恢复认证。

账号密码保存在所选安装范围的系统凭据存储中。用户与系统安装相互独立，支持离线预配置、自动重试、日志查看和卸载清理。

[下载 Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) · [问题反馈](https://github.com/SEUCXCS/wlan-auto-login/issues) · [构建教程](#构建教程) · [详细使用教程](#详细使用教程) · [贡献指南](CONTRIBUTING.md) · [MIT 许可证](LICENSE)

---

## 快速使用

Windows 和 Linux 使用 Releases 中的二进制文件；MacOS 需要自行构建并签名，或对发布的预构建 `.app` 在本机签名。下载时选择与系统和 CPU 架构匹配的附件；可用平台与文件以实际发布内容为准。

- [Linux](#quick-linux)
- [MacOS](#quick-macos)
- [Windows](#quick-windows)

<a id="quick-linux"></a>

### Linux

运行需要 NetworkManager、D-Bus 和 systemd。用户范围保存凭据还需要 Secret Service 实现，例如已解锁的 GNOME Keyring；系统范围默认使用 `systemd-creds` 加密凭据，需要支持该命令的 systemd。

从 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 下载适合当前架构的 Linux 二进制文件，解压后得到 `wlanlogin`。若没有对应附件，参照 [Linux 构建教程](#build-linux)。在包含该文件的目录执行：

```sh
mkdir -p "$HOME/.local/bin"
install -m 0755 ./wlanlogin "$HOME/.local/bin/wlanlogin"
export PATH="$HOME/.local/bin:$PATH"

wlanlogin --version
wlanlogin --user service install
wlanlogin --user service status
```

`service install` 会引导输入账号、密码和线路，保存凭据并安装、启动用户后台服务。未连接 `Student_CX` 时也可以提前配置。要在新终端中使用命令，将 `export PATH="$HOME/.local/bin:$PATH"` 加入所用 shell 的配置文件，例如 Bash 的 `~/.bashrc` 或 Zsh 的 `~/.zshrc`。

需要随开机启动的系统服务时，改用管理员权限执行；用户与系统两种安装范围选择一种即可：

```sh
sudo "$HOME/.local/bin/wlanlogin" --system service install
sudo "$HOME/.local/bin/wlanlogin" --system service status
```

安装范围、凭据存储和旧版 systemd 的替代方案见 [安装与后台自动登录](#安装与后台自动登录)。

<a id="quick-macos"></a>

### MacOS

需要完整的 `wlanlogin.app`，仅复制裸二进制文件不足以完成 Wi-Fi 定位授权。应用声明的最低系统版本为 macOS 13.0，预构建附件的兼容范围以发布说明为准。

#### 1. 获取应用包

两种方式任选其一：

- **自行构建**：前往 [MacOS 构建教程](#build-macos)，运行 `cargo xtask bundle macos` 打包。构建章节提供返回本节的链接。
- **给预构建应用签名**：若 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 提供未使用开发证书签名的 `.app`，下载与 Apple Silicon（`arm64`）或 Intel（`x86_64`）匹配的附件，解压后将 `wlanlogin.app` 放到 `~/Downloads/`，然后执行：

```sh
export WLANLOGIN_APP="$HOME/Downloads/wlanlogin.app"
```

预构建应用可以在本机重新签名，无需重新编译 Rust。它可能完全未签名，也可能带有 ad-hoc 签名；下文的 `codesign --force` 适用于这两种情况。没有适合的预构建附件时使用源码构建。

<a id="macos-signing"></a>

#### 2. 创建本地代码签名证书

本地使用可以创建自签名证书，无需 Apple Developer 付费账号。已有固定的代码签名证书时可直接使用，并在下文替换证书名称。

1. 打开“钥匙串访问”（可通过 Spotlight 搜索），选择“登录”钥匙串。
2. 从菜单选择“钥匙串访问 → 证书助理 → 创建证书”。
3. 名称填写 `wlanlogin-dev`，身份类型选择“自签名根证书”，证书类型选择“代码签名（Code Signing）”。按向导完成创建，将证书存入“登录”钥匙串。
4. 打开该证书的详情，在“信任”中将“代码签名”设为“始终信任”，按系统提示输入密码保存。
5. 确认证书展开后包含对应的私钥，并检查可用身份：

```sh
security find-identity -v -p codesigning
```

输出应包含 `wlanlogin-dev`。若显示 `0 valid identities found`，检查证书用途、信任设置、有效期、私钥是否存在，以及“登录”钥匙串是否已解锁。证书和私钥应保留供以后升级签名使用；仅保留同名证书而重新生成密钥，不能保持原来的签名身份。

#### 3. 签名并验证

先安装 Xcode Command Line Tools（已有时跳过）：

```sh
xcode-select --install
```

确认 `WLANLOGIN_APP` 指向上一步获取或构建的应用包，再签名：

```sh
codesign --force --sign "wlanlogin-dev" \
  --identifier dev.cxcs.tools.wlanlogin \
  --options runtime --timestamp=none "$WLANLOGIN_APP"

codesign --verify --deep --strict --verbose=2 "$WLANLOGIN_APP"
codesign -dv --verbose=4 "$WLANLOGIN_APP"
```

系统可能询问是否允许 `codesign` 访问证书私钥；确认使用的是刚创建的证书后授权。这里禁用时间戳，使本地自签名不依赖 Apple 时间戳服务。固定证书有助于升级时保持定位和钥匙串授权身份，但不保证系统永远不再询问。

本地签名不等同于 Developer ID 签名和 Apple 公证。若下载的应用仍被 Gatekeeper 阻止启动，在确认来源后前往“系统设置 → 隐私与安全性”，按系统提示选择“仍要打开”。

#### 4. 复制应用并添加命令入口

以下两种放置方式选择一种。macOS 的应用目录名称是 `Applications`。

**仅供当前用户使用**，复制到 `~/Applications` 并链接到 `~/.local/bin`：

```sh
mkdir -p "$HOME/Applications" "$HOME/.local/bin"
ditto "$WLANLOGIN_APP" "$HOME/Applications/wlanlogin.app"
ln -sf "$HOME/Applications/wlanlogin.app/Contents/MacOS/wlanlogin" \
  "$HOME/.local/bin/wlanlogin"
export PATH="$HOME/.local/bin:$PATH"
```

将 `export PATH="$HOME/.local/bin:$PATH"` 加入 `~/.zshrc`，以便新终端也能找到命令。

**供本机用户使用**，复制到 `/Applications` 并链接到 `/usr/local/bin`：

```sh
sudo ditto "$WLANLOGIN_APP" "/Applications/wlanlogin.app"
sudo mkdir -p /usr/local/bin
sudo ln -sf "/Applications/wlanlogin.app/Contents/MacOS/wlanlogin" \
  /usr/local/bin/wlanlogin
export PATH="/usr/local/bin:$PATH"
```

若 `/usr/local/bin` 尚未在 PATH 中，将上述 PATH 设置加入 `~/.zshrc`。先完成签名再复制，复制后不要修改包内文件。使用包内程序的符号链接，可以让 CLI 和 Wi-Fi 授权宿主共用完整应用包。

#### 5. 授权并安装后台服务

在已登录桌面的普通终端中执行：

```sh
wlanlogin --version
wlanlogin --user service install
wlanlogin --user service status
```

按提示填写校园网账号、密码和线路，并完成授权：

- **定位服务**：系统会按需将 WLAN Login 的授权宿主带到前台，询问是否允许定位。允许后才能读取当前 Wi-Fi 名称。程序只在识别到 `Student_CX` 时向校园网提交认证。
- **钥匙串访问**：保存或读取凭据时，系统可能要求解锁钥匙串、输入 macOS 登录密码或确认访问。核对请求来自 `wlanlogin` 后允许；希望后台持续读取时，按弹窗提供的选项选择“始终允许”。
- **后台活动**：如果系统提示后台项目，确认允许 WLAN Login 在后台运行；也可在系统设置中检查“登录项与扩展”中的后台活动设置，名称因 macOS 版本而异。

应用包放在 `/Applications` 并不表示已安装系统服务。需要系统范围时，使用 `sudo wlanlogin --system service install`；若命令仅链接到 `~/.local/bin`，使用 `sudo "$HOME/.local/bin/wlanlogin" --system service install`。系统服务也需要当前桌面用户完成定位授权，root 不能绕过该权限。

`service install` 会将完整应用包复制到所选范围的 Application Support 目录，再注册并启动后台服务。包内程序仍需指定子命令；双击 `.app` 或无参数运行不会自动完成安装。

#### 不小心拒绝了权限怎么办

**定位权限**：打开“系统设置 → 隐私与安全性 → 定位服务”，确认全局定位服务已开启，并启用 WLAN Login。此前已允许或拒绝时，重复运行命令不会再次弹出首次授权提示。若列表尚无该应用，先在桌面会话的终端运行一次交互式 `wlanlogin --user login`，让程序按需申请权限，再检查列表。设备受管理策略限制时，需要管理员解除限制。

**钥匙串访问**：解锁“登录”钥匙串，在桌面会话的终端中重新运行同一范围的 `service install` 并允许访问；系统范围使用 sudo。仍被拒绝时，在“钥匙串访问”中定位本程序的密码条目，打开“访问控制”，核对并添加实际安装的 `Contents/MacOS/wlanlogin` 程序，保存后重新安装。用户条目的服务名为 `dev.cxcs.tools.wlanlogin`，系统条目为 `dev.cxcs.tools.wlanlogin.system`，账户均为 `saved-login`；系统条目位于“系统”钥匙串，修改可能要求管理员密码。安装程序的路径见 [数据与安装目录](#数据与安装目录)。

后台不会主动弹出定位或钥匙串授权框，`--non-interactive` 也不会申请授权。修复后重新执行同一范围的 `service install`；服务已安装时也可执行 `service restart`，再用 `service logs -f` 检查。读取失败不会自动删除保存的凭据。

<a id="quick-windows"></a>

### Windows

从 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 下载适合当前架构的 Windows 二进制文件，解压后得到 `wlanlogin.exe`。若没有对应附件，参照 [Windows 构建教程](#build-windows)。

在该文件所在目录打开普通 PowerShell，安装用户后台任务：

```powershell
.\wlanlogin.exe --version
.\wlanlogin.exe --user service install
.\wlanlogin.exe --user service status
```

按提示填写账号、密码和线路。用户凭据保存到 Credential Manager，后台任务在当前用户登录后启动。需要随开机启动的系统服务时，改用“以管理员身份运行”的 PowerShell，在程序目录执行：

```powershell
.\wlanlogin.exe --system service install
.\wlanlogin.exe --system service status
```

系统范围使用机器 DPAPI 保存凭据。两种安装范围选择一种即可；要在任意目录运行命令，将 `wlanlogin.exe` 所在目录加入对应的用户或系统 PATH，再打开新终端。

## 构建教程

项目使用 Cargo workspace、Rust 2024 edition 和 resolver 3。先安装 [Rust 与 Cargo](https://www.rust-lang.org/tools/install)、Git 和各平台所需的原生工具。仓库未固定 Rust 工具链，也未声明 MSRV；edition 的最低版本不能代表整个依赖树的最低版本。

以下构建均在仓库根目录执行，使用已提交的 `Cargo.lock`，不需要手动更新依赖。

<a id="build-linux"></a>

### Linux

构建需要 C 工具链、`pkg-config` 和 D-Bus 开发库。Debian / Ubuntu 可安装：

```sh
sudo apt update
sudo apt install git build-essential pkg-config libdbus-1-dev
```

其他发行版安装对应的软件包；运行依赖见 [Linux 快速使用](#quick-linux)。Rust 安装完成后，构建应用：

```sh
git clone https://github.com/SEUCXCS/wlan-auto-login.git
cd wlan-auto-login
cargo build --release --locked -p wlanlogin-cli --bin wlanlogin
```

产物为 `target/release/wlanlogin`。可在该目录执行快速使用中的复制步骤，或在仓库根目录执行：

```sh
mkdir -p "$HOME/.local/bin"
install -m 0755 target/release/wlanlogin "$HOME/.local/bin/wlanlogin"
```

[返回 Linux 快速使用](#quick-linux)，继续配置 PATH 和安装服务。

<a id="build-macos"></a>

### MacOS

构建需要 macOS、Xcode Command Line Tools 和原生 Rust 工具链。先安装命令行工具：

```sh
xcode-select --install
```

安装 Rust 后，在终端克隆并打包：

```sh
git clone https://github.com/SEUCXCS/wlan-auto-login.git
cd wlan-auto-login
cargo xtask bundle macos --codesign-identity=-
export WLANLOGIN_APP="$PWD/target/release/bundle/wlanlogin.app"
```

该命令构建 `wlanlogin-cli` 的 `wlanlogin` 程序，生成 plist 并验证应用包，默认产物为 `target/release/bundle/wlanlogin.app`。这里先生成 ad-hoc 签名包，随后用本地固定证书重新签名；`cargo build` 只生成可执行文件，不能替代这一步。

[返回 MacOS 快速使用：创建证书、签名与安装](#macos-signing)。后续命令使用上面设置的 `WLANLOGIN_APP`。

打包也支持指定构建配置、架构和输出目录，例如：

```sh
rustup target add aarch64-apple-darwin
cargo xtask bundle macos --codesign-identity=- \
  --profile dev --target aarch64-apple-darwin
export WLANLOGIN_APP="$PWD/target/aarch64-apple-darwin/debug/bundle/wlanlogin.app"
```

Apple Silicon 的 target 为 `aarch64-apple-darwin`，Intel 为 `x86_64-apple-darwin`。指定 `--target` 后，产物路径增加 target 层级；`dev` 配置对应 `debug` 目录。还可使用 `--target-dir` 或 `CARGO_TARGET_DIR` 指定输出根目录，并以 xtask 输出的 `Bundle:` 路径为准。

已有适用于发布的 Developer ID Application 证书时，可在打包时直接指定：

```sh
cargo xtask bundle macos --codesign-identity "Developer ID Application: YOUR NAME (TEAMID)"
```

签名身份优先使用 `--codesign-identity`，其次为 `CODESIGN_IDENTITY`，默认是 `-`（ad-hoc）。显式使用 `--codesign-identity=-` 可覆盖环境变量。xtask 使用证书时会开启 hardened runtime 并请求 Apple 时间戳；本地自签名证书可能无法取得时间戳，建议采用上文先打包再 `--timestamp=none` 签名的流程。打包失败不会降级签名，也不会替换此前已验证的 bundle；打包不包含 Apple 公证或后台服务安装。

<a id="build-windows"></a>

### Windows

推荐使用 Rust 的 MSVC 工具链。安装 Git、[Rust 与 Cargo](https://www.rust-lang.org/tools/install)，以及 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)；在 Build Tools 安装器中选择“使用 C++ 的桌面开发”，并安装对应架构的 MSVC 编译工具和 Windows SDK。

在能找到 MSVC 工具的 PowerShell 或 Developer PowerShell 中执行：

```powershell
git clone https://github.com/SEUCXCS/wlan-auto-login.git
cd wlan-auto-login
cargo build --release --locked -p wlanlogin-cli --bin wlanlogin
```

默认产物为 `target\release\wlanlogin.exe`。如果选择 GNU target，需另行安装对应的 MinGW 工具链；仅添加 Rust target 不会安装原生链接工具。

[返回 Windows 快速使用](#quick-windows)，在 `target\release` 目录运行安装命令，或将程序复制到固定目录并加入 PATH。

## 详细使用教程

以下示例使用已加入 PATH 的 `wlanlogin`。Windows 未配置 PATH 时，在程序目录将命令替换为 `.\wlanlogin.exe`。参数详情可通过 `wlanlogin --help` 和各子命令的 `--help` 查看。

### 安装与后台自动登录

```sh
wlanlogin --user service install
wlanlogin --user service status
```

`service install` 会保存后台所需的凭据并安装、启动服务。缺少已保存的凭据时，进入登录流程并强制开启 Remember me；不在 `Student_CX` 时可先离线保存，接入校园 Wi-Fi 后再认证。

| 安装范围 | 启动时机 | 操作方式 |
| --- | --- | --- |
| 用户 `--user` | 当前用户登录后 | 普通终端执行 `wlanlogin --user service install` |
| 系统 `--system` | 随开机启动 | Linux / macOS 使用 `sudo wlanlogin --system service install`；Windows 使用管理员终端 |

普通终端默认选择用户范围，sudo／管理员终端默认选择系统范围。用户范围需要普通用户会话，系统服务的登录、注销及启停需要管理员权限。若 sudo 找不到用户 PATH 中的程序，使用完整路径，例如 `sudo "$HOME/.local/bin/wlanlogin" --system service install`。

两个范围分别保存配置、凭据和运行状态；切换范围前先卸载原服务，再在新范围保存凭据并安装。程序会安装后台专用副本，之后更新原始 CLI 文件不会自动升级正在运行的服务。

| 平台 | Wi-Fi 识别 | 用户服务 / 系统服务 | 用户凭据 / 系统凭据 |
| --- | --- | --- | --- |
| Linux | NetworkManager、D-Bus | systemd user / systemd system | Secret Service / systemd 加密凭据 |
| macOS | CoreWLAN、SystemConfiguration | SMAppService LaunchAgent / LaunchDaemon | 用户 Keychain / System Keychain |
| Windows | Native Wi-Fi API | Task Scheduler / Service Control Manager | Credential Manager / 机器 DPAPI |

Linux 旧系统可显式改用受文件权限保护的明文凭据：

```sh
sudo wlanlogin --system service install --credential-store protected-file
```

该模式仅适用于 Linux 系统范围，凭据以**明文**保存在仅管理员可读的文件中。默认加密模式失败时不会自动降级；macOS 和 Windows 不支持此选项。

macOS 安装会校验完整应用包，替换旧副本后检查实际安装程序的凭据读取与 Wi-Fi 能力，再注册、启动服务。失败时尝试恢复旧安装；回滚未完成时会保留恢复目录并报告位置。系统服务仍从当前活动桌面用户的授权宿主获取 Wi-Fi 状态；尚无桌面用户、用户切换或定位未授权时，会等待有效状态。无桌面会话的非交互系统安装，需要以后在登录用户的终端中通过 `sudo wlanlogin --system login` 完成定位授权。

macOS 后台服务通过 `SMAppService` 关联签名应用身份，直接启动应用包内的程序。首次注册或此前关闭过后台活动时，系统可能要求在“系统设置 → 通用 → 登录项”允许 WLAN Login；系统范围的后台服务需要管理员批准。此时 `service install` 会明确提示等待批准，保留安装和凭据；允许后执行同一范围的 `service start`。定位权限与后台活动批准相互独立，已有的定位授权不会因为卸载、重装或移动同一签名应用而自动重置。

Apple SDK 将包含原生 LaunchDaemon 的应用公证列为要求；本地自签名包不能据此保证系统范围可用。原生注册被系统拒绝时，安装会报告错误并尝试回滚。

### 登录、修改账号与保存凭据

```sh
wlanlogin login                         # 使用已保存凭据，缺少时打开交互界面
wlanlogin login --remember-me           # 登录并保存账号密码
wlanlogin login --interactive           # 打开表单，修改账号、密码或线路
wlanlogin login --interactive --remember-me
```

表单支持账号、密码、线路和 Remember me。Tab 切换控件，方向键选择线路，空格切换复选框，Esc 取消。只有识别到 `Student_CX` 时才执行校园网认证；其他网络或无法读取 SSID 时保留离线保存能力。

已认证在线时，`login` 只提示已在线，不重复提交凭据；本次提供的账号和线路并不因此视为通过认证。需要切换账号或线路时，先 `logout` 再登录；`--remember-me` 仍可保存新凭据供后续使用。系统范围的操作也要使用相同范围和管理员权限。

### 参数与自动化

| 参数 | 环境变量 | 说明 |
| --- | --- | --- |
| `--username` / `-u` | `WLANLOGIN_USERNAME` | 校园网账号 |
| `--password` / `-p` | `WLANLOGIN_PASSWORD` | 密码；也可用 `--password-stdin` 读取标准输入的一行 |
| `--line` | `WLANLOGIN_LINE` | `free1`（免费 1）、`free2`（免费 2）、`campus`（内网） |
| `--remember-me` | — | 保存凭据 |
| `--interactive` | — | 强制打开终端表单 |
| `--non-interactive` | — | 凭据不足时报错，不打开表单 |
| `--credential-store` | — | Linux 系统凭据存储模式：`native` 或 `protected-file` |

自动化时优先将密码通过标准输入传入，并选择 `--non-interactive`。`--password` 可能将密码留在 shell 历史或进程参数列表中。macOS 的非交互操作和后台服务不会主动请求系统授权，首次使用前应完成交互式授权。

### 权限查询与获取（macOS）

`permission` 当前只在 macOS 显示，支持所选范围的定位和钥匙串访问诊断：

```sh
wlanlogin permission list
wlanlogin --user permission check
wlanlogin --user permission check macos.keychain --json
wlanlogin --user permission get macos.location
sudo wlanlogin --system permission get macos.keychain
```

`list` 列出权限 ID 和说明。`check [ID]` 查询指定权限，省略 ID 时查询全部；不弹授权窗口。`get <ID>` 在交互式终端中请求指定权限，并返回实际结果；拒绝、需要交互或未授权时以非零退出码结束。`check` 成功取得状态时返回零，即使状态是 Missing、Denied 或 NeedsInteraction；无法完成查询时返回非零。定位宿主尚未运行或无法连接时，报告 Unavailable 及原因，可通过 `permission get macos.location` 启动宿主并按需申请授权。

`list`、`check`、`get` 均支持 `--json`。状态输出包含 `version`、`scope` 和 `permissions`，每项包含 `id`、`status`、`error`；不会输出账号密码。钥匙串检查的是当前程序对所选范围条目的读取能力，安装流程会另外启动实际安装副本复检。Missing 表示尚未保存凭据，获取权限不会创建凭据条目。

### 管理后台服务

```sh
wlanlogin service status       # 查看安装、版本与运行状态
wlanlogin service start        # 启动已安装服务
wlanlogin service stop         # 停止服务，保留自动启动设置
wlanlogin service restart      # 重启服务
```

`service` 统一管理所选范围的 systemd、launchd、SCM 或用户计划任务。`start`、`stop`、`restart` 只管理已安装服务，未安装时会报错。直接运行 `wlanlogin daemon` 会在当前进程运行自动登录循环，需要提前在同一范围保存凭据；macOS 的手动前台 daemon 也不会申请定位权限。

后台只在连接 `Student_CX` 且门户明确返回离线时提交认证，未知状态不自动登录。确认在线后每 4 分钟检查一次，登录成功后先在 30 秒后确认状态；新的 Wi-Fi 会话立即触发检查。首次取得状态前的临时网络错误按 1、2、4 秒最多快速重试三次，之后按 30、60、120 秒退避，最多等待 2 分钟。

手动注销会暂停当前连接的自动登录。恢复时需在同一范围手动登录，或重新接入目标 Wi-Fi；重启服务会保留同一连接的暂停状态。

### 日志与排障

```sh
wlanlogin service logs          # 最近 100 行
wlanlogin service logs -n 200   # 最近 200 行
wlanlogin service logs -f       # 持续查看，Ctrl+C 退出
wlanlogin service logs -n 0 -f  # 只查看新日志
```

日志读取所选范围配置目录中的 `daemon.log`；同一应用 ID 下的旧 Linux 服务若未配置应用日志，则读取 journal。服务停止后仍可查看已有日志，未安装时会报错。系统日志可能需要 sudo 或管理员权限。

日志使用英文事件文本、UTC RFC 3339 时间和结构化字段，默认级别为 `info`。前台诊断可使用：

```sh
RUST_LOG=wlanlogin=debug wlanlogin --user daemon
```

该环境变量只影响本次启动的进程，已安装服务需要在服务环境中配置。后台启动时若日志已超过 2 MiB，会清空该文件；不提供持续轮转或历史归档。

`retry_after_secs` 表示下次重新检查的等待时间，不保证届时提交登录。`Network check completed` 的 `outcome` 表示检查结果，例如 `online`、`waiting_for_target_wifi`、`automatic_login_paused`、`unknown_portal_state`；`next_check_secs` 为下次检查间隔。日志记录首次结果、变化和错误恢复，相同结果不会在每次轮询重复输出。

| 现象 | 处理方式 |
| --- | --- |
| `command not found` / 无法识别命令 | 检查程序路径、符号链接和 PATH，或用完整路径执行 |
| 仅保存凭据，没有认证 | 确认已连接 `Student_CX`，macOS 检查定位授权 |
| 密钥管理器不可用或需要交互 | 解锁凭据存储，在同一范围进行前台授权；macOS 参照上文权限恢复步骤 |
| 已在线，但新账号未生效 | 先注销再登录 |
| 服务未安装或安装不完整 | 在同一范围执行 `service status`，必要时重新 `service install` |
| 显式登录后后台未恢复 | 确认两者使用相同的 `--user` / `--system` 范围 |
| 认证被拒绝后不再自动重试 | 用 `login --interactive --remember-me` 更新凭据 |
| macOS 已连接 Wi-Fi 却无法读取 SSID | 检查全局定位服务及 WLAN Login 授权，修复后重新安装或重启服务 |

问题反馈请附平台、版本、安装范围、复现步骤及脱敏日志。协议细节见 [状态查询](docs/eportal-status.md)、[登录](docs/eportal-login.md) 和 [注销](docs/eportal-logout.md)。

### 注销与卸载

```sh
wlanlogin logout               # 注销并暂停当前连接的自动登录
wlanlogin logout --forget      # 注销、停止后台进程并删除所选范围的凭据
wlanlogin service uninstall            # 停止并卸载服务，保留凭据和配置
wlanlogin service uninstall --purge    # 卸载服务并清理应用数据、日志和凭据
```

`logout --forget` 保留服务安装及线路配置。未连接校园网时只清理本地凭据；门户注销失败或无法识别 Wi-Fi 时仍尝试删除凭据，但以非零退出码说明注销未完成。凭据删除失败同样报错，并保持自动登录暂停。以后可通过 `login --remember-me` 保存新凭据，再用 `service start` 恢复。

`service uninstall --purge` 清理服务注册、后台程序副本、应用管理的数据及凭据。停止或注销服务失败时不会继续删除数据；凭据删除失败时保留剩余文件，可修复原因后重试。它只作用于所选范围，不跨用户清理。

```sh
wlanlogin --user service uninstall --purge
sudo wlanlogin --system service uninstall --purge  # Linux / macOS 系统范围
```

Windows 系统范围在管理员终端执行 `wlanlogin --system service uninstall --purge`。macOS 服务停止、重启及卸载会等待托管应用退出，单独手动运行的前台 daemon 不受 `service stop` 影响；卸载完整包前也会停止 Wi-Fi 宿主。

macOS `service stop` 只停止当前会话的服务，保留原生注册及后续自动启动；`service uninstall` 会注销后台服务并等待注销完成，再删除安装副本。系统设置的后台列表可能需要重新进入页面后更新。`--purge` 额外删除所选范围的凭据和应用数据，不重置系统定位授权。

原始下载的 CLI、手动复制到 Applications 的应用包及 PATH 符号链接需要自行移除。`--purge` 保留其他文件、共享的 `cxcs-tools` 父目录及同级工具；系统服务审计记录由操作系统保留策略管理。macOS 包内存在未知文件或符号链接时会报错并保留它们。

### 升级

服务管理命令已从 `daemon start/stop/restart/status/logs` 和顶层 `install/uninstall` 迁到 `service` 下；`daemon` 专用于当前进程的自动登录循环。

同名新版使用新二进制或新应用包重新执行同一范围的 `service install`。macOS 应先用原来的固定证书签名，再更新 Applications 中的源应用并安装；ad-hoc 签名或更换证书可能需要重新授权。

macOS 使用新应用包重新 `service install` 时，会自动迁移同一应用 ID 的旧 LaunchAgent / LaunchDaemon，停止旧进程、移除外部启动定义并改用原生注册；迁移失败会尝试恢复旧应用和注册。新包必须通过 `cargo xtask bundle macos` 构建，包含签名内的启动定义。

从旧命令 `cxxy-wlanlogin-cli` 升级时，服务、配置和凭据不会自动迁移，也不提供旧命令或旧环境变量别名。保留旧版程序，先按原安装范围卸载：

```sh
cxxy-wlanlogin-cli --user uninstall --purge
sudo cxxy-wlanlogin-cli --system uninstall --purge  # Linux / macOS 系统范围
```

Windows 原系统安装在管理员终端执行 `cxxy-wlanlogin-cli --system uninstall --purge`。仅选择实际安装范围；没有服务但保存过旧凭据时，也可用旧版在对应范围执行清理。

然后在新程序的同一范围重新保存凭据并安装：

```sh
wlanlogin --user login --interactive --remember-me
wlanlogin --user service install
```

系统安装使用 `--system` 并提供管理员权限。自动化脚本同步改用 `wlanlogin`、`WLANLOGIN_USERNAME`、`WLANLOGIN_PASSWORD` 和 `WLANLOGIN_LINE`。

### 数据与安装目录

应用 ID 为 `dev.cxcs.tools.wlanlogin`，应用目录采用 `cxcs-tools/wlanlogin` 两级结构。以下为默认路径；Linux 用户目录遵循 XDG 环境变量，Windows 使用系统 Known Folder 路径。

| 平台 | 用户配置、日志与运行文件 | 系统配置与运行文件 |
| --- | --- | --- |
| Linux | `~/.config/cxcs-tools/wlanlogin/` | `/var/lib/cxcs-tools/wlanlogin/`、`/run/cxcs-tools/wlanlogin/`；凭据文件位于 `/etc/cxcs-tools/wlanlogin/` |
| macOS | `~/Library/Application Support/cxcs-tools/wlanlogin/` | `/Library/Application Support/cxcs-tools/wlanlogin/state/`、`/var/run/cxcs-tools/wlanlogin/`；凭据在 System Keychain |
| Windows | `%APPDATA%\cxcs-tools\wlanlogin\config\` | `%ProgramData%\cxcs-tools\wlanlogin\` 下的 `state`、`run`、`secrets` |

| 平台 | 用户后台程序副本 | 系统后台程序副本 |
| --- | --- | --- |
| Linux | `~/.local/share/cxcs-tools/wlanlogin/bin/wlanlogin-daemon` | `/usr/local/libexec/cxcs-tools/wlanlogin/wlanlogin-daemon` |
| macOS | `~/Library/Application Support/cxcs-tools/wlanlogin/wlanlogin.app` | `/Library/Application Support/cxcs-tools/wlanlogin/wlanlogin.app` |
| Windows | `%LOCALAPPDATA%\cxcs-tools\wlanlogin\data\bin\wlanlogin-daemon.exe` | `%ProgramFiles%\cxcs-tools\wlanlogin\wlanlogin-daemon.exe` |

macOS 实际后台可执行文件位于上述应用包的 `Contents/MacOS/wlanlogin`，钥匙串访问控制需要授权该实际安装程序。

| 用途 | 名称 |
| --- | --- |
| Linux systemd unit | `dev.cxcs.tools.wlanlogin.service` |
| macOS launchd Label / plist | `dev.cxcs.tools.wlanlogin` / `dev.cxcs.tools.wlanlogin.plist` |
| Windows 系统服务 | `dev.cxcs.tools.wlanlogin` |
| Windows 用户计划任务 | `dev.cxcs.tools.wlanlogin-{SID}` |
| 用户凭据 service / entry | `dev.cxcs.tools.wlanlogin` / `saved-login` |
| macOS 系统凭据 service / account | `dev.cxcs.tools.wlanlogin.system` / `saved-login` |

## 参与贡献

问题与建议请提交到 [Issues](https://github.com/SEUCXCS/wlan-auto-login/issues)，代码贡献请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。`crates/wlan-api`（`wlanlogin-lib`）负责门户协议，`crates/wlan-app`（`wlanlogin-cli`）负责应用及系统集成，`xtask` 负责 macOS 打包；模块职责与平台验证要求见贡献指南。自动化编码代理的工作约定见 [AGENT.md](AGENT.md)。

开发验证使用模拟门户、临时目录、模拟服务命令及隔离 IPC，不访问真实校园网或操作已安装服务、真实凭据。部分测试需要本地 socket 和子进程权限，Linux 测试还需要 `dbus-daemon`。在仓库根目录执行：

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --workspace
git diff --check
```

每个完整功能单独提交，提交信息遵循 Conventional Commits。交叉编译通过只代表编译检查，不代表目标操作系统的运行验证。

## 许可证

本项目采用 [MIT License](LICENSE)。
