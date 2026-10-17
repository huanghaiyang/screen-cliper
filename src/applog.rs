//! Tiny append-only application log.
//!
//! Desktop capture tools fail in ways that are invisible from the UI (a hotkey
//! taken by another app, the notification area being unavailable, a clipboard
//! write refused by a security policy). Those events are written here so users
//! can attach one file when reporting a problem.

use std::io::Write;

const MAX_LOG_BYTES: u64 = 512 * 1024;

pub fn log_file() -> std::path::PathBuf {
    crate::config::home_dir().join("app.log")
}

pub fn log(message: impl AsRef<str>) {
    let dir = crate::config::home_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("app.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = std::fs::remove_file(&path);
        }
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let st = crate::win::local_time();
    let _ = writeln!(
        file,
        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] {}",
        st.wYear,
        st.wMonth,
        st.wDay,
        st.wHour,
        st.wMinute,
        st.wSecond,
        message.as_ref()
    );
}

pub fn open_log() {
    let path = log_file();
    if path.exists() {
        let _ = std::process::Command::new("notepad").arg(&path).spawn();
    } else {
        crate::win::open_folder(&crate::config::home_dir());
    }
}
