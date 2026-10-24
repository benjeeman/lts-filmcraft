//! Move curves (spec sections 3.2 and 3.3): `E(u)` for every curve in its normative operation order, their speeds, and the
//! validity rules a written curve must pass (`bad_curve` with the failing axis).

use serde_json::json;

use super::types::{Curve, Refusal};

/// The largest speed `dE/du` a custom curve may reach, in units of the move's mean speed (I7).
pub const MAX_CURVE_SLOPE: f64 = 12.0;

/// The curve of a new move written without one: Ramp 20/20.
pub const DEFAULT_CURVE: Curve = Curve::Ramp { r_in: 0.2, r_out: 0.2 };

/// A derivative at most this large counts as zero in the slope rules, far below any slope that matters.
const FLAT: f64 = 1e-12;

/// The largest speed of a Bezier curve and the progress `u` where it occurs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlopePeak {
    /// In units of the move's mean speed; infinite where the curve's time stops while it still moves.
    pub slope: f64,
    pub at: f64,
}

/// `E(u)` of `curve`: `E(0) = 0` and `E(1) = 1` exactly (and outside `[0, 1]` the nearer of the two, 0 for NaN), else the
/// formula of spec section 3.2 in its operation order.
pub fn ease(curve: &Curve, u: f64) -> f64 {
    if u.is_nan() || u <= 0.0 {
        return 0.0;
    }
    if u >= 1.0 {
        return 1.0;
    }
    match *curve {
        Curve::Linear => u,
        Curve::EaseIn => u * u * (2.0 - u),
        Curve::EaseOut => u * (1.0 + u * (1.0 - u)),
        Curve::EaseInOut => u * u * (3.0 - 2.0 * u),
        Curve::Speeds { start, end } => speeds_ease(start, end, u),
        Curve::Bezier { x1, y1, x2, y2 } => CubicBezier::new(x1, y1, x2, y2).ease(u),
        Curve::Ramp { r_in, r_out } => ramp_ease(r_in, r_out, u),
    }
}

/// `dE/du` of `curve` at `u` in `[0, 1]`: the move's speed in units of its mean speed.
pub fn slope(curve: &Curve, u: f64) -> f64 {
    let u = if u.is_nan() { 0.0 } else { u.clamp(0.0, 1.0) };
    match *curve {
        Curve::Linear => speeds_slope(1.0, 1.0, u),
        Curve::EaseIn => speeds_slope(0.0, 1.0, u),
        Curve::EaseOut => speeds_slope(1.0, 0.0, u),
        Curve::EaseInOut => speeds_slope(0.0, 0.0, u),
        Curve::Speeds { start, end } => speeds_slope(start, end, u),
        Curve::Bezier { x1, y1, x2, y2 } => {
            let bezier = CubicBezier::new(x1, y1, x2, y2);
            bezier.speed(bezier.solve(u))
        }
        Curve::Ramp { r_in, r_out } => ramp_slope(r_in, r_out, u),
    }
}

/// The largest `dE/du` of `curve` over `[0, 1]`, in closed form.
pub fn peak_speed(curve: &Curve) -> f64 {
    match *curve {
        Curve::Linear => speeds_peak(1.0, 1.0),
        Curve::EaseIn => speeds_peak(0.0, 1.0),
        Curve::EaseOut => speeds_peak(1.0, 0.0),
        Curve::EaseInOut => speeds_peak(0.0, 0.0),
        Curve::Speeds { start, end } => speeds_peak(start, end),
        Curve::Bezier { x1, y1, x2, y2 } => bezier_max_slope(x1, y1, x2, y2).slope,
        Curve::Ramp { r_in, r_out } => ramp_cruise(r_in, r_out),
    }
}

/// Whether the cubic Bezier coordinate with control values `0, p1, p2, 1` never decreases on `[0, 1]`: `p1 >= 0`,
/// `1 - p2 >= 0` and `p2 - p1 >= -sqrt(p1 * (1 - p2))`.
pub fn monotone(p1: f64, p2: f64) -> bool {
    p1 >= 0.0 && 1.0 - p2 >= 0.0 && p2 - p1 >= -(p1 * (1.0 - p2)).sqrt()
}

