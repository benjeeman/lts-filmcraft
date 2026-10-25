//! Smooth through through the planner (spec section 3.7): the planner applies exactly the arrivals of M1's
//! `smooth_through` and keeps every waypoint's time and window; it refuses a wall inside the run (`move_crosses_wall`)
//! before it looks at locks (`smooth_through` cannot see walls), then a locked position (`locked`).

use serde_json::json;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;

/// Spec section 3.7's example: frame 0 cx 600, frame 50 cx 900, frame 100 cx 1250, full height.
fn walk() -> Fx {
    let mut fx = Fx::new(400);
    fx.install(one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 50, win(900.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 100, win(1250.0, 540.0, 1080.0), Arrival::Cut),
    ]));
    fx
}

fn smooth(ids: Vec<u32>, opts: SmoothOpts) -> Write {
    Write::SmoothThrough(SmoothThrough { plan: 1, positions: ids, opts, ..Default::default() })
}

#[test]
fn smooth_through_applies_the_arrivals_of_the_model_and_keeps_every_waypoint() {
    let mut fx = walk();
    let expected = smooth_through(fx.plan(1), &fx.r(), &[1, 2, 3], SmoothOpts::default()).expect("the example is valid");
    assert_eq!(expected[0], Arrival::Move { move_frames: 50, curve: Curve::Speeds { start: 0.0, end: 1.75 }, split_of: None });
    let planned = fx.apply(smooth(vec![1, 2, 3], SmoothOpts::default()));
    assert_eq!(planned.label, "Vertical: smooth through #1\u{2013}#3");
    assert_eq!(planned.updated, vec![2, 3]);
    let arrivals: Vec<Arrival> = fx.plan(1).positions.iter().skip(1).map(|p| p.arrival).collect();
    assert_eq!(arrivals, expected);
    let kept: Vec<(i64, Window)> = fx.plan(1).positions.iter().map(|p| (RATE.frame_at(p.at), p.window)).collect();
    assert_eq!(kept, vec![(0, win(600.0, 540.0, 1080.0)), (50, win(900.0, 540.0, 1080.0)), (100, win(1250.0, 540.0, 1080.0))]);
    assert!(!accepted(fx.write(smooth(vec![1, 2, 3], SmoothOpts::default()))).changed, "smoothing a smooth run again");
    // a moving start: the first segment enters at its mean speed
    let mut fx = walk();
    let moving = SmoothOpts { start_from_rest: false, end_at_rest: true };
    fx.apply(smooth(vec![1, 2, 3], moving));
    let Arrival::Move { curve: Curve::Speeds { start, .. }, .. } = fx.position(1, 2).arrival else { panic!("a speeds move") };
    assert_eq!(start, 1.0);
}

#[test]
fn a_wall_inside_the_run_is_refused_before_a_lock() {
    let mut fx = walk();
    fx.set_mut().walls.stored = vec![stored(70, WallOrigin::Manual, None)];
    fx.plan_mut(1).positions[2].locked = true;
    let e = fx.refuse(smooth(vec![1, 2, 3], SmoothOpts::default()));
    assert_eq!(e.code, "move_crosses_wall", "smooth_through cannot see walls: the planner checks them first");
    assert_eq!((e.detail["wall"].clone(), e.detail["sources"].clone()), (json!(70), json!(["manual"])));
    fx.set_mut().walls.stored.clear();
    let e = fx.refuse(smooth(vec![1, 2, 3], SmoothOpts::default()));
    assert_eq!((e.code, e.detail["ids"].clone()), ("locked", json!([3])));
    // a wall on the first waypoint is the run's start, not inside it; one frame later it is inside
    let mut fx = Fx::new(400);
    fx.install(one_plan(vec![
        at25(1, 10, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 60, win(900.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 110, win(1250.0, 540.0, 1080.0), Arrival::Cut),
    ]));
    fx.set_mut().walls.stored = vec![stored(10, WallOrigin::Manual, None)];
    assert!(accepted(fx.write(smooth(vec![1, 2, 3], SmoothOpts::default()))).changed, "(10, 110] holds no wall");
    fx.set_mut().walls.stored = vec![stored(11, WallOrigin::Manual, None)];
    assert_eq!(fx.refuse(smooth(vec![1, 2, 3], SmoothOpts::default())).code, "move_crosses_wall", "frame 11 is inside (10, 110]");
}

#[test]
fn the_run_must_be_three_consecutive_known_positions() {
    let fx = walk();
    let e = fx.refuse(smooth(vec![1, 2], SmoothOpts::default()));
    assert_eq!((e.code, e.detail["minimum"].clone()), ("out_of_range", json!(3)));
    let e = fx.refuse(smooth(vec![1, 2, 9], SmoothOpts::default()));
    assert_eq!((e.code, e.detail["position"].clone()), ("not_found", json!(9)));
    assert_eq!(fx.refuse(smooth(vec![1, 3, 2], SmoothOpts::default())).code, "out_of_range", "not consecutive in plan order");
}
