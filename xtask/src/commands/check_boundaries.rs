//! Enforce the application's native-source boundary independently of host OS.
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};
const FACADES: &[&str] = &[
    "app.rs",
    "app/bootstrap.rs",
    "infra.rs",
    "infra/error.rs",
    "infra/identity.rs",
    "system.rs",
    "system/shared/mod.rs",
    "system/shared/flow/mod.rs",
];
fn platform_predicate(value: &str) -> bool {
    value.contains("target_")
        || value
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|token| matches!(token, "unix" | "windows"))
}
fn validate_source(relative: &str, source: &str) -> Result<()> {
    if ["system/linux/", "system/macos/", "system/windows/"]
        .iter()
        .any(|root| relative.starts_with(root))
    {
        return Ok(());
    }
    let lines: Vec<_> = source.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        ensure!(
            !line.contains("cfg!(") || line.starts_with("//") || !platform_predicate(line),
            "{relative}:{}: target-dependent expression outside a platform implementation",
            index + 1
        );
        if line.starts_with("#[cfg") {
            let start = index;
            let mut attribute = line.to_owned();
            while !attribute.ends_with(']') && index + 1 < lines.len() {
                index += 1;
                attribute.push_str(lines[index].trim());
            }
            if platform_predicate(&attribute) {
                ensure!(
                    FACADES.contains(&relative),
                    "{relative}:{}: target-dependent attribute outside a selection facade",
                    start + 1
                );
                ensure!(
                    !attribute.starts_with("#[cfg_attr"),
                    "{relative}:{}: target-dependent lint is not a module selection",
                    start + 1
                );
                let mut next = index + 1;
                while next < lines.len()
                    && (lines[next].trim().starts_with("#[")
                        || lines[next].trim().starts_with("//")
                        || lines[next].trim().is_empty())
                {
                    next += 1;
                }
                let item = lines.get(next).copied().unwrap_or("").trim();
                let item = item
                    .strip_prefix("pub(crate) ")
                    .or_else(|| item.strip_prefix("pub(super) "))
                    .or_else(|| item.strip_prefix("pub "))
                    .unwrap_or(item);
                ensure!(
                    item.starts_with("mod ")
                        || item.starts_with("use ")
                        || (relative == "system.rs" && item.starts_with("compile_error!")),
                    "{relative}:{}: facade may select or re-export modules, not implement target-specific behavior",
                    start + 1
                );
            }
        }
        index += 1;
    }
    Ok(())
}
fn visit(root: &Path, directory: &Path, count: &mut usize) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            visit(root, &path, count)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace(char::from(92), "/");
            validate_source(&relative, &fs::read_to_string(&path)?)?;
            *count += 1;
        }
    }
    Ok(())
}
pub fn run() -> Result<()> {
    let root = crate::support::workspace::root()?.join("crates/wlan-app/src");
    let mut count = 0;
    visit(&root, &root, &mut count).context("Platform boundary check failed")?;
    println!("Platform boundaries checked: {count} Rust files");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_declarations_and_reexports_are_allowed_in_facades() {
        assert!(
            validate_source(
                "infra.rs",
                "#[cfg(windows)]
pub(crate) mod process;"
            )
            .is_ok()
        );
        assert!(
            validate_source(
                "infra.rs",
                "#[cfg(windows)]
fn mutate() {}"
            )
            .is_err()
        );
        assert!(
            validate_source(
                "infra.rs",
                "#[cfg_attr(windows, allow(dead_code))]
struct Native;"
            )
            .is_err()
        );
        assert!(
            validate_source(
                "system/shared/flow/install.rs",
                "#[cfg(windows)]
fn mutate() {}"
            )
            .is_err()
        );
        assert!(validate_source("app/daemon.rs", "let native = cfg!(windows);").is_err());
        assert!(
            validate_source(
                "system/windows/flow/install.rs",
                "#[cfg(windows)]
fn native() {}"
            )
            .is_ok()
        );
        assert!(
            validate_source(
                "app/daemon.rs",
                "#[cfg(test)]
mod tests {}"
            )
            .is_ok()
        );
    }
    #[test]
    fn multiline_predicates_are_checked() {
        assert!(
            validate_source(
                "app/daemon.rs",
                "#[cfg(all(
    test,
    target_os = \"linux\"
))]
fn native() {}"
            )
            .is_err()
        );
    }
}
