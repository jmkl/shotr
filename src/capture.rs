//! One BitBlt of the whole virtual desktop into a 32bpp top-down DIB section
//! (the screen's native BGRA layout, so no format conversion). The frame is
//! shown frozen under the selection overlay and cropped on mouse-up.
use crate::image::Image;
use std::ffi::c_void;
use std::ptr::null_mut;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct Frame {
    pub dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *const u8,
    pub w: i32,
    pub h: i32,
    pub vx: i32,
    pub vy: i32,
}

impl Frame {
    pub fn grab() -> Option<Frame> {
        unsafe {
            // Virtual desktop origin can be negative (monitors left of/above primary).
            let (vx, vy) = (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
            );
            let (w, h) = (
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            );
            if w <= 0 || h <= 0 {
                return None;
            }
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(screen);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // negative = top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = null_mut();
            let bmp = match CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(b) if !bits.is_null() => b,
                _ => {
                    ReleaseDC(None, screen);
                    let _ = DeleteDC(dc);
                    return None;
                }
            };
            let old = SelectObject(dc, HGDIOBJ(bmp.0));
            // CAPTUREBLT includes layered/transparent windows.
            let blt = BitBlt(dc, 0, 0, w, h, screen, vx, vy, SRCCOPY | CAPTUREBLT);
            ReleaseDC(None, screen);
            let f = Frame {
                dc,
                bmp,
                old,
                bits: bits as *const u8,
                w,
                h,
                vx,
                vy,
            };
            if blt.is_err() {
                return None;
            } // Drop frees GDI objects
            Some(f)
        }
    }

    /// `r` is in virtual-desktop coordinates. One copy; alpha forced opaque
    /// (GDI leaves it 0, which some apps honor when pasting).
    pub fn crop(&self, r: RECT) -> Option<Image> {
        let l = (r.left - self.vx).clamp(0, self.w) as usize;
        let t = (r.top - self.vy).clamp(0, self.h) as usize;
        let rt = (r.right - self.vx).clamp(0, self.w) as usize;
        let b = (r.bottom - self.vy).clamp(0, self.h) as usize;
        if rt <= l || b <= t {
            return None;
        }
        let (w, h, stride) = (rt - l, b - t, self.w as usize * 4);
        let src = unsafe { std::slice::from_raw_parts(self.bits, stride * self.h as usize) };
        let mut out = Vec::with_capacity(w * h * 4);
        for y in t..b {
            out.extend_from_slice(&src[y * stride + l * 4..y * stride + rt * 4]);
        }
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Some(Image {
            w: w as u32,
            h: h as u32,
            bgra: out,
        })
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}
