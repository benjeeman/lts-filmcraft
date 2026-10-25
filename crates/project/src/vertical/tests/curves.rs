//! T-M2: the named curves are FilmCraft's keyframe curves at influence 1/3 (`keyframe.rs`) and equal their `speeds` curves;
//! every curve is exact at both ends; the peak speeds of spec section 3.2.

use filmcraft_time::Tick;

use super::support::*;
use crate::keyframe::{Interpolation, Keyframe, Param, ParamValue};
use crate::vertical::*;

/// FilmCraft's keyframe evaluation of a 0 to 1 segment over `n` ticks at tick `k`, with the departing and the arriving
/// key's interpolation at the default influence of 1/3.
fn keyframe_curve(leave: Interpolation, arrive: Interpolation, k: i64, n: i64) -> f64 {
    let mut from = Keyframe::new(Tick(0), ParamValue::Float(0.0));
    from.interp = leave;
    let mut to = Keyframe::new(Tick(n), ParamValue::Float(1.0));
    to.interp = arrive;
    Param { value: ParamValue::Float(0.0), keyframes: vec![from, to] }.f64_at(Tick(k))
}

#[test]
fn t_m2_named_curves_are_filmcraft_keyframe_curves_at_one_third() {
    // CSS names: `easeIn` (slow start) is a keyframe "Ease Out" on the departing key, `easeOut` an "Ease In" on the arriving one
    let pairs = [
        (Curve::Linear, Interpolation::Linear, Interpolation::Linear),
        (Curve::EaseIn, Interpolation::EaseOut, Interpolation::Linear),
        (Curve::EaseOut, Interpolation::Linear, Interpolation::EaseIn),
        (Curve::EaseInOut, Interpolation::Bezier, Interpolation::Bezier),
    ];
    let n = 2000;
    for (curve, leave, arrive) in pairs {
        for k in 0..=n {
            let u = k as f64 / n as f64;
            let ours = ease(&curve, u);
            let keyframes = keyframe_curve(leave, arrive, k, n);
            assert!((ours - keyframes).abs() <= 1e-12, "{curve:?} at u = {u}: {ours} against the keyframe curve's {keyframes}");
        }
    }
}

#[test]
fn t_m2_named_curves_equal_their_speeds_curves() {
    let pairs = [(Curve::Linear, 1.0, 1.0), (Curve::EaseIn, 0.0, 1.0), (Curve::EaseOut, 1.0, 0.0), (Curve::EaseInOut, 0.0, 0.0)];
    for (curve, start, end) in pairs {
        let speeds = Curve::Speeds { start, end };
        for k in 0..=1000 {
            let u = f64::from(k) / 1000.0;
            let (named, cubic) = (ease(&curve, u), ease(&speeds, u));
            assert!((named - cubic).abs() <= 1e-15, "{curve:?} at u = {u}: {named} against speeds [{start}, {end}]: {cubic}");
        }
    }
}

#[test]
fn t_m2_every_curve_is_exact_at_both_ends() {
    let extra = [Curve::Bezier { x1: 0.9, y1: 0.0, x2: 0.1, y2: 1.0 }, Curve::Ramp { r_in: 0.0, r_out: 0.0 }, Curve::Ramp { r_in: 1.0, r_out: 0.0 }];
    for curve in every_curve().into_iter().chain(extra) {
        assert_eq!(ease(&curve, 0.0).to_bits(), 0.0_f64.to_bits(), "{curve:?} at 0");
        assert_eq!(ease(&curve, 1.0).to_bits(), 1.0_f64.to_bits(), "{curve:?} at 1");
        assert_eq!((ease(&curve, -0.5), ease(&curve, 1.5)), (0.0, 1.0), "{curve:?} outside [0, 1]");
    }
}

#[test]
fn t_m2_peak_speeds() {
    let ramp = Curve::Ramp { r_in: 0.2, r_out: 0.2 };
    let speeds = Curve::Speeds { start: 0.0, end: 1.75 };
    // speeds [0, 1.75]: E'(u) = 2.5 u - 0.75 u^2 peaks at u = 1 with 1.75
    let peaks = [(Curve::Linear, 1.0), (Curve::EaseIn, 4.0 / 3.0), (Curve::EaseOut, 4.0 / 3.0), (Curve::EaseInOut, 1.5), (ramp, 1.25), (speeds, 1.75)];
    for (curve, peak) in peaks {
        let closed = curve::peak_speed(&curve);
        assert!((closed - peak).abs() <= 1e-12, "{curve:?}: peak {closed} against {peak}");
        let dense = (0..=100_000).map(|k| curve::slope(&curve, f64::from(k) / 100_000.0)).fold(0.0, f64::max);
        assert!((dense - peak).abs() <= 1e-9, "{curve:?}: the dense peak {dense} against {peak}");
    }
    // the slope at the ends is the curve's end speed
    assert_eq!((curve::slope(&speeds, 0.0), curve::slope(&speeds, 1.0)), (0.0, 1.75));
    assert_eq!((curve::slope(&Curve::EaseIn, 0.0), curve::slope(&Curve::EaseOut, 1.0)), (0.0, 0.0));
}
