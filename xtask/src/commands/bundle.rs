//! Bundle dispatch; each platform owns its packaging workflow.
use anyhow::Result;

use crate::cli::bundle::{BundleArgs, BundlePlatform};

mod deb;
mod linux;
mod macos;
mod rpm;
mod windows;

pub fn run(options: &BundleArgs) -> Result<()> {
    match &options.platform {
        BundlePlatform::Macos(options) => macos::run(options),
        BundlePlatform::Windows(options) => windows::run(options),
        BundlePlatform::Deb(options) => deb::run(options),
        BundlePlatform::Rpm(options) => rpm::run(options),
    }
}
