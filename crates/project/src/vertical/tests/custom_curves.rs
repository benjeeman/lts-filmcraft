//! T-M4: custom curves (spec section 3.3). The monotone rule, the slope bound (`cubic-bezier(1, 0, 0, 1)` refused,
//! `(0.9, 0, 0.1, 1)` accepted), and the closed-form largest slope against a dense numeric maximum.

use super::support::*;
use crate::vertical::*;

/// The derivative of a Bezier coordinate with control values `0, p1, p2, 1`, from its Bernstein form.
fn derivative(p1: f64, p2: f64, s: f64) -> f64 {
    let t = 1.0 - s;
    3.0 * (t * t * p1 + 2.0 * t * s * (p2 - p1) + s * s * (1.0 - p2))
}

/// The largest `y'(s) / x'(s)` on a dense grid of `s`, refined by a ternary search around the best grid point.
fn dense_max_slope(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let ratio = |s: f64| derivative(y1, y2, s) / derivative(x1, x2, s);
    let n = 20_000;
    let (mut best_s, mut best) = (0.0, ratio(0.0));
    for k in 1..=n {
        let s = f64::from(k) / f64::from(n);
        let v = ratio(s);
        if v > best {
            (best_s, best) = (s, v);
        }
    }
    let step = 1.0 / f64::from(n);
    let (mut lo, mut hi) = ((best_s - step).max(0.0), (best_s + step).min(1.0));
    for _ in 0..200 {
        let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if ratio(a) < ratio(b) { lo = a } else { hi = b }
    }
    best.max(ratio(0.5 * (lo + hi)))
}

/// The `bad_curve` axis of a refused curve; None when the curve is accepted or refused with another code.
fn axis(curve: Curve) -> Option<String> {
    match check_curve(&curve) {
        Err(refusal) if refusal.code == "bad_curve" => refusal.detail["axis"].as_str().map(str::to_string),
        _ => None,
    }
}

#[test]
fn t_m4_the_monotone_rule_matches_a_dense_check() {
    let mut rng = Rng::new(0x7e57_0004);
    let mut checked = 0;
    for _ in 0..4000 {
        let (p1, p2) = (rng.range(-0.5, 1.5), rng.range(-0.5, 1.5));
        let lowest = (0..=2000).map(|k| derivative(p1, p2, f64::from(k) / 2000.0)).fold(f64::INFINITY, f64::min);
        // too close to zero to call on a grid
        if lowest.abs() >= 1e-5 {
            assert_eq!(curve::monotone(p1, p2), lowest > 0.0, "p1 = {p1}, p2 = {p2}: the lowest derivative is {lowest}");
            checked += 1;
        }
    }
    assert!(checked > 3000, "{checked} cases checked");
    // the edges of the rule
    assert!(curve::monotone(0.0, 1.0) && curve::monotone(1.0, 0.0) && curve::monotone(0.42, 0.58));
    assert!(!curve::monotone(-0.01, 0.5) && !curve::monotone(0.5, 1.01) && !curve::monotone(1.5, 0.9));
}

