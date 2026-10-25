//! T-M11, the edit hook's cases beyond one track of back-to-back clips (spec section 4.8; lane L1c round 3): video
//! transitions at the top level and inside a nest (step 9), a move that now departs from a `fit` position (step 8, I9),
//! a set with two plans, the renderer's nest depth (step 1), a roll that extends a clip's head with a suppressed edit
//! point and a stored wall on it (step 5), and a change of time remapping (step 9). Synthetic data only.

use filmcraft_time::Tick;
use serde_json::json;

use super::fixtures::{Fx, RATE, at25, one_plan, tk};
use super::hook_fixtures::*;
use super::support::{mv, win};
use crate::vertical::*;
use crate::{ClipId, ItemId, ParamValue, Project, Transition, TransitionAlign, TransitionId, find_effect};

/// A full-height window at `cx`.
fn full(cx: f64) -> Window {
    win(cx, 540.0, 1080.0)
}

/// The ids of a plan's positions flagged `PictureChanged`, in frame order.
fn picture_changed(plan: &VerticalPlan) -> Vec<u32> {
    plan.positions.iter().filter(|p| p.flags.contains(&Flag::PictureChanged)).map(|p| p.id).collect()
}

/// A transition `effect` of `frames` frames from `start`, between clips `from` and `to`.
fn transition(id: u64, effect: &str, start: i64, frames: i64, from: ClipId, to: ClipId) -> Transition {
    let (effect, align) = (find_effect(effect).expect("the transition exists").instance(), TransitionAlign::CenterAtCut);
    Transition { id: TransitionId(id), effect, start: tk(start), duration: tk(frames), from: Some(from), to: Some(to), align, reverse: false }
}

/// A ripple delete of frames 20-39 of sequence `seq` (inside its first clip), moving the transition of its first video
/// track with the clips after it, as the edit command does.
fn ripple_with_transition(p: &mut Project, seq: ItemId) {
    ripple_delete(p, seq, 20, 40);
    if let Some(t) = video_mut(p, seq, 0).transitions.first_mut() {
        t.start = Tick(t.start.0 - tk(20).0);
    }
}

#[test]
fn t_m11_a_transition_a_ripple_moves_exactly_flags_nothing_and_a_changed_one_flags_the_positions_over_it() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 10_000), (200, 100, 20_000)]);
    let ids = clip_ids(&fx.project, fx.seq);
    // an iris holds a point parameter whose "auto" value is NaN, which equals nothing: it is compared as it prints
    video_mut(&mut fx.project, fx.seq, 0).transitions.push(transition(9_001, "iris_round", 195, 10, ids[1], ids[2]));
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 100, full(1000.0), Arrival::Cut),
        at25(3, 200, full(1200.0), Arrival::Cut),
        at25(4, 250, full(800.0), Arrival::Cut),
    ]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_with_transition(p, fx.seq));
    follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(frames_of(new), vec![0, 20, 80, 180, 230], "an exact opener on the cut at 20");
    assert!(new.positions.iter().all(|p| p.flags.is_empty()), "the iris moved exactly with its clips: {:?}", new.positions);
    // the same ripple, with the iris twice as long: the positions over its frames, before and after, are flagged
    let mut longer = edited(&before, |p| {
        ripple_with_transition(p, fx.seq);
        if let Some(t) = video_mut(p, fx.seq, 0).transitions.first_mut() {
            (t.start, t.duration) = (tk(170), tk(20));
        }
    });
    follow(&before, &mut longer);
    assert_eq!(picture_changed(plan_in(&longer, fx.seq, 1)), vec![2, 3], "the iris covers 170-189 now, 175-184 before");
}

