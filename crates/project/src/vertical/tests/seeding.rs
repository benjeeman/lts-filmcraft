//! `vertical.seedShots` (spec section 4.5): an opening cut on every wall (and frame 0) in range that has none. `carry`
//! takes the window already shown there, so nothing changes visibly; `starting` takes the plan's starting window (or the
//! centred largest one). Origin `seeded`, unreviewed whoever runs it, one step, idempotent. An opener in front of the
//! first position gives its move, which had no effect, one: a move that then has no room becomes a cut, reported.

use serde_json::json;

use super::fixtures::*;
use super::support::{mv, win};
use crate::vertical::*;

/// Walls at 100, 200 and 300; a cut at 50 and a 30-frame move into 250 (from frame 220).
fn walled() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 5_000), (200, 100, 9_000), (300, 700, 13_000)]);
    fx.install(one_plan(vec![at25(1, 50, win(700.0, 540.0, 1080.0), Arrival::Cut), at25(2, 250, win(900.0, 540.0, 1080.0), mv(30, Curve::Linear))]));
    fx
}

fn seed(mode: SeedMode, from: Option<i64>, to: Option<i64>) -> Write {
    Write::SeedShots(SeedShots { plan: 1, from_frame: from, to_frame: to, mode, ..Default::default() })
}

#[test]
fn carry_seeds_an_opener_on_every_shot_with_the_window_already_shown() {
    let mut fx = walled();
    let planned = fx.apply(seed(SeedMode::Carry, None, None));
    assert_eq!(planned.label, "Vertical: seed 4 positions");
    assert_eq!(planned.added, vec![3, 4, 5, 6]);
    assert!(planned.changed_frames.is_empty(), "nothing changes visibly");
    let row = |p: &Position| (RATE.frame_at(p.at), p.window, p.arrival, p.origin, p.reviewed);
    let rows: Vec<(i64, Window, Arrival, Origin, bool)> = fx.plan(1).positions.iter().map(row).collect();
    let a = win(700.0, 540.0, 1080.0);
    let b = win(900.0, 540.0, 1080.0);
    assert_eq!(
        rows,
        vec![
            (0, a, Arrival::Cut, Origin::Seeded, false),
            (50, a, Arrival::Cut, Origin::Manual, false),
            (100, a, Arrival::Cut, Origin::Seeded, false),
            (200, a, Arrival::Cut, Origin::Seeded, false),
            (250, b, mv(30, Curve::Linear), Origin::Manual, false),
            (300, b, Arrival::Cut, Origin::Seeded, false),
        ]
    );
    assert!(fx.plan(1).positions.iter().all(|p| p.modified_by == Writer::Human));
    let all = shots(&fx.project, fx.seq(), 1);
    assert!(all.iter().all(|s| s.status == ShotStatus::Own), "{all:?}");
    assert!(!accepted(fx.write(seed(SeedMode::Carry, None, None))).changed, "seeding is idempotent");
}

#[test]
fn starting_seeds_the_starting_window_and_a_range_bounds_the_walls() {
    let mut fx = walled();
    fx.plan_mut(1).starting_window = Some(win(800.0, 540.0, 1080.0));
    let planned = fx.apply(seed(SeedMode::Starting, Some(150), Some(320)));
    assert_eq!(planned.added.len(), 2, "the walls at 200 and 300");
    let seeded: Vec<(i64, Window)> = fx.plan(1).positions.iter().filter(|p| p.origin == Origin::Seeded).map(|p| (RATE.frame_at(p.at), p.window)).collect();
    assert_eq!(seeded, vec![(200, win(800.0, 540.0, 1080.0)), (300, win(800.0, 540.0, 1080.0))]);
    assert_eq!(planned.changed_frames, vec![[200, 250], [300, 1_000]], "the move into 250 now leaves from the starting window");
    // without a starting window: the centred, largest one
    let mut fx = walled();
    fx.apply(seed(SeedMode::Starting, Some(0), Some(1)));
    assert_eq!(fx.plan(1).positions[0].window, win(960.0, 540.0, 1080.0));
}

/// One position, at `frame`, reached by a 20-frame move: a move on the first position has no effect until a position comes
/// before it.
fn ignored(locked: bool, frame: i64) -> Fx {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![Position { locked, ..at25(1, frame, win(700.0, 540.0, 1080.0), mv(20, Curve::Linear)) }]));
    fx
}

#[test]
fn an_opener_in_front_of_an_ignored_move_cuts_it_when_the_move_has_no_room() {
    let mut fx = ignored(false, 10);
    let planned = fx.apply(seed(SeedMode::Carry, None, None));
    assert_eq!(fx.frames(1), vec![0, 10]);
    assert_eq!(fx.position(1, 1).arrival, Arrival::Cut, "20 frames do not fit after the opener at 0 (I5)");
    let cut: Vec<&Change> = planned.changes.iter().filter(|c| c.code == "move_cut").collect();
    assert_eq!(cut.len(), 1);
    assert_eq!(cut[0].detail, json!({"plan": 1, "position": 1, "reason": "a position was added before it"}));
    assert!(planned.changed_frames.is_empty(), "nothing changes visibly");
    // a move that has its room keeps its arrival
    let mut fx = ignored(false, 30);
    fx.apply(seed(SeedMode::Carry, None, None));
    assert_eq!(fx.position(1, 1).arrival, mv(20, Curve::Linear));
    // a locked position is not changed: the seeding is refused
    assert_eq!(ignored(true, 10).refuse(seed(SeedMode::Carry, None, None)).code, "locked");
}

#[test]
fn a_single_seeded_position_stays_unreviewed() {
    let mut fx = walled();
    let planned = fx.apply(seed(SeedMode::Carry, Some(290), Some(310)));
    assert_eq!(planned.added.len(), 1);
    let seeded = fx.plan(1).positions.iter().find(|p| p.origin == Origin::Seeded).expect("the opener at 300");
    assert!(!seeded.reviewed, "seeding starts unreviewed, even when one position changes");
}