#[test]
fn t_m4_the_slope_bound() {
    assert_eq!(axis(Curve::Bezier { x1: 1.0, y1: 0.0, x2: 0.0, y2: 1.0 }).as_deref(), Some("slope"), "time stops mid-move");
    let refused = check_curve(&Curve::Bezier { x1: 1.0, y1: 0.0, x2: 0.0, y2: 1.0 }).err().map(|r| r.detail);
    let max_slope = refused.as_ref().and_then(|d| d["maxSlope"].as_f64());
    assert!(max_slope.is_none_or(|s| s > MAX_CURVE_SLOPE), "the detail names the slope: {refused:?}");
    assert_eq!(axis(Curve::Bezier { x1: 0.9, y1: 0.0, x2: 0.1, y2: 1.0 }), None, "a hard snap peaks at 10");
    let snap = curve::bezier_max_slope(0.9, 0.0, 0.1, 1.0);
    assert!((snap.slope - 10.0).abs() < 1e-9 && (snap.at - 0.5).abs() < 1e-9, "{snap:?}");
    let css = curve::bezier_max_slope(0.42, 0.0, 0.58, 1.0);
    assert!((css.slope - 1.0 / 0.58).abs() < 1e-9 && (css.at - 0.5).abs() < 1e-9, "the CSS ease-in-out peaks at 1.72: {css:?}");
    assert_eq!(axis(Curve::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 }), None);
    assert!(curve::bezier_max_slope(1.0, 0.0, 0.0, 1.0).slope > MAX_CURVE_SLOPE);
    // a vertical start: x'(0) = 0 while y'(0) > 0
    assert_eq!(curve::bezier_max_slope(0.0, 0.5, 0.5, 1.0).slope, f64::INFINITY);
    assert_eq!(axis(Curve::Bezier { x1: 0.0, y1: 0.5, x2: 0.5, y2: 1.0 }).as_deref(), Some("slope"));
}

#[test]
fn t_m4_the_closed_form_largest_slope_equals_a_dense_maximum() {
    let mut rng = Rng::new(0x7e57_0044);
    for case in 0..80 {
        let (x1, x2) = (rng.range(0.05, 0.95), rng.range(0.05, 0.95));
        let (y1, y2) = (rng.range(0.0, 1.0), rng.range(0.0, 1.0));
        let closed = curve::bezier_max_slope(x1, y1, x2, y2).slope;
        let dense = dense_max_slope(x1, y1, x2, y2);
        assert!((closed - dense).abs() <= 1e-9 * dense.max(1.0), "case {case}: ({x1}, {y1}, {x2}, {y2}) closed form {closed}, dense {dense}");
        let accepted = check_curve(&Curve::Bezier { x1, y1, x2, y2 }).is_ok();
        assert_eq!(accepted, dense <= MAX_CURVE_SLOPE, "case {case}: the bound is {MAX_CURVE_SLOPE}, the peak {dense}");
    }
}

#[test]
fn t_m4_each_rule_refuses_on_its_axis() {
    let bezier = |x1: f64, y1: f64, x2: f64, y2: f64| axis(Curve::Bezier { x1, y1, x2, y2 });
    // time runs backwards: x is not monotone
    assert_eq!(bezier(1.5, 0.0, 0.9, 1.0).as_deref(), Some("x"));
    assert_eq!(bezier(f64::NAN, 0.0, 0.7, 1.0).as_deref(), Some("x"));
    // the window reverses or overshoots: y is not monotone
    assert_eq!(bezier(0.3, -0.4, 0.7, 1.0).as_deref(), Some("y"));
    assert_eq!(bezier(0.3, 0.0, 0.7, 1.4).as_deref(), Some("y"));
    assert_eq!(bezier(0.3, 0.0, 0.7, f64::INFINITY).as_deref(), Some("y"));
    // speeds outside [0, 3] or not finite
    for (start, end) in [(3.5, 0.0), (-0.1, 1.0), (1.0, 3.0001), (f64::NAN, 1.0), (0.5, f64::INFINITY)] {
        assert_eq!(axis(Curve::Speeds { start, end }).as_deref(), Some("speeds"), "speeds [{start}, {end}]");
    }
    // inside [0, 3] every pair is monotone: 3 - a - b + sqrt(ab) is 0 at (3, 3) and positive elsewhere
    for (start, end) in [(0.0, 0.0), (3.0, 3.0), (3.0, 0.0), (0.0, 1.75), (1.5, 1.5)] {
        assert_eq!(axis(Curve::Speeds { start, end }), None, "speeds [{start}, {end}]");
    }
    for curve in [Curve::Linear, Curve::EaseIn, Curve::EaseOut, Curve::EaseInOut] {
        assert!(check_curve(&curve).is_ok(), "{curve:?}");
    }
}
