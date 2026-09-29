use crate::infra::error::Message;
/// Options shared by historical and streaming service-log readers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LogOptions {
    pub lines: u32,
    pub follow: bool,
}

use crate::infra::error::{Context, Result, ensure};
use std::io::Write;
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
        options: LogOptions,
        output: &mut impl Write,
    ) -> Result<Option<u64>> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.offset = Some(0);
                self.anchor.clear();
                return Ok(None);
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    (Message::FailedToReadServiceLogCheckFile {
                        value0: (path.display()).to_string(),
                    })
                    .to_string()
                });
            }
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file(),
            Message::LogPathIsNotARegularFile {
                value0: (path.display()).to_string()
            }
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

pub(crate) async fn file_logs(path: &Path, options: LogOptions) -> Result<()> {
    let mut cursor = Cursor::default();
    let mut output = std::io::stdout();
    match cursor.read(path, options, &mut output)? {
        None => tracing::info!("{}", Message::ServiceLogHasNotBeenCreated),
        Some(0) if options.lines > 0 => tracing::info!("{}", Message::NoServiceLogEntriesAvailable),
        _ => {}
    }
    if options.follow {
        loop {
            tokio::select! {
                signal = tokio::signal::ctrl_c() => { signal.context(Message::FailedToRegisterCtrlCListener)?; break; },
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
                LogOptions {
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
            LogOptions {
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
        let options = LogOptions {
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
        let options = LogOptions {
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
