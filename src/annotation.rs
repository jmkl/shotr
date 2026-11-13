//! Optional annotation editor (opened with Shift held on mouse release).
//! Annotations are vectors in image-pixel coordinates; the original pixels stay
//! untouched in `base` until Done, when everything is rendered once into a
//! fresh DIB (GDI+ for anti-aliased shapes, GDI for text) and sent to the
//! normal clipboard + WebP pipeline.
use crate::image::Image;
use crate::smooth::{PenStroke, Pt};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::Once;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::GdiPlus as gp;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const TB: i32 = 36; // toolbar height
const MINW: i32 = 660; // toolbar needs this width
const COLORS: [u32; 6] = [
    0xFFE53935, 0xFFFB8C00, 0xFFFDD835, 0xFF43A047, 0xFF1E88E5, 0xFF000000,
];
const WIDTHS: [f32; 3] = [2.0, 4.0, 8.0];
const SMOOTHING: f32 = 0.7; // brush stabilizer 0..1, TODO: move to settings

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Pen,
    Rect,
    Arrow,
    Text,
}
#[derive(Clone, Copy, PartialEq)]
enum Item {
    Tool(Tool),
    Color(u32),
    Width,
    Undo,
    Redo,
    Ok,
    Cancel,
}
enum Act {
    None,
    Done,
    Cancel,
}

enum Ann {
    Pen(PenStroke),
    Rect(Pt, Pt, u32, f32),
    Arrow(Pt, Pt, u32, f32),
    Text(Pt, String, u32, f32), // f32 = font pixel height
}

struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    w: i32,
    h: i32,
}
impl Dib {
    fn new(w: i32, h: i32) -> Option<Dib> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
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
                    let _ = DeleteDC(dc);
                    return None;
                }
            };
            let old = SelectObject(dc, HGDIOBJ(bmp.0));
            Some(Dib {
                dc,
                bmp,
                old,
                bits: bits as *mut u8,
                w,
                h,
            })
        }
    }
    fn px(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.w * self.h * 4) as usize) }
    }
    fn px_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h * 4) as usize) }
    }
}
impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

struct Editor {
    hwnd: HWND,
    base: Dib,
    back: Dib,
    w: i32,
    h: i32,
    ww: i32,
    anns: Vec<Ann>,
    redo: Vec<Ann>,
    cur: Option<Ann>,
    drag: bool,
    text: Option<(Pt, String)>,
    tool: Tool,
    color: u32,
    wi: usize,
}

thread_local! { static ED: RefCell<Option<Editor>> = RefCell::new(None); } // UI thread only

fn img_rect(e: &Editor) -> RECT {
    RECT {
        left: 0,
        top: TB,
        right: e.ww,
        bottom: TB + e.h,
    }
}

fn with<R>(f: impl FnOnce(&mut Editor) -> R) -> Option<R> {
    ED.with(|e| e.borrow_mut().as_mut().map(f))
}
pub fn is_open() -> bool {
    ED.with(|e| e.borrow().is_some())
}

impl Editor {
    fn width(&self) -> f32 {
        WIDTHS[self.wi]
    }
    fn commit_text(&mut self) {
        if let Some((p, s)) = self.text.take() {
            if !s.is_empty() {
                self.anns
                    .push(Ann::Text(p, s, self.color, 14.0 + self.width() * 4.0));
                self.redo.clear();
            }
        }
    }
    fn export(&mut self) -> Option<Image> {
        self.commit_text();
        let mut out = Dib::new(self.w, self.h)?;
        out.px_mut().copy_from_slice(self.base.px());
        let items: Vec<&Ann> = self.anns.iter().collect();
        unsafe {
            draw_all(out.dc, &items);
            let _ = GdiFlush();
        }
        let mut v = out.px().to_vec();
        for p in v.chunks_exact_mut(4) {
            p[3] = 255;
        } // GDI text zeroes alpha
        Some(Image {
            w: self.w as u32,
            h: self.h as u32,
            bgra: v,
        })
    }
    fn toolbar(&mut self, it: Item) -> Act {
        match it {
            Item::Tool(t) => {
                self.commit_text();
                self.tool = t;
            }
            Item::Color(c) => self.color = c,
            Item::Width => self.wi = (self.wi + 1) % WIDTHS.len(),
            Item::Undo => self.undo(),
            Item::Redo => self.redo_op(),
            Item::Ok => return Act::Done,
            Item::Cancel => return Act::Cancel,
        }
        Act::None
    }
    fn undo(&mut self) {
        self.commit_text();
        if let Some(a) = self.anns.pop() {
            self.redo.push(a);
        }
    }
    fn redo_op(&mut self) {
        if let Some(a) = self.redo.pop() {
            self.anns.push(a);
        }
    }
}

