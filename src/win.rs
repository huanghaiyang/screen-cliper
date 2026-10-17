//! Minimal Win32 FFI helpers (no `windows` crate dependency).
//!
//! Only the handful of calls ScreenCliper needs: DPI awareness, cursor position,
//! local wall-clock time for filename templates, single-instance guard and
//! launching Explorer on a folder.

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::sync::OnceLock;

pub type HWND = isize;
pub type HANDLE = isize;

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SYSTEMTIME {
    pub wYear: u16,
    pub wMonth: u16,
    pub wDayOfWeek: u16,
    pub wDay: u16,
    pub wHour: u16,
    pub wMinute: u16,
    pub wSecond: u16,
    pub wMilliseconds: u16,
}

#[link(name = "user32")]
extern "system" {
    fn SetProcessDpiAwarenessContext(ctx: isize) -> i32;
    fn GetCursorPos(point: *mut POINT) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn ShowWindow(hwnd: HWND, cmd: i32) -> i32;
    fn SetForegroundWindow(hwnd: HWND) -> i32;
    fn EnumWindows(callback: extern "system" fn(HWND, isize) -> i32, lparam: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetClassNameW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(st: *mut SYSTEMTIME);
    fn CreateMutexW(attrs: *const u8, initial_owner: i32, name: *const u16) -> HANDLE;
    fn GetLastError() -> u32;
    fn GetCurrentProcessId() -> u32;
}

const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;
const ERROR_ALREADY_EXISTS: u32 = 183;

static DPI_DONE: OnceLock<()> = OnceLock::new();

/// Make the process per-monitor-DPI-aware v2 before any window exists, so captured
/// pixel coordinates and window sizes are real physical pixels instead of being
/// silently scaled by the OS.
pub fn init_dpi() {
    DPI_DONE.get_or_init(|| unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0 {
            // Fall back to the older API on systems where v2 is unavailable.
            #[link(name = "shcore")]
            extern "system" {
                fn SetProcessDpiAwareness(value: i32) -> i32;
            }
            let _ = SetProcessDpiAwareness(2); // PROCESS_PER_MONITOR_DPI_AWARE
        }
    });
}

pub fn cursor_position() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        if GetCursorPos(&mut p) != 0 {
            (p.x, p.y)
        } else {
            (0, 0)
        }
    }
}

pub fn local_time() -> SYSTEMTIME {
    let mut st = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut st) };
    st
}

pub fn virtual_screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(78), GetSystemMetrics(79)) }
}

static MUTEX: OnceLock<isize> = OnceLock::new();

/// Returns true when this is the first instance. The mutex handle is intentionally
/// leaked for the lifetime of the process.
pub fn acquire_single_instance(key: &str) -> bool {
    let mut wide: Vec<u16> = format!("Global\\{key}").encode_utf16().collect();
    wide.push(0);
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
        if handle == 0 {
            return true;
        }
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        let _ = MUTEX.set(handle);
        !already
    }
}

/// Best-effort focus request for an already-running instance.
pub fn show_foreground(hwnd: HWND) {
    unsafe {
        ShowWindow(hwnd, 9); // SW_RESTORE
        SetForegroundWindow(hwnd);
    }
}

pub fn open_folder(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

// ---------------------------------------------------------------------------
// Window show/hide
//
// eframe's `ViewportCommand::Visible` is only applied while its event loop is
// pumping, and a hidden window can stop receiving repaints — which would leave
// the app permanently invisible after a screenshot. Talking to Win32 directly
// makes hide/restore independent of the UI loop.
// ---------------------------------------------------------------------------

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;
const SW_RESTORE: i32 = 9;

pub fn current_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

struct FindCtx {
    pid: u32,
    found: HWND,
}

extern "system" fn enum_window_proc(hwnd: HWND, lparam: isize) -> i32 {
    unsafe {
        let ctx = &mut *(lparam as *mut FindCtx);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != ctx.pid {
            return 1;
        }
        let mut buf = [0u16; 64];
        let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len > 0 {
            let name = String::from_utf16_lossy(&buf[..len as usize]);
            // winit names its top-level windows "Window Class"; skip the helper
            // windows (tray, hotkey, IME, the 15x15 event target).
            if name == "Window Class" {
                ctx.found = hwnd;
                return 0;
            }
        }
        1
    }
}

/// Find this process' main winit window handle.
pub fn find_main_window() -> Option<HWND> {
    let mut ctx = FindCtx {
        pid: current_pid(),
        found: 0,
    };
    unsafe {
        EnumWindows(enum_window_proc, &mut ctx as *mut FindCtx as isize);
    }
    if ctx.found != 0 {
        Some(ctx.found)
    } else {
        None
    }
}

pub fn set_window_visible(hwnd: HWND, visible: bool) {
    unsafe {
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

pub fn restore_and_focus(hwnd: HWND) {
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}
