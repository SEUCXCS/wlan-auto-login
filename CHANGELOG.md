# 变更日志

本文件记录项目的重要变更，格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号与 Cargo workspace 版本一致并遵循[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 新增

- Debian / Ubuntu 软件包：`cargo xtask bundle deb` 生成 `.deb`，最低支持 Debian 12 与 Ubuntu 24.04。
- Fedora 软件包：`cargo xtask bundle rpm` 生成 `.rpm`，最低支持 Fedora 43。
- Arch Linux 软件包：`packaging/archlinux/PKGBUILD` 打包当前检出的已提交版本。
- 中文与英文 man 手册 `wlanlogin(1)`，随 Linux 软件包安装。
- 发布流程同时上传 `.deb` 与 Arch Linux 软件包，CI 在推送与 PR 时检查两者的构建。
- 本变更日志。

## [2.0.0] - 2026-11-25

发布标签为 `v2.0.0`；该版本发布的程序误将自身版本报告为 0.2.0。

### 新增

- 可配置的门户诊断日志：`config set log.level debug` 记录脱敏后的完整门户 HTTP 请求与响应 ([#17](https://github.com/SEUCXCS/wlan-auto-login/pull/17))。
- `config get` / `set` / `list` / `remove` 子命令，按作用域读写持久化配置。

### 变更

- 登录与注销改用门户 ePortal v4 接口 ([#16](https://github.com/SEUCXCS/wlan-auto-login/pull/16))。
- 发布产物与 CI 不再包含 macOS Intel (x86_64) 版本。
- 重写 README、门户协议文档与贡献指南。

### 修复

- macOS 后台服务升级中断后可恢复 ([#15](https://github.com/SEUCXCS/wlan-auto-login/pull/15))。
- 门户日志保留 HTTP 响应中的原始消息文本。
- 兼容带填充字符的 JSONP 回调。

## [0.1.1] - 2026-11-11

### 新增

- Windows 安装向导 `wlanlogin-windows-setup-<arch>.exe`：可选择用户或系统范围，安装失败时回滚 ([#13](https://github.com/SEUCXCS/wlan-auto-login/pull/13))。
- `cargo xtask bundle windows` 构建并逐字节校验安装程序；发布流程同时上传安装程序与便携压缩包。
- Linux、Windows、macOS 原生平台 CI 测试 ([#11](https://github.com/SEUCXCS/wlan-auto-login/pull/11))。

### 变更

- 各平台原生实现隔离至独立目录，并由 `cargo xtask check-boundaries` 检查平台源码边界 ([#9](https://github.com/SEUCXCS/wlan-auto-login/pull/9))。

### 修复

- Windows 服务安装与回滚：计划任务改用 UTF-16 编码并在替换前校验，回滚可重复执行，原生命令诊断按系统编码解码 ([#10](https://github.com/SEUCXCS/wlan-auto-login/pull/10))。
- Windows 存储与运行时加固 ([#8](https://github.com/SEUCXCS/wlan-auto-login/pull/8))。
- Windows 安装程序的系统范围安装与维护行为。

## [0.1.0] - 2026-10-14

首个发布版本。

### 新增

- Student_CX 门户登录、注销与状态查询，提供异步协议库 `wlanlogin-lib`。
- 命令行程序 `wlanlogin`：TUI 交互登录、离线保存凭据，以及 `--password-stdin`、`--non-interactive` 等脚本参数。
- 后台守护：Wi-Fi 网络监测、自动登录与退避重试、本地 IPC 控制。
- `service install` / `uninstall` / `start` / `stop` / `restart` / `status` / `logs`，以用户或系统范围管理 systemd、SMAppService 与 Windows 计划任务 / 服务。
- 原生凭据存储：Secret Service、`systemd-creds`、macOS 钥匙串、Windows 凭据管理器与 DPAPI。
- macOS 签名应用包（`cargo xtask bundle macos`），以及定位与钥匙串权限管理（`permission` 子命令）。
- 发布流程构建 Linux、Windows 与 macOS 二进制。

[Unreleased]: https://github.com/SEUCXCS/wlan-auto-login/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/SEUCXCS/wlan-auto-login/compare/v0.1.1...v2.0.0
[0.1.1]: https://github.com/SEUCXCS/wlan-auto-login/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/SEUCXCS/wlan-auto-login/releases/tag/v0.1.0
