use anyhow::Result;
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
            "[{} {}] ",
            event.metadata().level(),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        )?;
        context.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
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
        .map_err(|error| anyhow::anyhow!("无法初始化日志：{error}"))
}
