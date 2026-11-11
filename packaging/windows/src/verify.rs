//! Inspect MSI tables and extract its cabinet without executing installation actions.
use crate::{Scope, win};
use anyhow::{Context, Result, ensure};
use std::{fs, io::Write, path::Path, process::Command, ptr};
use windows_sys::Win32::System::ApplicationInstallationAndServicing::*;

struct Handle(u32);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            MsiCloseHandle(self.0);
        }
    }
}
fn result(code: u32) -> Result<()> {
    ensure!(code == 0, "MSI inspection failed ({code})");
    Ok(())
}
fn view(database: &Handle, sql: &str) -> Result<Handle> {
    unsafe {
        let mut view = 0;
        result(MsiDatabaseOpenViewW(
            database.0,
            win::wide(sql).as_ptr(),
            &mut view,
        ))?;
        let view = Handle(view);
        result(MsiViewExecute(view.0, 0))?;
        Ok(view)
    }
}
fn string(record: &Handle, column: u32) -> Result<String> {
    unsafe {
        let mut length = 0;
        let code = MsiRecordGetStringW(record.0, column, ptr::null_mut(), &mut length);
        ensure!(
            code == 0 || code == 234,
            "Could not inspect MSI field ({code})"
        );
        let mut buffer = vec![0; length as usize + 1];
        length += 1;
        result(MsiRecordGetStringW(
            record.0,
            column,
            buffer.as_mut_ptr(),
            &mut length,
        ))?;
        Ok(String::from_utf16(&buffer[..length as usize])?)
    }
}
fn rows(database: &Handle, sql: &str) -> Result<Vec<Vec<String>>> {
    let view = view(database, sql)?;
    let mut rows = Vec::new();
    loop {
        let mut record = 0;
        let code = unsafe { MsiViewFetch(view.0, &mut record) };
        if code == 259 {
            break;
        }
        result(code)?;
        let record = Handle(record);
        let columns = unsafe { MsiRecordGetFieldCount(record.0) };
        rows.push(
            (1..=columns)
                .map(|column| string(&record, column))
                .collect::<Result<Vec<_>>>()?,
        );
    }
    Ok(rows)
}

pub fn run(arguments: &[String]) -> Result<()> {
    let [msi, binary, readme, license, scope] = arguments else {
        anyhow::bail!("Invalid package inspection arguments");
    };
    let scope = Scope::parse(scope)?;
    let mut database = 0;
    result(unsafe { MsiOpenDatabaseW(win::wide(msi).as_ptr(), ptr::null(), &mut database) })?;
    let database = Handle(database);
    let files = rows(&database, "SELECT `File`, `FileName` FROM `File`")?;
    ensure!(
        files.len() == 3,
        "The MSI must contain exactly the three portable files"
    );
    let environment = rows(&database, "SELECT `Name`, `Value` FROM `Environment`")?;
    ensure!(
        environment.len() == 1
            && environment[0][0].contains("PATH")
            && environment[0][0].contains('*') == (scope == Scope::Machine)
            && environment[0][1] == "[~];[INSTALLDIR]",
        "PATH must append the scoped installation directory"
    );
    for (action, rollback) in [("InstallService", false), ("RollbackService", true)] {
        let records = rows(
            &database,
            &format!("SELECT `Type` FROM `CustomAction` WHERE `Action`='{action}'"),
        )?;
        let kind: u32 = records
            .first()
            .and_then(|row| row.first())
            .context("Missing service action")?
            .parse()?;
        ensure!(
            kind & 0x402 == 0x402
                && (kind & 0x100 != 0) == rollback
                && (kind & 0x800 != 0) == (scope == Scope::Machine),
            "MSI service action context or rollback flags are invalid"
        );
    }
    let media = rows(&database, "SELECT `Cabinet` FROM `Media`")?;
    let cabinet = media
        .first()
        .and_then(|row| row.first())
        .context("Missing embedded cabinet")?;
    let cabinet = cabinet
        .strip_prefix('#')
        .context("The cabinet must be embedded")?;
    let stream = view(
        &database,
        &format!("SELECT `Data` FROM `_Streams` WHERE `Name`='{cabinet}'"),
    )?;
    let mut record = 0;
    result(unsafe { MsiViewFetch(stream.0, &mut record) })?;
    let record = Handle(record);
    let temporary = tempfile::Builder::new()
        .prefix("wlanlogin-package-check-")
        .tempdir()?;
    win::private_directory(temporary.path(), false)?;
    let cabinet_file = temporary.path().join("payload.cab");
    let mut file = fs::File::create(&cabinet_file)?;
    loop {
        let mut buffer = [0; 16384];
        let mut length = buffer.len() as u32;
        result(unsafe {
            MsiRecordReadStream(record.0, 1, buffer.as_mut_ptr().cast(), &mut length)
        })?;
        if length == 0 {
            break;
        }
        file.write_all(&buffer[..length as usize])?;
    }
    drop(file);
    let extracted = temporary.path().join("files");
    fs::create_dir(&extracted)?;
    let expanded = Command::new("expand.exe")
        .arg(&cabinet_file)
        .arg("-F:*")
        .arg(&extracted)
        .output()?;
    ensure!(
        expanded.status.success(),
        "Could not inspect embedded cabinet contents"
    );
    for (expected_name, source) in [
        ("wlanlogin.exe", binary),
        ("README.md", readme),
        ("LICENSE", license),
    ] {
        let entry = files
            .iter()
            .find(|row| row[1].split('|').next_back() == Some(expected_name))
            .context("Missing portable payload file")?;
        ensure!(
            fs::read(extracted.join(&entry[0]))? == fs::read(Path::new(source))?,
            "Packaged {expected_name} differs from the portable asset"
        );
    }
    println!(
        "Verified {} payload, scoped PATH and rollback actions",
        scope.cli_flag()
    );
    Ok(())
}