#[test]
fn t_m11_a_transition_inside_a_nest_flags_the_positions_over_it_and_one_moved_exactly_does_not() {
    let mut fx = Fx::with_clips(&[(0, 50, 5_000)]);
    // the nest: X [0, 100) and Y [100, 200) of the matte; the episode shows it from frame 50
    let settings = crate::SequenceSettings { frame_rate: RATE, ..Default::default() };
    let nest = fx.project.new_sequence("Nest", settings, 1, 0, None);
    let media = fx.media;
    let x = fx.make_item(media, tk(0), 100, 0);
    let y = fx.make_item(media, tk(100), 100, 10_000);
    let dissolve = transition(9_002, "cross_dissolve", 95, 10, x.id, y.id);
    video_mut(&mut fx.project, nest, 0).items.extend([x, y]);
    let item = fx.make_item(nest, tk(50), 200, 0);
    fx.put(0, item);
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 100, full(1000.0), Arrival::Cut),
        at25(3, 150, full(1200.0), Arrival::Cut),
        at25(4, 200, full(800.0), Arrival::Cut),
    ]));
    let before = fx.project.clone();
    let with_dissolve = |p: &mut Project| video_mut(p, nest, 0).transitions.push(dissolve.clone());
    let mut after = edited(&before, with_dissolve);
    follow(&before, &mut after);
    // the nest's frames 95-104 are the episode's 145-154
    assert_eq!(picture_changed(plan_in(&after, fx.seq, 1)), vec![2, 3]);
    // with the dissolve in the nest, a ripple in the episode before the nest moves it exactly: nothing is flagged
    let base = edited(&before, with_dissolve);
    let mut rippled = edited(&base, |p| ripple_delete(p, fx.seq, 10, 20));
    follow(&base, &mut rippled);
    let new = plan_in(&rippled, fx.seq, 1);
    assert_eq!(fr(find(new, 3).expect("#3").at), 140);
    assert!(new.positions.iter().all(|p| p.flags.is_empty()), "{:?}", new.positions);
}

#[test]
fn t_m11_a_move_left_departing_from_a_fit_position_becomes_a_cut() {
    let mut fx = Fx::with_clips(&[(0, 200, 0)]);
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        Position { layout: Layout::Fit, ..at25(2, 50, full(960.0), Arrival::Cut) },
        at25(3, 51, full(800.0), Arrival::Cut),
        at25(4, 80, full(1000.0), mv(20, Curve::Linear)),
    ]));
    assert!(validate(&fx.project, fx.seq()).is_empty());
    let a = clip_ids(&fx.project, fx.seq)[0];
    let before = fx.project.clone();
    // at 200 % old frames 50 and 51 both land on 25: #3, which #4's move departed from, goes, and the move would depart from
    // the fit position #2
    let mut after = edited(&before, |p| retime(p, fx.seq, a, 2.0, 100));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!((ids_of(new), frames_of(new)), (vec![1, 2, 4], vec![0, 25, 40]));
    let moved = find(new, 4).expect("#4");
    assert_eq!(moved.arrival, Arrival::Cut, "a layout changes by cut only (I9)");
    assert!(moved.flags.contains(&Flag::MoveShortened), "{:?}", moved.flags);
    assert!(find(new, 2).expect("#2").flags.contains(&Flag::PictureChanged), "#3 showed another window on 25");
    assert_eq!(plan_report(&report, fx.seq, 1).removed, vec![3]);
    assert!(report_changes(&report, fx.seq, "move_shortened").iter().any(|c| c.detail["position"] == json!(4)));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_every_plan_of_a_set_follows_the_edit_with_its_own_ids() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    let mut set = one_plan(vec![at25(1, 100, full(812.4), Arrival::Cut), at25(2, 150, full(960.0), mv(50, Curve::EaseInOut))]);
    let four_five = OutputSize { width: 1080, height: 1350 };
    let positions = vec![at25(1, 50, full(800.0), Arrival::Cut), at25(2, 300, full(1100.0), mv(40, Curve::Linear))];
    set.plans.push(VerticalPlan { positions, next_position_id: 3, ..VerticalPlan::new(2, "4:5", four_five) });
    set.next_plan_id = 3;
    fx.install(set);
    assert!(validate(&fx.project, fx.seq()).is_empty());
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 120, 130));
    let report = follow(&before, &mut after);
    assert_eq!(frames_of(plan_in(&after, fx.seq, 1)), vec![100, 119, 120, 140], "plan 1: the worked example");
    let second = plan_in(&after, fx.seq, 2);
    assert_eq!((ids_of(second), frames_of(second)), (vec![1, 3, 2], vec![50, 120, 290]), "plan 2: an exact opener on the cut");
    assert_eq!(find(second, 2).expect("#2").arrival, mv(40, Curve::Linear));
    for plan in [1, 2] {
        assert_follows(&before, &after, fx.seq, plan, &deleted(400, 120, 130), &format!("plan {plan}"));
    }
    let (one, two) = (plan_report(&report, fx.seq, 1), plan_report(&report, fx.seq, 2));
    assert_eq!((one.added, one.moved, two.added, two.moved), (vec![3, 4], vec![2], vec![3], vec![2]));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

