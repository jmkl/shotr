//! Organic, stabilized brush.
//!  1. Lazy brush: the pen tip trails the cursor on a "string" of `radius` px.
//!     Hand jitter smaller than the string never moves it, and curves round off,
//!     which makes circles and loops easy.
//!  2. Speed -> width: slow = thicker, fast = thinner (smoothed), like a real brush.
//!  3. Catmull-Rom spline through the tip path, widths interpolated alongside.
//!  4. Tapered start and end, rendered as one filled outline polygon + round caps.
//!  5. On release: tip glides to the cursor, then the whole stroke is re-smoothed:
//!     resampled to even spacing, Taubin-filtered (removes wobble without
//!     shrinking circles) and its widths smoothed, so the committed stroke is
//!     smoother than the live preview, like Procreate's StreamLine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    #[inline]
    pub fn dist2(self, o: Pt) -> f32 {
        let (dx, dy) = (self.x - o.x, self.y - o.y);
        dx * dx + dy * dy
    }
    #[inline]
    pub fn dist(self, o: Pt) -> f32 {
        self.dist2(o).sqrt()
    }
    #[inline]
    fn lerp(self, o: Pt, t: f32) -> Pt {
        Pt {
            x: self.x + (o.x - self.x) * t,
            y: self.y + (o.y - self.y) * t,
        }
    }
}

#[inline]
fn cr(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * (2.0 * b
        + (-a + c) * t
        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
        + (-a + 3.0 * b - 3.0 * c + d) * t3)
}

/// Catmull-Rom through `ctrl`, with per-point widths `cw` interpolated the same way.
pub fn flatten(ctrl: &[Pt], cw: &[f32], out: &mut Vec<Pt>, ow: &mut Vec<f32>) {
    out.clear();
    ow.clear();
    match ctrl.len() {
        0 => return,
        1 => {
            out.push(ctrl[0]);
            ow.push(cw[0]);
            return;
        }
        _ => {}
    }
    out.push(ctrl[0]);
    ow.push(cw[0]);
    let n = ctrl.len();
    for i in 0..n - 1 {
        let (i0, i3) = (i.saturating_sub(1), (i + 2).min(n - 1));
        let (p0, p1, p2, p3) = (ctrl[i0], ctrl[i], ctrl[i + 1], ctrl[i3]);
        let steps = ((p1.dist(p2) / 2.0).ceil() as usize).clamp(1, 16);
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            out.push(Pt {
                x: cr(p0.x, p1.x, p2.x, p3.x, t),
                y: cr(p0.y, p1.y, p2.y, p3.y, t),
            });
            ow.push(cr(cw[i0], cw[i], cw[i + 1], cw[i3], t).max(0.3));
        }
    }
}

/// Polygon outline of a variable-width stroke, with tapered ends, plus end caps.
fn outline(pts: &[Pt], w: &[f32], taper_out: bool, poly: &mut Vec<Pt>, caps: &mut Vec<(Pt, f32)>) {
    poly.clear();
    caps.clear();
    let n = pts.len();
    if n == 0 {
        return;
    }
    if n == 1 {
        caps.push((pts[0], w[0] * 1.25));
        return;
    }
    let mut cum = vec![0.0f32; n];
    for i in 1..n {
        cum[i] = cum[i - 1] + pts[i].dist(pts[i - 1]);
    }
    let total = cum[n - 1];
    let width_at = |i: usize| {
        let mut k = (0.3 + 0.7 * cum[i] / 10.0).min(1.0); // ease in over 10 px
        if taper_out {
            k = k.min(0.15 + 0.85 * (total - cum[i]) / 16.0);
        } // ease out over 16 px
        (w[i] * k).max(0.5)
    };
    let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut tan = Pt { x: 1.0, y: 0.0 };
    for i in 0..n {
        let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(n - 1)]);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let l = (dx * dx + dy * dy).sqrt();
        if l > 1e-4 {
            tan = Pt {
                x: dx / l,
                y: dy / l,
            };
        }
        let (h, p) = (width_at(i) * 0.5, pts[i]);
        left.push(Pt {
            x: p.x - tan.y * h,
            y: p.y + tan.x * h,
        });
        right.push(Pt {
            x: p.x + tan.y * h,
            y: p.y - tan.x * h,
        });
    }
    poly.extend(left);
    poly.extend(right.into_iter().rev());
    caps.push((pts[0], width_at(0)));
    caps.push((pts[n - 1], width_at(n - 1)));
}

