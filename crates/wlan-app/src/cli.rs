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
    /// 登录校园网；凭据不足时打开交互界面
    Login(LoginArgs),
    /// 注销当前校园网会话
    Logout,
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
    /// 缺少凭据时直接报错，不打开 TUI
    #[arg(long)]
    pub non_interactive: bool,
}