/// Nests `depth` sequences one in another, the innermost holding one 300-frame clip of the matte, and shows the
/// outermost in the episode from frame 0. Returns the innermost.
fn nested(fx: &mut Fx, depth: usize) -> ItemId {
    let (innermost, _) = add_nest(fx, 300, 0);
    let mut outer = innermost;
    for level in 1..depth {
        let settings = crate::SequenceSettings { frame_rate: RATE, ..Default::default() };
        let next = fx.project.new_sequence(&format!("Nest {level}"), settings, 1, 0, None);
        let item = fx.make_item(outer, tk(0), 300, 0);
        video_mut(&mut fx.project, next, 0).items.push(item);
        outer = next;
    }
    let item = fx.make_item(outer, tk(0), 300, 0);
    fx.put(0, item);
    innermost
}

#[test]
fn t_m11_the_hook_follows_nests_to_the_renderers_depth_and_no_deeper() {
    for (depth, followed) in [(8, true), (9, false)] {
        let mut fx = Fx::with_clips(&[]);
        let innermost = nested(&mut fx, depth);
        fx.install(one_plan(vec![at25(1, 0, full(600.0), Arrival::Cut), at25(2, 150, full(1000.0), Arrival::Cut)]));
        let before = fx.project.clone();
        let mut after = edited(&before, |p| ripple_delete(p, innermost, 100, 110));
        let report = follow(&before, &mut after);
        let frames = frames_of(plan_in(&after, fx.seq, 1));
        if followed {
            assert_eq!(frames.last(), Some(&140), "{depth} nests: #2 follows its picture 10 frames left");
        } else {
            assert!(report.is_empty(), "{depth} nests: past the renderer's depth the nest shows nothing, and nothing follows");
            assert_eq!(frames, vec![0, 150]);
        }
    }
}

#[test]
fn t_m11_a_roll_keeps_a_suppressed_edit_point_suppressed_and_takes_its_walls_to_the_new_head() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 10_000)]);
    let ids = clip_ids(&fx.project, fx.seq);
    let positions = || vec![at25(1, 0, full(600.0), Arrival::Cut), at25(2, 150, full(1000.0), Arrival::Cut)];
    let mut set = one_plan(positions());
    set.walls.suppressed = vec![tk(100)];
    fx.install(set);
    let before = fx.project.clone();
    assert!(!wall_frames_of(&before, fx.seq).contains(&100), "the person suppressed B's edit point");
    // a roll: B's head takes A's last 10 frames, and B's edit point moves to 90
    let mut after = edited(&before, |p| roll(p, fx.seq, ids[0], ids[1], 10));
    let report = follow(&before, &mut after);
    let suppressed: Vec<i64> = set_in(&after, fx.seq).walls.suppressed.iter().map(|t| fr(*t)).collect();
    assert_eq!(suppressed, vec![90], "the suppression follows B's head, as an opener would");
    assert!(!wall_frames_of(&after, fx.seq).contains(&90), "so the edit point stays suppressed");
    assert!(stored_walls(&after, fx.seq, WallOrigin::Reanchored).is_empty(), "no wall is stored where the person suppressed one");
    assert!(report_changes(&report, fx.seq, "suppression_dropped").is_empty());
    assert_eq!(frames_of(plan_in(&after, fx.seq, 1)), vec![0, 150]);
    // a stored wall and a candidate on the edit point follow it too
    let mut set = one_plan(positions());
    set.walls.stored = vec![StoredWall { at: tk(100), origin: WallOrigin::Manual, score: None }];
    set.walls.candidates = vec![Candidate { at: tk(100), score: 0.25 }];
    fx.install(set);
    let before = fx.project.clone();
    let mut after = edited(&before, |p| roll(p, fx.seq, ids[0], ids[1], 10));
    follow(&before, &mut after);
    let walls = &set_in(&after, fx.seq).walls;
    assert_eq!(walls.stored, vec![StoredWall { at: tk(90), origin: WallOrigin::Manual, score: None }]);
    assert_eq!(walls.candidates, vec![Candidate { at: tk(90), score: 0.25 }]);
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_a_time_remapping_change_flags_the_positions_over_the_clip() {
    let mut fx = Fx::with_clips(&[(0, 150, 0), (150, 150, 10_000)]);
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 50, full(800.0), Arrival::Cut),
        at25(3, 150, full(1200.0), Arrival::Cut),
        at25(4, 220, full(1000.0), Arrival::Cut),
    ]));
    let b = clip_ids(&fx.project, fx.seq)[1];
    let before = fx.project.clone();
    let mut after = edited(&before, |p| set_param(clip_mut(p, fx.seq, b), "time_remap", "speed", ParamValue::Float(150.0)));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(picture_changed(new), vec![3, 4], "the positions whose frames show B");
    assert_eq!(frames_of(new), vec![0, 50, 150, 220], "no frame moves: clips map time without their time remapping");
    assert_eq!(plan_report(&report, fx.seq, 1).flagged, vec![3, 4]);
}
