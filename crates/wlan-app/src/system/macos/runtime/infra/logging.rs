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

pub(crate) struct Format;
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

    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
