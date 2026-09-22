use super::LogOptions;
use anyhow::{Context, Result, ensure};
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
        let mut file =
            match File::open(path) {
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
pub(super) async fn file_logs(path: &Path, options: LogOptions) -> Result<()> {
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
mod tests;
