//! Saves to <Pictures>\Screenshots\Screenshot_YYYYMMDD_HHMMSS.webp
//! (known-folder lookup, so OneDrive-redirected Pictures works).
use std::io::{self, Write};
use std::path::PathBuf;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

fn pictures_dir() -> PathBuf {
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None) {
            let s = p.to_string();
            CoTaskMemFree(Some(p.0 as *const _));
            if let Ok(s) = s {
                return PathBuf::from(s);
            }
        }
    }
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Pictures"))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn save(data: &[u8]) -> io::Result<PathBuf> {
    let dir = pictures_dir().join("Screenshots");
    std::fs::create_dir_all(&dir)?;
    let t = unsafe { GetLocalTime() };
    let base = format!(
        "Screenshot_{:04}{:02}{:02}_{:02}{:02}{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    );
    for n in 0..100 {
        // create_new: never overwrite; suffix if two shots share a second
        let name = if n == 0 {
            format!("{base}.webp")
        } else {
            format!("{base}_{n}.webp")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(data)?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many files in one second",
    ))
}
