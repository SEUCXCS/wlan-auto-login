// Native async workflows require more trait-resolution depth than the default.
#![recursion_limit = "256"]

mod app;
mod infra;
mod system;

fn main() -> std::process::ExitCode {
    app::run()
}
