mod app;
mod config;
mod control;
mod credentials;
mod logging;
mod scope;
mod service;
mod shutdown;
mod tui;
mod wifi;

fn main() -> std::process::ExitCode {
    app::run()
}
