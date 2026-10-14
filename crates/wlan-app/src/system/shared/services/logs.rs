//! Default file-log adapter shared by platforms with application log files.
use crate::infra::{error::Result, file_log::file_logs, paths::Paths};
use crate::system::define::services::{LogBackend, LogOptions, LogSource, Plan};
pub(crate) struct FileBackend;
impl LogBackend for FileBackend {
    fn source(&self, paths: &Paths, _plan: &Plan, _options: LogOptions) -> Result<LogSource> {
        Ok(LogSource::File(paths.config.join("daemon.log")))
    }
    async fn read(
        &self,
        source: LogSource,
        options: LogOptions,
        output: &mut impl std::io::Write,
        stop: impl std::future::Future<Output = std::io::Result<()>>,
    ) -> Result<()> {
        match source {
            LogSource::File(path) => file_logs(&path, options, output, stop).await,
            #[cfg(any(target_os = "linux", test))]
            _ => crate::infra::error::bail!("File log adapter received a non-file source"),
        }
    }
}