fn layout() -> Vec<(RECT, Item)> {
    let mut v = vec![];
    let mut x = 4;
    let mut add = |wd: i32, it: Item| {
        v.push((
            RECT {
                left: x,
                top: 4,
                right: x + wd,
                bottom: TB - 4,
            },
            it,
        ));
        x += wd + 4;
    };
    for t in [Tool::Pen, Tool::Rect, Tool::Arrow, Tool::Text] {
        add(52, Item::Tool(t));
    }
    for c in COLORS {
        add(24, Item::Color(c));
    }
    add(52, Item::Width);
    add(52, Item::Undo);
    add(52, Item::Redo);
    add(40, Item::Ok);
    add(40, Item::Cancel);
    v
}

fn colorref(c: u32) -> COLORREF {
    COLORREF(((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF))
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn lo_hi(lp: LPARAM) -> (i32, i32) {
    (
        (lp.0 & 0xFFFF) as i16 as i32,
        ((lp.0 >> 16) & 0xFFFF) as i16 as i32,
    )
}

fn gdiplus_init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let mut token = 0usize;
        let input = gp::GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        gp::GdiplusStartup(&mut token, &input, null_mut()); // never shut down; the OS reclaims it
    });
}

/// Draws annotations onto `hdc` (image-pixel coordinates).
unsafe fn draw_all(hdc: HDC, items: &[&Ann]) {
    let mut g: *mut gp::GpGraphics = null_mut();
    let _ = gp::GdipCreateFromHDC(hdc, &mut g);
    if g.is_null() {
        return;
    }
    gp::GdipSetSmoothingMode(g, gp::SmoothingModeAntiAlias);
    for a in items {
        let (color, width) = match a {
            Ann::Pen(s) => {
                draw_brush(g, s);
                continue;
            }
            Ann::Rect(_, _, c, w) | Ann::Arrow(_, _, c, w) => (*c, *w),
            Ann::Text(p, s, c, size) => {
                gp::GdipFlush(g, gp::FlushIntentionSync);
                text(hdc, *p, s, *c, *size);
                continue;
            }
        };
        let mut pen: *mut gp::GpPen = null_mut();
        let _ = gp::GdipCreatePen1(color, width, gp::UnitPixel, &mut pen);
        if pen.is_null() {
            continue;
        }
        gp::GdipSetPenStartCap(pen, gp::LineCapRound);
        gp::GdipSetPenEndCap(pen, gp::LineCapRound);
        if !matches!(a, Ann::Rect(..)) {
            gp::GdipSetPenLineJoin(pen, gp::LineJoinRound);
        }
        let pf = |p: Pt| gp::PointF { X: p.x, Y: p.y };
        match a {
            Ann::Pen(_) => {}
            Ann::Rect(p, q, ..) => {
                gp::GdipDrawRectangle(
                    g,
                    pen,
                    p.x.min(q.x),
                    p.y.min(q.y),
                    (p.x - q.x).abs(),
                    (p.y - q.y).abs(),
                );
            }
            Ann::Arrow(p, q, _, w) => {
                if p.dist2(*q) >= 4.0 {
                    let ang = (q.y - p.y).atan2(q.x - p.x);
                    let hl = 10.0 + 3.0 * *w;
                    let wing = |d: f32| gp::PointF {
                        X: q.x - hl * (ang + d).cos(),
                        Y: q.y - hl * (ang + d).sin(),
                    };
                    gp::GdipDrawLine(g, pen, p.x, p.y, q.x, q.y);
                    let head = [wing(-0.5), pf(*q), wing(0.5)];
                    gp::GdipDrawLines(g, pen, head.as_ptr(), 3);
                }
            }
            Ann::Text(..) => {}
        }
        gp::GdipDeletePen(pen);
    }
    gp::GdipDeleteGraphics(g);
}

