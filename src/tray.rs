//! System tray icon + context menu (Shell_NotifyIconW).
use windows::core::{w, Result};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_TRAY: u32 = WM_APP + 1;
pub const CMD_QUIT: usize = 1001;
fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

pub fn load_ico_from_bytes(ico_bytes: &[u8], width: i32, height: i32) -> Result<HICON> {
    unsafe {
        let offset =
            LookupIconIdFromDirectoryEx(ico_bytes.as_ptr(), true, width, height, LR_DEFAULTCOLOR);

        if offset <= 0 {
            return Err(windows::core::Error::from_win32());
        }

        let offset = offset as usize;
        if offset >= ico_bytes.len() {
            return Err(windows::core::Error::from_win32());
        }

        CreateIconFromResourceEx(
            &ico_bytes[offset..],
            true,
            0x00030000,
            width,
            height,
            LR_DEFAULTCOLOR,
        )
    }
}

pub fn add(hwnd: HWND) -> Result<()> {
    let ico_data = include_bytes!("../assets/icon.ico");
    let mut nid = data(hwnd);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;

    // Fetch system small-icon dimensions so the tray icon is crisp at any DPI
    let (width, height) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
    let hicon = load_ico_from_bytes(ico_data, width, height)?;
    nid.hIcon = hicon;

    // Safely copy tooltip while guaranteeing space for the null terminator
    let tip: Vec<u16> = "shotr - PrintScreen to capture".encode_utf16().collect();
    let len = tip.len().min(nid.szTip.len() - 1);
    nid.szTip[..len].copy_from_slice(&tip[..len]);

    let res = unsafe { Shell_NotifyIconW(NIM_ADD, &nid).ok() };

    // Always free the local HICON handle, whether NIM_ADD succeeded or failed
    unsafe {
        let _ = DestroyIcon(hicon);
    }

    res
}
pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data(hwnd));
    }
}

/// Shows the right-click menu; returns the chosen command id (0 = none).
pub fn show_menu(hwnd: HWND) -> usize {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu dismisses when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        );
        let _ = PostMessageW(hwnd, WM_NULL, None, None);
        let _ = DestroyMenu(menu);
        cmd.0 as usize
    }
}
