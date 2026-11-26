//! Environment diagnostics, printed with `vmi-analysis-client --diagnose`.

use std::path::{Path, PathBuf};

use crate::config_candidates;

/// Print what the client can and cannot do in this session.
pub fn report(config_dir: &Path) {
    println!("VMI Analysis Client diagnostics");
    println!(
        "executable: {}",
        std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "unknown".into())
    );
    println!("config dir: {}", config_dir.display());
    println!();

    println!("writable locations:");
    for (label, dir) in config_candidates() {
        println!(
            "  {label:<22} {:<6} {}",
            if crate::is_writable_dir(&dir) {
                "yes"
            } else {
                "no"
            },
            dir.display()
        );
    }

    #[cfg(windows)]
    {
        println!();
        println!("session id: {}", session_id());
        println!("window station: {}", window_station());
        println!("desktop: {}", desktop_name());
        println!(
            "explorer tray window: {}",
            match shell_window() {
                Some(handle) => format!("found (0x{handle:x})"),
                None => "not reachable".to_owned(),
            }
        );
    }
}

/// The default application-config directory for this platform.
pub fn default_config_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.vmi.analysis.client")
    } else if cfg!(target_os = "macos") {
        home_dir().join("Library/Application Support/com.vmi.analysis.client")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".config"))
            .join("com.vmi.analysis.client")
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Directory beside the executable, used when the profile is locked down.
pub fn portable_config_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("data"))
}

#[cfg(windows)]
fn user_object_name(handle: windows_sys::Win32::Foundation::HANDLE) -> String {
    use windows_sys::Win32::System::StationsAndDesktops::{GetUserObjectInformationW, UOI_NAME};

    let mut buffer = [0u16; 256];
    let mut needed = 0u32;
    let ok = unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            buffer.as_mut_ptr() as *mut core::ffi::c_void,
            (buffer.len() * 2) as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return "unknown".to_owned();
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

#[cfg(windows)]
fn window_station() -> String {
    use windows_sys::Win32::System::StationsAndDesktops::GetProcessWindowStation;

    user_object_name(unsafe { GetProcessWindowStation() })
}

#[cfg(windows)]
fn desktop_name() -> String {
    use windows_sys::Win32::System::StationsAndDesktops::GetThreadDesktop;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;

    user_object_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) })
}

#[cfg(windows)]
fn session_id() -> u32 {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;

    let mut id = 0u32;
    unsafe {
        ProcessIdToSessionId(std::process::id(), &mut id);
    }
    id
}

#[cfg(windows)]
fn shell_window() -> Option<usize> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    let class: Vec<u16> = OsStr::new("Shell_TrayWnd")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let handle = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
    if handle.is_null() {
        None
    } else {
        Some(handle as usize)
    }
}