/// Brush stroke = one anti-aliased filled outline + round end caps / click dot.
unsafe fn draw_brush(g: *mut gp::GpGraphics, s: &PenStroke) {
    let mut fill: *mut gp::GpSolidFill = null_mut();
    let _ = gp::GdipCreateSolidFill(s.color, &mut fill);
    if fill.is_null() {
        return;
    }
    let br = fill as *mut gp::GpBrush;
    if s.poly.len() >= 3 {
        let pts: Vec<gp::PointF> = s
            .poly
            .iter()
            .map(|p| gp::PointF { X: p.x, Y: p.y })
            .collect();
        gp::GdipFillPolygon(g, br, pts.as_ptr(), pts.len() as i32, gp::FillModeWinding);
    }
    for &(c, d) in &s.caps {
        gp::GdipFillEllipse(g, br, c.x - d / 2.0, c.y - d / 2.0, d, d);
    }
    gp::GdipDeleteBrush(br);
}

unsafe fn text(hdc: HDC, p: Pt, s: &str, color: u32, size: f32) {
    // charset 1 = DEFAULT_CHARSET, quality 4 = ANTIALIASED_QUALITY
    let font = CreateFontW(
        -(size as i32),
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        1,
        0,
        0,
        4,
        0,
        w!("Segoe UI"),
    );
    let old = SelectObject(hdc, HGDIOBJ(font.0));
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, colorref(color));
    let _ = TextOutW(hdc, p.x as i32, p.y as i32, &wide(s));
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(font.0));
}

pub fn open(img: Image, left: i32, top: i32) {
    if is_open() {
        return;
    }
    gdiplus_init();
    let (w, h) = (img.w as i32, img.h as i32);
    let (Some(mut base), Some(back)) = (Dib::new(w, h), Dib::new(w, h)) else {
        return;
    };
    base.px_mut().copy_from_slice(&img.bgra);
    drop(img);
    unsafe {
        let ww = w.max(MINW);
        let (vx, vy) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        );
        let (vw, vh) = (
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        );
        // Sit over the selection (toolbar just above it), kept inside the desktop.
        let x = left.min(vx + vw - ww).max(vx);
        let y = (top - TB).min(vy + vh - (TB + h)).max(vy);
        let Ok(hinst) = GetModuleHandleW(None) else {
            return;
        };
        let class = w!("shotr_editor");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!(""),
            WS_POPUP,
            x,
            y,
            ww,
            TB + h,
            None,
            None,
            hinst,
            None,
        ) else {
            return;
        };
        ED.with(|e| {
            *e.borrow_mut() = Some(Editor {
                hwnd,
                base,
                back,
                w,
                h,
                ww,
                anns: vec![],
                redo: vec![],
                cur: None,
                drag: false,
                text: None,
                tool: Tool::Pen,
                color: COLORS[0],
                wi: 1,
            })
        });
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);
    }
}