/// The largest speed of `cubic-bezier(x1, y1, x2, y2)`, in closed form: at the ends and at the roots in `(0, 1)` of
/// `y''x' - y'x''`.
pub fn bezier_max_slope(x1: f64, y1: f64, x2: f64, y2: f64) -> SlopePeak {
    let bezier = CubicBezier::new(x1, y1, x2, y2);
    let (x, y) = (bezier.x, bezier.y);
    // `y''x' - y'x''`: the cubic terms cancel, which leaves a quadratic in `s`
    let roots = unit_roots(6.0 * (y.a * x.b - x.a * y.b), 6.0 * (y.a * x.c - x.a * y.c), 2.0 * (y.b * x.c - x.b * y.c));
    let mut peak = SlopePeak { slope: bezier.speed(0.0), at: 0.0 };
    for s in roots.into_iter().flatten().chain([1.0]) {
        let slope = bezier.speed(s);
        if slope > peak.slope {
            peak = SlopePeak { slope, at: x.at(s) };
        }
    }
    peak
}

/// Whether a curve may be stored (I7). A refusal is `bad_curve` with the failing axis: `x`, `y`, `slope`, `speeds` or `ramp`.
pub fn check_curve(curve: &Curve) -> Result<(), Refusal> {
    match *curve {
        Curve::Linear | Curve::EaseIn | Curve::EaseOut | Curve::EaseInOut => Ok(()),
        Curve::Speeds { start, end } => check_speeds(start, end),
        Curve::Bezier { x1, y1, x2, y2 } => check_bezier(x1, y1, x2, y2),
        Curve::Ramp { r_in, r_out } => check_ramp(r_in, r_out),
    }
}

/// The curve's name in labels: "Linear", "Ease in", "Ease out", "Ease in-out", "Speeds", "Custom" or "Ramp 20/20".
pub fn curve_name(curve: &Curve) -> String {
    match *curve {
        Curve::Linear => "Linear".into(),
        Curve::EaseIn => "Ease in".into(),
        Curve::EaseOut => "Ease out".into(),
        Curve::EaseInOut => "Ease in-out".into(),
        Curve::Speeds { .. } => "Speeds".into(),
        Curve::Bezier { .. } => "Custom".into(),
        Curve::Ramp { r_in, r_out } => format!("Ramp {}/{}", percent(r_in), percent(r_out)),
    }
}

/// A share as a whole percentage, for labels.
fn percent(share: f64) -> i64 {
    (share * 100.0).round() as i64
}

/// `speeds [a, b]` is valid when both speeds lie in `[0, 3]` and its time never runs backwards: `3 - a - b + sqrt(ab) >= 0`,
/// the monotone rule with `y1 = a / 3` and `y2 = 1 - b / 3`.
fn check_speeds(start: f64, end: f64) -> Result<(), Refusal> {
    let in_range = |v: f64| (0.0..=3.0).contains(&v);
    let forward = 3.0 - start - end + (start * end).sqrt() >= 0.0;
    if in_range(start) && in_range(end) && forward {
        return Ok(());
    }
    let detail = json!({"axis": "speeds", "start": start, "end": end});
    Err(Refusal::new("bad_curve", "speeds must lie in [0, 3] and keep the move from reversing", detail))
}

