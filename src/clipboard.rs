//! Native CF_DIB clipboard copy (what Paint, Office, browsers, chat apps read).
//!

use crate::image::Image;
use std::time::Duration;
use windows::core::{Error, Result};
use windows::Win32::Foundation::{GlobalFree, HANDLE};
use windows::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_DIB;

pub fn copy(img: &Image) -> Result<()> {
    let (w, h) = (img.w as usize, img.h as usize);
    let stride = w * 4;
    let hdr = std::mem::size_of::<BITMAPINFOHEADER>();
    unsafe {
        let hmem = GlobalAlloc(GMEM_MOVEABLE, hdr + stride * h)?;
        let p = GlobalLock(hmem) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(hmem);
            return Err(Error::from_win32());
        }
        std::ptr::write(
            p as *mut BITMAPINFOHEADER,
            BITMAPINFOHEADER {
                biSize: hdr as u32,
                biWidth: w as i32,
                biHeight: h as i32, // positive = bottom-up: negative heights confuse many apps
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (stride * h) as u32,
                ..Default::default()
            },
        );
        for y in 0..h {
            // flip rows while copying (the only copy)
            std::ptr::copy_nonoverlapping(
                img.bgra.as_ptr().add(y * stride),
                p.add(hdr + (h - 1 - y) * stride),
                stride,
            );
        }
        let _ = GlobalUnlock(hmem);

        // Another app may hold the clipboard briefly; retry.
        let mut opened = false;
        for _ in 0..20 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !opened {
            let _ = GlobalFree(hmem);
            return Err(Error::from_win32());
        }
        let res = EmptyClipboard()
            .and_then(|_| SetClipboardData(CF_DIB.0 as u32, HANDLE(hmem.0)).map(|_| ()));
        if res.is_err() {
            let _ = GlobalFree(hmem);
        } // on success the clipboard owns hmem
        let _ = CloseClipboard();
        res
    }
}
