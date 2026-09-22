#[cfg(target_os = "linux")]
use super::status::SystemRunner;
#[cfg(any(target_os = "linux", test))]
use super::{Invocation, status::Runner};
use super::{
    Plan, plan,
    status::{Artifacts, Manager},
};
use crate::{cli::LogsArgs, config::Paths};
use anyhow::{Context, Result, ensure};
use std::io::Write;
#[cfg(any(target_os = "macos", windows, test))]
use std::path::PathBuf;

#[derive(Debug)]
enum Source {
    #[cfg(any(target_os = "linux", test))]
    Journal(Invocation),
    #[cfg(any(target_os = "macos", windows, test))]
    File(PathBuf),
}

fn source(paths: &Paths, plan: &Plan, options: LogsArgs) -> Result<Source> {
    let files = Artifacts::read(plan)?;
    ensure!(
        files.definition || files.executable,
        "后台服务尚未安装，请先执行 install"
    );
    ensure!(files.complete(), "后台服务安装不完整，请重新执行 install");
    // Reading diagnostics must also work when the service is stopped or its
    // registration is broken. No service manager query or mutation is needed.
    match &plan.manager {
        #[cfg(any(target_os = "linux", test))]
        Manager::Systemd { system } => {
            let _ = paths;
            let mut command = Invocation::new(
                "journalctl",
                [
                    if *system { "--system" } else { "--user" },
                    "--unit",
                    plan.manager.name(),
                    "--no-pager",
                    "--output=cat",
                    "--quiet",
                    "--lines",
                    &options.lines.to_string(),
                ],
            );
            if options.follow {
                command.args.push("--follow".into());
            }
            Ok(Source::Journal(command))
        }
        #[cfg(any(target_os = "macos", test))]
        Manager::Launchd { .. } => {
            let _ = options;
            Ok(Source::File(paths.config.join("daemon.log")))
        }
        #[cfg(any(windows, test))]
        Manager::TaskScheduler { .. } | Manager::WindowsService => {
            let _ = options;
            Ok(Source::File(paths.config.join("daemon.log")))
        }
    }
}

pub async fn logs(paths: &Paths, options: LogsArgs) -> Result<()> {
    let source = source(paths, &plan(paths)?, options)?;
    let result = match source {
        #[cfg(target_os = "linux")]
        Source::Journal(command) if options.follow => {
            follow_journal(&command, tokio::signal::ctrl_c()).await
        }
        #[cfg(target_os = "linux")]
        Source::Journal(command) => {
            journal_snapshot(&command, &SystemRunner, &mut std::io::stdout()).await
        }
        #[cfg(any(target_os = "macos", windows))]
        Source::File(path) => file_logs(&path, options).await,
        // Other platform implementations are compiled for unit tests only.
        #[cfg(test)]
        _ => unreachable!("the native service plan selects the native log source"),
    };
    // Closing a downstream pipe is a normal way to stop consuming logs.
    match result {
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe) =>
        {
            Ok(())
        }
        result => result,
    }
}