/// A custom curve is valid when both coordinates are finite and never decrease (time never runs backwards; the window
/// never reverses or overshoots), and its speed never exceeds `MAX_CURVE_SLOPE` times the mean speed.
fn check_bezier(x1: f64, y1: f64, x2: f64, y2: f64) -> Result<(), Refusal> {
    let x_valid = x1.is_finite() && x2.is_finite() && monotone(x1, x2);
    if !x_valid {
        let detail = json!({"axis": "x", "x1": x1, "x2": x2});
        return Err(Refusal::new("bad_curve", "the curve's time runs backwards: x1 and x2 make x decrease", detail));
    }
    let y_valid = y1.is_finite() && y2.is_finite() && monotone(y1, y2);
    if !y_valid {
        let detail = json!({"axis": "y", "y1": y1, "y2": y2});
        return Err(Refusal::new("bad_curve", "the window would reverse or overshoot: y1 and y2 make y decrease", detail));
    }
    let peak = bezier_max_slope(x1, y1, x2, y2);
    if peak.slope.is_nan() || peak.slope > MAX_CURVE_SLOPE {
        // an infinite slope prints as null
        let detail = json!({"axis": "slope", "maxSlope": peak.slope, "at": peak.at, "limit": MAX_CURVE_SLOPE});
        return Err(Refusal::new("bad_curve", format!("the curve moves faster than {MAX_CURVE_SLOPE} times its mean speed"), detail));
    }
    Ok(())
}

/// A ramp is valid when `in` and `out` are finite, at least 0, and add up to at most 1.
fn check_ramp(r_in: f64, r_out: f64) -> Result<(), Refusal> {
    let valid = r_in.is_finite() && r_out.is_finite() && r_in >= 0.0 && r_out >= 0.0 && r_in + r_out <= 1.0;
    if valid {
        return Ok(());
    }
    let detail = json!({"axis": "ramp", "in": r_in, "out": r_out});
    Err(Refusal::new("bad_curve", "a ramp's in and out must be at least 0 and add up to at most 1", detail))
}

/// `E(u)` of `speeds [a, b]`: `u * (v * v * a + u * (v * (3 - b) + u))` with `v = 1 - u`.
fn speeds_ease(a: f64, b: f64, u: f64) -> f64 {
    let v = 1.0 - u;
    u * (v * v * a + u * (v * (3.0 - b) + u))
}

/// `E'(u)` of `speeds [a, b]`: `a + 2 (3 - b - 2a) u + 3 (a + b - 2) u^2`, which is `a` at 0 and `b` at 1.
fn speeds_slope(a: f64, b: f64, u: f64) -> f64 {
    a + 2.0 * (3.0 - b - 2.0 * a) * u + 3.0 * (a + b - 2.0) * u * u
}

/// The largest `E'` of `speeds [a, b]` on `[0, 1]`: an end speed, or the vertex of the parabola where it opens downward
/// inside the interval.
fn speeds_peak(a: f64, b: f64) -> f64 {
    let ends = a.max(b);
    let (linear, quadratic) = (2.0 * (3.0 - b - 2.0 * a), 3.0 * (a + b - 2.0));
    if quadratic >= 0.0 {
        return ends;
    }
    let vertex = -linear / (2.0 * quadratic);
    if vertex > 0.0 && vertex < 1.0 { ends.max(speeds_slope(a, b, vertex)) } else { ends }
}

/// The cruise speed of the ramp `{in: a, out: b}`, `v = 1 / (1 - (a + b) / 2)`: its peak speed.
fn ramp_cruise(a: f64, b: f64) -> f64 {
    1.0 / (1.0 - 0.5 * (a + b))
}

/// `E(u)` of the ramp `{in: a, out: b}` in the normative order of spec section 3.2: constant acceleration, a cruise at
/// `v`, constant deceleration. A branch divides by `a` or `b` only when it is positive. The result is
/// `e.max(0.0).min(1.0)` in that order (NaN gives 0), which `clamp` would not reproduce.
#[allow(clippy::manual_clamp)]
fn ramp_ease(a: f64, b: f64, u: f64) -> f64 {
    let v = ramp_cruise(a, b);
    let e = if u < a {
        0.5 * v * u * u / a
    } else if u <= 1.0 - b {
        v * (u - 0.5 * a)
    } else {
        let w = 1.0 - u;
        1.0 - 0.5 * v * w * w / b
    };
    e.max(0.0).min(1.0)
}

