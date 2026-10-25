//! T-M10 (spec section 3.8): `vertical.fitMove` returns the minimal move length: the limits hold at `d` and fail at
//! `d - 1`. At the amber default (0.14 output widths per second: 6.048 output px per frame at 1080 wide and 25p) a
//! full-width pan needs 224 frames with Ramp 20/20 and 268 with ease in-out, a third of the width 75 and 90; the move of
//! spec section 3.10 (cx 812.4 to 960) needs 55 frames. `speed_limit_unmet` carries `alternative: {"arrival": "cut"}`.

use serde_json::json;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;

const AMBER: f64 = 6.048;
const RAMP: Curve = Curve::Ramp { r_in: 0.2, r_out: 0.2 };

/// The leftmost full-height window, then a cut `distance` sequence px to the right at frame `at`.
fn pan(distance: f64, at: i64) -> Fx {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 0, win(303.75, 540.0, 1080.0), Arrival::Cut), at25(2, at, win(303.75 + distance, 540.0, 1080.0), Arrival::Cut)]));
    fx
}

fn fit_move(max_speed: f64, max_accel: Option<f64>, curve: Option<Curve>) -> Write {
    Write::FitMove(FitMove { plan: 1, position: 2, max_speed, max_accel, curve, ..Default::default() })
}

/// The measured peak speed and acceleration of the move into position 2.
fn peaks(fx: &Fx) -> (f64, f64) {
    let m = measure(&fx.project, fx.seq(), 1, &MeasureOpts::default()).expect("measured");
    let mv = m.moves.iter().find(|m| m.position == 2).copied().unwrap_or_default();
    (mv.peak_speed, mv.peak_accel)
}

/// The fitted length of `fx`'s move into position 2, after applying `w`; and the peaks at that length and one less.
fn fitted(fx: &mut Fx, w: Write) -> (u32, (f64, f64), (f64, f64)) {
    fx.apply(w);
    let Arrival::Move { move_frames, .. } = fx.position(1, 2).arrival else { return (0, (0.0, 0.0), (0.0, 0.0)) };
    let at_d = peaks(fx);
    let shorter = Write::SetArrival(SetArrival {
        plan: 1,
        positions: vec![2],
        arrival: ArrivalKind::Move,
        move_frames: Some(i64::from(move_frames) - 1),
        ..Default::default()
    });
    fx.apply(shorter);
    let at_less = peaks(fx);
    (move_frames, at_d, at_less)
}

#[test]
fn t_m10_a_full_width_pan_at_amber_needs_224_frames_with_ramp_and_268_with_ease_in_out() {
    for (curve, frames) in [(RAMP, 224), (Curve::EaseInOut, 268)] {
        let mut fx = pan(607.5, 400);
        let (d, (speed, _), (slower, _)) = fitted(&mut fx, fit_move(AMBER, None, Some(curve)));
        assert_eq!(d, frames, "{curve:?}");
        assert!(speed <= AMBER, "{curve:?}: the limit holds at {d} ({speed})");
        assert!(slower > AMBER, "{curve:?}: and fails at {} ({slower})", d - 1);
    }
}

#[test]
fn t_m10_a_third_of_the_width_and_the_move_of_section_3_10() {
    for (curve, frames) in [(RAMP, 75), (Curve::EaseInOut, 90)] {
        let mut fx = pan(202.5, 400);
        assert_eq!(fitted(&mut fx, fit_move(AMBER, None, Some(curve))).0, frames, "{curve:?}");
    }
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 100, win(812.4, 540.0, 1080.0), Arrival::Cut), at25(2, 400, win(960.0, 540.0, 1080.0), Arrival::Cut)]));
    let (d, (speed, _), (slower, _)) = fitted(&mut fx, fit_move(AMBER, None, None));
    assert_eq!(d, 55, "a cut given no curve becomes a Ramp 20/20 move");
    assert!((speed - 5.963_636_363_636_6).abs() < 1e-9, "{speed}");
    assert!((slower - 6.074_074_074_074_2).abs() < 1e-9, "{slower}");
}

