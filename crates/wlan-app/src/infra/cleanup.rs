//! Remove only known application files; never recursively delete shared directories.
use crate::infra::error::Message;
use crate::infra::error::{Context, Result, ensure};
use crate::infra::identity::{APP_NAME, ORGANIZATION};
use std::{fs, io::ErrorKind, path::Path};

fn app_root(path: &Path) -> Option<&Path> {
    path.ancestors().find(|part| {
        part.file_name().is_some_and(|name| name == APP_NAME)
            && part
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == ORGANIZATION)
    })
}

pub fn validate_directory(path: &Path) -> Result<()> {
    // Validate the shared organization directory too, but never prune it.
    let root = app_root(path).and_then(Path::parent).unwrap_or(path);
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    Message::RefusingToCleanDataThroughANon {
                        value0: (part.display()).to_string()
                    }
                );
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    ensure!(
                        metadata.file_attributes() & 0x400 == 0,
                        Message::RefusingToCleanDataThroughAReparse {
                            value0: (part.display()).to_string()
                        }
                    );
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    (Message::FailedToInspectDirectory {
                        value0: (part.display()).to_string(),
                    })
                    .to_string()
                });
            }
        }
        if part == root {
            break;
        }
    }
    Ok(())
}

pub fn remove_file(path: &Path) -> Result<()> {
    validate_directory(
        path.parent()
            .context(Message::CleanupPathHasNoParentDirectory)?,
    )?;
    // remove_file unlinks a leaf symlink instead of following its target.
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            (Message::FailedToRemoveFile {
                value0: (path.display()).to_string(),
            })
            .to_string()
        }),
    }
}

pub fn prune_empty_directories(path: &Path) -> Result<()> {
    validate_directory(path)?;
    let root = app_root(path).unwrap_or(path);
    for part in path.ancestors() {
        match fs::remove_dir(part) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) if error.kind() == ErrorKind::DirectoryNotEmpty => break,
            Err(error) => {
                return Err(error).with_context(|| {
                    (Message::FailedToRemoveEmptyDirectory {
                        value0: (part.display()).to_string(),
                    })
                    .to_string()
                });
            }
        }
        if part == root {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pruning_stops_before_the_shared_organization_even_when_empty() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let app = dir.path().join("SEUCXCS/wlanlogin");
        let bin = app.join("data/bin");
        fs::create_dir_all(&bin)?;
        prune_empty_directories(&bin)?;
        prune_empty_directories(&bin)?;
        assert!(!app.exists());
        assert!(dir.path().join("SEUCXCS").is_dir());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_rejects_redirected_organization_and_app_directories() -> Result<()> {
        for redirect_organization in [false, true] {
            let dir = tempfile::tempdir()?;
            let outside = dir.path().join("outside");
            let source = if redirect_organization {
                outside.join("wlanlogin/state")
            } else {
                outside.join("state")
            };
            fs::create_dir_all(&source)?;
            fs::write(source.join("daemon.log"), "preserve")?;
            let organization = dir.path().join("SEUCXCS");
            let link = if redirect_organization {
                organization.clone()
            } else {
                fs::create_dir(&organization)?;
                organization.join("wlanlogin")
            };
            std::os::unix::fs::symlink(&outside, link)?;
            let state = organization.join("wlanlogin/state");
            assert!(remove_file(&state.join("daemon.log")).is_err());
            assert!(prune_empty_directories(&state).is_err());
            assert_eq!(fs::read_to_string(source.join("daemon.log"))?, "preserve");
        }
        Ok(())
    }
}
