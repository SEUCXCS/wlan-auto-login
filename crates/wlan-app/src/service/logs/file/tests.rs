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