#[test]
fn t_m10_the_move_keeps_its_own_curve_and_meets_an_acceleration_limit() {
    // an ease in-out move is refitted with its own curve
    let mut fx = pan(607.5, 400);
    let curve = Some(Curve::EaseInOut);
    let ease = SetArrival { plan: 1, positions: vec![2], arrival: ArrivalKind::Move, move_frames: Some(30), curve, ..Default::default() };
    fx.apply(Write::SetArrival(ease));
    let (d, _, _) = fitted(&mut fx, fit_move(AMBER, None, None));
    assert_eq!(d, 268, "its own curve, ease in-out");
    assert_eq!(fx.position(1, 2).arrival, Arrival::Move { move_frames: 267, curve: Curve::EaseInOut, split_of: None }, "the shorter check kept the curve");
    // with a high speed limit and maxAccel 0.5 output px per frame squared, the acceleration decides
    for (curve, frames) in [(RAMP, 117), (Curve::EaseInOut, 113)] {
        let mut fx = pan(607.5, 400);
        let (d, (_, accel), (_, harder)) = fitted(&mut fx, fit_move(100.0, Some(0.5), Some(curve)));
        assert_eq!(d, frames, "{curve:?}");
        assert!(accel <= 0.5 && harder > 0.5, "{curve:?}: {accel} at {d}, {harder} at {}", d - 1);
    }
}

/// A zoom about the frame's centre from height `from` to height `to`, a cut at frame 400.
fn zoom(from: f64, to: f64) -> Fx {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 0, win(960.0, 540.0, from), Arrival::Cut), at25(2, 400, win(960.0, 540.0, to), Arrival::Cut)]));
    fx
}

#[test]
fn t_m10_a_zoom_out_is_measured_against_its_growing_height() {
    // a linear zoom out from 480 to 1080 px high has D_out = 1376.8 output px, so the spec's start, ceil(D_out / 30) = 46,
    // is no lower bound here: each frame's speed is measured against the larger height before it, and 31 frames hold
    let mut fx = zoom(480.0, 1080.0);
    let (d, (speed, _), (faster, _)) = fitted(&mut fx, fit_move(30.0, None, Some(Curve::Linear)));
    assert_eq!(d, 31);
    assert!(speed <= 30.0 && faster > 30.0, "{speed} at 31, {faster} at 30");
    // the zoom in keeps the bound (ceil(611.9 / 30) = 21) and needs 30 frames
    let mut fx = zoom(1080.0, 480.0);
    let (d, (speed, _), (faster, _)) = fitted(&mut fx, fit_move(30.0, None, Some(Curve::Linear)));
    assert_eq!(d, 30);
    assert!(speed <= 30.0 && faster > 30.0, "{speed} at 30, {faster} at 29");
}

#[test]
fn t_m10_too_little_room_is_speed_limit_unmet_with_a_cut_as_the_alternative() {
    let fx = pan(607.5, 100);
    let e = fx.refuse(fit_move(AMBER, None, Some(RAMP)));
    assert_eq!(e.code, "speed_limit_unmet");
    assert_eq!((e.detail["position"].clone(), e.detail["maxMoveFrames"].clone()), (json!(2), json!(100)));
    assert_eq!(e.detail["alternative"], json!({"arrival": "cut"}));
    let peak = e.detail["peak"].as_f64().expect("the peak at the longest move");
    assert!((peak - 13.5).abs() < 1e-9, "1.25 x 1080 / 100: {peak}");
    // a wall bounds the room as the previous position does
    let mut fx = pan(607.5, 400);
    fx.set_mut().walls.stored = vec![stored(350, WallOrigin::Manual, None)];
    let e = fx.refuse(fit_move(AMBER, None, Some(RAMP)));
    assert_eq!((e.code, e.detail["maxMoveFrames"].clone()), ("speed_limit_unmet", json!(50)));
    // nothing changes by itself: the cut stays a cut
    assert_eq!(fx.position(1, 2).arrival, Arrival::Cut);
}
