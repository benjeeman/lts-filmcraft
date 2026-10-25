//! T-M34, the model half (spec section 4.2): applying a measurement over `[from, to)` replaces the stored `detected`
//! walls and the candidates there; `manual`, `agent`, `marker` and `reanchored` walls and every suppression stay; a deleted
//! detected wall stays suppressed; a measured cut inside a locked move stays a candidate (`wall_blocked_by_locked`); a
//! cut inside another move follows W5; the range joins `analysed`; applying the same measurement again changes nothing.

use serde_json::json;

use super::fixtures::*;
use super::support::{mv, win};
use crate::vertical::*;

/// Walls of every origin, a suppression, two candidates, and a locked 100-frame move into frame 1000 (from frame 900).
fn measured_before() -> Fx {
    let mut fx = Fx::new(2_000);
    let mut walk = at25(2, 1_000, win(1100.0, 540.0, 1080.0), mv(100, Curve::Linear));
    walk.locked = true;
    let mut set = one_plan(vec![at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut), walk]);
    set.walls.stored = vec![
        stored(100, WallOrigin::Detected, Some(0.6)),
        stored(300, WallOrigin::Manual, None),
        stored(500, WallOrigin::Detected, Some(0.7)),
        stored(600, WallOrigin::Agent, Some(0.8)),
        stored(700, WallOrigin::Marker, None),
        stored(800, WallOrigin::Reanchored, None),
        stored(1_500, WallOrigin::Detected, Some(0.9)),
    ];
    set.walls.suppressed = vec![tk(400)];
    set.walls.candidates = vec![Candidate { at: tk(450), score: 0.2 }, Candidate { at: tk(1_600), score: 0.3 }];
    fx.install(set);
    fx
}

fn frame(frame: i64, score: f64) -> MeasuredFrame {
    MeasuredFrame { frame, score }
}

/// A measurement of `[0, 1200)`: cuts on 100, 250, 300 (a manual wall), 400 (suppressed) and 950 (inside the locked move),
/// possible cuts on 460 and 1100.
fn measurement() -> Measurement {
    Measurement {
        from_frame: 0,
        to_frame: 1_200,
        cuts: vec![frame(100, 0.625), frame(250, 0.5), frame(300, 0.375), frame(400, 0.5625), frame(950, 0.4375)],
        candidates: vec![frame(460, 0.25), frame(1_100, 0.125)],
    }
}

fn apply(m: Measurement) -> Write {
    Write::ApplyMeasurement(ApplyMeasurement { measurement: m, ..Default::default() })
}

#[test]
fn t_m34_applying_replaces_only_detected_walls_and_candidates_in_range() {
    let mut fx = measured_before();
    let planned = fx.apply(apply(measurement()));
    let walls = &fx.set().walls;
    assert_eq!(
        walls.stored,
        vec![
            stored(100, WallOrigin::Detected, Some(0.625)),
            stored(250, WallOrigin::Detected, Some(0.5)),
            stored(300, WallOrigin::Manual, None),
            stored(600, WallOrigin::Agent, Some(0.8)),
            stored(700, WallOrigin::Marker, None),
            stored(800, WallOrigin::Reanchored, None),
            stored(1_500, WallOrigin::Detected, Some(0.9)),
        ],
        "detected walls in range are the measurement's; every other wall stays, and so does a detected wall outside the range"
    );
    assert_eq!(walls.suppressed, vec![tk(400)], "a measured cut on a suppressed frame is not applied");
    let candidates: Vec<(i64, f32)> = walls.candidates.iter().map(|c| (RATE.frame_at(c.at), c.score)).collect();
    assert_eq!(candidates, vec![(460, 0.25), (950, 0.4375), (1_100, 0.125), (1_600, 0.3)], "the cut inside the locked move stays a candidate");
    assert_eq!(walls.analysed, vec![FrameRange { from: tk(0), to: tk(1_200) }]);
    let blocked = with_code(&planned.issues, "wall_blocked_by_locked");
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].level, Level::Info);
    assert_eq!(blocked[0].detail, json!({"plan": 1, "position": 2, "frame": 950}));
    assert_eq!(fx.position(1, 2).arrival, mv(100, Curve::Linear), "the locked move is untouched");
    assert_eq!(planned.label, "Vertical: apply measured cuts 00:00:00:00\u{2013}00:00:48:00 (+2, 3 possible)");
    // the same measurement again restates the same walls
    let again = accepted(fx.write(apply(measurement())));
    assert!(!again.changed);
}

#[test]
fn t_m34_re_measuring_keeps_a_deleted_detected_wall_suppressed() {
    let mut fx = measured_before();
    fx.apply(apply(measurement()));
    fx.apply(Write::SetWalls(SetWalls { remove: vec![250], ..Default::default() }));
    assert_eq!(fx.set().walls.suppressed, vec![tk(250), tk(400)]);
    let mut second = measurement();
    second.cuts = vec![frame(150, 0.5), frame(250, 0.75)];
    fx.apply(apply(second));
    let detected: Vec<i64> = fx.set().walls.stored.iter().filter(|w| w.origin == WallOrigin::Detected).map(|w| RATE.frame_at(w.at)).collect();
    assert_eq!(detected, vec![150, 1_500], "a measured cut on a suppressed frame is not applied");
    assert!(!fx.wall_frames().contains(&250), "a wall a person deleted does not come back");
    assert_eq!(fx.set().walls.suppressed, vec![tk(250), tk(400)]);
}

#[test]
fn t_m34_a_narrower_range_leaves_the_rest_alone() {
    let mut fx = measured_before();
    let narrow = Write::ApplyMeasurement(ApplyMeasurement { measurement: measurement(), from_frame: Some(0), to_frame: Some(200), ..Default::default() });
    fx.apply(narrow);
    let detected: Vec<(i64, Option<f32>)> =
        fx.set().walls.stored.iter().filter(|w| w.origin == WallOrigin::Detected).map(|w| (RATE.frame_at(w.at), w.score)).collect();
    assert_eq!(detected, vec![(100, Some(0.625)), (500, Some(0.7)), (1_500, Some(0.9))]);
    let candidates: Vec<i64> = fx.set().walls.candidates.iter().map(|c| RATE.frame_at(c.at)).collect();
    assert_eq!(candidates, vec![450, 1_600], "no candidate of the measurement lies in [0, 200)");
    assert_eq!(fx.set().walls.analysed, vec![FrameRange { from: tk(0), to: tk(200) }]);
}

#[test]
fn t_m34_a_cut_inside_an_unlocked_move_follows_w5() {
    let mut fx = measured_before();
    fx.plan_mut(1).positions[1].locked = false;
    let e = fx.refuse(apply(measurement()));
    assert_eq!(e.code, "would_split_move");
    assert_eq!(e.detail, json!({"plan": 1, "position": 2, "wall": 950}));
    let shorten = Write::ApplyMeasurement(ApplyMeasurement { measurement: measurement(), on_conflict: OnConflict::Shorten, ..Default::default() });
    fx.apply(shorten);
    let p = fx.position(1, 2);
    assert_eq!((p.arrival, p.flags.clone()), (mv(50, Curve::Linear), vec![Flag::MoveShortened]));
    assert!(fx.set().walls.stored.contains(&stored(950, WallOrigin::Detected, Some(0.4375))));
}
