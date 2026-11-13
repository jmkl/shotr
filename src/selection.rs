//! Fullscreen selection overlay over the whole virtual desktop. The screen is
//! grabbed first and shown frozen: dimmed everywhere except the selection.
use crate::capture::Frame;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static OPEN: AtomicBool = AtomicBool::new(false);
thread_local! { static FRAME: RefCell<Option<Frame>> = RefCell::new(None); } // UI thread only

struct Sel {
    origin: POINT,
    cur: POINT,
    dragging: bool,
    vx: i32,
    vy: i32,
}
static STATE: Mutex<Sel> = Mutex::new(Sel {
    origin: POINT { x: 0, y: 0 },
    cur: POINT { x: 0, y: 0 },
    dragging: false,
    vx: 0,
    vy: 0,
});
fn st() -> std::sync::MutexGuard<'static, Sel> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn norm(a: POINT, b: POINT) -> RECT {
    RECT {
        left: a.x.min(b.x),
        top: a.y.min(b.y),
        right: a.x.max(b.x),
        bottom: a.y.max(b.y),
    }
}
fn pt(lp: LPARAM) -> POINT {
    POINT {
        x: (lp.0 & 0xFFFF) as i16 as i32,
        y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32,
    }
}

pub fn show() {
    if crate::annotation::is_open() {
        return;
    }
    if OPEN.swap(true, Ordering::SeqCst) {
        return;
    } // duplicate press
    let fail = || OPEN.store(false, Ordering::SeqCst);
    // Capture BEFORE the overlay exists so it never appears in the shot.
    let Some(frame) = Frame::grab() else {
        return fail();
    };
    let (vx, vy, vw, vh) = (frame.vx, frame.vy, frame.w, frame.h);
    unsafe {
        let hinst: HINSTANCE = match GetModuleHandleW(None) {
            Ok(h) => h.into(),
            Err(_) => return fail(),
        };
        let class = w!("shotr_overlay");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc); // harmless failure if already registered
        {
            let mut s = st();
            s.vx = vx;
            s.vy = vy;
            s.dragging = false;
        }
        FRAME.with(|f| *f.borrow_mut() = Some(frame));
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!(""),
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
            None,
            None,
            hinst,
            None,
        );
        let Ok(hwnd) = hwnd else {
            FRAME.with(|f| *f.borrow_mut() = None);
            return fail();
        };
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd); // needed to receive Esc
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            let p = pt(lp);
            {
                let mut s = st();
                s.origin = p;
                s.cur = p;
                s.dragging = true;
            }
            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (old, new) = {
                let mut s = st();
                if !s.dragging {
                    return LRESULT(0);
                }
                let old = norm(s.origin, s.cur);
                s.cur = pt(lp);
                (old, norm(s.origin, s.cur))
            };
            let r = RECT {
                // repaint only old+new rects, with room for the label
                left: old.left.min(new.left) - 120,
                top: old.top.min(new.top) - 40,
                right: old.right.max(new.right) + 120,
                bottom: old.bottom.max(new.bottom) + 40,
            };
            let _ = InvalidateRect(hwnd, Some(&r), false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let done = {
                let mut s = st();
                if !s.dragging {
                    None
                } else {
                    s.dragging = false;
                    let mut r = norm(s.origin, s.cur);
                    r.left += s.vx;
                    r.right += s.vx;
                    r.top += s.vy;
                    r.bottom += s.vy;
                    Some(r)
                }
            };
            let _ = ReleaseCapture();
            if let Some(r) = done {
                let img = FRAME.with(|f| f.borrow().as_ref().and_then(|fr| fr.crop(r)));
                if let Some(img) = img {
                    // Shift held on release = open the annotation editor.
                    if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                        crate::annotation::open(img, r.left, r.top);
                    } else {
                        crate::app::finish(img);
                    }
                }
            }
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let r = ps.rcPaint;
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if w > 0 && h > 0 {
                // Double-buffer just the dirty rect.
                let mem = CreateCompatibleDC(hdc);
                let bmp = CreateCompatibleBitmap(hdc, w, h);
                let old = SelectObject(mem, HGDIOBJ(bmp.0));
                let _ = SetViewportOrgEx(mem, -r.left, -r.top, None);
                let (dragging, sel) = {
                    let s = st();
                    (s.dragging, norm(s.origin, s.cur))
                };
                FRAME.with(|f| {
                    if let Some(fr) = f.borrow().as_ref() {
                        let _ = BitBlt(mem, r.left, r.top, w, h, fr.dc, r.left, r.top, SRCCOPY);
                        // Dim: alpha-blend a 1x1 black bitmap stretched over the dirty rect.
                        let dim = CreateCompatibleDC(hdc);
                        let db = CreateCompatibleBitmap(hdc, 1, 1);
                        let dold = SelectObject(dim, HGDIOBJ(db.0));
                        let _ = PatBlt(dim, 0, 0, 1, 1, BLACKNESS);
                        let bf = BLENDFUNCTION {
                            BlendOp: AC_SRC_OVER as u8,
                            BlendFlags: 0,
                            SourceConstantAlpha: 140,
                            AlphaFormat: 0,
                        };
                        let _ = AlphaBlend(mem, r.left, r.top, w, h, dim, 0, 0, 1, 1, bf);
                        SelectObject(dim, dold);
                        let _ = DeleteObject(HGDIOBJ(db.0));
                        let _ = DeleteDC(dim);
                        if dragging {
                            // selection shown undimmed
                            let _ = BitBlt(
                                mem,
                                sel.left,
                                sel.top,
                                sel.right - sel.left,
                                sel.bottom - sel.top,
                                fr.dc,
                                sel.left,
                                sel.top,
                                SRCCOPY,
                            );
                        }
                    }
                });
                if dragging {
                    FrameRect(mem, &sel, HBRUSH(GetStockObject(WHITE_BRUSH).0));
                    let label: Vec<u16> = format!(
                        "{} x {} | Press shift to anotate",
                        sel.right - sel.left,
                        sel.bottom - sel.top
                    )
                    .encode_utf16()
                    .collect();
                    SetBkMode(mem, TRANSPARENT);
                    SetTextColor(mem, COLORREF(0x00FFFFFF));
                    let _ = TextOutW(mem, sel.left, sel.bottom + 6, &label);
                }
                let _ = BitBlt(hdc, r.left, r.top, w, h, mem, r.left, r.top, SRCCOPY);
                SelectObject(mem, old);
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                let _ = DeleteDC(mem);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            FRAME.with(|f| *f.borrow_mut() = None); // frees the ~full-desktop bitmap
            OPEN.store(false, Ordering::SeqCst);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