/// Even arc-length resampling (keeps both endpoints), widths interpolated along.
fn resample(ctrl: &[Pt], cw: &[f32], step: f32) -> (Vec<Pt>, Vec<f32>) {
    let n = ctrl.len();
    let mut cum = vec![0.0f32; n];
    for i in 1..n {
        cum[i] = cum[i - 1] + ctrl[i].dist(ctrl[i - 1]);
    }
    let total = cum[n - 1];
    if n < 3 || total < step {
        return (ctrl.to_vec(), cw.to_vec());
    }
    let m = (total / step).ceil() as usize;
    let (mut op, mut ow) = (Vec::with_capacity(m + 1), Vec::with_capacity(m + 1));
    let mut seg = 0;
    for k in 0..=m {
        let s = total * k as f32 / m as f32;
        while seg + 2 < n && cum[seg + 1] < s {
            seg += 1;
        }
        let len = cum[seg + 1] - cum[seg];
        let t = if len > 1e-6 {
            ((s - cum[seg]) / len).clamp(0.0, 1.0)
        } else {
            0.0
        };
        op.push(ctrl[seg].lerp(ctrl[seg + 1], t));
        ow.push(cw[seg] + (cw[seg + 1] - cw[seg]) * t);
    }
    (op, ow)
}

/// Taubin (lambda|mu) smoothing: removes wobble but, unlike plain averaging,
/// does not shrink curves, so circles stay circles. Endpoints stay fixed.
fn taubin(p: &mut [Pt], iters: usize) {
    if p.len() < 3 {
        return;
    }
    for _ in 0..iters {
        for f in [0.5f32, -0.53] {
            let src = p.to_vec();
            for i in 1..p.len() - 1 {
                p[i].x += f * ((src[i - 1].x + src[i + 1].x) * 0.5 - src[i].x);
                p[i].y += f * ((src[i - 1].y + src[i + 1].y) * 0.5 - src[i].y);
            }
        }
    }
}

fn smooth_scalar(w: &mut [f32], passes: usize) {
    if w.len() < 3 {
        return;
    }
    for _ in 0..passes {
        let src = w.to_vec();
        for i in 1..w.len() - 1 {
            w[i] = 0.25 * src[i - 1] + 0.5 * src[i] + 0.25 * src[i + 1];
        }
    }
}

pub struct PenStroke {
    pub color: u32, // 0xAARRGGBB
    base: f32,
    radius: f32, // lazy-brush string length
    strength: f32,
    brush: Pt,
    wstate: f32,
    ctrl: Vec<Pt>,
    cw: Vec<f32>,
    flat: Vec<Pt>,
    fw: Vec<f32>,
    pub poly: Vec<Pt>,        // filled outline
    pub caps: Vec<(Pt, f32)>, // (center, diameter) round caps / click dot
}

impl PenStroke {
    /// `base` = nominal width in px; `strength` 0..=1 sets the string length (3..25 px).
    pub fn begin(color: u32, base: f32, strength: f32, p: Pt) -> Self {
        let s = strength.clamp(0.0, 1.0);
        let mut st = Self {
            color,
            base,
            radius: 3.0 + 22.0 * s,
            strength: s,
            brush: p,
            wstate: base * 0.8,
            ctrl: vec![p],
            cw: vec![base * 0.8],
            flat: vec![],
            fw: vec![],
            poly: vec![],
            caps: vec![],
        };
        st.rebuild(false);
        st
    }

    pub fn add(&mut self, raw: Pt) {
        let d = raw.dist(self.brush);
        if d <= self.radius {
            return;
        } // inside the slack: jitter is ignored
        let step = d - self.radius;
        self.brush = self.brush.lerp(raw, step / d);
        let t = (step / 12.0).clamp(0.0, 1.0); // pixels moved per event ~ speed
        self.wstate += (self.base * (1.25 - 0.8 * t) - self.wstate) * 0.25;
        if self.brush.dist(*self.ctrl.last().unwrap()) >= 1.0 {
            self.ctrl.push(self.brush);
            self.cw.push(self.wstate);
        }
        self.rebuild(false);
    }

