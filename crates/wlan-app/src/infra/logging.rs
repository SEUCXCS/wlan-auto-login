use crate::infra::error::Message;
use crate::infra::error::Result;
use crate::infra::{
    filesystem::{private_dir, write_private},
    paths::Paths,
};
use std::fs;
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer, writer::BoxMakeWriter},
    registry::LookupSpan,
};

struct Format;
impl<S, N> FormatEvent<S, N> for Format
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        write!(
            writer,
            "{} {:<5} {}: ",
            utc_time(),
            event.metadata().level(),
            event.metadata().target()
        )?;
        context.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

fn utc_time() -> String {
    #[cfg(unix)]
    {
        // SAFETY: libc writes to our thread-local tm buffer; gmtime_r is thread safe.
        unsafe {
            let now = libc::time(std::ptr::null_mut());
            let mut value: libc::tm = std::mem::zeroed();
            if libc::gmtime_r(&now, &mut value).is_null() {
                return "unknown-time".into();
            }
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                value.tm_year + 1900,
                value.tm_mon + 1,
                value.tm_mday,
                value.tm_hour,
                value.tm_min,
                value.tm_sec
            )
        }
    }
    #[cfg(windows)]
    {
        // SAFETY: GetSystemTime fills the correctly sized SYSTEMTIME buffer (Win7 supported).
        unsafe {
            let mut value = std::mem::zeroed();
            windows_sys::Win32::System::SystemInformation::GetSystemTime(&mut value);
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                value.wYear, value.wMonth, value.wDay, value.wHour, value.wMinute, value.wSecond
            )
        }
    }
}

pub fn init(file: Option<std::fs::File>) -> Result<()> {
    let writer = match file {
        Some(file) => BoxMakeWriter::new(std::sync::Mutex::new(file)),
        None => BoxMakeWriter::new(std::io::stderr),
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_ansi(false)
        .with_writer(writer)
        .event_format(Format)
        .try_init()
        .map_err(|error| {
            crate::infra::error::error!(Message::FailedToInitializeLoggingError {
                error: (error).to_string()
            })
        })
}

pub(crate) fn background_log(paths: &Paths) -> Result<fs::File> {
    let path = paths.config.join("daemon.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
        write_private(&path, b"")?;
    }
    private_dir(&paths.config)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };

    #[derive(Clone)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| std::io::Error::other("buffer poisoned"))?
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn logs_include_utc_level_target_and_safe_structured_diagnostics() -> Result<()> {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = Buffer(bytes.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .event_format(Format)
            .finish();
        let error = crate::infra::error::Error::from(cxxy_wlanlogin_lib::Error::Rejected {
            message: "password=never-log-this\nforged log entry".into(),
            credential_rejected: true,
        });
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: "wlan::daemon", error = %error, retry_after_secs = 30, "{}", Message::CheckFailed);
        });
        let output = String::from_utf8(
            bytes
                .lock()
                .map_err(|_| crate::infra::error::error!("buffer poisoned"))?
                .clone(),
        )?;
        assert_eq!(output.lines().count(), 1);
        assert_eq!(&output[10..11], "T");
        assert_eq!(&output[19..20], "Z");
        assert!(output.contains("WARN  wlan::daemon:"));
        assert!(output.contains("retry_after_secs=30"));
        assert!(output.contains("Credentials rejected"));
        assert!(!output.contains("never-log-this"));
        assert!(!output.contains("forged"));
        assert!(output.is_ascii());
        Ok(())
    }
}
