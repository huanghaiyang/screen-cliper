//! Windows "run at login" support via the per-user Run registry key.
//!
//! HKCU is used on purpose: it needs no administrator rights, which keeps the
//! autostart switch working for normal users.

use crate::config::APP_NAME;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = APP_NAME;

pub fn exe_path() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("screen-cliper.exe"))
}

/// The exact command line stored in the registry.
pub fn command_line() -> String {
    format!("\"{}\" --minimized", exe_path().display())
}

#[cfg(windows)]
pub fn is_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey(RUN_KEY) else {
        return false;
    };
    let Ok(value) = key.get_value::<String, _>(VALUE_NAME) else {
        return false;
    };
    let mut stored = value.trim().trim_matches('"').to_string();
    if let Some(idx) = stored.to_lowercase().find(" --") {
        stored.truncate(idx);
    }
    let stored_path = std::path::Path::new(&stored);
    let ours = exe_path();
    paths_equal(stored_path, &ours)
}

#[cfg(windows)]
pub fn set_enabled(enable: bool) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if enable {
        let (key, _) = hkcu
            .create_subkey(RUN_KEY)
            .map_err(|e| format!("无法写入注册表启动项：{e}"))?;
        key.set_value(VALUE_NAME, &command_line())
            .map_err(|e| format!("写入开机自启动命令失败：{e}"))?;
        Ok(())
    } else {
        let key = hkcu
            .open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)
            .map_err(|e| format!("无法打开注册表启动项：{e}"))?;
        match key.delete_value(VALUE_NAME) {
            Ok(()) => Ok(()),
            Err(err) => {
                // Deleting a value that is not there is already the desired state.
                if err.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(format!("移除开机自启动失败：{err}"))
                }
            }
        }
    }
}

fn paths_equal(a: &std::path::Path, b: &std::path::Path) -> bool {
    let norm = |p: &std::path::Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_enabled(_enable: bool) -> Result<(), String> {
    Err("仅支持 Windows".into())
}

