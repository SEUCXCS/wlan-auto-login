mod app;
mod infra;
mod service;
mod system;
mod tui;

fn main() -> std::process::ExitCode {
    app::run()
}
