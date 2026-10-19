//! Pequenos utilitários Win32 usados pelos dois aplicativos.

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, MessageBoxW, SetForegroundWindow, ShowWindow, MB_ICONERROR, SW_RESTORE,
};

pub fn local_time() -> SYSTEMTIME {
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    t
}

pub fn local_time_hms() -> String {
    let t = local_time();
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Cria um mutex nomeado; devolve `None` se outra instância já o criou.
pub fn single_instance(name: &str) -> Option<HANDLE> {
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide(name).as_ptr()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        None
    } else {
        Some(handle)
    }
}

pub fn focus_window(title: &str) {
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide(title).as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}

pub fn error_box(text: &str) {
    unsafe {
        MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(crate::APP_NAME).as_ptr(), MB_ICONERROR);
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "ArCondicionadoMidea";
/// Argumento que faz o app abrir só na bandeja.
pub const TRAY_ARG: &str = "--bandeja";

/// Comando gravado para iniciar com o Windows, se houver.
fn autostart_command() -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(RUN_VALUE).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if r != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn autostart_enabled() -> bool {
    autostart_command().is_some()
}

/// Se a inicialização automática aponta para este executável mas sem abrir na
/// bandeja (como gravava a versão em Python), acrescenta o argumento.
pub fn upgrade_autostart() {
    let Some(cmd) = autostart_command() else { return };
    let Ok(exe) = std::env::current_exe() else { return };
    let quoted = format!("\"{}\"", exe.display());
    if cmd.trim().eq_ignore_ascii_case(&quoted) {
        set_autostart(true);
    }
}

pub fn set_autostart(enable: bool) {
    unsafe {
        if enable {
            let exe = std::env::current_exe().unwrap_or_default();
            let cmd = wide(&format!("\"{}\" {TRAY_ARG}", exe.display()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(RUN_VALUE).as_ptr(),
                REG_SZ,
                cmd.as_ptr().cast(),
                (cmd.len() * 2) as u32,
            );
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(RUN_VALUE).as_ptr());
        }
    }
}
