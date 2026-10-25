//! The spatial path of a move (spec section 3.4): the straight line in `(cx, cy, h)` between two windows, with a geometric
//! zoom schedule so a linear curve zooms at a constant apparent rate. `log` and `expm1` come from the pure-Rust `libm`, so
//! every target computes the same bits.

use super::types::Window;

/// `G(e)`: the progress along the line for the eased progress `e`. `e` itself for a pure pan (`h_from == h_to`), else
/// `expm1(e * L) / expm1(L)` with `L = log(h_to / h_from)`, so that `h(e) = h_from * (h_to / h_from)^e`.
pub fn progress(h_from: f64, h_to: f64, e: f64) -> f64 {
    match zoom(h_from, h_to) {
        Some((l, m)) => libm::expm1(e * l) / m,
        None => e,
    }
}

/// The window at eased progress `e` of the move from `from` to `to`: `cx = cx_i + (cx_j - cx_i) * g` and the same for `cy`
/// and `h`, with `g = G(e)`.
pub fn path(from: &Window, to: &Window, e: f64) -> Window {
    let g = progress(from.h, to.h, e);
    Window { cx: from.cx + (to.cx - from.cx) * g, cy: from.cy + (to.cy - from.cy) * g, h: from.h + (to.h - from.h) * g }
}

/// `G'(0) = L / expm1(L)`: how fast the path leaves its start per unit of `e` (1 for a pan).
pub fn start_rate(h_from: f64, h_to: f64) -> f64 {
    zoom(h_from, h_to).map_or(1.0, |(l, m)| l / m)
}

/// `G'(1) = L * (1 + expm1(L)) / expm1(L)`: how fast the path reaches its end per unit of `e` (1 for a pan).
pub fn end_rate(h_from: f64, h_to: f64) -> f64 {
    zoom(h_from, h_to).map_or(1.0, |(l, m)| l * (1.0 + m) / m)
}

/// `L = log(h_to / h_from)` and `expm1(L)` of a zoom. None for a pure pan, and for heights whose ratio has no usable
/// logarithm (a zero, negative or non-finite height: bad data, which then moves as a pan).
fn zoom(h_from: f64, h_to: f64) -> Option<(f64, f64)> {
    if h_from == h_to {
        return None;
    }
    let l = libm::log(h_to / h_from);
    let m = libm::expm1(l);
    (l.is_finite() && m.is_finite() && m != 0.0).then_some((l, m))
}
