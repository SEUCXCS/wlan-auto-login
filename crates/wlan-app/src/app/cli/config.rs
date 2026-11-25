use crate::{
    app::controller::config,
    infra::{error::Result, paths::Paths},
};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// 读取配置值
    Get { name: String },
    /// 保存配置值
    Set { name: String, value: String },
    /// 移除配置值并恢复默认
    Remove { name: String },
    /// 列出配置值
    List,
}

pub(crate) async fn run(command: ConfigCommand, paths: &Paths) -> Result<()> {
    match command {
        ConfigCommand::Get { name } => println!("{}", config::get::run(paths, name.parse()?)?),
        ConfigCommand::Set { name, value } => config::set::run(paths, name.parse()?, value).await?,
        ConfigCommand::Remove { name } => config::remove::run(paths, name.parse()?).await?,
        ConfigCommand::List => {
            for (key, value) in config::list::run(paths)? {
                println!("{key}={value}");
            }
        }
    }
    Ok(())
}
