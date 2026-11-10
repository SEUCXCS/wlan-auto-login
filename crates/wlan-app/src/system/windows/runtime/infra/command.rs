//! Execute bounded service-manager commands and expose a testable runner.
//! In-memory service-manager responses shared by unit tests.
use crate::infra::error::Message;
use crate::infra::error::{Context, Result, bail};
use std::{
    ffi::{OsStr, OsString},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputEncoding {
    Utf8,
    NativeOem,
}

#[derive(Clone, Debug)]
pub(crate) struct Invocation {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    pub(crate) optional: bool,
    encoding: OutputEncoding,
}
impl Invocation {
    pub(crate) fn new(
        program: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Self {
        let program = program.as_ref().to_owned();
        let name = std::path::Path::new(&program)
            .file_name()
            .unwrap_or(&program)
            .to_string_lossy()
            .to_ascii_lowercase();
        let encoding = if matches!(name.as_str(), "schtasks" | "schtasks.exe" | "sc.exe") {
            OutputEncoding::NativeOem
        } else {
            OutputEncoding::Utf8
        };
        Self {
            program,
            args: args.into_iter().map(|v| v.as_ref().to_owned()).collect(),
            optional: false,
            encoding,
        }
    }
    pub(crate) fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub(crate) async fn output(&self) -> Result<CommandOutput> {
        let mut command = tokio::process::Command::new(&self.program);
        command.args(&self.args).kill_on_drop(true);

        command.creation_flags(0x08000000); // CREATE_NO_WINDOW, available on Win7.
        let result = tokio::time::timeout(Duration::from_secs(30), command.output())
            .await
            .context(Message::ServiceManagerCommandTimedOut)??;
        Ok(CommandOutput {
            success: result.status.success(),
            stdout: decode_output(&result.stdout, self.encoding)?,
            stderr: decode_output(&result.stderr, self.encoding)?,
        })
    }
}

pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}
impl CommandOutput {
    pub(crate) fn check(&self, command: &Invocation) -> Result<()> {
        if !self.success {
            bail!(Message::CommandFailed {
                value0: (command.program.to_string_lossy()).to_string(),
                value1: format!("{:?}", command.args),
                value2: (if self.stderr.trim().is_empty() {
                    self.stdout.trim()
                } else {
                    self.stderr.trim()
                })
                .to_string()
            });
        }
        Ok(())
    }
}

pub(crate) trait Runner {
    fn output(
        &self,
        command: &Invocation,
    ) -> impl std::future::Future<Output = Result<CommandOutput>>;
}

pub(crate) struct SystemRunner;
impl Runner for SystemRunner {
    async fn output(&self, command: &Invocation) -> Result<CommandOutput> {
        command.output().await
    }
}

fn decode_output(bytes: &[u8], encoding: OutputEncoding) -> Result<String> {
    match encoding {
        OutputEncoding::Utf8 => Ok(String::from_utf8_lossy(bytes).into_owned()),
        OutputEncoding::NativeOem => {
            decode_code_page(bytes, windows_sys::Win32::Globalization::CP_OEMCP)
        }
    }
}
fn decode_code_page(bytes: &[u8], code_page: u32) -> Result<String> {
    use windows_sys::Win32::Globalization::{MB_ERR_INVALID_CHARS, MultiByteToWideChar};
    if bytes.is_empty() {
        return Ok(String::new());
    }
    let length = i32::try_from(bytes.len()).context("Native command output is too large")?;
    // SAFETY: both calls borrow live buffers; the first determines the exact UTF-16 size.
    let size = unsafe {
        MultiByteToWideChar(
            code_page,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            length,
            std::ptr::null_mut(),
            0,
        )
    };
    if size == 0 {
        return Err(std::io::Error::last_os_error())
            .context("Failed to decode native command output");
    }
    let mut wide = vec![0u16; size as usize];
    // SAFETY: wide has the size returned for the same immutable input.
    let written = unsafe {
        MultiByteToWideChar(
            code_page,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            length,
            wide.as_mut_ptr(),
            size,
        )
    };
    if written == 0 {
        return Err(std::io::Error::last_os_error())
            .context("Failed to decode native command output");
    }
    String::from_utf16(&wide[..written as usize]).context("Native command returned invalid Unicode")
}
#[cfg(test)]
pub(crate) mod tests {
    use super::{CommandOutput, Invocation, Runner};
    use crate::infra::error::Message;
    use crate::infra::error::{Context, Result};
    use std::{cell::RefCell, collections::VecDeque};

    #[derive(Default)]
    pub(crate) struct FakeRunner {
        pub outputs: RefCell<VecDeque<CommandOutput>>,
        pub calls: RefCell<Vec<Invocation>>,
    }
    impl FakeRunner {
        pub fn new(outputs: impl IntoIterator<Item = (bool, &'static str)>) -> Self {
            Self {
                outputs: RefCell::new(
                    outputs
                        .into_iter()
                        .map(|(success, stdout)| CommandOutput {
                            success,
                            stdout: stdout.into(),
                            stderr: String::new(),
                        })
                        .collect(),
                ),
                calls: RefCell::new(Vec::new()),
            }
        }
    }
    impl Runner for FakeRunner {
        async fn output(&self, command: &Invocation) -> Result<CommandOutput> {
            self.calls.borrow_mut().push(command.clone());
            self.outputs
                .borrow_mut()
                .pop_front()
                .context(Message::UnexpectedServiceCommand)
        }
    }

    pub const RUNNING: &str = "LoadState=loaded\nActiveState=active\nSubState=running\nUnitFileState=enabled\nMainPID=123\nResult=success\n";
    pub const STOPPED: &str = "LoadState=loaded\nActiveState=inactive\nSubState=dead\nUnitFileState=enabled\nMainPID=0\nResult=success\n";
    #[test]
    fn legacy_native_errors_are_decoded_without_changing_utf8_protocols() -> Result<()> {
        use super::{OutputEncoding, decode_code_page, decode_output};
        assert_eq!(
            decode_code_page(&[0xb4, 0xed, 0xce, 0xf3, 0x3a, 0x20, 0x35], 936)?,
            "错误: 5"
        );
        assert_eq!(
            decode_output("中文 UTF-8 protocol".as_bytes(), OutputEncoding::Utf8)?,
            "中文 UTF-8 protocol"
        );
        assert!(decode_code_page(&[0xff], 65001).is_err());
        assert_eq!(
            Invocation::new("SCHTASKS.EXE", ["/Query"]).encoding,
            OutputEncoding::NativeOem
        );
        assert_eq!(
            Invocation::new("powershell.exe", ["-Command", "ignored"]).encoding,
            OutputEncoding::Utf8
        );
        Ok(())
    }
    #[tokio::test]
    async fn real_missing_task_error_contains_no_replacement_characters() -> Result<()> {
        let name = format!(
            "wlanlogin-read-only-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(std::io::Error::other)?
                .as_nanos()
        );
        let output = Invocation::new("schtasks", ["/Query", "/TN", &name])
            .output()
            .await?;
        assert!(!output.success);
        let diagnostic = output.stdout + &output.stderr;
        assert!(!diagnostic.trim().is_empty());
        assert!(!diagnostic.contains(char::REPLACEMENT_CHARACTER));
        Ok(())
    }
}
