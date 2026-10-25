//! Resampling a window of the 16:9 frame into the vertical frame (Vertical Edit, spec §6.2).
//!
//! [`resample_window`] reconstructs output pixel `(i, j)` at the sequence point `(x + (i + ½)·k_x, y + (j + ½)·k_y)` of
//! the window `[x, y, w, h]`, with `k_x = w / OW` and `k_y = h / OH`. Positions and weights are f64, pixels f32. The
//! filter is separable (a horizontal pass over the rows the window touches, then a vertical pass), with a kernel of
//! its own, [`Kernel`]: Catmull-Rom (Keys cubic, a = −½, support 2·max(1, k)) or bilinear (the triangle, support
//! max(1, k)). The support widens with `k` when the window is larger than the output, so shrinking is area-correct.
//! Weights are normalised to sum 1; taps outside the frame use the edge pixel, so there is no dark rim; each output
//! pixel is a fixed-order sum, so the thread count never changes a byte. After filtering, colour channels are clamped
//! to [0, α] and α to [0, 1]. [`Image::transformed`] is not used: it treats outside as transparent and switches 2× box
//! levels.
//!
//! **Centred at every scale.** For `k ≤ 1` both kernels reproduce linear ramps: the weights of an output pixel are
//! centred on its sample point. The widened kernel (`k > 1`) is centred only at integer `k`. Elsewhere its discrete
//! first moment `Σ w·(s − pos)` is not zero: up to 0.029 source px for Catmull-Rom (near k = 1.41) and 0.086 for
//! bilinear (near k = 1.2 and 1.7), more than the 0.02 of spec §9 C16. So for `k > 1` each output pixel's weights
//! get a first-moment correction (`zero_first_moment`) that keeps their sum at 1 and makes that moment 0. Both kernels
//! then reproduce linear ramps at every `k`, and every kernel is centred on the same point.
//!
//! Each call takes a table of taps per axis: for every output index the destination covers, the source indices
//! (clamped to the edge) with their weights, zero weights left out. A table is a pure function of the kernel, the sizes
//! and the window and destination on its axis, so each thread keeps the last few it built and reuses one while those
//! inputs do not change (a held window reuses both, a pan its vertical one); the bits are the same either way. The
//! horizontal pass filters each source row the vertical taps reach into the covered output columns and stores the
//! result as f32; the vertical pass sums those rows in f64. Both passes compute every pixel on its own, in parallel,
//! into a buffer reserved before it is filled, so a buffer that cannot be allocated gives a transparent image instead of
//! an abort.

use std::cell::RefCell;
use std::rc::Rc;

use rayon::prelude::*;

use crate::Image;

/// The reconstruction kernel of [`resample_window`].
///
/// For `k > 1` either one is widened, then centred on the sample point (see the module documentation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kernel {
    /// Keys cubic with a = −½, support 2·max(1, k) source pixels.
    #[default]
    CatmullRom,
    /// The triangle, support max(1, k) source pixels.
    Bilinear,
}

impl Kernel {
    /// Half the support, in units of the kernel's scale `max(1, k)`.
    fn radius(self) -> f64 {
        match self {
            Kernel::CatmullRom => 2.0,
            Kernel::Bilinear => 1.0,
        }
    }

    /// The weight at the distance `t`, in units of the kernel's scale.
    fn weight(self, t: f64) -> f64 {
        let t = t.abs();
        match self {
            Kernel::CatmullRom if t < 1.0 => (1.5 * t - 2.5) * t * t + 1.0,
            Kernel::CatmullRom if t < 2.0 => ((-0.5 * t + 2.5) * t - 4.0) * t + 2.0,
            Kernel::Bilinear if t < 1.0 => 1.0 - t,
            _ => 0.0,
        }
    }
}

/// Resample the window `[x, y, w, h]` of `src` (sequence pixels: top-left corner and size) into an image of
/// `out = (OW, OH)` pixels with `kernel` (spec §6.2).
///
/// `dest = [dx, dy, dw, dh]` is the rectangle of the output, in output pixels, that the window maps onto (the fit
/// layout); `None` is the whole output. Output pixel `(i, j)` reconstructs the frame at `x + (i + ½ − dx)·w / dw`,
/// `y + (j + ½ − dy)·h / dh`. Pixels `dest` does not reach are transparent, and pixels it covers in part are weighted
/// by the covered fraction of their area.
///
/// Along each axis the kernel's scale is `max(1, k)`, capped at the frame's size on that axis. The cap only applies when
/// one output pixel spans more than the whole frame (a destination under three output pixels long); it bounds the work.
/// For `k > 1` the weights get the first-moment correction of the module documentation.
///
/// Never panics. A window or `dest` with a value that is not finite or a size that is not positive, a window outside
/// the frame grown by the frame's own size on every side, a `dest` so small that a sample position is not finite, a
/// `src` that is empty, malformed or not a valid frame size, a horizontal pass of more pixels than the largest frame,
/// and a buffer that cannot be allocated give a transparent image. Nothing the size of a frame is allocated before the
/// input is checked. An `out` that is not a valid frame size (`filmcraft_project::validate_frame_size`), or whose
/// image cannot be allocated, gives a 1×1 transparent image, the fallback `render_sequence` uses.
pub fn resample_window(src: &Image, window: [f64; 4], out: (usize, usize), dest: Option<[f64; 4]>, kernel: Kernel) -> Image {
    if !valid_size(out) {
        return Image::new(1, 1);
    }
    let dest = dest.unwrap_or([0.0, 0.0, out.0 as f64, out.1 as f64]);
    let filtered = axes(src, window, out, dest, kernel).and_then(|(xs, ys)| {
        let (first_row, mid) = horizontal(src, &xs, &ys)?;
        vertical(&mid, first_row, &xs, &ys, out)
    });
    filtered.or_else(|| blank(out)).unwrap_or_else(|| Image::new(1, 1))
}

/// Check the input as [`resample_window`] describes and take the taps of both axes; `None` refuses it.
fn axes(src: &Image, window: [f64; 4], out: (usize, usize), dest: [f64; 4], kernel: Kernel) -> Option<(Rc<Axis>, Rc<Axis>)> {
    let len = src.w.checked_mul(src.h)?.checked_mul(4)?;
    if len != src.px.len() || !valid_size((src.w, src.h)) {
        return None;
    }
    let [x, y, w, h] = window;
    let [dx, dy, dw, dh] = dest;
    if window.iter().chain(&dest).any(|v| !v.is_finite()) || w <= 0.0 || h <= 0.0 || dw <= 0.0 || dh <= 0.0 {
        return None;
    }
    let (fw, fh) = (src.w as f64, src.h as f64);
    if x < -fw || x + w > 2.0 * fw || y < -fh || y + h > 2.0 * fh {
        return None;
    }
    Some((kept_axis(kernel, src.w, out.0, [x, w], [dx, dw])?, kept_axis(kernel, src.h, out.1, [y, h], [dy, dh])?))
}

