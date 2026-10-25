//! T-M14, merged gestures (spec section 4.8, model half): every sub-step of a 16:9 trim drag is followed from the gesture's
//! base, whose plans the hook takes, so the result depends only on the base and the sub-step. A drag that returns to its
//! start leaves the plan bit-identical, a lossy sub-step (a position past the trimmed end) never sticks, and a drag across
//! an opener keeps it. The engine half (through `edit_merged`) is lane L2d's. Synthetic data only.

use std::collections::BTreeMap;

use super::fixtures::{Fx, at25, one_plan, tk};
use super::hook_fixtures::*;
use super::support::{mv, same_bits, win};
use crate::vertical::*;
use crate::{ClipId, ItemId, Project};

/// The gesture's base: A [0, 100) and B [100, 250); #5 is a cut near A's end, #3 opens B.
fn base() -> (Fx, ClipId, ClipId) {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 150, 10_000)]);
    let opener = Position { label: "B".into(), reviewed: true, ..at25(3, 100, win(1200.0, 540.0, 1080.0), Arrival::Cut) };
    fx.install(one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 40, win(800.0, 540.0, 1080.0), mv(20, Curve::Linear)),
        at25(5, 95, win(700.0, 540.0, 1080.0), Arrival::Cut),
        opener,
        at25(4, 180, win(1000.0, 600.0, 900.0), mv(50, Curve::EaseInOut)),
    ]));
    let ids = clip_ids(&fx.project, fx.seq);
    (fx, ids[0], ids[1])
}

/// The sub-step of a ripple trim drag of A's tail at `by` frames from the base, made from `current`, the engine's working
/// copy (which holds the previous sub-step's plans): A is `100 + by` frames long and B starts where A ends.
fn sub_step(current: &Project, seq: ItemId, a: ClipId, b: ClipId, by: i64) -> Project {
    edited(current, |p| {
        clip_mut(p, seq, a).duration = tk(100 + by);
        clip_mut(p, seq, b).start = tk(100 + by);
    })
}

#[test]
fn t_m14_a_trim_drag_that_returns_to_its_start_leaves_the_plan_bit_identical() {
    let (fx, a, b) = base();
    let start = fx.project.clone();
    let mut current = start.clone();
    let mut seen: BTreeMap<i64, VerticalSet> = BTreeMap::new();
    for by in [-5, -10, -15, -10, -5, 0, 5, 10, 5, 0] {
        let mut after = sub_step(&current, fx.seq, a, b, by);
        follow(&start, &mut after);
        let set = set_in(&after, fx.seq).clone();
        // the same sub-step made straight from the base, with no drag before it
        let mut direct = sub_step(&start, fx.seq, a, b, by);
        follow(&start, &mut direct);
        assert_eq!(&set, set_in(&direct, fx.seq), "sub-step {by} depends only on the base and the sub-step");
        if let Some(earlier) = seen.get(&by) {
            assert_eq!(&set, earlier, "sub-step {by} gives what it gave before");
        }
        seen.insert(by, set);
        current = after;
    }
    assert_eq!(set_in(&current, fx.seq), set_in(&start, fx.seq), "back at its start, the plan is bit-identical");
    let short = seen.get(&-10).expect("sub-step -10");
    assert!(short.plans[0].positions.iter().all(|p| p.id != 5), "#5 at 95 lies past A's end at -10");
    let long = seen.get(&5).expect("sub-step 5");
    assert!(long.plans[0].positions.iter().any(|p| p.id == 5), "and is back once A reaches it again");
}

#[test]
fn t_m14_a_drag_across_an_opener_keeps_it() {
    let (fx, a, b) = base();
    let start = fx.project.clone();
    let mut current = start.clone();
    for by in [-5, -10, 5, 10] {
        let mut after = sub_step(&current, fx.seq, a, b, by);
        let report = follow(&start, &mut after);
        let plan = plan_in(&after, fx.seq, 1);
        let opener = find(plan, 3).expect("B's opener");
        assert_eq!(fr(opener.at), 100 + by, "sub-step {by}: the opener follows B");
        assert!(same_bits(&opener.window, &win(1200.0, 540.0, 1080.0)), "sub-step {by}");
        assert_eq!((opener.label.as_str(), opener.reviewed, opener.flags.is_empty()), ("B", true, true), "sub-step {by}: an exact remap");
        assert_eq!(fr(find(plan, 4).expect("#4").at), 180 + by, "sub-step {by}");
        assert_eq!(find(plan, 5).is_some(), by > -5, "sub-step {by}: #5 at 95 shows only while A reaches it");
        let map: Vec<Option<i64>> = (0..250).map(|f| if f < 100 { (f < 100 + by).then_some(f) } else { Some(f + by) }).collect();
        assert_follows(&start, &after, fx.seq, 1, &map, &format!("sub-step {by}"));
        assert!(plan_report(&report, fx.seq, 1).moved.contains(&3), "sub-step {by}");
        current = after;
    }
}