/// `E'(u)` of the ramp `{in: a, out: b}`: `v u / a` while it accelerates, `v` while it cruises (its corners included, as
/// in `E`), `v (1 - u) / b` while it decelerates.
fn ramp_slope(a: f64, b: f64, u: f64) -> f64 {
    let v = ramp_cruise(a, b);
    if u < a {
        v * u / a
    } else if u <= 1.0 - b {
        v
    } else {
        v * (1.0 - u) / b
    }
}

/// One coordinate of a CSS cubic Bezier with control values `0, p1, p2, 1`, as `((a s + b) s + c) s`.
#[derive(Clone, Copy, Debug)]
struct Coordinate {
    a: f64,
    b: f64,
    c: f64,
}

impl Coordinate {
    /// The coefficients, once per curve, in the order of spec section 3.3: `c = 3 p1`, `b = 3 (p2 - p1) - c`,
    /// `a = 1 - c - b`.
    fn new(p1: f64, p2: f64) -> Self {
        let c = 3.0 * p1;
        let b = 3.0 * (p2 - p1) - c;
        Self { a: 1.0 - c - b, b, c }
    }

    /// The coordinate at `s`.
    fn at(self, s: f64) -> f64 {
        ((self.a * s + self.b) * s + self.c) * s
    }

    /// Its first derivative at `s`.
    fn rate(self, s: f64) -> f64 {
        (3.0 * self.a * s + 2.0 * self.b) * s + self.c
    }

    /// Its second derivative at `s`.
    fn bend(self, s: f64) -> f64 {
        6.0 * self.a * s + 2.0 * self.b
    }
}

/// `cubic-bezier(x1, y1, x2, y2)`: time `x` and progress `y` as functions of the curve parameter `s`.
#[derive(Clone, Copy, Debug)]
struct CubicBezier {
    x: Coordinate,
    y: Coordinate,
}

impl CubicBezier {
    fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Self { x: Coordinate::new(x1, x2), y: Coordinate::new(y1, y2) }
    }

    /// The parameter `s` where `x(s) = u`: exactly 64 bisection steps from `[0, 1]` (spec section 3.3), deterministic and
    /// free of divisions, so any agent reproduces it bit for bit.
    fn solve(self, u: f64) -> f64 {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..64 {
            let mid = 0.5 * (lo + hi);
            if self.x.at(mid) < u { lo = mid } else { hi = mid }
        }
        0.5 * (lo + hi)
    }

    /// `E(u) = y(s)` where `x(s) = u`.
    fn ease(self, u: f64) -> f64 {
        self.y.at(self.solve(u))
    }

    /// The speed `dy/dx` at `s`. Where `x'` vanishes it is the limit of the ratio of the first derivatives that do not:
    /// infinite while the window still moves as time stops, else the ratio of the second derivatives, else of the third
    /// (`6 a_y / 6 a_x`; `x` has no flat stretch, so `a_x` is not 0 where its first two derivatives vanish).
    fn speed(self, s: f64) -> f64 {
        let (dx, dy) = (self.x.rate(s), self.y.rate(s));
        if dx > FLAT {
            return dy / dx;
        }
        if dy > FLAT {
            return f64::INFINITY;
        }
        let (ddx, ddy) = (self.x.bend(s), self.y.bend(s));
        if ddx.abs() > FLAT {
            return ddy / ddx;
        }
        if ddy.abs() > FLAT {
            return f64::INFINITY;
        }
        self.y.a / self.x.a
    }
}

/// The real roots in `(0, 1)` of `a s^2 + b s + c`, by the quadratic formula without cancellation (a nearly linear
/// quadratic keeps its small root exact).
fn unit_roots(a: f64, b: f64, c: f64) -> [Option<f64>; 2] {
    let inside = |s: f64| (s > 0.0 && s < 1.0).then_some(s);
    if a == 0.0 {
        // a line; a constant has no isolated root
        return [(b != 0.0).then_some(-c / b).and_then(inside), None];
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return [None, None];
    }
    let q = -0.5 * (b + discriminant.sqrt().copysign(b));
    if q == 0.0 {
        // `b = c = 0`: a double root at 0
        return [None, None];
    }
    [inside(q / a), inside(c / q)]
}
