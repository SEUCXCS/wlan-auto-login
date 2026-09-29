mod app;
mod credentials;
mod infra;
mod service;
mod system;
mod tui;
mod wifi;

fn main() -> std::process::ExitCode {
    app::run()
}
