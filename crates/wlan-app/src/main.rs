mod app;
mod credentials;
mod infra;
mod service;
mod tui;
mod wifi;

fn main() -> std::process::ExitCode {
    app::run()
}
