use crate::infra::{
    config::LogLevel,
    error::{Message, Result, ensure},
    filesystem::{private_dir, write_private},
    paths::Paths,
    scope::Scope,
};
use std::{fs, io::ErrorKind};
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    EnvFilter,
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer, writer::BoxMakeWriter},
    registry::LookupSpan,
};

#[derive(Default)]
pub(crate) struct Format {
    pub(super) scope: Option<Scope>,
}

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
        let now = time::OffsetDateTime::now_utc();
        let metadata = event.metadata();
        write!(
            writer,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z {:<5} {}: ",
            now.year(),
            u8::from(now.month()),
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
            metadata.level(),
            metadata.target()
        )?;
        if let Some(scope) = self.scope
            && metadata.fields().field("scope").is_none()
        {
            write!(writer, "scope=\"{}\" ", scope.label())?;
        }
        context.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

pub(crate) fn init(file: Option<fs::File>, scope: Option<Scope>, level: LogLevel) -> Result<()> {
    let writer = match file {
        Some(file) => BoxMakeWriter::new(std::sync::Mutex::new(file)),
        None => BoxMakeWriter::new(std::io::stderr),
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        match level {
            LogLevel::Info => "info",
            LogLevel::Debug => "info,wlanlogin=debug",
        }
        .into()
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .event_format(Format { scope })
        .try_init()
        .map_err(|error| {
            Message::FailedToInitializeLoggingError {
                error: error.to_string(),
            }
            .into()
        })
}

pub(crate) fn background_log(paths: &Paths) -> Result<fs::File> {
    private_dir(&paths.config)?;
    let path = paths.config.join("daemon.log");
    if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
        match tempfile::NamedTempFile::new_in(&paths.config)?.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error.into()),
        }
    }
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(metadata.is_file(), Message::InvalidLogPath);
    if metadata.len() > 2 * 1024 * 1024 {
        write_private(&path, b"")?;
    }
    Ok(fs::OpenOptions::new().append(true).open(path)?)
}
