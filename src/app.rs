//! App lifecycle: single-instance guard, hidden top-level window, message loop.
//! The window is a hidden *top-level* window (not message-only) so it still
//! receives the TaskbarCreated broadcast and can re-add the tray icon after
//! an Explorer restart.
use crate::{clipboard, hotkey, image, selection, storage, tray};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn show_error(msg: &str) {
    let wide: Vec<u16> = msg.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide.as_ptr()),
            w!("shotr"),
            MB_OK | MB_ICONERROR,
        )
    };
}

/// Background jobs still running (so shutdown can let them finish).
static PENDING: AtomicUsize = AtomicUsize::new(0);

/// Clipboard + WebP encode + save, off the UI thread. One short-lived thread
/// per screenshot, so nothing runs while idle.
pub fn finish(mut img: image::Image) {
    PENDING.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        let _ = clipboard::copy(&img); // a clipboard failure must not block saving
        let ok = image::encode_webp(&mut img).map_or(false, |m| storage::save(&m).is_ok());
        if !ok {
            unsafe {
                let _ = MessageBeep(MB_ICONHAND);
            }
        }
        PENDING.fetch_sub(1, Ordering::SeqCst);
    });
}

pub fn run() -> Result<()> {
    unsafe {
        // Report physical pixels on mixed-DPI multi-monitor setups.
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );

        // Single instance: second launch exits silently.
        let _mutex = CreateMutexW(None, true, w!("Local\\shotr_single_instance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = w!("shotr_main");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(windows::core::Error::from_win32());
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("shotr"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            hinst,
            None,
        )?;

        tray::add(hwnd)?;
        if let Err(e) = hotkey::register(hwnd) {
            // Recoverable: stay alive in the tray so the user can quit cleanly.
            show_error(&format!(
                "Could not register PrintScreen (in use by another app?): {e}"
            ));
        }

        // Blocks until a message arrives: 0% CPU while idle.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY if wp.0 as i32 == hotkey::HOTKEY_ID => {
            selection::show(); // ignores repeat presses while the overlay is open
            LRESULT(0)
        }
        tray::WM_TRAY => {
            if lp.0 as u32 == WM_RBUTTONUP {
                if tray::show_menu(hwnd) == tray::CMD_QUIT {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // Give in-flight saves up to 3 s so quitting never truncates a file.
            for _ in 0..300 {
                if PENDING.load(Ordering::SeqCst) == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            hotkey::unregister(hwnd);
            tray::remove(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