    /// Mouse-up: glide to the release point, smooth, taper.
    pub fn finish(&mut self, target: Pt) {
        let d = self.brush.dist(target);
        if d > 0.5 {
            let steps = (d / 4.0).ceil().clamp(1.0, 64.0) as usize;
            for i in 1..=steps {
                self.ctrl
                    .push(self.brush.lerp(target, i as f32 / steps as f32));
                self.cw.push(self.wstate);
            }
        }
        self.polish();
        self.rebuild(true);
    }

    /// Final re-smoothing pass of the committed stroke. Increase the iteration
    /// count (6 + 20*strength) for an even softer result.
    fn polish(&mut self) {
        if self.ctrl.len() < 3 {
            return;
        }
        let (mut p, mut w) = resample(&self.ctrl, &self.cw, 4.0);
        if p.len() >= 3 {
            taubin(&mut p, 6 + (20.0 * self.strength) as usize);
            smooth_scalar(&mut w, 40);
        }
        self.ctrl = p;
        self.cw = w;
    }

    fn rebuild(&mut self, finished: bool) {
        flatten(&self.ctrl, &self.cw, &mut self.flat, &mut self.fw);
        outline(
            &self.flat,
            &self.fw,
            finished,
            &mut self.poly,
            &mut self.caps,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: f32, y: f32) -> Pt {
        Pt { x, y }
    }

    #[test]
    fn single_and_two_points() {
        let (mut o, mut w) = (vec![], vec![]);
        flatten(&[p(1.0, 1.0)], &[4.0], &mut o, &mut w);
        assert_eq!(o.len(), 1);
        flatten(&[p(0.0, 0.0), p(10.0, 0.0)], &[4.0, 4.0], &mut o, &mut w);
        assert!(o.iter().all(|q| q.y.abs() < 1e-4));
    }
    #[test]
    fn click_makes_a_dot() {
        let s = PenStroke::begin(0xFF000000, 4.0, 0.6, p(5.0, 5.0));
        assert_eq!(s.caps.len(), 1);
        assert!(s.poly.is_empty());
    }
    #[test]
    fn jitter_is_absorbed() {
        let mut s = PenStroke::begin(0xFF000000, 4.0, 0.6, p(0.0, 0.0));
        for i in 1..60 {
            s.add(p(i as f32 * 3.0, if i % 2 == 0 { 2.0 } else { -2.0 }));
        }
        assert!(s.ctrl.iter().all(|q| q.y.abs() <= 2.01));
        assert!(s.poly.len() >= 6);
    }
    #[test]
    fn polish_removes_wobble_and_keeps_ends() {
        let mut ctrl: Vec<Pt> = (0..40)
            .map(|i| p(i as f32 * 4.0, if i % 2 == 0 { 3.0 } else { -3.0 }))
            .collect();
        let (first, last) = (ctrl[0], ctrl[39]);
        taubin(&mut ctrl, 12);
        assert!(ctrl[2..38].iter().all(|q| q.y.abs() < 0.5));
        assert_eq!((ctrl[0], ctrl[39]), (first, last));
    }
    #[test]
    fn resample_gives_even_spacing() {
        let c = [p(0.0, 0.0), p(1.0, 0.0), p(10.0, 0.0), p(40.0, 0.0)];
        let (o, w) = resample(&c, &[4.0; 4], 4.0);
        assert_eq!(o.len(), w.len());
        assert!(o.windows(2).all(|q| (q[0].dist(q[1]) - 4.0).abs() < 0.01));
    }
    #[test]
    fn ends_exactly_at_release_point() {
        let mut s = PenStroke::begin(0xFF000000, 4.0, 0.6, p(140.0, 100.0));
        for i in 1..60 {
            let a = i as f32 / 60.0 * std::f32::consts::TAU * 0.97;
            s.add(p(100.0 + 40.0 * a.cos(), 100.0 + 40.0 * a.sin()));
        }
        let end = p(140.0, 106.0);
        s.finish(end);
        assert!(s.ctrl.last().unwrap().dist(end) < 1e-3); // no snapping to the start
    }
}
