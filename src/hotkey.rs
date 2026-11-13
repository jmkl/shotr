//! Global PrintScreen via RegisterHotKey: event-driven (WM_HOTKEY), zero polling.
use windows::core::Result;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

pub const HOTKEY_ID: i32 = 1;

/// MOD_NOREPEAT suppresses auto-repeat so holding the key fires once.
/// Fails (Err) if another app already owns PrintScreen; caller handles it.
pub fn register(hwnd: HWND) -> Result<()> {
    unsafe { RegisterHotKey(hwnd, HOTKEY_ID, MOD_NOREPEAT, VK_SNAPSHOT.0 as u32) }
}

pub fn unregister(hwnd: HWND) {
    unsafe {
        let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
    }
}
