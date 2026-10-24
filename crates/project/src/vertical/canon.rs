//! The storage grid (spec section 2.6). Stored `cx`, `cy` and `h` lie on the 1/1000 px grid; a write that clamps, snaps or
//! fits rounds inward, `h` first and the centre from the stored `h`, so the stored window stays inside the frame.

use super::eval::limits;
use super::types::{OutputSize, Raster, Window};

/// The legal centres at one height, already on the grid: `[ceil_g(lo), floor_g(hi)]` for `cx` and for `cy`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CentreBounds {
    pub cx: [f64; 2],
    pub cy: [f64; 2],
}

/// `q(v) = (v * 1000.0).round() / 1000.0`: the nearest grid value (halves away from zero, as `f64::round` rounds).
pub fn q(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// `floor_g(v) = (v * 1000.0 + 1e-9).floor() / 1000.0`: the grid value at or below `v`. The tolerance of 1e-12 px
/// keeps a value that float noise put just below a grid point on that point.
pub fn floor_g(v: f64) -> f64 {
    (v * 1000.0 + 1e-9).floor() / 1000.0
}

/// `ceil_g(v) = (v * 1000.0 - 1e-9).ceil() / 1000.0`: the grid value at or above `v`, with the same tolerance.
pub fn ceil_g(v: f64) -> f64 {
    (v * 1000.0 - 1e-9).ceil() / 1000.0
}

/// Whether `v` is finite and on the grid: `q(v) == v`, bit for bit.
pub fn on_grid(v: f64) -> bool {
    v.is_finite() && q(v).to_bits() == v.to_bits()
}

/// `v` rounded down onto the 1e-6 grid of computed speeds, with `floor_g`'s tolerance of 1e-12: a speed that float
/// noise put just below a grid point stays on it (`1.7499999999999996` gives 1.75). Rounding down keeps a speed under
/// the caps that make its curve valid.
pub fn floor_micro(v: f64) -> f64 {
    (v * 1_000_000.0 + 1e-6).floor() / 1_000_000.0
}

/// The legal centres for a window of height `h` of `out`'s aspect on `r`, on the grid: with `w = h * OW / OH`,
/// `[ceil_g(w / 2), floor_g(W - w / 2)]` for `cx` and `[ceil_g(h / 2), floor_g(H - h / 2)]` for `cy`.
pub fn centre_bounds(r: &Raster, out: OutputSize, h: f64) -> CentreBounds {
    let w = h * f64::from(out.width) / f64::from(out.height);
    let cx = [ceil_g(w * 0.5), floor_g(f64::from(r.width) - w * 0.5)];
    let cy = [ceil_g(h * 0.5), floor_g(f64::from(r.height) - h * 0.5)];
    CentreBounds { cx, cy }
}

/// Steps 1-3 of the directed rounding: `h` onto the grid inside the limits, then the centre onto the grid inside the
/// bounds that height allows (the frame's centre when they cross). Every write that clamps, snaps or fits stores this.
pub fn clamp_window(w: Window, r: &Raster, out: OutputSize) -> Window {
    let heights = limits(r, out);
    // `max` then `min`, never `clamp`: the limits cross for an output that needs more than 4x upscale (I11)
    let h = q(w.h).max(heights.min_height).min(heights.max_height);
    let bounds = centre_bounds(r, out, h);
    let cx = inward(q(w.cx), bounds.cx, f64::from(r.width) / 2.0);
    let cy = inward(q(w.cy), bounds.cy, f64::from(r.height) / 2.0);
    Window { cx, cy, h }
}

/// `v` moved inside `[lo, hi]`, or `centre` when the bounds cross (the window is wider or taller than the frame).
fn inward(v: f64, [lo, hi]: [f64; 2], centre: f64) -> f64 {
    if lo > hi { centre } else { v.max(lo).min(hi) }
}

/// Step 4: a write without `clamp` stores `q` of each value and then checks the invariants.
pub fn quantize_window(w: Window) -> Window {
    Window { cx: q(w.cx), cy: q(w.cy), h: q(w.h) }
}

/// The height of a zoom in percent of the largest window: `q(max_height * 100.0 / zoom)`, where `max_height` is the
/// frame's limit on the grid. None for a zoom that is not a positive finite number. The write then checks I4 or clamps.
pub fn h_from_zoom(max_height: f64, zoom: f64) -> Option<f64> {
    (zoom.is_finite() && zoom > 0.0).then(|| q(max_height * 100.0 / zoom))
}

/// The zoom of a height in percent of the largest window: `max_height * 100.0 / h` (derived, never stored).
pub fn zoom_percent(max_height: f64, h: f64) -> f64 {
    max_height * 100.0 / h
}
