use crate::config::Paths;
use anyhow::{Context, Result};
use zeroize::Zeroizing;

pub(super) async fn crypt(operation: &str, input: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    crypt_with(std::ffi::OsStr::new("systemd-creds"), operation, input).await
}

async fn crypt_with(
    program: &std::ffi::OsStr,
    operation: &str,
    input: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    use std::{process::Stdio, time::Duration};
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::new(program);
    command
        .args([operation, "--name=login", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().context("无法运行 systemd-creds；需要支持加密凭据的 systemd，旧系统可显式选择 --credential-store protected-file")?;
    let mut stdin = child.stdin.take().context("无法打开凭据输入通道")?;
    let output = tokio::time::timeout(Duration::from_secs(30), async {
        stdin.write_all(input).await?;
        drop(stdin);
        child.wait_with_output().await
    })
    .await
    .context("系统凭据加解密超时")??;
    anyhow::ensure!(
        output.status.success(),
        "系统凭据{operation}失败；未降级存储方式"
    );
    Ok(Zeroizing::new(output.stdout))
}

pub fn linux_credential_directive(paths: &Paths) -> Result<String> {
    if paths.secrets.join("login.cred").try_exists()? {
        Ok(format!(
            "LoadCredentialEncrypted=login:{}\n",
            paths.secrets.join("login.cred").display()
        ))
    } else if paths.secrets.join("login.json").try_exists()? {
        // The explicit protected-file backend also supports systemd versions
        // predating LoadCredential; the root service reads its private file.
        Ok(String::new())
    } else {
        anyhow::bail!("缺少系统凭据，请先执行 sudo login --remember-me")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CredentialMode, MachineStore, validate_private};
    use super::*;
    use crate::credentials::decode;
    use cxxy_wlanlogin_lib::Credentials;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[tokio::test]
    async fn secret_is_sent_to_cipher_over_stdin_and_errors_do_not_echo_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let helper = dir.path().join("mock-creds");
        fs::write(
            &helper,
            "#!/bin/sh\n[ \"$#\" = 4 ] || exit 1\n[ \"$2\" = --name=login ] || exit 1\ncat\n",
        )?;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
        let secret = b"fake-test-secret";
        assert_eq!(
            &*crypt_with(helper.as_os_str(), "encrypt", secret).await?,
            secret
        );
        fs::write(&helper, "#!/bin/sh\ncat >&2\nexit 1\n")?;
        let error = crypt_with(helper.as_os_str(), "encrypt", secret)
            .await
            .err()
            .context("failure expected")?;
        assert!(!format!("{error:#}").contains("fake-test-secret"));
        Ok(())
    }

    #[tokio::test]
    async fn protected_file_is_explicit_private_and_cannot_silently_replace_encryption()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("private"));
        let store = MachineStore::new(paths.clone(), Some(CredentialMode::ProtectedFile))?;
        store
            .save(Credentials {
                username: "test-student".into(),
                password: "test-secret".into(),
            })
            .await?;
        let path = paths.secrets.join("login.json");
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(&paths.secrets)?.permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            decode(&fs::read_to_string(&path)?)?.username,
            "test-student"
        );
        assert!(MachineStore::new(paths.clone(), Some(CredentialMode::Native)).is_err());
        fs::remove_file(path)?;
        fs::write(paths.secrets.join("login.cred"), "mock encrypted data")?;
        assert!(MachineStore::new(paths, Some(CredentialMode::ProtectedFile)).is_err());
        Ok(())
    }

    #[test]
    fn machine_secret_rejects_symlinks_and_broad_file_permissions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let secret = dir.path().join("secret");
        fs::write(&secret, "test data")?;
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644))?;
        assert!(validate_private(&secret).is_err());
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600))?;
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&secret, &link)?;
        assert!(validate_private(&link).is_err());
        Ok(())
    }
}
