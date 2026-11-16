//! Windows: тёмный заголовок окна, фокус и мигание кнопки на панели задач, проводник, тема системы.
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::os::windows::process::CommandExt;
use windows_sys::Win32::Foundation::HWND;

pub fn hwnd(w: &slint::Window) -> Option<HWND> {
    let h = w.window_handle();
    match h.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as HWND),
        _ => None,
    }
}

/// Тёмный системный заголовок, как у приложений Windows 11 (DWMWA_USE_IMMERSIVE_DARK_MODE).
pub fn set_dark_title(w: &slint::Window, dark: bool) {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER};
    let Some(h) = hwnd(w) else { return };
    let v: i32 = dark as i32;
    unsafe {
        for attr in [20u32, 19] {
            if DwmSetWindowAttribute(h, attr, &v as *const i32 as *const _, 4) == 0 {
                break;
            }
        }
        SetWindowPos(h, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
    }
}

/// Поднять окно; если Windows не даёт забрать фокус — мигать кнопкой на панели задач.
pub fn focus_or_flash(w: &slint::Window) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FlashWindowEx, IsIconic, SetForegroundWindow, ShowWindow, FLASHWINFO, FLASHW_ALL, FLASHW_TIMERNOFG, SW_RESTORE,
    };
    let Some(h) = hwnd(w) else { return };
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        if SetForegroundWindow(h) == 0 {
            let fw = FLASHWINFO { cbSize: std::mem::size_of::<FLASHWINFO>() as u32, hwnd: h, dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG, uCount: 3, dwTimeout: 0 };
            FlashWindowEx(&fw);
        }
    }
}

/// Windows в тёмном режиме для приложений (AppsUseLightTheme = 0).
pub fn system_dark() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0".encode_utf16().collect();
    let val: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut data: u32 = 1;
    let mut size: u32 = 4;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut data as *mut u32 as *mut _, &mut size) };
    r == 0 && data == 0
}

pub fn set_app_id() {
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    let id: Vec<u16> = "ToPicture.App\0".encode_utf16().collect();
    unsafe { SetCurrentProcessExplicitAppUserModelID(id.as_ptr()) };
}

/// Открыть папку или файл в проводнике.
pub fn open(path: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let p: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), p.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
}

/// Показать файл в папке (выделенным); файла нет — открыть его папку.
pub fn show_in_folder(path: &str) {
    let p = std::path::Path::new(path);
    if p.exists() {
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{path}\"")).spawn();
    } else if let Some(d) = p.parent() {
        open(&d.to_string_lossy());
    }
}
