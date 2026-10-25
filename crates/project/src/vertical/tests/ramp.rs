//! T-M23: the Ramp curve (spec section 3.2). Its values bit for bit in the normative order, continuity at its corners, the
//! peak of its continuous speed and of its speed on the frame grid, monotonicity without a division by zero, the
//! `bad_curve {axis: "ramp"}` refusals, and Ramp 20/20 as the default curve.

use crate::vertical::*;

#[test]
fn t_m23_ramp_values_bit_for_bit() {
    let ramp = Curve::Ramp { r_in: 0.2, r_out: 0.2 };
    assert_eq!(ease(&ramp, 0.0).to_bits(), 0.0_f64.to_bits());
    assert_eq!(ease(&ramp, 1.0).to_bits(), 1.0_f64.to_bits());
    for (u, e) in [(0.1, 0.03125), (0.5, 0.5), (0.9, 0.96875), (0.8, 0.8750000000000001)] {
        assert_eq!(ease(&ramp, u).to_bits(), f64::to_bits(e), "E({u}) is {e} exactly");
    }
}

#[test]
fn t_m23_ramp_is_continuous_at_its_corners() {
    for (r_in, r_out) in [(0.2, 0.2), (0.1, 0.3), (0.35, 0.05), (0.5, 0.5), (0.0, 0.4), (0.3, 0.0), (0.25, 0.75)] {
        let ramp = Curve::Ramp { r_in, r_out };
        for corner in [r_in, 1.0 - r_out] {
            if corner <= 0.0 || corner >= 1.0 {
                continue;
            }
            let (below, at, above) = (ease(&ramp, corner.next_down()), ease(&ramp, corner), ease(&ramp, corner.next_up()));
            assert!((at - below).abs() <= 1e-15 && (above - at).abs() <= 1e-15, "{ramp:?} at {corner}: {below}, {at}, {above}");
            assert!(below <= at && at <= above && below < 1.0 && above > 0.0, "{ramp:?} moves through {corner}");
        }
    }
}

#[test]
fn t_m23_the_peak_of_the_continuous_speed() {
    for (r_in, r_out) in [(0.2, 0.2), (0.1, 0.3), (0.5, 0.5), (0.0, 0.0), (0.25, 0.75), (0.0, 0.6), (0.45, 0.05)] {
        let ramp = Curve::Ramp { r_in, r_out };
        let v = 1.0 / (1.0 - (r_in + r_out) / 2.0);
        // a dense grid that holds both corners
        let grid = (0..=100_000).map(|k| f64::from(k) / 100_000.0).chain([r_in, 1.0 - r_out]);
        let peak = grid.map(|u| curve::slope(&ramp, u)).fold(0.0, f64::max);
        assert!((peak - v).abs() <= 1e-12, "{ramp:?}: the peak of E' is {peak}, not {v}");
        assert!((curve::peak_speed(&ramp) - v).abs() <= 1e-12, "{ramp:?}: peak_speed");
    }
    assert_eq!(curve::peak_speed(&Curve::Ramp { r_in: 0.2, r_out: 0.2 }), 1.25);
}

#[test]
fn t_m23_the_peak_on_the_frame_grid() {
    // a move of `distance` px over `d` frames peaks at v * distance / d only when a whole frame lies in the cruise
    let distance = 262.4;
    for (r_in, r_out) in [(0.2, 0.2), (0.1, 0.3), (0.5, 0.5), (0.45, 0.45), (0.0, 0.2), (0.3, 0.7)] {
        let ramp = Curve::Ramp { r_in, r_out };
        let v = 1.0 / (1.0 - (r_in + r_out) / 2.0);
        for d in [2, 3, 5, 7, 10, 24, 55, 224] {
            let n = f64::from(d);
            let peak = (1..=d).map(|f| (ease(&ramp, f64::from(f) / n) - ease(&ramp, f64::from(f - 1) / n)) * distance).fold(0.0, f64::max);
            let bound = v * distance / n;
            let cruise_frame = (r_in * n - 1e-9).ceil() < ((1.0 - r_out) * n + 1e-9).floor();
            if cruise_frame {
                assert!((peak - bound).abs() <= 1e-9 * bound, "{ramp:?} over {d} frames: {peak} against {bound}");
            } else {
                assert!(peak <= bound * (1.0 + 1e-12), "{ramp:?} over {d} frames: {peak} above {bound}");
            }
            assert!(peak > 0.0, "{ramp:?} over {d} frames moves");
        }
    }
}

#[test]
fn t_m23_ramp_is_monotone_and_never_divides_by_zero() {
    for (r_in, r_out) in [(0.0, 0.0), (0.0, 0.3), (0.3, 0.0), (0.2, 0.2), (0.5, 0.5), (1.0, 0.0), (0.0, 1.0)] {
        let ramp = Curve::Ramp { r_in, r_out };
        let mut last = 0.0;
        for k in 0..=10_000 {
            let e = ease(&ramp, f64::from(k) / 10_000.0);
            assert!(e.is_finite() && e >= last, "{ramp:?} at step {k}: {e} after {last}");
            last = e;
        }
        assert_eq!(last, 1.0, "{ramp:?}");
    }
    // in = out = 0 is linear, bit for bit
    for k in 0..=1000 {
        let u = f64::from(k) / 1000.0;
        assert_eq!(ease(&Curve::Ramp { r_in: 0.0, r_out: 0.0 }, u).to_bits(), u.to_bits(), "u = {u}");
    }
}

#[test]
fn t_m23_bad_ramps_are_refused_on_the_ramp_axis() {
    for (r_in, r_out) in [(-0.1, 0.2), (0.2, -0.1), (f64::NAN, 0.2), (0.2, f64::NAN), (0.7, 0.4), (f64::INFINITY, 0.0)] {
        let refused = check_curve(&Curve::Ramp { r_in, r_out }).err();
        let shape = refused.as_ref().map(|r| (r.code, r.detail["axis"].as_str().map(str::to_string)));
        assert_eq!(shape, Some(("bad_curve", Some("ramp".to_string()))), "in {r_in}, out {r_out}");
    }
    for (r_in, r_out) in [(0.0, 0.0), (0.2, 0.2), (0.5, 0.5), (1.0, 0.0), (0.0, 1.0), (0.3, 0.7)] {
        assert!(check_curve(&Curve::Ramp { r_in, r_out }).is_ok(), "in {r_in}, out {r_out}");
    }
}

#[test]
fn t_m23_ramp_20_20_is_the_default_curve() {
    assert_eq!(DEFAULT_CURVE, Curve::Ramp { r_in: 0.2, r_out: 0.2 });
    assert_eq!(curve::curve_name(&DEFAULT_CURVE), "Ramp 20/20");
}
