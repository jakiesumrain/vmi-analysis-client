//! Notification-area icon and its menu.
//!
//! Windows uses a native notify icon, so the client keeps no webview and no
//! visible window. Other platforms get a no-op implementation for now.

/// Menu actions, handled by the async side of the client.
#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    OpenPanel,
    Reconnect,
    Disconnect,
    Quit,
}

pub use imp::{set_tooltip, start};

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::mem::zeroed;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    use tokio::sync::mpsc::UnboundedSender;
    use windows_sys::core::PCWSTR;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadImageW, PostMessageW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, HICON, HMENU,
        IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, MF_SEPARATOR, MF_STRING, MSG, TPM_NONOTIFY,
        TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW,
    };

    use super::TrayAction;

    const WM_APP: u32 = 0x8000;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const WM_NULL: u32 = 0;
    const TRAY_CALLBACK: u32 = WM_APP + 1;
    const TRAY_ID: u32 = 1;

    const MENU_OPEN: usize = 1;
    const MENU_RECONNECT: usize = 2;
    const MENU_DISCONNECT: usize = 3;
    const MENU_QUIT: usize = 4;

    static WINDOW: AtomicUsize = AtomicUsize::new(0);
    static ACTIONS: OnceLock<UnboundedSender<TrayAction>> = OnceLock::new();
    static TIP: Mutex<String> = Mutex::new(String::new());

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text).encode_wide().chain(Some(0)).collect()
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn send(action: TrayAction) {
        if let Some(sender) = ACTIONS.get() {
            let _ = sender.send(action);
        }
    }

    fn notify_icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        unsafe {
            let mut data: NOTIFYICONDATAW = zeroed();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = hwnd;
            data.uID = TRAY_ID;
            data.uCallbackMessage = TRAY_CALLBACK;
            data
        }
    }

    fn copy_tip(data: &mut NOTIFYICONDATAW, text: &str) {
        let encoded: Vec<u16> = OsStr::new(text)
            .encode_wide()
            .take(data.szTip.len() - 1)
            .collect();
        data.szTip[..encoded.len()].copy_from_slice(&encoded);
        data.szTip[encoded.len()] = 0;
    }

    fn update_icon(hwnd: HWND, icon: HICON, text: &str) {
        unsafe {
            let mut data = notify_icon_data(hwnd);
            if !icon.is_null() {
                data.uFlags |= NIF_ICON;
                data.hIcon = icon;
            }
            if !text.is_empty() {
                data.uFlags |= NIF_TIP;
                copy_tip(&mut data, text);
            }
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    fn remove_icon(hwnd: HWND) {
        unsafe {
            let data = notify_icon_data(hwnd);
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }

    fn load_icon(path: &Path) -> HICON {
        let path = wide_path(path);
        unsafe {
            let handle = LoadImageW(
                std::ptr::null_mut(),
                path.as_ptr() as PCWSTR,
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            );
            handle as HICON
        }
    }

    /// Write the embedded icon into the config directory, then load it.
    fn prepare_icon(config_dir: &Path) -> HICON {
        let icon_path = config_dir.join("vmi.ico");
        if std::fs::write(&icon_path, include_bytes!("../assets/vmi.ico")).is_err() {
            return std::ptr::null_mut();
        }
        load_icon(&icon_path)
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        message: u32,
        _wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            TRAY_CALLBACK => {
                let event = (lparam & 0xFFFF) as u32;
                if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                    show_menu(hwnd);
                } else if event == WM_LBUTTONDBLCLK {
                    send(TrayAction::OpenPanel);
                }
                0
            }
            WM_DESTROY => {
                remove_icon(hwnd);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, _wparam, lparam),
        }
    }

    fn show_menu(hwnd: HWND) {
        unsafe {
            let mut point = POINT { x: 0, y: 0 };
            let _ = GetCursorPos(&mut point);

            let menu: HMENU = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            let open = wide("Open panel");
            let reconnect = wide("Reconnect");
            let disconnect = wide("Disconnect");
            let quit = wide("Quit client");
            AppendMenuW(menu, MF_STRING, MENU_OPEN, open.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(menu, MF_STRING, MENU_RECONNECT, reconnect.as_ptr());
            AppendMenuW(menu, MF_STRING, MENU_DISCONNECT, disconnect.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(menu, MF_STRING, MENU_QUIT, quit.as_ptr());

            // The window has no focus, so make it foreground first or the menu
            // closes as soon as it appears.
            let _ = SetForegroundWindow(hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                point.x,
                point.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            let _ = DestroyMenu(menu);
            PostMessageW(hwnd, WM_NULL, 0, 0);

            match command as usize {
                MENU_OPEN => send(TrayAction::OpenPanel),
                MENU_RECONNECT => send(TrayAction::Reconnect),
                MENU_DISCONNECT => send(TrayAction::Disconnect),
                MENU_QUIT => send(TrayAction::Quit),
                _ => {}
            }
        }
    }

    pub fn start(config_dir: std::path::PathBuf, actions: UnboundedSender<TrayAction>) {
        let _ = ACTIONS.set(actions);
        std::thread::spawn(move || unsafe {
            let class_name = wide("VmiAnalysisClientTray");
            let title = wide("VMI Analysis Client");
            let hinstance = GetModuleHandleW(std::ptr::null());

            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&class);

            // A hidden top-level window, not a message-only window: the context
            // menu needs a foreground-able owner.
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return;
            }
            WINDOW.store(hwnd as usize, Ordering::SeqCst);

            let icon = prepare_icon(&config_dir);
            {
                let mut data = notify_icon_data(hwnd);
                data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                data.hIcon = icon;
                let text = TIP.lock().map(|tip| tip.clone()).unwrap_or_default();
                let text = if text.is_empty() {
                    "VMI Analysis Client".to_owned()
                } else {
                    text
                };
                copy_tip(&mut data, &text);
                // Explorer may not be ready right after logon; retry briefly.
                let mut added = 0;
                for _ in 0..5 {
                    added = Shell_NotifyIconW(NIM_ADD, &data);
                    if added != 0 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                if added == 0 {
                    // Usually means this process is not on the interactive
                    // desktop, or security software blocked the icon.
                    eprintln!(
                        "[warn] could not add the notification-area icon; the panel still works"
                    );
                }
            }

            let mut message: MSG = zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            remove_icon(hwnd);
            let _ = DestroyWindow(hwnd);
            WINDOW.store(0, Ordering::SeqCst);
        });
    }

    pub fn set_tooltip(text: &str) {
        if let Ok(mut tip) = TIP.lock() {
            *tip = text.to_owned();
        }
        let hwnd = WINDOW.load(Ordering::SeqCst) as HWND;
        if !hwnd.is_null() {
            update_icon(hwnd, std::ptr::null_mut(), text);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use tokio::sync::mpsc::UnboundedSender;

    use super::TrayAction;

    pub fn start(_config_dir: std::path::PathBuf, _actions: UnboundedSender<TrayAction>) {}

    pub fn set_tooltip(_text: &str) {}
}