unsafe fn finish_edit(hwnd: HWND) {
    let img = with(|e| e.export()).flatten();
    let _ = DestroyWindow(hwnd); // after the borrow ends: WM_DESTROY re-borrows
    if let Some(img) = img {
        crate::app::finish(img);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let act = match msg {
        WM_LBUTTONDOWN => {
            let (x, y) = lo_hi(lp);
            with(|e| {
                if y < TB {
                    for (r, it) in layout() {
                        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                            let a = e.toolbar(it);
                            let _ = InvalidateRect(e.hwnd, None, false);
                            return a;
                        }
                    }
                } else if x < e.w {
                    let p = Pt {
                        x: x as f32,
                        y: (y - TB) as f32,
                    };
                    let (c, wd) = (e.color, e.width());
                    match e.tool {
                        Tool::Pen => {
                            e.cur = Some(Ann::Pen(PenStroke::begin(c, wd * 1.6, SMOOTHING, p)));
                            e.drag = true;
                        }
                        Tool::Rect => {
                            e.cur = Some(Ann::Rect(p, p, c, wd));
                            e.drag = true;
                        }
                        Tool::Arrow => {
                            e.cur = Some(Ann::Arrow(p, p, c, wd));
                            e.drag = true;
                        }
                        Tool::Text => {
                            e.commit_text();
                            e.text = Some((p, String::new()));
                        }
                    }
                    if e.drag {
                        SetCapture(e.hwnd);
                    }
                    let _ = InvalidateRect(e.hwnd, Some(&img_rect(e)), false);
                }
                Act::None
            })
            .unwrap_or(Act::None)
        }
        WM_MOUSEMOVE => {
            let (x, y) = lo_hi(lp);
            with(|e| {
                if e.drag {
                    let p = Pt {
                        x: x as f32,
                        y: (y - TB) as f32,
                    };
                    match e.cur.as_mut() {
                        Some(Ann::Pen(s)) => s.add(p),
                        Some(Ann::Rect(_, b, ..)) | Some(Ann::Arrow(_, b, ..)) => *b = p,
                        _ => {}
                    }
                    let _ = InvalidateRect(e.hwnd, Some(&img_rect(e)), false);
                }
                Act::None
            })
            .unwrap_or(Act::None)
        }
        WM_LBUTTONUP => {
            let (x, y) = lo_hi(lp);
            with(|e| {
                if e.drag {
                    e.drag = false;
                    let _ = ReleaseCapture();
                    let p = Pt {
                        x: x as f32,
                        y: (y - TB) as f32,
                    };
                    if let Some(mut a) = e.cur.take() {
                        let keep = match &mut a {
                            Ann::Pen(s) => {
                                s.finish(p);
                                true
                            }
                            Ann::Rect(q, b, ..) | Ann::Arrow(q, b, ..) => {
                                *b = p;
                                q.dist2(p) >= 4.0
                            }
                            _ => true,
                        };
                        if keep {
                            e.anns.push(a);
                            e.redo.clear();
                        }
                    }
                    let _ = InvalidateRect(e.hwnd, Some(&img_rect(e)), false);
                }
                Act::None
            })
            .unwrap_or(Act::None)
        }
        WM_CHAR => {
            let c = wp.0 as u32;
            with(|e| {
                if e.text.is_some() {
                    match c {
                        8 => {
                            if let Some((_, s)) = e.text.as_mut() {
                                s.pop();
                            }
                        }
                        13 => e.commit_text(),
                        27 => e.text = None,
                        c if c >= 32 => {
                            if let (Some((_, s)), Some(ch)) = (e.text.as_mut(), char::from_u32(c)) {
                                s.push(ch);
                            }
                        }
                        _ => {}
                    }
                    let _ = InvalidateRect(e.hwnd, Some(&img_rect(e)), false);
                }
                Act::None
            })
            .unwrap_or(Act::None)
        }
        WM_KEYDOWN => {
            let k = wp.0;
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            with(|e| {
                let typing = e.text.is_some();
                let a = if ctrl && !typing && k == VK_Z.0 as usize && !shift {
                    e.undo();
                    Act::None
                } else if ctrl
                    && !typing
                    && (k == VK_Y.0 as usize || (k == VK_Z.0 as usize && shift))
                {
                    e.redo_op();
                    Act::None
                } else if !typing && k == VK_RETURN.0 as usize {
                    Act::Done
                } else if !typing && k == VK_ESCAPE.0 as usize {
                    Act::Cancel
                } else {
                    Act::None
                }; // typing: WM_CHAR handles Enter/Esc/Backspace
                let _ = InvalidateRect(e.hwnd, Some(&img_rect(e)), false);
                a
            })
            .unwrap_or(Act::None)
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            ED.with(|e| {
                if let Some(ed) = e.borrow().as_ref() {
                    paint(hdc, ed, ps.rcPaint);
                }
            });
            let _ = EndPaint(hwnd, &ps);
            return LRESULT(0);
        }
        WM_DESTROY => {
            ED.with(|e| *e.borrow_mut() = None);
            return LRESULT(0);
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    };
    match act {
        Act::Done => finish_edit(hwnd),
        Act::Cancel => {
            let _ = DestroyWindow(hwnd);
        }
        Act::None => {}
    }
    LRESULT(0)
}

unsafe fn paint(hdc: HDC, e: &Editor, dirty: RECT) {
    if dirty.bottom > TB {
        // Image: original pixels + vector annotations into the back buffer, then one blit.
        let _ = BitBlt(e.back.dc, 0, 0, e.w, e.h, e.base.dc, 0, 0, SRCCOPY);
        let typing = e
            .text
            .as_ref()
            .map(|(p, s)| Ann::Text(*p, format!("{s}|"), e.color, 14.0 + e.width() * 4.0));
        let items: Vec<&Ann> = e
            .anns
            .iter()
            .chain(e.cur.iter())
            .chain(typing.iter())
            .collect();
        draw_all(e.back.dc, &items);
        let _ = GdiFlush();
        let _ = BitBlt(hdc, 0, TB, e.w, e.h, e.back.dc, 0, 0, SRCCOPY);
        if e.ww > e.w {
            // toolbar is wider than a narrow image
            let gap = RECT {
                left: e.w,
                top: TB,
                right: e.ww,
                bottom: TB + e.h,
            };
            FillRect(hdc, &gap, HBRUSH(GetStockObject(DKGRAY_BRUSH).0));
        }
    }
    if dirty.top < TB {
        // Toolbar: composed off-screen and blitted once, so it never flickers.
        let tdc = CreateCompatibleDC(hdc);
        let tbmp = CreateCompatibleBitmap(hdc, e.ww, TB);
        let told = SelectObject(tdc, HGDIOBJ(tbmp.0));
        FillRect(
            tdc,
            &RECT {
                left: 0,
                top: 0,
                right: e.ww,
                bottom: TB,
            },
            HBRUSH(GetStockObject(DKGRAY_BRUSH).0),
        );
        let font = SelectObject(tdc, GetStockObject(DEFAULT_GUI_FONT));
        SetBkMode(tdc, TRANSPARENT);
        for (r, it) in layout() {
            if let Item::Color(c) = it {
                let b = CreateSolidBrush(colorref(c));
                FillRect(tdc, &r, b);
                if c == e.color {
                    FrameRect(tdc, &r, HBRUSH(GetStockObject(WHITE_BRUSH).0));
                }
                let _ = DeleteObject(HGDIOBJ(b.0));
            } else {
                let active = it == Item::Tool(e.tool);
                FillRect(
                    tdc,
                    &r,
                    HBRUSH(GetStockObject(if active { GRAY_BRUSH } else { LTGRAY_BRUSH }).0),
                );
                let label = match it {
                    Item::Tool(Tool::Pen) => "Pen".to_string(),
                    Item::Tool(Tool::Rect) => "Rect".into(),
                    Item::Tool(Tool::Arrow) => "Arrow".into(),
                    Item::Tool(Tool::Text) => "Text".into(),
                    Item::Width => format!("W {}", e.width() as i32),
                    Item::Undo => "Undo".into(),
                    Item::Redo => "Redo".into(),
                    Item::Ok => "OK".into(),
                    _ => "X".into(),
                };
                SetTextColor(tdc, COLORREF(0));
                let _ = TextOutW(tdc, r.left + 6, r.top + 5, &wide(&label));
            }
        }
        SelectObject(tdc, font);
        let _ = BitBlt(hdc, 0, 0, e.ww, TB, tdc, 0, 0, SRCCOPY);
        SelectObject(tdc, told);
        let _ = DeleteObject(HGDIOBJ(tbmp.0));
        let _ = DeleteDC(tdc);
    }
}
