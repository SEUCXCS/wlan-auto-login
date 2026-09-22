use clap::{Args, Parser, Subcommand};
use cxxy_wlanlogin_lib::Line;

#[derive(Parser)]
#[command(version, about = "Student_CX 校园网登录工具")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// 登录校园网；不在 Student_CX 时仅保存凭据，凭据不足时打开交互界面
    Login(LoginArgs),
    /// 注销当前校园网会话
    Logout,
    /// 使用已保存凭据在前台运行后台登录循环
    Daemon {
        #[arg(long, hide = true)]
        background: bool,
    },
    /// 安装用户登录后自动启动的服务；首次登录强制保存凭据
    Install(LoginArgs),
    /// 停止并卸载后台服务，保留已保存的账号密码
    Uninstall,
}

#[derive(Args, Default)]
pub struct LoginArgs {
    #[arg(short, long, env = "CXXY_WLAN_USERNAME", hide_env_values = true)]
    pub username: Option<String>,
    #[arg(
        short,
        long,
        env = "CXXY_WLAN_PASSWORD",
        hide_env_values = true,
        conflicts_with = "password_stdin"
    )]
    pub password: Option<String>,
    /// 从标准输入读取一行密码
    #[arg(long)]
    pub password_stdin: bool,
    #[arg(long, env = "CXXY_WLAN_LINE", value_name = "free1|free2|campus")]
    pub line: Option<Line>,
    /// 登录成功后将账号和密码保存至系统密钥管理器
    #[arg(long)]
    pub remember_me: bool,
    /// 即使已有保存的凭据，也打开 TUI 以修改账号、密码或线路
    #[arg(long, alias = "tui", conflicts_with = "non_interactive")]
    pub interactive: bool,
    /// 缺少凭据时直接报错，不打开 TUI
    #[arg(long)]
    pub non_interactive: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_tui_and_non_interactive_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from(["app", "login", "--interactive", "--non-interactive"]).is_err()
        );
        let parsed = Cli::try_parse_from(["app", "login", "--interactive", "--remember-me"]);
        assert!(matches!(
            parsed,
            Ok(Cli {
                command: Command::Login(LoginArgs {
                    interactive: true,
                    remember_me: true,
                    ..
                })
            })
        ));
    }
}
