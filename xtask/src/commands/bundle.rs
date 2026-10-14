//! Bundle dispatch; each platform owns its packaging workflow.
use anyhow::Result;

use crate::cli::bundle::{BundleArgs, BundlePlatform};

mod macos;

pub fn run(options: &BundleArgs) -> Result<()> {
    match &options.platform {
        BundlePlatform::Macos(options) => macos::run(options),
    }
}
