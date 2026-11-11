pub fn require_macos() -> anyhow::Result<()> {
    anyhow::bail!("macOS bundling requires a macOS host with Xcode Command Line Tools")
}