/// What an [`Axis`] is built from, bit for bit.
#[derive(Clone, Copy, PartialEq, Eq)]
struct AxisKey {
    kernel: Kernel,
    n_src: usize,
    n_out: usize,
    window: [u64; 2],
    dest: [u64; 2],
}

/// How many axes each thread keeps, the most recently used first: both axes of a `fill` frame and the four of a `fit`
/// frame (its background and its picture), with room to spare.
const KEPT_AXES: usize = 8;

/// The largest axis kept, in taps (16 bytes each). A 1080×1920 output of an 8K frame takes about 20 000.
const MAX_KEPT_TAPS: usize = 1 << 17;

thread_local! {
    /// The axes this thread built last, the most recently used first.
    static AXES: RefCell<Vec<(AxisKey, Rc<Axis>)>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
thread_local! {
    /// How many axes this thread has built (tests only).
    static BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// [`Axis::new`], from this thread's kept axes when one was built from the same inputs (spec §6.2: the weight tables
/// are computed once and reused while the window does not change). An axis is a pure function of its inputs, so the
/// result is the same either way.
fn kept_axis(kernel: Kernel, n_src: usize, n_out: usize, window: [f64; 2], dest: [f64; 2]) -> Option<Rc<Axis>> {
    let key = AxisKey { kernel, n_src, n_out, window: window.map(f64::to_bits), dest: dest.map(f64::to_bits) };
    let kept = AXES.try_with(|axes| {
        let mut axes = axes.try_borrow_mut().ok()?;
        let at = axes.iter().position(|(k, _)| *k == key)?;
        let entry = axes.remove(at);
        let axis = Rc::clone(&entry.1);
        axes.insert(0, entry);
        Some(axis)
    });
    if let Ok(Some(axis)) = kept {
        return Some(axis);
    }
    let axis = Rc::new(Axis::new(kernel, n_src, n_out, window, dest)?);
    #[cfg(test)]
    let _ = BUILT.try_with(|n| n.set(n.get() + 1));
    if axis.taps.len() <= MAX_KEPT_TAPS {
        let _ = AXES.try_with(|axes| {
            if let Ok(mut axes) = axes.try_borrow_mut() {
                axes.insert(0, (key, Rc::clone(&axis)));
                axes.truncate(KEPT_AXES);
            }
        });
    }
    Some(axis)
}

/// The taps of one axis: a [`Cell`] for each output index the destination covers, from `lo` on.
struct Axis {
    lo: usize,
    cells: Vec<Cell>,
    /// The taps of every cell, in order: the source index, clamped to the edge, and the weight.
    taps: Vec<(usize, f64)>,
}

/// An output index the destination covers.
struct Cell {
    /// The fraction of the output pixel the destination covers along the axis.
    cover: f64,
    /// The cell's taps are `taps[first..end]`.
    first: usize,
    end: usize,
}

impl Axis {
    /// The axis that maps `window = [start, size]` of `n_src` source pixels onto `dest = [d0, dsize]` of `n_out` output
    /// pixels. `None` when the destination covers no output pixel or a sample position is not finite.
    fn new(kernel: Kernel, n_src: usize, n_out: usize, window: [f64; 2], dest: [f64; 2]) -> Option<Axis> {
        let ([start, size], [d0, dsize]) = (window, dest);
        let last = n_src.checked_sub(1)?;
        let lo = d0.floor().max(0.0).min(n_out as f64) as usize;
        let hi = (d0 + dsize).ceil().max(0.0).min(n_out as f64) as usize;
        if hi <= lo {
            return None;
        }
        let k = size / dsize;
        let scale = if k > 1.0 { k.min(n_src as f64) } else { 1.0 };
        let reach = kernel.radius() * scale;
        let mut axis = Axis { lo, cells: Vec::with_capacity(hi - lo), taps: Vec::new() };
        // The distance `s − pos` of each tap of the current cell, for the first-moment correction.
        let mut dist = Vec::new();
        for o in lo..hi {
            let o = o as f64;
            let cover = ((o + 1.0).min(d0 + dsize) - o.max(d0)).max(0.0);
            // The sample position in source index space, where source pixel `s` has its centre at `s`.
            let pos = start + (o + 0.5 - d0) * k - 0.5;
            if !pos.is_finite() {
                return None;
            }
            // Further outside the frame than the kernel reaches, every tap is the edge pixel: moving the position to
            // just past that point gives the same pixel and keeps the tap indices small.
            let pos = pos.max(-reach - 1.0).min(last as f64 + reach + 1.0);
            let first = axis.taps.len();
            dist.clear();
            let mut sum = 0.0f64;
            for s in (pos - reach).ceil() as i64..=(pos + reach).floor() as i64 {
                let d = s as f64 - pos;
                let weight = kernel.weight(d / scale);
                // Zero weights are left out, so the identity window takes exactly one tap, of weight 1.
                if weight != 0.0 {
                    axis.taps.push((edge(s, last), weight));
                    dist.push(d);
                    sum += weight;
                }
            }
            if !sum.is_finite() || sum <= 0.0 {
                return None;
            }
            let taps = axis.taps.get_mut(first..).unwrap_or_default();
            for tap in taps.iter_mut() {
                tap.1 /= sum;
            }
            if scale > 1.0 {
                zero_first_moment(taps, &dist);
            }
            axis.cells.push(Cell { cover, first, end: axis.taps.len() });
        }
        Some(axis)
    }

    /// The taps of the `n`-th output index the destination covers; none past the last.
    fn taps_at(&self, n: usize) -> &[(usize, f64)] {
        self.cells.get(n).and_then(|cell| self.taps.get(cell.first..cell.end)).unwrap_or_default()
    }
}

/// Centre the normalised weights of one output pixel on its sample position; `dist` holds each tap's distance
/// `d = s − pos`. With `g = |w|`, `μ = Σ g·d / Σ g` and `c = Σ w·d / Σ g·(d − μ)²`, each weight becomes
/// `w − c·g·(d − μ)`: the changes sum to zero, so the weights still sum to 1, and the first moment `Σ w·d` becomes 0.
/// For the triangle this is the linear correction `w·(1 + a + b·d)`. Scaling the change by `|w|` keeps it where the
/// kernel is, and keeps the equations well posed for Catmull-Rom, whose signed second moment is close to zero. Weights
/// that all sit at one distance have no spread and are left as they are.
fn zero_first_moment(taps: &mut [(usize, f64)], dist: &[f64]) {
    let (mut g0, mut g1, mut m1) = (0.0f64, 0.0f64, 0.0f64);
    for (&(_, w), &d) in taps.iter().zip(dist) {
        g0 += w.abs();
        g1 += w.abs() * d;
        m1 += w * d;
    }
    let mu = g1 / g0;
    let mut spread = 0.0f64;
    for (&(_, w), &d) in taps.iter().zip(dist) {
        spread += w.abs() * (d - mu) * (d - mu);
    }
    let c = m1 / spread;
    if c.is_finite() {
        for (tap, &d) in taps.iter_mut().zip(dist) {
            tap.1 -= c * tap.1.abs() * (d - mu);
        }
    }
}

/// Clamp to edge: the source index of tap `s` on an axis whose last index is `last`.
fn edge(s: i64, last: usize) -> usize {
    usize::try_from(s.max(0)).map_or(last, |s| s.min(last))
}

/// The horizontal pass: each source row the vertical taps reach, from the first of them on, filtered into the output
/// columns the destination covers. Returned with the index of that first row; `None` when it would hold more pixels
/// than the largest frame or cannot be allocated.
fn horizontal(src: &Image, xs: &Axis, ys: &Axis) -> Option<(usize, Vec<[f32; 4]>)> {
    let first_row = ys.taps.iter().map(|t| t.0).min()?;
    let rows = ys.taps.iter().map(|t| t.0).max()?.checked_sub(first_row)?.checked_add(1)?;
    let cols = xs.cells.len();
    let pixels = rows.checked_mul(cols)?;
    if pixels == 0 || pixels as u64 > filmcraft_project::MAX_FRAME_PIXELS {
        return None;
    }
    let stride = src.w * 4;
    let mid = pixels_of(rows, cols, |r, c| {
        let at = (first_row + r) * stride;
        let line = src.px.get(at..at + stride).unwrap_or_default();
        let mut acc = [0.0f64; 4];
        for &(i, weight) in xs.taps_at(c) {
            if let Some(px) = line.get(i * 4..i * 4 + 4) {
                for (a, v) in acc.iter_mut().zip(px) {
                    *a += weight * f64::from(*v);
                }
            }
        }
        acc.map(|a| a as f32)
    })?;
    Some((first_row, mid))
}

/// The vertical pass: the output image, each pixel the destination covers summed from the rows of `mid` and then
/// finished (clamped, and weighted by its coverage), every other pixel transparent. `None` when the image cannot be
/// allocated.
fn vertical(mid: &[[f32; 4]], first_row: usize, xs: &Axis, ys: &Axis, out: (usize, usize)) -> Option<Image> {
    let cols = xs.cells.len();
    let px = pixels_of(out.1, out.0, |j, i| {
        let (Some(c), Some(r)) = (i.checked_sub(xs.lo), j.checked_sub(ys.lo)) else { return [0.0; 4] };
        let (Some(cx), Some(cy)) = (xs.cells.get(c), ys.cells.get(r)) else { return [0.0; 4] };
        let mut acc = [0.0f64; 4];
        for &(t, weight) in ys.taps_at(r) {
            if let Some(px) = mid.get(t.saturating_sub(first_row) * cols + c) {
                for (a, v) in acc.iter_mut().zip(px) {
                    *a += weight * f64::from(*v);
                }
            }
        }
        finish(acc, cx.cover * cy.cover)
    })?;
    Some(Image { w: out.0, h: out.1, px: px.into_flattened() })
}

/// A filtered pixel, clamped (α to [0, 1], colour to [0, α], NaN to 0) and weighted by `cover`.
fn finish([r, g, b, a]: [f64; 4], cover: f64) -> [f32; 4] {
    let a = if a > 0.0 { a.min(1.0) } else { 0.0 };
    let colour = |c: f64| if c > 0.0 { c.min(a) } else { 0.0 };
    [(colour(r) * cover) as f32, (colour(g) * cover) as f32, (colour(b) * cover) as f32, (a * cover) as f32]
}

/// The pixels of a `rows × cols` image in row order, each computed on its own by `pixel(row, column)`, in parallel,
/// into a buffer reserved before it is filled. `None` when the buffer cannot be allocated. The result never depends
/// on how rayon splits the work.
fn pixels_of(rows: usize, cols: usize, pixel: impl Fn(usize, usize) -> [f32; 4] + Sync + Send) -> Option<Vec<[f32; 4]>> {
    let n = rows.checked_mul(cols)?;
    let mut px: Vec<[f32; 4]> = Vec::new();
    px.try_reserve_exact(n).ok()?;
    // Within one rayon job the indices come in order, so the row and the column step along; a job's first index is
    // divided out (`cols` is not zero there: the range is empty when it is).
    let step = |next: &mut (usize, usize, usize), p: usize| {
        let (row, col) = if next.0 == p { (next.1, next.2) } else { (p / cols, p % cols) };
        *next = if col + 1 == cols { (p + 1, row + 1, 0) } else { (p + 1, row, col + 1) };
        pixel(row, col)
    };
    (0..n).into_par_iter().map_init(|| (usize::MAX, 0, 0), step).collect_into_vec(&mut px);
    Some(px)
}

/// A transparent image of `out`; `None` when it cannot be allocated.
fn blank(out: (usize, usize)) -> Option<Image> {
    let px = pixels_of(out.1, out.0, |_, _| [0.0; 4])?;
    Some(Image { w: out.0, h: out.1, px: px.into_flattened() })
}

/// Whether `size` is a valid frame size (`filmcraft_project::validate_frame_size`).
fn valid_size((w, h): (usize, usize)) -> bool {
    match (u32::try_from(w), u32::try_from(h)) {
        (Ok(w), Ok(h)) => filmcraft_project::validate_frame_size(w, h).is_ok(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KERNELS: [Kernel; 2] = [Kernel::CatmullRom, Kernel::Bilinear];

    /// T-R1's tolerance, for every `k`. Both kernels reproduce linear ramps (for `k > 1` through the first-moment
    /// correction), so only f32 rounding is left: about 2e-4 px on a UHD frame. Spec C16 asks 1e-3 px for k ≤ 1 and
    /// 0.02 for k > 1, where the uncorrected widened kernel is off by up to 0.029 px (Catmull-Rom) and 0.086 (bilinear).
    const TOL: f64 = 1e-3;

    /// SplitMix64, so every run tests the same cases.
    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// Uniform in [lo, hi).
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64)
        }

        /// Uniform in 0..n, for n ≥ 1.
        fn below(&mut self, n: usize) -> usize {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// T-R1's frame: R = (i + ½)/W and G = (j + ½)/H at pixel (i, j), B = 0, α = 1.
    fn ramp(w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for (n, p) in img.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (i, j) = (n % w, n / w);
            *p = [((i as f64 + 0.5) / w as f64) as f32, ((j as f64 + 0.5) / h as f64) as f32, 0.0, 1.0];
        }
        img
    }

    /// A random premultiplied image without a zero or negative channel (T-R3 compares bits): α in [0.05, 1), colour in
    /// (0, α].
    fn noise(rng: &mut Rng, w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for p in img.px.as_chunks_mut::<4>().0 {
            let a = rng.range(0.05, 1.0);
            *p = [(a * rng.range(0.01, 1.0)) as f32, (a * rng.range(0.01, 1.0)) as f32, (a * rng.range(0.01, 1.0)) as f32, a as f32];
        }
        img
    }

    /// A window of the output's aspect at scale `k` (sequence px per output px), at a random sub-pixel position inside
    /// the frame, derived as spec §2.3 does: `w = h·OW/OH`, `x = cx − w/2`, `y = cy − h/2`.
    fn window_in(rng: &mut Rng, frame: (usize, usize), out: (usize, usize), k: f64) -> [f64; 4] {
        let h = k * out.1 as f64;
        let w = h * out.0 as f64 / out.1 as f64;
        let cx = rng.range(w * 0.5, frame.0 as f64 - w * 0.5);
        let cy = rng.range(h * 0.5, frame.1 as f64 - h * 0.5);
        [cx - w * 0.5, cy - h * 0.5, w, h]
    }

    /// T-R1's oracle: each interior output pixel of the ramp decodes, as (R·W, G·H), to the sequence point it samples,
    /// `(x + (i + ½ − dx)·k_x, y + (j + ½ − dy)·k_y)`, within [`TOL`] px. Interior: `dest` covers the whole pixel, and
    /// every tap of the widest kernel, plus one pixel, lies inside the frame. Returns how many pixels were checked.
    fn check_ramp(src: &Image, window: [f64; 4], out: (usize, usize), dest: Option<[f64; 4]>, kernel: Kernel) -> usize {
        let img = resample_window(src, window, out, dest, kernel);
        assert_eq!((img.w, img.h), out, "{kernel:?} {window:?}: output size");
        let [x, y, w, h] = window;
        let [dx, dy, dw, dh] = dest.unwrap_or([0.0, 0.0, out.0 as f64, out.1 as f64]);
        let (kx, ky) = (w / dw, h / dh);
        let (mx, my) = (2.0 * kx.max(1.0) + 1.0, 2.0 * ky.max(1.0) + 1.0);
        let (fw, fh) = (src.w as f64, src.h as f64);
        let mut checked = 0;
        for j in 0..out.1 {
            let (top, sy) = (j as f64, y + (j as f64 + 0.5 - dy) * ky);
            if top < dy || top + 1.0 > dy + dh || sy - 0.5 - my < 0.0 || sy - 0.5 + my > fh - 1.0 {
                continue;
            }
            for i in 0..out.0 {
                let (left, sx) = (i as f64, x + (i as f64 + 0.5 - dx) * kx);
                if left < dx || left + 1.0 > dx + dw || sx - 0.5 - mx < 0.0 || sx - 0.5 + mx > fw - 1.0 {
                    continue;
                }
                let p = img.get(i, j);
                let (ex, ey) = (f64::from(p[0]) * fw - sx, f64::from(p[1]) * fh - sy);
                assert!(ex.abs() <= TOL && ey.abs() <= TOL, "{kernel:?} {window:?} -> {out:?} {dest:?}: pixel ({i}, {j}) is {ex:+.6} / {ey:+.6} px off");
                checked += 1;
            }
        }
        assert!(checked > 0, "{kernel:?} {window:?} -> {out:?} {dest:?}: no interior pixel");
        checked
    }

    /// Keys' cubic convolution kernel with a = −½, written from its definition.
    fn keys(t: f64) -> f64 {
        let a = -0.5;
        let t = t.abs();
        if t < 1.0 {
            (a + 2.0) * t.powi(3) - (a + 3.0) * t.powi(2) + 1.0
        } else if t < 2.0 {
            a * t.powi(3) - 5.0 * a * t.powi(2) + 8.0 * a * t - 4.0 * a
        } else {
            0.0
        }
    }

    fn triangle(t: f64) -> f64 {
        (1.0 - t.abs()).max(0.0)
    }

    fn reference_weight(kernel: Kernel, t: f64) -> f64 {
        match kernel {
            Kernel::CatmullRom => keys(t),
            Kernel::Bilinear => triangle(t),
        }
    }

    /// One axis of T-R4's reference: the weights of the taps `taps` around `u` (index space) at the kernel's scale
    /// `scale`, normalised; for `scale > 1` with the first-moment correction, written as `w + |w|·(α + β·d)` with α
    /// and β solving the two moment equations (Σ w = 1, Σ w·d = 0) by Cramer's rule.
    fn reference_axis(kernel: Kernel, u: f64, scale: f64, taps: &[i64]) -> Vec<f64> {
        let d: Vec<f64> = taps.iter().map(|&s| s as f64 - u).collect();
        let raw: Vec<f64> = d.iter().map(|&d| reference_weight(kernel, d / scale)).collect();
        let total: f64 = raw.iter().sum();
        let w: Vec<f64> = raw.iter().map(|v| v / total).collect();
        if scale <= 1.0 {
            return w;
        }
        let g = |p: i32| -> f64 { w.iter().zip(&d).map(|(v, d)| v.abs() * d.powi(p)).sum() };
        let (g0, g1, g2) = (g(0), g(1), g(2));
        let r0 = 1.0 - w.iter().sum::<f64>();
        let m1: f64 = w.iter().zip(&d).map(|(v, d)| v * d).sum();
        let det = g0 * g2 - g1 * g1;
        let (alpha, beta) = ((r0 * g2 + g1 * m1) / det, (-g0 * m1 - g1 * r0) / det);
        w.iter().zip(&d).map(|(v, d)| v + v.abs() * (alpha + beta * d)).collect()
    }

    /// The length of the overlap of `[a0, a1)` and `[b0, b1)`.
    fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
        (a1.min(b1) - a0.max(b0)).max(0.0)
    }

    /// T-R4's reference: each output pixel on its own, the 2D kernel (the product of [`reference_axis`] on both axes)
    /// over a square of taps wider than its support, every tap fetched with clamp-to-edge, all in f64; then the clamp
    /// to [0, α] and the coverage of `dest`.
    fn naive(src: &Image, window: [f64; 4], out: (usize, usize), dest: Option<[f64; 4]>, kernel: Kernel) -> Vec<f64> {
        let [x, y, w, h] = window;
        let [dx, dy, dw, dh] = dest.unwrap_or([0.0, 0.0, out.0 as f64, out.1 as f64]);
        let (kx, ky) = (w / dw, h / dh);
        let (sx, sy) = (kx.max(1.0), ky.max(1.0));
        let reach = if kernel == Kernel::CatmullRom { 2.0 } else { 1.0 };
        let (rx, ry) = ((reach * sx).ceil() as i64 + 1, (reach * sy).ceil() as i64 + 1);
        let fetch = |s: i64, t: i64| src.get(s.clamp(0, src.w as i64 - 1) as usize, t.clamp(0, src.h as i64 - 1) as usize);
        let mut res = vec![0.0; out.0 * out.1 * 4];
        for j in 0..out.1 {
            let v = y + (j as f64 + 0.5 - dy) * ky - 0.5;
            let ts: Vec<i64> = (v.floor() as i64 - ry..=v.floor() as i64 + ry).collect();
            let wy = reference_axis(kernel, v, sy, &ts);
            for i in 0..out.0 {
                let cover = overlap(i as f64, i as f64 + 1.0, dx, dx + dw) * overlap(j as f64, j as f64 + 1.0, dy, dy + dh);
                if cover <= 0.0 {
                    continue;
                }
                let u = x + (i as f64 + 0.5 - dx) * kx - 0.5;
                let ss: Vec<i64> = (u.floor() as i64 - rx..=u.floor() as i64 + rx).collect();
                let wx = reference_axis(kernel, u, sx, &ss);
                let mut acc = [0.0f64; 4];
                for (&t, wt) in ts.iter().zip(&wy) {
                    for (&s, ws) in ss.iter().zip(&wx) {
                        let p = fetch(s, t);
                        for ch in 0..4 {
                            acc[ch] += wt * ws * f64::from(p[ch]);
                        }
                    }
                }
                let a = acc[3].clamp(0.0, 1.0);
                let o = &mut res[(j * out.0 + i) * 4..][..4];
                for ch in 0..3 {
                    o[ch] = acc[ch].clamp(0.0, a) * cover;
                }
                o[3] = a * cover;
            }
        }
        res
    }

    /// T-R5's zone plate, 0.5 + 0.5·cos(π·r²/R0) around the frame centre: its local frequency r/R0 (cycles per sequence
    /// px) reaches the frame's Nyquist limit at r = R0/2, and the plate is a flat 0.5 beyond, so the frame holds no alias.
    const R0: f64 = 2500.0;
    const ZONE_CENTRE: (f64, f64) = (1920.0, 1080.0);

    fn zone(x: f64, y: f64) -> f64 {
        let r2 = (x - ZONE_CENTRE.0).powi(2) + (y - ZONE_CENTRE.1).powi(2);
        if r2 <= (R0 / 2.0).powi(2) { 0.5 + 0.5 * (std::f64::consts::PI * r2 / R0).cos() } else { 0.5 }
    }

    /// T-R1, k ≤ 1: random sub-pixel windows and zooms on a 1080p and a UHD frame, and the full-height 9:16 window of a
    /// 1080p frame into 1080×1920 (k = 0.5625). Every interior pixel decodes to its sequence point within 1e-3 px.
    #[test]
    fn t_r1_ramp_oracle_magnifying() {
        let mut rng = Rng(0x7231);
        let mut checked = 0;
        for frame in [(1920, 1080), (3840, 2160)] {
            let src = ramp(frame.0, frame.1);
            for _ in 0..12 {
                let k = rng.range(0.25, 1.0);
                let window = window_in(&mut rng, frame, (90, 160), k);
                for kernel in KERNELS {
                    checked += check_ramp(&src, window, (90, 160), None, kernel);
                }
            }
        }
        let src = ramp(1920, 1080);
        let x = rng.range(0.0, 1920.0 - 607.5);
        for kernel in KERNELS {
            checked += check_ramp(&src, [x, 0.0, 607.5, 1080.0], (1080, 1920), None, kernel);
        }
        assert!(checked > 1_000_000, "only {checked} pixels were checked");
    }

    /// T-R1, k > 1: a UHD frame through the full-height window of each output preset (spec §5.14) at full size
    /// (1080×1920: k = 1.125; 720×1280: 1.6875; 4:5 1080×1350: 1.6; 1:1 1080×1080: 2), the fit layout of a whole 1080p
    /// and UHD frame into 1080×1920 (k = 1.78 and 3.56), and random sub-pixel windows with k from 1 to 8 (scaled renders
    /// and the fit layout go past the presets). Every interior pixel decodes to its sequence point within 1e-3 px.
    #[test]
    fn t_r1_ramp_oracle_shrinking() {
        let (uhd, hd) = (ramp(3840, 2160), ramp(1920, 1080));
        let mut rng = Rng(0x7232);
        let mut checked = 0;
        for out in [(1080, 1920), (720, 1280), (1080, 1350), (1080, 1080)] {
            let w = 2160.0 * out.0 as f64 / out.1 as f64;
            let x = rng.range(0.0, 3840.0 - w);
            for kernel in KERNELS {
                checked += check_ramp(&uhd, [x, 0.0, w, 2160.0], out, None, kernel);
            }
        }
        for src in [&hd, &uhd] {
            let (fw, fh) = (src.w as f64, src.h as f64);
            let dh = 1080.0 * fh / fw;
            for kernel in KERNELS {
                checked += check_ramp(src, [0.0, 0.0, fw, fh], (1080, 1920), Some([0.0, (1920.0 - dh) / 2.0, 1080.0, dh]), kernel);
            }
        }
        for n in 0..24 {
            let k = if n < 12 { rng.range(1.000_001, 2.0) } else { rng.range(2.0, 8.0) };
            let window = window_in(&mut rng, (3840, 2160), (90, 160), k);
            for kernel in KERNELS {
                checked += check_ramp(&uhd, window, (90, 160), None, kernel);
            }
        }
        assert!(checked > 10_000_000, "only {checked} pixels were checked");
    }

    /// The first-moment correction in the tables: on a long axis (no tap reaches an edge), every output index's weights
    /// sum to 1 and are centred on its sample position within 1e-12 source px, for both kernels and k from 0.25 to 8;
    /// at k ≤ 1 and at integer k, where the plain kernel is already centred, they are the plain kernel's.
    #[test]
    fn taps_are_centred_at_every_scale() {
        let mut rng = Rng(0x7237);
        for _ in 0..400 {
            let k = if rng.below(4) == 0 { 1.0 + rng.below(4) as f64 } else { rng.range(0.25, 8.0) };
            let start = rng.range(100.0, 101.0);
            for kernel in KERNELS {
                let Some(axis) = Axis::new(kernel, 1000, 16, [start, 16.0 * k], [0.0, 16.0]) else { panic!("{kernel:?} k = {k}: no axis") };
                for o in 0..axis.cells.len() {
                    let pos = start + (o as f64 + 0.5) * k - 0.5;
                    let taps = axis.taps_at(o);
                    let (mut sum, mut moment) = (0.0f64, 0.0f64);
                    for &(s, w) in taps {
                        sum += w;
                        moment += w * (s as f64 - pos);
                    }
                    assert!((sum - 1.0).abs() <= 1e-12 && moment.abs() <= 1e-12, "{kernel:?} k = {k}: cell {o} sums to {sum}, first moment {moment}");
                    if k <= 1.0 || k.fract() == 0.0 {
                        let plain: Vec<f64> = taps.iter().map(|&(s, _)| kernel.weight((s as f64 - pos) / k.max(1.0))).collect();
                        let total: f64 = plain.iter().sum();
                        for (&(s, w), p) in taps.iter().zip(&plain) {
                            assert!((w - p / total).abs() <= 1e-12, "{kernel:?} k = {k}: tap {s} of cell {o} is {w}, the plain kernel's {}", p / total);
                        }
                    }
                }
            }
        }
    }

    /// Spec §6.2: a thread reuses the axes it built while the window does not change, and a kept axis gives the bits of
    /// a new one. A window, a pan of it along x, then the window again: the first call builds both axes, the pan only
    /// the horizontal one, the third call none, and it gives the first call's bits.
    #[test]
    fn tables_are_reused_while_the_window_holds() {
        let built = || BUILT.with(std::cell::Cell::get);
        let same = |p: &Image, q: &Image| p.px.len() == q.px.len() && p.px.iter().zip(&q.px).all(|(x, y)| x.to_bits() == y.to_bits());
        let mut rng = Rng(0x7238);
        let src = noise(&mut rng, 96, 54);
        let (window, panned) = ([20.25, 0.0, 30.375, 54.0], [41.5, 0.0, 30.375, 54.0]);
        for kernel in KERNELS {
            let start = built();
            let first = resample_window(&src, window, (45, 80), None, kernel);
            assert_eq!(built() - start, 2, "{kernel:?}: the first frame builds both axes");
            let pan = resample_window(&src, panned, (45, 80), None, kernel);
            assert_eq!(built() - start, 3, "{kernel:?}: a pan builds only the horizontal axis");
            let again = resample_window(&src, window, (45, 80), None, kernel);
            assert_eq!(built() - start, 3, "{kernel:?}: the same window again builds no axis");
            assert!(same(&first, &again), "{kernel:?}: a kept axis changed the bits");
            assert!(!same(&first, &pan), "{kernel:?}: the pan did not move the window");
        }
    }

    /// T-R2: a constant-colour frame through windows touching each edge, the corners and the full height gives exactly
    /// that colour (1e-6) and α = 1 in every output pixel, first and last rows and columns included.
    #[test]
    fn t_r2_no_dark_rim() {
        let c = [0.2f32, 0.4, 0.6, 1.0];
        let hd = Image::filled(1920, 1080, c);
        let small = Image::filled(384, 216, c);
        let cases = [
            // the full height into 1080×1920 (k = 0.5625): output row 0 samples y = 0.28125
            (&hd, [656.25, 0.0, 607.5, 1080.0], (1080, 1920)),
            // the full height at the left and the right edge, shrunk into 90×160 (k = 6.75)
            (&hd, [0.0, 0.0, 607.5, 1080.0], (90, 160)),
            (&hd, [1312.5, 0.0, 607.5, 1080.0], (90, 160)),
            // one edge each (k = 0.625)
            (&hd, [0.0, 431.7, 56.25, 100.0], (90, 160)),
            (&hd, [1863.75, 431.7, 56.25, 100.0], (90, 160)),
            (&hd, [911.3, 0.0, 56.25, 100.0], (90, 160)),
            (&hd, [911.3, 980.0, 56.25, 100.0], (90, 160)),
            // the top-left and the bottom-right corner (k = 0.375)
            (&hd, [0.0, 0.0, 33.75, 60.0], (90, 160)),
            (&hd, [1886.25, 1020.0, 33.75, 60.0], (90, 160)),
            // k = 1.125 as in a UHD sequence: the full height at both side edges
            (&small, [0.0, 0.0, 121.5, 216.0], (108, 192)),
            (&small, [262.5, 0.0, 121.5, 216.0], (108, 192)),
            // k = 1.6875 as in the 720×1280 preset of a UHD sequence, at both side edges
            (&small, [0.0, 0.0, 121.5, 216.0], (72, 128)),
            (&small, [262.5, 0.0, 121.5, 216.0], (72, 128)),
        ];
        for kernel in KERNELS {
            for (src, window, out) in cases {
                let img = resample_window(src, window, out, None, kernel);
                assert_eq!((img.w, img.h), out, "{kernel:?} {window:?}: output size");
                for (n, p) in img.px.as_chunks::<4>().0.iter().enumerate() {
                    let off = (0..4).map(|ch| (p[ch] - c[ch]).abs()).fold(0.0f32, f32::max);
                    assert!(off <= 1e-6, "{kernel:?} {window:?} -> {out:?}: pixel ({}, {}) is {p:?} instead of {c:?}", n % out.0, n / out.0);
                }
            }
        }
    }

    /// T-R3: a full-frame window into an output of the frame's size returns the input bit for bit, with either kernel,
    /// and with `dest` left out or naming the whole output.
    #[test]
    fn t_r3_identity_is_bit_for_bit() {
        let mut rng = Rng(0x7233);
        for (w, h) in [(37, 23), (320, 180), (1, 1), (2, 3)] {
            let src = noise(&mut rng, w, h);
            let full = [0.0, 0.0, w as f64, h as f64];
            for kernel in KERNELS {
                for dest in [None, Some(full)] {
                    let img = resample_window(&src, full, (w, h), dest, kernel);
                    let same = (img.w, img.h) == (w, h) && img.px.iter().zip(&src.px).all(|(a, b)| a.to_bits() == b.to_bits());
                    assert!(same, "{kernel:?} {w}x{h} dest {dest:?}: the identity window changed the image");
                }
            }
        }
    }

    /// T-R4: the separable production code against the naive per-pixel f64 reference, both kernels, on random images
    /// and windows: magnification and shrinking (with the first-moment correction), windows reaching past the frame
    /// edges (clamp-to-edge), with and without a destination rectangle. Every channel within 2e-6.
    #[test]
    fn t_r4_separable_matches_naive_reference() {
        let mut rng = Rng(0x7234);
        for _ in 0..48 {
            let (w, h) = (12 + rng.below(37), 12 + rng.below(25));
            let src = noise(&mut rng, w, h);
            let out = (1 + rng.below(24), 1 + rng.below(24));
            let (ow, oh) = (out.0 as f64, out.1 as f64);
            let k = rng.range(0.2, 3.5).min(2.0 * w as f64 / ow).min(2.0 * h as f64 / oh);
            let (ww, wh) = (k * ow, k * oh);
            let window = [rng.range(-0.5 * w as f64, 1.5 * w as f64 - ww), rng.range(-0.5 * h as f64, 1.5 * h as f64 - wh), ww, wh];
            let dest = if rng.below(3) == 0 {
                let (dw, dh) = (rng.range(0.3, 1.2) * ow, rng.range(0.3, 1.2) * oh);
                Some([rng.range(-0.25 * ow, ow - 0.5 * dw), rng.range(-0.25 * oh, oh - 0.5 * dh), dw, dh])
            } else {
                None
            };
            for kernel in KERNELS {
                let got = resample_window(&src, window, out, dest, kernel);
                assert_eq!((got.w, got.h), out, "{kernel:?} {window:?}: output size");
                let want = naive(&src, window, out, dest, kernel);
                for (n, (g, r)) in got.px.iter().zip(&want).enumerate() {
                    let d = (f64::from(*g) - r).abs();
                    assert!(d <= 2e-6, "{kernel:?} {w}x{h} {window:?} -> {out:?} dest {dest:?}: pixel {} channel {} is off by {d:e}", n / 4, n % 4);
                }
            }
        }
    }

    /// T-R5: a 4K zone plate through the full-height 9:16 window (1215×2160) shrunk into 540×960 (k = 2.25) and
    /// 270×480 (k = 4.5). Where the plate has at most 0.1 cycles per output px the output follows it (0.03). Where it
    /// has 1 or 2 cycles per output px, point sampling folds the plate to near-zero frequencies (moiré rings of the full
    /// 0.5 amplitude); the widened kernels must leave at most 0.15 there (at most 0.105 in the model).
    #[test]
    fn t_r5_zone_plate_shrinks_alias_free() {
        let mut src = Image::new(3840, 2160);
        for (n, p) in src.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v = zone((n % 3840) as f64 + 0.5, (n / 3840) as f64 + 0.5) as f32;
            *p = [v, v, v, 1.0];
        }
        let window = [1312.5, 0.0, 1215.0, 2160.0];
        for out in [(540, 960), (270, 480)] {
            let (kx, ky) = (window[2] / out.0 as f64, window[3] / out.1 as f64);
            for kernel in KERNELS {
                let img = resample_window(&src, window, out, None, kernel);
                assert_eq!((img.w, img.h), out, "{kernel:?}: output size");
                let (mut passband, mut pass_err, mut band, mut alias, mut point) = (0, 0.0f64, 0, 0.0f64, 0.0f64);
                for j in 0..out.1 {
                    let y = window[1] + (j as f64 + 0.5) * ky;
                    for i in 0..out.0 {
                        let x = window[0] + (i as f64 + 0.5) * kx;
                        let r = (x - ZONE_CENTRE.0).hypot(y - ZONE_CENTRE.1);
                        if r > R0 / 2.0 - 2.0 * ky - 2.0 {
                            continue;
                        }
                        // the plate's frequency at this pixel, in cycles per output px
                        let f = r / R0 * ky;
                        let v = f64::from(img.get(i, j)[0]);
                        if f <= 0.1 {
                            passband += 1;
                            pass_err = pass_err.max((v - zone(x, y)).abs());
                        }
                        if (0.85..=1.15).contains(&f) || (1.85..=2.15).contains(&f) {
                            band += 1;
                            alias = alias.max((v - 0.5).abs());
                            point = point.max((f64::from(src.get(x as usize, y as usize)[0]) - 0.5).abs());
                        }
                    }
                }
                assert!(passband >= 100 && band >= 1000, "{kernel:?} {out:?}: {passband} passband and {band} alias-band pixels");
                assert!(point >= 0.4, "{kernel:?} {out:?}: point sampling leaves only {point} in the alias band");
                assert!(pass_err <= 0.03, "{kernel:?} {out:?}: the passband is off by {pass_err}");
                assert!(alias <= 0.15, "{kernel:?} {out:?}: {alias} of aliasing left (point sampling: {point})");
            }
        }
    }

    /// The destination rectangle (the fit layout, spec §6.1 step 5): the window lands on `dest`, pixels outside it are
    /// transparent, and pixels it covers in part are weighted by the covered fraction of their area.
    #[test]
    fn dest_rect_places_the_window_and_weights_partial_pixels() {
        // the whole 1080p frame into [0, y0, OW, OW·H/W] of a 54×96 output: rows 32 and 63 are covered 0.1875
        let c = [0.2f32, 0.4, 0.6, 1.0];
        let flat = Image::filled(1920, 1080, c);
        let dh = 54.0 * 1080.0 / 1920.0;
        let fit = [0.0, (96.0 - dh) / 2.0, 54.0, dh];
        // a rectangle that cuts pixels on all four sides, over the ramp at k = 0.75
        let ramp_src = ramp(640, 360);
        let window = [100.25, 50.5, 30.0, 22.5];
        let cut = [3.25, 5.5, 40.0, 30.0];
        for kernel in KERNELS {
            let img = resample_window(&flat, [0.0, 0.0, 1920.0, 1080.0], (54, 96), Some(fit), kernel);
            assert_eq!((img.w, img.h), (54, 96), "{kernel:?} fit: output size");
            for j in 0..96 {
                let cover = overlap(j as f64, j as f64 + 1.0, fit[1], fit[1] + fit[3]);
                for i in 0..54 {
                    let p = img.get(i, j);
                    for ch in 0..4 {
                        let want = f64::from(c[ch]) * cover;
                        assert!((f64::from(p[ch]) - want).abs() <= 1e-6, "{kernel:?} fit: pixel ({i}, {j}) channel {ch} is {} instead of {want}", p[ch]);
                    }
                }
            }
            let img = resample_window(&ramp_src, window, (48, 40), Some(cut), kernel);
            assert_eq!((img.w, img.h), (48, 40), "{kernel:?} cut: output size");
            let (kx, ky) = (window[2] / cut[2], window[3] / cut[3]);
            for j in 0..40 {
                for i in 0..48 {
                    let cover = overlap(i as f64, i as f64 + 1.0, cut[0], cut[0] + cut[2]) * overlap(j as f64, j as f64 + 1.0, cut[1], cut[1] + cut[3]);
                    let p = img.get(i, j);
                    assert!((f64::from(p[3]) - cover).abs() <= 1e-6, "{kernel:?} cut: pixel ({i}, {j}) has alpha {} for a coverage of {cover}", p[3]);
                    if cover > 0.0 {
                        let x = window[0] + (i as f64 + 0.5 - cut[0]) * kx;
                        let y = window[1] + (j as f64 + 0.5 - cut[1]) * ky;
                        let (dx, dy) = (f64::from(p[0] / p[3]) * 640.0 - x, f64::from(p[1] / p[3]) * 360.0 - y);
                        assert!(dx.abs() <= 1e-3 && dy.abs() <= 1e-3, "{kernel:?} cut: pixel ({i}, {j}) decodes {dx:+.6} / {dy:+.6} px off");
                    }
                }
            }
        }
    }

    /// Spec §6.2, "Deterministic": every output pixel is a fixed-order sum, so 1, 2, 3 or 8 threads give the same bits.
    #[test]
    fn same_bits_with_any_thread_count() {
        let mut rng = Rng(0x7236);
        let src = noise(&mut rng, 400, 225);
        let cases = [([37.3, 11.9, 61.875, 110.0], (54, 96)), ([150.6, 0.0, 126.5625, 225.0], (54, 96)), ([7.7, 3.1, 33.75, 60.0], (90, 160))];
        for (window, out) in cases {
            for kernel in KERNELS {
                let run = |threads| {
                    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
                    pool.install(|| resample_window(&src, window, out, None, kernel))
                };
                let one = run(1);
                for threads in [2, 3, 8] {
                    let other = run(threads);
                    let same = (one.w, one.h) == (other.w, other.h) && one.px.iter().zip(&other.px).all(|(a, b)| a.to_bits() == b.to_bits());
                    assert!(same, "{kernel:?} {window:?}: {threads} threads give other bits than 1");
                }
            }
        }
    }

    /// Hostile input never panics (FilmCraft `AGENTS.md` §0): non-finite or non-positive windows and destinations,
    /// windows far outside the frame, and empty or malformed frames give a finite image of the requested size with every
    /// channel in [0, 1]; an impossible output size gives the 1×1 transparent fallback.
    #[test]
    fn hostile_input_never_panics() {
        let frame = Image::filled(16, 9, [0.25, 0.5, 0.75, 1.0]);
        let empty = Image::new(0, 0);
        let malformed = Image { w: 16, h: 9, px: vec![0.5; 7] };
        let windows = [
            [f64::NAN, 0.0, 8.0, 4.5],
            [0.0, f64::INFINITY, 8.0, 4.5],
            [0.0, 0.0, -8.0, 4.5],
            [0.0, 0.0, 8.0, 0.0],
            [0.0, 0.0, 1e300, 1e300],
            [-1e300, 1e300, 8.0, 4.5],
            [1e9, -1e9, 1e-300, 1e-300],
            [-16.0, -9.0, 48.0, 27.0],
            [3.5, 2.25, 9.0, 5.0],
        ];
        let dests = [
            None,
            Some([f64::NAN, 0.0, 4.0, 4.0]),
            Some([0.0, 0.0, 0.0, 4.0]),
            Some([0.0, 0.0, 1e-12, 1e-12]),
            Some([-1e300, -1e300, 1e300, 1e300]),
            Some([-1e300, 2.5, 2e300, 4.25]),
            Some([1.5, 2.5, 3.0, 4.0]),
        ];
        for kernel in KERNELS {
            for src in [&frame, &empty, &malformed] {
                for window in windows {
                    for dest in dests {
                        for out in [(9, 16), (1, 1)] {
                            let img = resample_window(src, window, out, dest, kernel);
                            assert_eq!((img.w, img.h, img.px.len()), (out.0, out.1, out.0 * out.1 * 4), "{kernel:?} {window:?} {dest:?}");
                            assert!(img.px.iter().all(|v| (0.0f32..=1.0).contains(v)), "{kernel:?} {window:?} {dest:?}: a channel is outside [0, 1]");
                        }
                    }
                }
            }
            for out in [(0, 0), (0, 16), (9, 0), (usize::MAX, 2), (40_000, 1), (20_000, 20_000)] {
                let img = resample_window(&frame, [0.0, 0.0, 16.0, 9.0], out, None, kernel);
                assert_eq!((img.w, img.h), (1, 1), "{kernel:?} {out:?}: the fallback size");
                assert!(img.px.iter().all(|v| *v == 0.0), "{kernel:?} {out:?}: the fallback is transparent");
            }
        }
    }

    /// Degenerate scales stay bounded: a destination so small that a sample position overflows gives a transparent
    /// image, and three frames onto half an output pixel (one output pixel spans more than the frame, so the kernel's
    /// scale is capped at the frame's size) give the frame's colour weighted by the coverage.
    #[test]
    fn degenerate_scales_stay_bounded() {
        let c = [0.25f32, 0.5, 0.75, 1.0];
        let frame = Image::filled(16, 9, c);
        for kernel in KERNELS {
            let img = resample_window(&frame, [3.5, 2.25, 9.0, 5.0], (9, 16), Some([0.0, 0.0, 1e-320, 1e-320]), kernel);
            assert!((img.w, img.h) == (9, 16) && img.px.iter().all(|v| *v == 0.0), "{kernel:?}: an overflowing position is not transparent");
            let img = resample_window(&frame, [-16.0, -9.0, 48.0, 27.0], (9, 16), Some([0.0, 0.0, 0.5, 0.5]), kernel);
            assert_eq!((img.w, img.h), (9, 16), "{kernel:?}: output size");
            for (n, p) in img.px.as_chunks::<4>().0.iter().enumerate() {
                let want: f64 = if n == 0 { 0.25 } else { 0.0 };
                for ch in 0..4 {
                    assert!((f64::from(p[ch]) - f64::from(c[ch]) * want).abs() <= 1e-6, "{kernel:?}: pixel {n} channel {ch} is {}", p[ch]);
                }
            }
        }
    }
}