#[cfg(any(target_os = "linux", test))]
async fn journal_snapshot(
    command: &Invocation,
    runner: &impl Runner,
    output: &mut impl Write,
) -> Result<()> {
    let result = runner.output(command).await?;
    result.check(command)?;
    if result.stdout.is_empty() {
        tracing::info!("暂无可读取的服务日志");
    } else {
        output.write_all(result.stdout.as_bytes())?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn follow_journal(
    command: &Invocation,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    use std::process::Stdio;
    let mut child = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("无法读取 journal 日志，请确认 journalctl 可用且当前用户有读取权限")?;
    tokio::select! {
        result = child.wait() => {
            let status = result?;
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if matches!(status.signal(), Some(libc::SIGINT | libc::SIGPIPE)) { return Ok(()); }
            }
            ensure!(status.success() || status.code() == Some(130), "journalctl 执行失败：{status}");
            Ok(())
        },
        signal = stop => {
            signal.context("无法监听 Ctrl+C")?;
            child.start_kill()?;
            child.wait().await?;
            Ok(())
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
use file::file_logs;
#[cfg(any(target_os = "macos", windows, test))]
mod file {
    use super::*;
    use std::{
        fs::File,
        io::{BufRead, BufReader, Read, Seek, SeekFrom},
        path::Path,
    };

    #[derive(Default)]
    struct Cursor {
        offset: Option<u64>,
        anchor: Vec<u8>,
    }

    impl Cursor {
        // Reopen by path on every poll so atomic replacement and Windows log
        // rotation work without holding a file handle across service restarts.
        fn read(
            &mut self,
            path: &Path,
            options: LogsArgs,
            output: &mut impl Write,
        ) -> Result<Option<u64>> {
            let mut file = match File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.offset = Some(0);
                    self.anchor.clear();
                    return Ok(None);
                }
                Err(error) => return Err(error).with_context(|| {
                    format!(
                        "无法读取服务日志 {}；请确认当前用户有读取权限，系统日志可能需要管理员权限",
                        path.display()
                    )
                }),
            };
            let metadata = file.metadata()?;
            ensure!(
                metadata.is_file(),
                "日志路径不是普通文件：{}",
                path.display()
            );
            let end = metadata.len();
            let start = if let Some(offset) = self.offset {
                let mut anchor = vec![0; self.anchor.len()];
                let same = if offset <= end {
                    file.seek(SeekFrom::Start(offset - anchor.len() as u64))?;
                    file.read_exact(&mut anchor).is_ok() && anchor == self.anchor
                } else {
                    false
                };
                if same { offset } else { 0 }
            } else {
                tail_start(&mut file, end, options.lines)?
            };
            file.seek(SeekFrom::Start(start))?;
            let mut consumed = 0;
            {
                let mut reader = BufReader::new((&mut file).take(end - start));
                let mut line = Vec::new();
                loop {
                    line.clear();
                    let count = reader.read_until(b'\n', &mut line)?;
                    if count == 0 || (options.follow && !line.ends_with(b"\n")) {
                        break;
                    }
                    output.write_all(String::from_utf8_lossy(&line).as_bytes())?;
                    consumed += count as u64;
                }
            }
            let offset = start + consumed;
            let anchor_start = offset.saturating_sub(64);
            file.seek(SeekFrom::Start(anchor_start))?;
            self.anchor.clear();
            file.take(offset - anchor_start)
                .read_to_end(&mut self.anchor)?;
            self.offset = Some(offset);
            output.flush()?;
            Ok(Some(consumed))
        }
    }

    fn tail_start(file: &mut File, end: u64, lines: u32) -> Result<u64> {
        if lines == 0 {
            return Ok(end);
        }
        let mut position = end;
        let mut found = 0;
        let mut block = [0; 8192];
        while position > 0 {
            let count = position.min(block.len() as u64) as usize;
            position -= count as u64;
            file.seek(SeekFrom::Start(position))?;
            file.read_exact(&mut block[..count])?;
            for index in (0..count).rev() {
                if block[index] == b'\n' && position + index as u64 != end - 1 {
                    found += 1;
                    if found == lines {
                        return Ok(position + index as u64 + 1);
                    }
                }
            }
        }
        Ok(0)
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(super) async fn file_logs(path: &Path, options: LogsArgs) -> Result<()> {
        let mut cursor = Cursor::default();
        let mut output = std::io::stdout();
        match cursor.read(path, options, &mut output)? {
            None => tracing::info!("服务日志尚未生成"),
            Some(0) if options.lines > 0 => tracing::info!("暂无可读取的服务日志"),
            _ => {}
        }
        if options.follow {
            loop {
                tokio::select! {
                    signal = tokio::signal::ctrl_c() => { signal.context("无法监听 Ctrl+C")?; break; },
                    _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                        cursor.read(path, options, &mut output)?;
                    }
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;

        #[test]
        fn tail_preserves_unicode_empty_lines_and_unterminated_records() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let path = dir.path().join("daemon.log");
            for (contents, lines, expected) in [
                ("旧日志\n第二行\n第三行\n", 2, "第二行\n第三行\n"),
                ("第一行\n\n最后一行", 2, "\n最后一行"),
                ("最后一行\n", 0, ""),
                ("", 100, ""),
            ] {
                fs::write(&path, contents)?;
                let mut output = Vec::new();
                Cursor::default().read(
                    &path,
                    LogsArgs {
                        lines,
                        follow: false,
                    },
                    &mut output,
                )?;
                assert_eq!(output, expected.as_bytes());
            }
            let contents = format!("{}\n末行\n", "旧".repeat(6000));
            fs::write(&path, contents)?;
            let mut output = Vec::new();
            Cursor::default().read(
                &path,
                LogsArgs {
                    lines: 1,
                    follow: false,
                },
                &mut output,
            )?;
            assert_eq!(output, "末行\n".as_bytes());
            Ok(())
        }

        #[test]
        fn follow_handles_creation_append_truncation_and_replacement() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let path = dir.path().join("daemon.log");
            let mut cursor = Cursor::default();
            let mut output = Vec::new();
            let options = LogsArgs {
                lines: 100,
                follow: true,
            };
            assert_eq!(cursor.read(&path, options, &mut output)?, None);
            assert!(!path.exists());
            fs::write(&path, "第一条\n")?;
            cursor.read(&path, options, &mut output)?;
            fs::OpenOptions::new()
                .append(true)
                .open(&path)?
                .write_all("第二条\n".as_bytes())?;
            cursor.read(&path, options, &mut output)?;
            cursor.read(&path, options, &mut output)?;
            assert_eq!(output, "第一条\n第二条\n".as_bytes());
            output.clear();
            fs::write(&path, "短\n")?;
            cursor.read(&path, options, &mut output)?;
            let replacement = dir.path().join("replacement");
            fs::write(&replacement, "不同的新日志，文件比原来长\n")?;
            fs::remove_file(&path)?;
            fs::rename(replacement, &path)?;
            cursor.read(&path, options, &mut output)?;
            assert_eq!(output, "短\n不同的新日志，文件比原来长\n".as_bytes());
            Ok(())
        }

        #[test]
        fn follow_from_zero_waits_for_complete_utf8_records() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let path = dir.path().join("daemon.log");
            fs::write(&path, "历史\n")?;
            let options = LogsArgs {
                lines: 0,
                follow: true,
            };
            let mut cursor = Cursor::default();
            let mut output = Vec::new();
            cursor.read(&path, options, &mut output)?;
            let mut writer = fs::OpenOptions::new().append(true).open(&path)?;
            let bytes = "新日志\n".as_bytes();
            writer.write_all(&bytes[..1])?;
            cursor.read(&path, options, &mut output)?;
            assert!(output.is_empty());
            writer.write_all(&bytes[1..])?;
            cursor.read(&path, options, &mut output)?;
            assert_eq!(output, bytes);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::status::tests::FakeRunner;
    use super::*;
    use crate::scope::Scope;
    use std::fs;

    #[test]
    fn sources_match_each_platform_and_installation_scope() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let options = LogsArgs {
            lines: 37,
            follow: true,
        };
        for scope in [Scope::User, Scope::System] {
            let mut paths = Paths::test(dir.path().join(scope.label()));
            paths.scope = scope;
            let mut plan =
                super::super::linux_plan(dir.path().join("daemon"), dir.path().join("service"))?;
            plan.scope = scope;
            plan.manager = Manager::Systemd {
                system: scope == Scope::System,
            };
            fs::write(&plan.definition, &plan.content)?;
            fs::write(&plan.executable, "mock, never run")?;
            let Source::Journal(command) = source(&paths, &plan, options)? else {
                anyhow::bail!("journal expected")
            };
            let arguments: Vec<_> = command
                .args
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect();
            assert_eq!(command.program, "journalctl");
            assert_eq!(
                arguments,
                [
                    if scope == Scope::System {
                        "--system"
                    } else {
                        "--user"
                    },
                    "--unit",
                    "cxxy-wlanlogin.service",
                    "--no-pager",
                    "--output=cat",
                    "--quiet",
                    "--lines",
                    "37",
                    "--follow"
                ]
            );
            for manager in [
                Manager::Launchd {
                    domain: "system".into(),
                    target: "test".into(),
                },
                Manager::TaskScheduler {
                    name: "test".into(),
                },
                Manager::WindowsService,
            ] {
                plan.manager = manager;
                let Source::File(path) = source(&paths, &plan, options)? else {
                    anyhow::bail!("file expected")
                };
                assert_eq!(path, paths.config.join("daemon.log"));
            }
            assert!(
                !paths.config.exists(),
                "viewing logs must not create state directories"
            );
        }
        Ok(())
    }

    #[test]
    fn missing_or_partial_installation_is_rejected_without_creating_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("config"));
        let plan = super::super::linux_plan(dir.path().join("daemon"), dir.path().join("service"))?;
        let options = LogsArgs {
            lines: 100,
            follow: false,
        };
        assert!(
            source(&paths, &plan, options)
                .err()
                .context("expected installation error")?
                .to_string()
                .contains("尚未安装")
        );
        fs::write(&plan.definition, &plan.content)?;
        assert!(
            source(&paths, &plan, options)
                .err()
                .context("expected installation error")?
                .to_string()
                .contains("不完整")
        );
        assert!(!paths.config.exists());
        assert!(!plan.executable.exists());
        Ok(())
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn journal_follow_cancels_the_child_and_reports_failures() -> Result<()> {
        let command = Invocation::new("sleep", ["30"]);
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            follow_journal(&command, std::future::ready(Ok(()))),
        )
        .await??;
        let command = Invocation::new("false", std::iter::empty::<&str>());
        assert!(
            follow_journal(&command, std::future::pending())
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn journal_keeps_original_messages_and_reports_access_failures() -> Result<()> {
        let command = Invocation::new("journalctl", ["--user"]);
        let runner = FakeRunner::new([(true, "[INFO 2026-09-22 10:30:00] 日志\n")]);
        let mut output = Vec::new();
        journal_snapshot(&command, &runner, &mut output).await?;
        assert_eq!(output, "[INFO 2026-09-22 10:30:00] 日志\n".as_bytes());
        let runner = FakeRunner::new([(false, "access denied")]);
        assert!(
            journal_snapshot(&command, &runner, &mut Vec::new())
                .await
                .is_err()
        );
        let runner = FakeRunner::new([(true, "")]);
        let mut output = Vec::new();
        journal_snapshot(&command, &runner, &mut output).await?;
        assert!(output.is_empty());
        Ok(())
    }
}
