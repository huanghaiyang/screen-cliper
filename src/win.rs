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

// ---------------------------------------------------------------------------
// Console handling
//
// Release builds are GUI-subsystem executables: no console window is created, so
// there is no stray black console sitting on the desktop (it used to be captured
// in every screenshot). CLI modes still need their output, so they attach to the
// terminal that launched them.
// ---------------------------------------------------------------------------

const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE_VALUE: isize = -1;

#[link(name = "kernel32")]
extern "system" {
    fn AttachConsole(pid: u32) -> i32;
    fn FreeConsole() -> i32;
    fn SetStdHandle(which: u32, handle: HANDLE) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sec: *const std::ffi::c_void,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn GetConsoleWindow() -> HWND;
    fn GetConsoleProcessList(processes: *mut u32, count: u32) -> u32;
}

/// Point stdout/stderr at the parent terminal. Returns false when we were not
/// launched from a console (Explorer, autostart), which is the normal case.
pub fn attach_parent_console() -> bool {
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return false;
        }
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            );
            if handle != INVALID_HANDLE_VALUE && handle != 0 {
                SetStdHandle(which, handle);
            }
        }
        true
    }
}

/// Hide the console window before a capture so nothing console-shaped can end up
/// in the frame. Only a console this process owns is hidden: when the app was
/// started from a terminal, the console belongs to the terminal and must stay.
pub fn hide_console_window() {
    if !console_is_private() {
        return;
    }
    if let Some(hwnd) = console_window() {
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// True when this process got its own console window (Explorer launch of a
/// console-subsystem build) rather than sharing a terminal.
fn console_is_private() -> bool {
    let mut pids = [0u32; 2];
    let count = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) };
    count <= 1
}

pub fn show_console_window() {
    if let Some(hwnd) = console_window() {
        unsafe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

/// The console window owned by this process, if any (debug builds only).
pub fn console_window() -> Option<HWND> {
    let hwnd = unsafe { GetConsoleWindow() };
    if hwnd != 0 { Some(hwnd) } else { None }
}

/// Release GUI builds have no console at all; make sure nothing lingers.
pub fn detach_console_if_any() {
    unsafe {
        FreeConsole();
    }
}

// ---------------------------------------------------------------------------
// GDI screen grab (fallback backend)
//
// Windows Graphics Capture is the primary path because it is HDR-aware and is
// unaffected by GPU hardware-overlay planes. It can be refused by the OS in
// restricted processes and is unavailable on very old builds, so a plain BitBlt
// grab stays around as a fallback. On HDR desktops BitBlt may hand back black,
// which is why the caller validates the result instead of trusting it.
// ---------------------------------------------------------------------------

type HDC = isize;
type HBITMAP = isize;
type HGDIOBJ = isize;

const SRCCOPY: u32 = 0x00CC_0020;
const CAPTUREBLT: u32 = 0x4000_0000;
const DIB_RGB_COLORS: u32 = 0;
const BI_RGB: u32 = 0;
const SW_SHOWNOACTIVATE: i32 = 4;

#[repr(C)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
struct BITMAPINFO {
    header: BITMAPINFOHEADER,
    colors: [u32; 3],
}

#[link(name = "user32")]
extern "system" {
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> i32;
    fn CreateDIBSection(
        hdc: HDC,
        bmi: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut u8,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(obj: HGDIOBJ) -> i32;
    fn BitBlt(dst: HDC, x: i32, y: i32, w: i32, h: i32, src: HDC, sx: i32, sy: i32, rop: u32)
        -> i32;
}

/// A raw BGRA grab converted to RGBA8.
pub struct ScreenBitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// RAII cleanup so every early return releases the DCs and the DIB.
struct GdiGuard {
    screen: HDC,
    mem: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
}

impl Drop for GdiGuard {
    fn drop(&mut self) {
        unsafe {
            if self.old != 0 {
                SelectObject(self.mem, self.old);
            }
            if self.dib != 0 {
                DeleteObject(self.dib);
            }
            if self.mem != 0 {
                DeleteDC(self.mem);
            }
            if self.screen != 0 {
                ReleaseDC(0, self.screen);
            }
        }
    }
}

/// Grab a screen rectangle with BitBlt. Coordinates are physical pixels in the
/// virtual-screen space (`GetDC(NULL)` covers all monitors).
pub fn capture_screen_gdi(x: i32, y: i32, width: i32, height: i32) -> Result<ScreenBitmap, String> {
    if width <= 0 || height <= 0 {
        return Err("抓屏区域无效".to_string());
    }
    unsafe {
        let screen = GetDC(0);
        if screen == 0 {
            return Err("GetDC 失败".to_string());
        }
        let mem = CreateCompatibleDC(screen);
        if mem == 0 {
            ReleaseDC(0, screen);
            return Err("CreateCompatibleDC 失败".to_string());
        }
        let mut bits: *mut u8 = std::ptr::null_mut();
        let bmi = BITMAPINFO {
            header: BITMAPINFOHEADER {
                bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                bi_width: width,
                // Negative height = top-down DIB, matching image crate's row order.
                bi_height: -height,
                bi_planes: 1,
                bi_bit_count: 32,
                bi_compression: BI_RGB,
                bi_size_image: 0,
                bi_x_pels_per_meter: 0,
                bi_y_pels_per_meter: 0,
                bi_clr_used: 0,
                bi_clr_important: 0,
            },
            colors: [0; 3],
        };
        let dib = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, 0, 0);
        if dib == 0 || bits.is_null() {
            DeleteDC(mem);
            ReleaseDC(0, screen);
            return Err("CreateDIBSection 失败".to_string());
        }
        let old = SelectObject(mem, dib);
        let guard = GdiGuard {
            screen,
            mem,
            dib,
            old,
        };
        let copied = BitBlt(
            mem,
            0,
            0,
            width,
            height,
            screen,
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        ) != 0;
        if !copied {
            drop(guard);
            return Err("BitBlt 失败".to_string());
        }
        let len = (width as usize) * (height as usize) * 4;
        let src = std::slice::from_raw_parts(bits, len);
        let mut rgba = Vec::with_capacity(len);
        let mut i = 0;
        while i + 3 < len {
            rgba.push(src[i + 2]);
            rgba.push(src[i + 1]);
            rgba.push(src[i]);
            rgba.push(255);
            i += 4;
        }
        drop(guard);
        Ok(ScreenBitmap {
            width: width as u32,
            height: height as u32,
            rgba,
        })
    }
}