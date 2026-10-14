# WLAN Auto Login

`wlanlogin` 是东南大学成贤学院 `Student_CX` 校园 Wi-Fi 的自动登录工具，可在 Linux、macOS 和 Windows 上运行。程序提供命令行界面（CLI）和终端交互界面（TUI），用于登录、注销和保存凭据。后台服务在连接校园 Wi-Fi 后检查认证状态，并在门户确认离线时重新登录。

[下载 Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) · [提交问题](https://github.com/SEUCXCS/wlan-auto-login/issues)

## 目录

- [WLAN Auto Login](#wlan-auto-login)
  - [目录](#目录)
  - [安装](#安装)
    - [依赖](#依赖)
    - [安装范围](#安装范围)
    - [Linux](#linux)
    - [macOS](#macos)
      - [获取应用包](#获取应用包)
      - [创建签名证书](#创建签名证书)
      - [签名与验证](#签名与验证)
      - [安装命令入口](#安装命令入口)
    - [Windows](#windows)
    - [升级](#升级)
      - [更新当前命令](#更新当前命令)
  - [使用](#使用)
    - [命令行](#命令行)
    - [登录与保存凭据](#登录与保存凭据)
    - [参数与自动化](#参数与自动化)
    - [权限查询与获取](#权限查询与获取)
    - [后台服务](#后台服务)
    - [注销与卸载](#注销与卸载)
  - [构建](#构建)
    - [Linux 源码构建](#linux-源码构建)
    - [macOS 应用打包](#macos-应用打包)
    - [Windows 源码构建](#windows-源码构建)
  - [排障](#排障)
    - [日志](#日志)
    - [macOS 授权](#macos-授权)
    - [常见问题](#常见问题)
  - [数据与安装目录](#数据与安装目录)
  - [贡献](#贡献)
  - [许可证](#许可证)

## 安装

[Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 提供预构建附件，Linux 和 Windows 可直接使用，macOS 需要对下载的 `wlanlogin.app` 完成签名（具体详见下方安装教程）。当然，你也可以自行[构建](#构建)程序。

### 依赖

运行依赖如下：

| 平台 | Wi-Fi 识别 | 后台服务 | 用户凭据 | 系统凭据 |
| --- | --- | --- | --- | --- |
| Linux | NetworkManager、D-Bus | systemd 用户服务或系统服务 | Secret Service，例如 GNOME Keyring、KWallet | `systemd-creds` 加密凭据 |
| macOS | CoreWLAN、SystemConfiguration、定位授权 | `SMAppService` LaunchAgent 或 LaunchDaemon | 用户 Keychain | System Keychain |
| Windows | Native Wi-Fi API | Task Scheduler 或 Service Control Manager | Credential Manager | 机器 DPAPI |

Linux 用户范围保存凭据时，Secret Service 应已启动并解锁。Linux 系统范围默认使用 `systemd-creds`，所用 systemd 应提供这个命令。

macOS 最低系统版本为 macOS 13.0。

源码构建还使用以下工具。安装方法见[构建](#构建)。

- Rust、Cargo 和 Git。
- Linux：C 工具链、`pkg-config` 和 D-Bus 开发库。
- macOS：Xcode Command Line Tools 和原生 Rust 工具链。
- Windows：与 Rust target 对应的 MSVC 或 GNU 构建工具。

### 安装范围

| 范围 | 启动时机 | 安装方式 |
| --- | --- | --- |
| 用户 `--user` | 当前用户登录后 | 在普通用户会话中执行 `wlanlogin --user service install` |
| 系统 `--system` | 随开机启动 | Linux 和 macOS 使用 sudo；Windows 使用管理员终端 |

普通终端默认选择用户范围。sudo 或管理员终端默认选择系统范围。系统范围的登录、注销和服务启停要求管理员权限。

`service install` 保存后台服务所用的凭据，并安装、启动服务。缺少已保存的凭据时，程序打开登录表单并启用 Remember me。不在 `Student_CX` 时，程序先保存凭据，接入校园 Wi-Fi 后再认证。

切换范围时，先卸载原服务，再在新范围保存凭据并安装。程序为后台服务安装专用副本。更新原始 CLI 文件不会自动更新后台副本。

<a id='quick-linux'></a>

### Linux

1. 下载与 CPU 架构匹配的 Linux 附件并解压或[从源码构建](#build-linux)。二进制产物名为 `wlanlogin`。

2. 在程序所在目录安装命令。

   ```sh
   mkdir -p "$HOME/.local/bin"
   install -m 0755 ./wlanlogin "$HOME/.local/bin/wlanlogin"
   export PATH="$HOME/.local/bin:$PATH"
   ```

3. 在普通终端安装用户后台服务。

   ```sh
   wlanlogin --user service install
   ```

   按提示填写账号、密码和线路。未连接校园 Wi-Fi 时也可提前配置。

4. 查看服务状态。

   ```sh
   wlanlogin --user service status
   ```

5. 将 PATH 设置加入 shell 配置文件。

   将 `export PATH="$HOME/.local/bin:$PATH"` 加入 Bash 的 `~/.bashrc` 或 Zsh 的 `~/.zshrc`。

如果选择系统范围，用下面的命令替代第 3 步：

```sh
sudo "$HOME/.local/bin/wlanlogin" --system service install
```

查看系统服务状态时也使用同一范围：

```sh
sudo "$HOME/.local/bin/wlanlogin" --system service status
```

旧版 Linux 系统可显式选择 `protected-file` 凭据存储。

> [!WARNING]
> 仅在接受明文存储时选择 `protected-file`。账号和密码会保存在仅管理员可读的文件中。

```sh
sudo "$HOME/.local/bin/wlanlogin" --system service install --credential-store protected-file
```

`protected-file` 只用于 Linux 系统范围。默认加密模式失败时，程序不会自动切换为明文存储。macOS 和 Windows 不提供这个选项。

<a id='quick-macos'></a>

### macOS

#### 获取应用包

1. 从 [Releases](https://github.com/SEUCXCS/wlan-auto-login/releases) 下载并解压完整的 `wlanlogin.app`。在 Finder 中将应用包放入用户应用目录 `~/Applications`，目录不存在时先创建。也可按 [macOS 应用打包](#build-macos)构建并复制到同一目录。

2. 在终端设置已安装的应用包路径。

   ```sh
   export WLANLOGIN_APP="$HOME/Applications/wlanlogin.app"
   ```

预构建应用可能未签名，也可能使用 ad-hoc 签名。两者均可用下文的 `codesign --force` 在本机进行签名。

<a id='macos-signing'></a>

#### 创建签名证书

已有固定代码签名证书时，使用原证书并替换下文的 `wlanlogin-dev`。

1. 打开“钥匙串访问”，选择“登录”钥匙串。
2. 选择“钥匙串访问 → 证书助理 → 创建证书”。
3. 将名称设为 `wlanlogin-dev`。
4. 将身份类型设为“自签名根证书”。
5. 将证书类型设为“代码签名（Code Signing）”。
6. 按向导创建证书并存入“登录”钥匙串。
7. 打开证书详情，将“代码签名”信任设为“始终信任”。
8. 按系统提示输入密码并保存。
9. 展开证书，确认存在对应私钥。
10. 检查可用签名身份。

    ```sh
    security find-identity -v -p codesigning
    ```

输出应包含 `wlanlogin-dev`。如果输出为 `0 valid identities found`，检查证书用途、信任设置、有效期和私钥。确认“登录”钥匙串已解锁。

保留证书和私钥供升级时使用。重新生成同名证书不能保留原签名身份。

#### 签名与验证

1. 如果尚未安装 Xcode Command Line Tools，安装工具。

   ```sh
   xcode-select --install
   ```

2. 确认 `WLANLOGIN_APP` 指向已复制到 `~/Applications/wlanlogin.app` 的完整应用包。
3. 使用固定证书签名。

   ```sh
   codesign --force --sign "wlanlogin-dev" \
     --identifier dev.cxcs.tools.wlanlogin \
     --options runtime --timestamp=none "$WLANLOGIN_APP"
   ```

   如果系统询问私钥访问权限，核对证书后允许 `codesign` 访问。

4. 验证应用签名。

   ```sh
   codesign --verify --deep --strict --verbose=2 "$WLANLOGIN_APP"
   codesign -dv --verbose=4 "$WLANLOGIN_APP"
   ```

`--timestamp=none` 让本地签名不依赖 Apple 时间戳服务。固定证书有助于升级时保留定位和钥匙串授权，但系统仍可能再次询问权限。

本地签名不等同于 Developer ID 签名或 Apple 公证。如果 Gatekeeper 阻止启动，确认应用来源后，打开“系统设置 → 隐私与安全性”，选择“仍要打开”。

#### 安装命令入口

完成签名后，创建指向应用包内程序的 CLI 符号链接，使 CLI 和 Wi-Fi 授权宿主共用 `~/Applications/wlanlogin.app`。签名后不修改包内文件。

1. 在 `~/.local/bin` 创建命令入口。

   ```sh
   mkdir -p "$HOME/.local/bin"
   ln -sf "$HOME/Applications/wlanlogin.app/Contents/MacOS/wlanlogin" \
     "$HOME/.local/bin/wlanlogin"
   export PATH="$HOME/.local/bin:$PATH"
   ```

> 如需供其他本机用户使用，可将已签名的应用包复制到 `/Applications`，并在 `/usr/local/bin` 创建命令入口：
>
> ```sh
> sudo ditto "$WLANLOGIN_APP" "/Applications/wlanlogin.app"
> sudo mkdir -p /usr/local/bin
> sudo ln -sf "/Applications/wlanlogin.app/Contents/MacOS/wlanlogin" \
>   /usr/local/bin/wlanlogin
> export PATH="/usr/local/bin:$PATH"
> ```

2. 将所选 PATH 设置加入 `~/.zshrc`。
3. 在已登录桌面的普通终端安装用户后台服务。

   ```sh
   wlanlogin --user service install
   ```

4. 按提示填写账号、密码和线路。
5. 按提示完成系统授权。

   - 定位服务：允许 CXCS WLAN Login 的授权宿主读取 Wi-Fi 名称。
   - 钥匙串访问：核对请求来自 `wlanlogin` 后允许访问；可选择“始终允许”。
   - 后台活动：允许 CXCS WLAN Login 在后台运行；设置位置随系统版本变化。

6. 查看服务状态。

   ```sh
   wlanlogin --user service status
   ```

`service install` 将完整应用包复制到所选范围的 Application Support 目录，然后注册并启动服务。双击 `.app` 或无参数运行不会自动安装服务。

如果选择系统范围，在当前桌面用户的终端执行：

```sh
sudo "$HOME/.local/bin/wlanlogin" --system service install
```

如果命令入口位于 `/usr/local/bin`，使用 `sudo wlanlogin --system service install`。应用放在 `/Applications` 不代表系统服务已安装。root 不能绕过当前桌面用户的定位授权。

macOS 安装会校验完整应用包。替换旧副本后，程序检查凭据读取和 Wi-Fi 能力，再注册、启动服务。失败时，程序尝试恢复旧安装。回滚未完成时，程序保留恢复目录并报告位置。

系统服务仍从当前活动桌面用户的授权宿主获取 Wi-Fi 状态。没有桌面用户、用户切换或定位未授权时，服务等待有效状态。无桌面会话的非交互安装之后，应在登录用户的终端执行 `sudo wlanlogin --system login`，完成定位授权。

首次注册或此前关闭后台活动时，系统可能要求在“系统设置 → 通用 → 登录项”允许 CXCS WLAN Login。系统范围的后台服务要求管理员批准。`service install` 会保留安装和凭据，并提示等待批准。批准后，在同一范围执行 `service start`。

定位权限与后台活动批准相互独立。卸载、重装或移动同一签名应用不会自动重置定位授权。Apple 软件开发工具包（SDK）将包含原生 LaunchDaemon 的应用公证列为要求。本地自签名包不能保证系统范围可用；原生注册被拒绝时，程序报告错误并尝试回滚。

<a id='quick-windows'></a>

### Windows

1. 下载与 CPU 架构匹配的 Windows 附件并解压。

   解压后的程序名为 `wlanlogin.exe`。若没有对应附件，先[从源码构建](#build-windows)。

2. 在程序目录打开普通 PowerShell，安装用户后台任务。

   ```powershell
   .\wlanlogin.exe --user service install
   ```

3. 按提示填写账号、密码和线路。
4. 查看后台任务状态。

   ```powershell
   .\wlanlogin.exe --user service status
   ```

5. 如果要在其他目录使用命令，将程序目录加入 PATH。

   设置后打开新终端。

用户凭据保存到 Credential Manager。后台任务在当前用户登录后启动。

如果选择系统范围，在程序目录打开管理员 PowerShell，执行：

```powershell
.\wlanlogin.exe --system service install
.\wlanlogin.exe --system service status
```

系统范围使用机器 DPAPI 保存凭据，后台服务随开机启动。用户与系统范围选择一个即可。

### Linux 桌面会话与钱包

用户 daemon 仍随用户服务启动；启动本身不读取钱包。连接到 Student_CX、自动登录未暂停、门户明确离线且确实需要凭据时，才进行后台读取。未保存凭据、钱包锁定或服务未就绪时，daemon 继续等待；服务上线或钱包解锁会触发重试，另有定时重试。后台不会弹出解锁或授权窗口；自动登录 GNOME 后钱包未解锁时，可通过前台 `wlanlogin --user login` 或桌面钱包管理器解锁。

Linux 用户后台读取统一先检查活动用户会话、显示 socket 和激活环境，包括凭据服务已经运行的情况。确认就绪后，程序使用官方库访问标准 `org.freedesktop.secrets`，由 D-Bus 按标准机制启动或重启服务。这是通用的 Linux 会话初始化顺序，适用于实现 Secret Service 的桌面钱包。系统范围的凭据读取不依赖图形会话。

常见桌面通常维护标准会话目标与环境，无需额外设置。未使用标准目标的自定义桌面可在图形会话就绪后执行：

```sh
wlanlogin --user service session-ready
```

它将当前会话的显示环境同步给 D-Bus / systemd，并向已运行 daemon 发送确认通知；注销前可执行 `wlanlogin --user service session-end`。通知只保存在 daemon 内存中，并绑定实际登录会话；每次读取前检查会话存活，读取后再次核对会话身份。重启 daemon 后自定义桌面需重新发送就绪通知。

`service status` 显示暂未读取、等待会话／凭据服务／钱包解锁、需要前台交互、权限被拒绝、未保存、可用或读取失败。读取失败区分超时、格式损坏、重复条目、解密及任务失败，并显示阶段、诊断码和恢复提示。daemon 持续等待，不因这些失败退出或删除凭据；未识别的错误不会被判为权限问题。查询状态不会读取或启动钱包。Linux 系统范围使用自身的 systemd 加密凭据或显式 protected-file，不依赖桌面钱包。

### 升级

#### 更新当前命令

1. 获取新版二进制文件或完整应用包。

   macOS 按[安装教程](#quick-macos)将新版应用包复制到原来的 `~/Applications/wlanlogin.app`，并使用原来的固定证书签名。使用 `/Applications/wlanlogin.app` 的用户，再将已签名的新版应用包复制到该位置。保持符号链接的目标路径一致。ad-hoc 签名或更换证书可能要求重新授权。

2. 在原安装范围重新执行 `service install`。

   ```sh
   wlanlogin --user service install
   ```

   系统范围使用 `--system` 和管理员权限。

## 使用

### 命令行

以下示例使用已加入 PATH 的 `wlanlogin`。Windows 未配置 PATH 时，在程序目录将命令替换为 `.\wlanlogin.exe`。

```sh
wlanlogin --version
wlanlogin --help
wlanlogin login --help
wlanlogin service --help
wlanlogin permission --help
wlanlogin daemon --help
```

| 命令 | 用途 |
| --- | --- |
| `login` | 登录，或离线保存凭据 |
| `logout` | 注销当前校园网会话 |
| `permission list/check/get` | 查询或获取当前平台支持的权限 |
| `service install` | 保存凭据并安装自动启动服务 |
| `daemon` | 在当前进程运行自动登录循环 |
| `service start/stop/restart/status/logs` | 管理程序自行安装的服务 |
| `service uninstall` | 卸载程序自行安装的服务 |

### 登录与保存凭据

```sh
wlanlogin login
wlanlogin login --remember-me
wlanlogin login --interactive
wlanlogin login --interactive --remember-me
```

`login` 使用已保存的凭据；缺少凭据时打开 TUI。`--interactive` 强制打开表单，`--remember-me` 保存账号和密码。

表单包含账号、密码、线路和 Remember me：

- Tab：切换控件。
- 方向键：选择线路。
- 空格：切换复选框。
- Esc：取消。

程序只在识别到 `Student_CX` 时提交认证。其他网络或无法读取 Wi-Fi 名称（SSID）时，程序仍可离线保存凭据。

已认证在线时，`login` 提示已在线，不重复提交凭据。本次填写的账号和线路不会因此视为已通过认证。`--remember-me` 仍可保存新凭据供后续使用。

如果要切换账号或线路，先注销，再登录：

```sh
wlanlogin logout
wlanlogin login --interactive --remember-me
```

系统范围使用相同范围和管理员权限。

### 参数与自动化

| 参数 | 环境变量 | 用途 |
| --- | --- | --- |
| `--username` 或 `-u` | `WLANLOGIN_USERNAME` | 校园网账号 |
| `--password` 或 `-p` | `WLANLOGIN_PASSWORD` | 密码 |
| `--password-stdin` | — | 从标准输入读取一行密码，与 `--password` 互斥 |
| `--line` | `WLANLOGIN_LINE` | `free1`（免费 1）、`free2`（免费 2）或 `campus`（内网） |
| `--remember-me` | — | 保存凭据 |
| `--interactive` | — | 强制打开终端表单 |
| `--non-interactive` | — | 凭据不足时报错，不打开表单 |
| `--credential-store` | — | Linux 系统范围的 `native` 或 `protected-file` 存储 |

自动化时，宜通过标准输入传入密码，并使用 `--non-interactive`。`--password` 可能把密码留在 shell 历史或进程参数列表中。

macOS 非交互命令和后台服务不会主动申请系统授权。首次使用前，在桌面会话中完成交互式授权。

### 权限查询与获取

`permission` 在所有平台提供统一入口，实际支持的权限由平台决定。Linux 和 Windows 当前没有权限实现，`list` 返回空列表，未指定 ID 的 `check` 返回空结果，不触发系统授权；指定不支持的权限 ID 时返回错误。

macOS 支持所选范围的定位和钥匙串访问诊断：

```sh
wlanlogin permission list
wlanlogin --user permission check
wlanlogin --user permission check macos.keychain --json
wlanlogin --user permission get macos.location
sudo wlanlogin --system permission get macos.keychain
```

`list` 列出权限 ID 和说明。`check [ID]` 查询指定权限，省略 ID 时查询全部；不弹授权窗口。`get <ID>` 在交互式终端中请求指定权限，并返回实际结果；拒绝、需要交互或未授权时以非零退出码结束。`check` 成功取得状态时返回零，即使状态是 Missing、Denied 或 NeedsInteraction；无法完成查询时返回非零。定位宿主尚未运行或无法连接时，报告 Unavailable 及原因，可通过 `permission get macos.location` 启动宿主并按需申请授权。

`list`、`check`、`get` 均支持 `--json`。状态输出包含 `version`、`scope` 和 `permissions`，每项包含 `id`、`status`、`error`；不会输出账号密码。钥匙串检查的是当前程序对所选范围条目的读取能力，安装流程会另外启动实际安装副本复检。Missing 表示尚未保存凭据，获取权限不会创建凭据条目。

### 后台服务

程序自行安装的服务使用以下命令：

```sh
wlanlogin --user service status
wlanlogin --user service start
wlanlogin --user service stop
wlanlogin --user service restart
```

`start`、`stop` 和 `restart` 只管理已安装服务。未安装时，命令报错。系统服务使用 `--system` 和管理员权限。

直接运行 `wlanlogin --user daemon` 会在当前进程运行自动登录循环。启动前，先在同一范围保存凭据。macOS 的前台 daemon 也不会申请定位权限。

后台服务只在连接 `Student_CX` 且门户确认离线时提交认证。门户状态未知时，服务不会自动登录。

后台读取超时后会继续保留同一个读取任务；任务稍后完成时，程序会重新检查其许可，提交登录前再检查一次。Linux 用户范围的结果绑定 logind 会话 ID、启动时间及本地会话代数；注销、重新登录或会话变更后，旧结果会丢弃并等待重新读取。系统范围不依赖桌面会话；macOS 和 Windows 保持原生钥匙串／凭据管理器的后台访问规则。

所有平台的结果都绑定所选范围、配置目录和凭据代数。常规保存或删除凭据前先持久化新的代数，失败则停止存储操作；即使存储操作随后失败，旧读取结果也会失效。

检查间隔如下：

- 确认在线后，每 4 分钟检查一次。
- 登录成功后，先在 30 秒后确认状态。
- 新的 Wi-Fi 会话立即触发检查。
- 首次取得状态前的临时网络错误，按 1、2、4 秒最多快速重试 3 次。
- 后续错误按 30、60、120 秒退避，最多等待 2 分钟。

手动注销会暂停当前连接的自动登录。在同一范围手动登录，或重新接入目标 Wi-Fi，可以恢复自动登录。重启服务会保留同一连接的暂停状态。

### 注销与卸载

```sh
wlanlogin logout
wlanlogin service uninstall
```

`logout` 注销并暂停当前连接的自动登录。`service uninstall` 停止并卸载程序自行安装的服务，保留凭据和配置。

**警告：** 先记录仍要使用的凭据，再执行 `logout --forget`。命令会删除所选范围的账号和密码。

```sh
wlanlogin logout --forget
```

`logout --forget` 停止后台进程，但保留服务安装及线路配置。未连接校园网时，命令只清理本地凭据。门户注销失败或无法识别 Wi-Fi 时，命令仍尝试删除凭据，并以非零退出码报告注销未完成。凭据删除失败时，命令也报错，并保持自动登录暂停。

恢复时，用 `login --remember-me` 保存新凭据，再用 `service start` 启动同一范围的服务。

**警告：** 先保存仍要保留的配置和凭据，再执行 `service uninstall --purge`。命令会删除所选范围的服务注册、后台副本、应用数据和凭据。

```sh
wlanlogin --user service uninstall --purge
sudo wlanlogin --system service uninstall --purge
```

第 2 条命令用于 Linux 和 macOS。Windows 系统范围在管理员终端执行 `wlanlogin --system service uninstall --purge`。

停止或注销服务失败时，程序不会继续删除数据。凭据删除失败时，程序保留剩余文件。修复原因后可重试。清理只作用于所选范围，不跨用户删除。完整卸载确认后台进程停止后直接清理凭据，不要求待删除的 `control.json` 能被解析。

macOS 的停止、重启和卸载会等待托管应用退出。`service stop` 只停止当前会话的服务，保留原生注册和后续自动启动。单独运行的前台 daemon 不受 `service stop` 影响。

macOS `service uninstall` 注销后台服务，等待注销完成，再删除安装副本。卸载完整应用包前，程序也会停止 Wi-Fi 宿主。`--purge` 不重置系统定位授权。系统设置中的后台列表可能在重新打开页面后才更新。

原始下载文件、手动复制到 Applications 的应用包和 PATH 符号链接应自行移除。清理保留共享的 `SEUCXCS` 父目录及同级工具。系统服务审计记录按操作系统策略保留。macOS 包内存在未知文件或符号链接时，程序报错并保留这些文件。

## 构建

项目使用 Cargo workspace、Rust 2024 edition 和 resolver 3。仓库未固定 Rust 工具链，也未声明最低 Rust 版本（MSRV）。edition 的最低版本不能代表整个依赖树的最低版本。

构建使用已提交的 `Cargo.lock`，不必手动更新依赖。除另有说明外，以下命令在仓库根目录执行。

<a id='build-linux'></a>

### Linux 源码构建

1. 安装原生构建依赖。

   Debian 或 Ubuntu：

   ```sh
   sudo apt update
   sudo apt install git build-essential pkg-config libdbus-1-dev
   ```

   Arch Linux:

   ```sh
   sudo pacman -S --needed git base-devel rust pkgconf dbus
   ```

   已有 rustup 工具链时，可省略 Arch 命令中的 `rust`。其他发行版安装对应软件包。

2. 如果尚未安装 Rust，安装 [Rust 与 Cargo](https://www.rust-lang.org/tools/install)。

3. 如果尚未获取源码，克隆仓库。

   ```sh
   git clone https://github.com/SEUCXCS/wlan-auto-login.git
   cd wlan-auto-login
   ```

4. 构建可执行文件。

   ```sh
   cargo build --release --locked -p wlanlogin-cli --bin wlanlogin
   ```

   产物为 `target/release/wlanlogin`。

5. 将产物安装到用户命令目录。

   ```sh
   mkdir -p "$HOME/.local/bin"
   install -m 0755 target/release/wlanlogin "$HOME/.local/bin/wlanlogin"
   ```

<a id='build-macos'></a>

### macOS 应用打包

1. 如果尚未安装 Xcode Command Line Tools，安装工具。

   ```sh
   xcode-select --install
   ```

2. 安装 [Rust 与 Cargo](https://www.rust-lang.org/tools/install)。
3. 如果尚未获取源码，克隆仓库。

   ```sh
   git clone https://github.com/SEUCXCS/wlan-auto-login.git
   cd wlan-auto-login
   ```

4. 生成 ad-hoc 签名应用包。

   ```sh
   cargo xtask bundle macos --codesign-identity=-
   ```

5. 将完整应用包复制到用户应用目录，并设置后续使用的路径。

   ```sh
   mkdir -p "$HOME/Applications"
   ditto "$PWD/target/release/bundle/wlanlogin.app" "$HOME/Applications/wlanlogin.app"
   export WLANLOGIN_APP="$HOME/Applications/wlanlogin.app"
   ```

6. 按[创建签名证书](#macos-signing)及后续步骤完成签名、命令入口和后台服务安装。

可指定构建配置、架构和输出目录：

```sh
rustup target add aarch64-apple-darwin
cargo xtask bundle macos --codesign-identity=- \
  --profile dev --target aarch64-apple-darwin
mkdir -p "$HOME/Applications"
ditto "$PWD/target/aarch64-apple-darwin/debug/bundle/wlanlogin.app" \
  "$HOME/Applications/wlanlogin.app"
export WLANLOGIN_APP="$HOME/Applications/wlanlogin.app"
```

Apple Silicon 的 target 为 `aarch64-apple-darwin`，Intel 为 `x86_64-apple-darwin`。指定 target 后，产物路径增加 target 层级。`dev` 配置对应 `debug` 目录。`--target-dir` 或 `CARGO_TARGET_DIR` 可指定输出根目录；以 xtask 输出的 `Bundle:` 路径为准。

应用的 `CFBundleShortVersionString` 保持 Cargo 发布版本；`CFBundleVersion` 使用 xtask 自动分配的递增整数，首次为 `1`。同一工作副本的所有 profile、target 和输出目录共用仓库根目录 `.bundle-build/build-number`，该目录不提交到 Git，`cargo clean` 不会重置计数。构建号在签名前分配，后续打包失败也不复用该编号；xtask 成功时输出 `Build number:`。保留这个目录以延续本地编号，新克隆的工作副本从 `1` 开始。分发同一发布版本的新构建无需修改展示版本。

已有 Developer ID Application 证书时，可直接指定签名身份：

```sh
cargo xtask bundle macos --codesign-identity "Developer ID Application: YOUR NAME (TEAMID)"
```

签名身份依次取自 `--codesign-identity`、`CODESIGN_IDENTITY`，默认使用 `-`（ad-hoc）。显式设置 `--codesign-identity=-` 可覆盖环境变量。

xtask 使用证书签名时启用 hardened runtime，并请求 Apple 时间戳。本地自签名证书可能无法取得时间戳，宜先生成 ad-hoc 包，再用 `--timestamp=none` 签名。

打包失败时，xtask 不会降级签名，也不会替换此前已验证的应用包。打包不包含 Apple 公证或后台服务安装。

<a id='build-windows'></a>

### Windows 源码构建

1. 安装 Git 和 [Rust 与 Cargo](https://www.rust-lang.org/tools/install)。
2. 安装 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)。

   使用 MSVC 工具链时，选择“使用 C++ 的桌面开发”，并安装对应架构的编译工具和 Windows SDK。

3. 在能找到 MSVC 工具的 PowerShell 中克隆仓库。

   ```powershell
   git clone https://github.com/SEUCXCS/wlan-auto-login.git
   cd wlan-auto-login
   ```

4. 构建可执行文件。

   ```powershell
   cargo build --release --locked -p wlanlogin-cli --bin wlanlogin
   ```

   产物为 `target\release\wlanlogin.exe`。

5. 按 [Windows 安装](#quick-windows)配置命令和服务。

如果选择 GNU target，另行安装对应的 MinGW 工具链。仅添加 Rust target 不会安装原生链接工具。

## 排障

### 日志

程序自行安装的服务使用以下命令：

```sh
wlanlogin service logs
wlanlogin service logs -n 200
wlanlogin service logs -f
wlanlogin service logs -n 0 -f
```

默认显示最近 100 行。`-f` 持续查看，Ctrl+C 退出。`-n 0 -f` 只查看新日志。

日志来自所选范围配置目录中的 `daemon.log`。服务停止后仍可查看已有日志，未安装时命令报错。系统范围日志可能要求 sudo 或管理员权限。

日志使用英文事件文本、UTC RFC 3339 时间和结构化字段。默认级别为 `info`。前台诊断可使用：

```sh
RUST_LOG=wlanlogin=debug wlanlogin --user daemon
```

`RUST_LOG` 只影响本次启动的进程。已安装服务从服务环境读取日志级别。后台启动时，如果 `daemon.log` 超过 2 MiB，程序清空日志文件。程序不持续轮转日志，也不保留历史归档。

`retry_after_secs` 表示下次检查前的等待时间，不保证下次检查会提交登录。`Network check completed` 的 `outcome` 表示检查结果：

- `online`：已在线。
- `waiting_for_target_wifi`：等待目标 Wi-Fi。
- `automatic_login_paused`：自动登录已暂停。
- `unknown_portal_state`：门户状态未知。

`next_check_secs` 表示下次检查间隔。日志记录首次结果、状态变化和错误恢复。同一结果不会在每次轮询时重复输出。

### macOS 授权

后台服务和 `--non-interactive` 命令不会主动弹出定位或钥匙串授权框。授权失败不会自动删除已保存的凭据。

定位权限恢复步骤：

1. 打开“系统设置 → 隐私与安全性 → 定位服务”。
2. 开启全局定位服务并启用 CXCS WLAN Login。
3. 如果列表中没有应用，在桌面终端触发交互式授权。

   ```sh
   wlanlogin --user login --interactive
   ```

4. 再次检查定位服务设置。

已允许或拒绝定位后，重复运行命令不会再次弹出首次授权提示。设备受管理策略限制时，由管理员解除限制。

钥匙串权限恢复步骤：

1. 在“钥匙串访问”中解锁“登录”钥匙串。
2. 在桌面终端重新执行同一范围的 `service install`。
3. 核对访问请求后允许授权。
4. 如果仍被拒绝，找到程序的密码条目。
5. 打开“访问控制”，添加实际安装的 `Contents/MacOS/wlanlogin`。
6. 保存设置后，在同一范围重新安装。

用户条目的服务名为 `dev.cxcs.tools.wlanlogin`。系统条目的服务名为 `dev.cxcs.tools.wlanlogin.system`，位于“系统”钥匙串。两者的账户均为 `saved-login`。修改系统条目可能要求管理员密码。程序路径见[数据与安装目录](#数据与安装目录)。

修复授权后，重新执行同一范围的 `service install`。服务已安装时，也可执行 `service restart`，再通过 `service logs -f` 检查。

### 常见问题

| 现象 | 检查或恢复方法 |
| --- | --- |
| `command not found` 或无法识别命令 | 检查路径、符号链接和 PATH，或使用完整路径 |
| 仅保存凭据，没有认证 | 确认已连接 `Student_CX`；macOS 检查定位授权 |
| 凭据存储不可用或要求交互 | 解锁凭据存储，在同一范围完成前台授权 |
| 已在线，但新账号未生效 | 先注销，再登录 |
| 程序自行安装的服务不完整 | 在同一范围执行 `service status`，必要时重新 `service install` |
| 显式登录后后台未恢复 | 确认登录和后台服务使用相同的 `--user` 或 `--system` 范围 |
| 认证被拒绝后不再自动重试 | 用 `login --interactive --remember-me` 更新凭据 |
| macOS 已连接 Wi-Fi，但无法读取 SSID | 修复定位授权后，重新安装或重启服务 |

反馈问题时，附上平台、版本、安装范围、复现步骤和脱敏日志。门户协议说明见[状态查询](docs/eportal-status.md)、[登录](docs/eportal-login.md)和[注销](docs/eportal-logout.md)。

## 数据与安装目录

应用 ID 为 `dev.cxcs.tools.wlanlogin`。应用目录使用 `SEUCXCS/wlanlogin` 两级结构。Linux 用户路径遵循 XDG 环境变量；Windows 路径使用系统 Known Folder。

| 平台 | 用户配置、日志与运行文件 | 系统配置与运行文件 |
| --- | --- | --- |
| Linux | `~/.config/SEUCXCS/wlanlogin/` | `/var/lib/SEUCXCS/wlanlogin/`、`/run/SEUCXCS/wlanlogin/`；凭据位于 `/etc/SEUCXCS/wlanlogin/` |
| macOS | `~/Library/Application Support/SEUCXCS/wlanlogin/` | `/Library/Application Support/SEUCXCS/wlanlogin/state/`、`/var/run/SEUCXCS/wlanlogin/`；凭据在 System Keychain |
| Windows | `%APPDATA%\SEUCXCS\wlanlogin\config\` | `%ProgramData%\SEUCXCS\wlanlogin\` 下的 `state`、`run`、`secrets` |

程序自行安装的后台副本路径如下：

| 平台 | 用户后台副本 | 系统后台副本 |
| --- | --- | --- |
| Linux | `~/.local/share/SEUCXCS/wlanlogin/bin/wlanlogin-daemon` | `/usr/local/libexec/SEUCXCS/wlanlogin/wlanlogin-daemon` |
| macOS | `~/Library/Application Support/SEUCXCS/wlanlogin/wlanlogin.app` | `/Library/Application Support/SEUCXCS/wlanlogin/wlanlogin.app` |
| Windows | `%LOCALAPPDATA%\SEUCXCS\wlanlogin\data\bin\wlanlogin-daemon.exe` | `%ProgramFiles%\SEUCXCS\wlanlogin\wlanlogin-daemon.exe` |

macOS 后台程序位于对应应用包的 `Contents/MacOS/wlanlogin`。钥匙串访问控制授权这个实际安装路径。

| 用途 | 名称 |
| --- | --- |
| Linux systemd unit | `dev.cxcs.tools.wlanlogin.service` |
| macOS launchd Label 与 plist | `dev.cxcs.tools.wlanlogin`、`dev.cxcs.tools.wlanlogin.plist` |
| Windows 系统服务 | `dev.cxcs.tools.wlanlogin` |
| Windows 用户计划任务 | `dev.cxcs.tools.wlanlogin-{SID}` |
| 用户凭据服务名与条目 | `dev.cxcs.tools.wlanlogin`、`saved-login` |
| macOS 系统凭据服务名与账户 | `dev.cxcs.tools.wlanlogin.system`、`saved-login` |

## 贡献

通过 [Issues](https://github.com/SEUCXCS/wlan-auto-login/issues)提问或报告问题。项目接受 Pull Request（PR）。提交前，阅读[贡献指南](CONTRIBUTING.md)。每个完整功能单独提交，提交信息遵循 Conventional Commits。

| 目录 | Cargo 包名或用途 |
| --- | --- |
| `crates/wlan-api` | `wlanlogin-lib`，门户协议 |
| `crates/wlan-app` | `wlanlogin-cli`，CLI、TUI、后台服务和系统集成 |
| `xtask` | macOS 应用打包 |
| `docs` | 门户协议说明 |

开发验证使用模拟门户、临时目录、模拟服务命令和隔离 IPC。不访问真实校园网，不修改已安装服务或真实凭据。部分测试使用本地 socket 和子进程，Linux 测试还使用 `dbus-daemon`。

在仓库根目录执行：

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --workspace
git diff --check
```

交叉编译通过只代表编译检查，不代表目标系统的运行验证。模块职责和平台验证要求见[贡献指南](CONTRIBUTING.md)。自动化编码代理约定见 [AGENT.md](AGENT.md)。

## 许可证

[MIT License](LICENSE) © 2026 SEUCXCS contributors。
