//! Walls (spec sections 2.3 and 4.1-4.3). T-M16: clip starts off the frame grid show on `frame_at(T - 1) + 1`, a 25 to
//! 29.97 rate change, a punch-in through edit is a wall and a plain one is not. The through-edit predicate on synthetic
//! adjacent pairs. The wall rules W1, W4, W5, W7, W8 and W9. T-M7: a valid plan never has a move containing a wall, and
//! random wall and part additions are refused or shorten moves exactly as W5 says.

use filmcraft_time::{FrameRate, Tick};
use serde_json::json;

use super::fixtures::*;
use super::support::{OUT, Rng, mv, random_curve, random_window, win};
use crate::vertical::*;
use crate::{ItemId, Keyframe, MulticamSel, ParamValue, TrackItem, find_effect};

/// Sets a static parameter of an effect of `item`.
fn set_param(item: &mut TrackItem, effect: &str, param: &str, value: ParamValue) {
    let p = item.effect_mut(effect).and_then(|e| e.param_mut(param)).expect("the parameter exists");
    p.value = value;
}

/// The changes of `planned` with `code`.
fn changes_with<'a>(planned: &'a Planned, code: &str) -> Vec<&'a Change> {
    planned.changes.iter().filter(|c| c.code == code).collect()
}

#[test]
fn t_m16_a_clip_start_off_the_frame_grid_is_a_wall_on_the_first_frame_that_shows_it() {
    let mut fx = Fx::with_clips(&[(0, 100, 0)]);
    fx.add_clip_at(0, Tick(tk(100).0 + 1), 90, 5_000);
    fx.add_clip_at(0, Tick(tk(200).0 - 1), 50, 9_000);
    fx.add_clip(0, 300, 100, 13_000);
    assert_eq!(fx.wall_frames(), vec![101, 200, 300], "frame 0 is never a wall");
    for f in [101, 200, 300] {
        let w = wall_at(&fx, f).expect("an edit wall");
        assert_eq!((w.at, w.sources.clone(), w.score), (tk(f), vec![WallSource::Edit], None), "frame {f}");
    }
    assert_eq!(shown_frame(RATE, Tick(tk(100).0 + 1)), 101);
    assert_eq!(shown_frame(RATE, Tick(tk(200).0 - 1)), 200);
    assert_eq!(shown_frame(RATE, tk(300)), 300);
    assert_eq!(shown_frame(RATE, Tick(0)), 0);
    assert_eq!(shown_frame(RATE, Tick(-1)), 0);
}

#[test]
fn t_m16_a_rate_change_puts_walls_on_the_shown_frames_of_the_new_rate() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 5_000)]);
    assert_eq!(fx.wall_frames(), vec![100]);
    // the second clip starts at 4 s; at 29.97 frame 119 starts at 3.970 s and frame 120 at 4.004 s
    fx.seq_mut().settings.frame_rate = FrameRate::FPS_29_97;
    let rate = FrameRate::FPS_29_97;
    let walls = walls(&fx.project, fx.seq());
    assert_eq!(walls.len(), 1);
    assert_eq!(walls[0].at, rate.tick_of(120), "the wall is frame-aligned at the new rate");
    assert_eq!(shown_frame(rate, tk(100)), 120);
    assert_eq!(raster(fx.seq()).end_frame, 240, "8 s at 29.97");
}

#[test]
fn t_m16_a_punch_in_through_edit_is_a_wall_and_a_plain_one_is_not() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 100)]);
    assert_eq!(fx.wall_frames(), Vec::<i64>::new(), "a plain through edit (Add Edit) is no wall");
    let right = fx.seq().video_tracks[0].items[1].id;
    set_param(fx.item_mut(right), "motion", "scale", ParamValue::Float(115.0));
    assert_eq!(fx.wall_frames(), vec![100], "a punch-in on the same take is a shot change");
    assert_eq!(wall_at(&fx, 100).expect("the punch-in").sources, vec![WallSource::Edit]);
    // a Crop on the right piece only is a shot change too; the same Crop on both is not
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 100)]);
    let crop = find_effect("crop").expect("Crop exists").instance();
    let (left, right) = (fx.seq().video_tracks[0].items[0].id, fx.seq().video_tracks[0].items[1].id);
    fx.item_mut(right).effects.push(crop.clone());
    assert_eq!(fx.wall_frames(), vec![100]);
    fx.item_mut(left).effects.push(crop);
    assert_eq!(fx.wall_frames(), Vec::<i64>::new());
}

#[test]
fn graphics_adjustment_layers_and_disabled_clips_or_tracks_make_no_wall() {
    let mut fx = Fx::with_clips(&[(0, 300, 0)]);
    // an adjustment layer and a graphic on the second video track
    let adjustment = fx.project.add_item(
        "Adjustment Layer",
        crate::Label::Lavender,
        crate::ItemKind::AdjustmentLayer { width: 1920, height: 1080, rate: RATE, duration: tk(1_000) },
        None,
    );
    let graphic = fx.project.add_item("Graphic", crate::Label::Rose, crate::ItemKind::Graphic { width: 1920, height: 1080, rate: RATE }, None);
    let a = fx.make_item(adjustment, tk(50), 20, 0);
    fx.put(1, a);
    let g = fx.make_item(graphic, tk(100), 20, 0);
    fx.put(1, g);
    assert_eq!(fx.wall_frames(), Vec::<i64>::new(), "graphics and adjustment layers are no picture cut");
    // a picture clip on the second track is a wall; disabled, or on a disabled track, it is not
    let clip = fx.add_clip(1, 200, 30, 7_000);
    assert_eq!(fx.wall_frames(), vec![200]);
    fx.item_mut(clip).enabled = false;
    assert_eq!(fx.wall_frames(), Vec::<i64>::new());
    fx.item_mut(clip).enabled = true;
    fx.seq_mut().video_tracks[1].enabled = false;
    assert_eq!(fx.wall_frames(), Vec::<i64>::new());
    // a nested sequence is a picture clip
    fx.seq_mut().video_tracks[1].enabled = true;
    let nest = fx.project.new_sequence("Nest", crate::SequenceSettings { frame_rate: RATE, ..Default::default() }, 1, 0, None);
    let n = fx.make_item(nest, tk(250), 10, 0);
    fx.put(1, n);
    assert_eq!(fx.wall_frames(), vec![200, 250]);
    // without edit points, none of them is a wall
    fx.install(one_plan(Vec::new()));
    fx.set_mut().walls.use_edit_points = false;
    assert_eq!(fx.wall_frames(), Vec::<i64>::new());
}

#[test]
fn effective_walls_list_every_source_and_drop_suppressed_frames() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 900, 5_000)]);
    let mut set = one_plan(Vec::new());
    set.walls.stored = vec![stored(100, WallOrigin::Detected, Some(0.75)), stored(300, WallOrigin::Manual, None), stored(500, WallOrigin::Agent, Some(0.5))];
    set.walls.suppressed = vec![tk(500)];
    set.walls.candidates = vec![Candidate { at: tk(700), score: 0.2 }];
    set.parts = vec![part25(1, "P01", 300, 600)];
    fx.install(set);
    let walls = walls(&fx.project, fx.seq());
    let listed: Vec<(i64, Vec<WallSource>, Option<f32>)> = walls.iter().map(|w| (RATE.frame_at(w.at), w.sources.clone(), w.score)).collect();
    assert_eq!(
        listed,
        vec![
            (100, vec![WallSource::Edit, WallSource::Detected], Some(0.75)),
            (300, vec![WallSource::Part, WallSource::Manual], None),
            (600, vec![WallSource::Part], None),
        ],
        "a suppressed frame is never a wall; a candidate is never a wall"
    );
}

#[test]
fn the_through_edit_predicate_on_synthetic_adjacent_pairs() {
    let mut fx = Fx::new(10);
    let media = fx.media;
    let a = fx.make_item(media, tk(0), 100, 0);
    let b = fx.make_item(media, tk(100), 100, 100);
    let case = |a: &TrackItem, b: &TrackItem| (is_through_edit(a, b), geometry_differs(a, b), is_shot_change(a, b));
    assert_eq!(case(&a, &b), (true, false, false), "Add Edit: the auto (NaN) points of both pieces compare equal");
    // media continuation within 2 ticks
    let slack = |ticks: i64| TrackItem { source_in: Tick(tk(100).0 + ticks), ..b.clone() };
    assert_eq!(case(&a, &slack(2)), (true, false, false));
    assert_eq!(case(&a, &slack(-2)), (true, false, false));
    assert_eq!(case(&a, &slack(3)), (false, false, true));
    assert_eq!(case(&a, &TrackItem { source_in: tk(101), ..b.clone() }), (false, false, true), "a jump in media time");
    assert_eq!(case(&a, &TrackItem { start: tk(101), ..b.clone() }), (false, false, true), "a gap on the timeline");
    assert_eq!(case(&a, &TrackItem { item: ItemId(u64::MAX), ..b.clone() }), (false, false, true), "another item");
    assert_eq!(case(&a, &TrackItem { speed: 2.0, ..b.clone() }), (false, false, true), "another speed");
    assert_eq!(case(&a, &TrackItem { reverse: true, ..b.clone() }), (false, false, true), "reversed");
    // frame holds
    let held = |t: Option<Tick>| TrackItem { frame_hold: t, ..a.clone() };
    let held_b = |t: Option<Tick>| TrackItem { frame_hold: t, ..b.clone() };
    assert_eq!(case(&held(Some(tk(10))), &held_b(Some(tk(10)))), (true, false, false));
    assert_eq!(case(&held(Some(tk(10))), &held_b(Some(tk(11)))), (false, false, true));
    assert_eq!(case(&held(Some(tk(10))), &b), (false, false, true));
    // multicam angles
    let angle = |item: &TrackItem, angle: u32| TrackItem { multicam: Some(MulticamSel { enabled: true, angle }), ..item.clone() };
    assert_eq!(case(&angle(&a, 0), &angle(&b, 0)), (true, false, false));
    assert_eq!(case(&angle(&a, 0), &angle(&b, 1)), (false, false, true), "an angle switch is a cut");
    // geometry: Motion parameters and keyframes, enabled geometry effects
    let mut punch = b.clone();
    set_param(&mut punch, "motion", "scale", ParamValue::Float(115.0));
    assert_eq!(case(&a, &punch), (true, true, true));
    let mut keyed = b.clone();
    if let Some(p) = keyed.effect_mut("motion").and_then(|e| e.param_mut("rotation")) {
        p.keyframes.push(Keyframe::new(tk(0), ParamValue::Float(0.0)));
        p.keyframes.push(Keyframe::new(tk(50), ParamValue::Float(10.0)));
    }
    assert_eq!(case(&a, &keyed), (true, true, true), "Motion keyframes differ");
    let with = |item: &TrackItem, effect: &str, enabled: bool| {
        let mut item = item.clone();
        let mut e = find_effect(effect).expect("the effect exists").instance();
        e.enabled = enabled;
        item.effects.push(e);
        item
    };
    assert_eq!(case(&a, &with(&b, "crop", true)), (true, true, true), "Crop");
    assert_eq!(case(&a, &with(&b, "crop", false)), (true, false, false), "a disabled Crop changes nothing");
    assert_eq!(case(&a, &with(&b, "transform", true)), (true, true, true), "the Transform category");
    assert_eq!(case(&a, &with(&b, "auto_reframe", true)), (true, true, true), "Auto Reframe");
    assert_eq!(case(&a, &with(&b, "lens_distortion", true)), (true, true, true), "the Distort category");
    assert_eq!(case(&a, &with(&b, "brightness_contrast", true)), (true, false, false), "not a geometry effect");
    assert_eq!(case(&with(&a, "crop", true), &with(&b, "crop", true)), (true, false, false), "the same Crop on both pieces");
}

#[test]
fn w1_a_move_lives_in_one_shot() {
    // an edit wall at frame 100
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 200, 5_000)]);
    fx.install(one_plan(vec![at25(1, 50, win(700.0, 540.0, 1080.0), Arrival::Cut)]));
    let crossing = PositionFields { arrival: Some(ArrivalKind::Move), move_frames: Some(60), ..keys(900.0, 540.0, 1080.0) };
    let e = fx.refuse(set_at(150, crossing));
    assert_eq!(e.code, "move_crosses_wall");
    assert_eq!((e.detail["frame"].clone(), e.detail["wall"].clone()), (json!(150), json!(100)));
    assert_eq!((e.detail["sources"].clone(), e.detail["maxMoveFrames"].clone()), (json!(["edit"]), json!(50)));
    // the start of a move may sit on the wall; written without a curve it is Ramp 20/20
    let on_wall = PositionFields { arrival: Some(ArrivalKind::Move), move_frames: Some(50), ..keys(900.0, 540.0, 1080.0) };
    let planned = fx.apply(set_at(150, on_wall));
    assert!(planned.changed);
    assert_eq!(fx.frames(1), vec![50, 150]);
    assert_eq!(fx.plan(1).positions[1].arrival, mv(50, DEFAULT_CURVE));
    assert_eq!(planned.added, vec![2]);
    assert_eq!(planned.changed_frames, vec![[101, 300]]);
}

#[test]
fn w4_a_cut_off_a_wall_is_a_jump_and_its_shift_scale_and_overlap_are_reported() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 200, 5_000)]);
    let (a, b, c) = (win(700.0, 540.0, 1080.0), win(1200.0, 540.0, 1080.0), win(1000.0, 540.0, 900.0));
    fx.install(one_plan(vec![at25(1, 0, a, Arrival::Cut), at25(2, 100, b, Arrival::Cut), at25(3, 200, c, Arrival::Cut), at25(4, 250, c, Arrival::Cut)]));
    let derived = derive_plan(&fx.project, fx.seq(), 1).expect("the plan exists");
    let tags: Vec<Tag> = derived.positions.iter().map(|p| p.tag).collect();
    assert_eq!(tags, vec![Tag::Start, Tag::Cut, Tag::Jump, Tag::Same]);
    let m = measure(&fx.project, fx.seq(), 1, &MeasureOpts::default()).expect("measured");
    assert_eq!(m.jumps.len(), 1, "{:?}", m.jumps);
    let jump = m.jumps[0];
    assert_eq!((jump.position, jump.frame), (3, 200));
    assert!((jump.shift - 0.395_061_728_395_061_76).abs() < 1e-12, "{}", jump.shift);
    assert!((jump.scale - 1.2).abs() < 1e-12, "{}", jump.scale);
    assert!((jump.iou - 0.406_290_024_192_400_73).abs() < 1e-12, "{}", jump.iou);
    let breaks: Vec<(u32, i64, BreakKind)> = m.breaks.iter().map(|k| (k.position, k.frame, k.kind)).collect();
    assert_eq!(breaks, vec![(2, 100, BreakKind::Cut), (3, 200, BreakKind::Jump)], "a SAME position is no window break");
    // no finding without the caller's threshold
    assert!(with_code(&m.findings, "jump_similar").is_empty());
}

/// A plan with a cut at 100 and a 60-frame ease in-out move into frame 200 (it starts on frame 140), on a sequence
/// without edit walls.
fn moving() -> Fx {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 100, win(700.0, 540.0, 1080.0), Arrival::Cut), at25(2, 200, win(1100.0, 540.0, 1080.0), mv(60, Curve::EaseInOut))]));
    fx
}

fn add_wall(frame: i64, on_conflict: OnConflict) -> Write {
    Write::SetWalls(SetWalls { add: vec![WallAdd { frame, score: None }], on_conflict, ..Default::default() })
}

#[test]
fn w5_a_new_wall_inside_a_move_is_refused_by_default() {
    let fx = moving();
    let e = fx.refuse(add_wall(170, OnConflict::Refuse));
    assert_eq!(e.code, "would_split_move");
    assert_eq!((e.detail["plan"].clone(), e.detail["position"].clone(), e.detail["wall"].clone()), (json!(1), json!(2), json!(170)));
    // the move's start frame is not inside it
    let planned = accepted(fx.write(add_wall(140, OnConflict::Refuse)));
    assert!(planned.changed);
    let set = planned.set.expect("a set");
    assert_eq!(set.walls.stored, vec![stored(140, WallOrigin::Manual, None)], "a person's wall is manual");
    assert_eq!(set.plans[0].positions, fx.plan(1).positions, "no position changes");
}

#[test]
fn w5_shorten_starts_the_move_at_the_wall_flags_it_and_reports_it() {
    let mut fx = moving();
    let planned = fx.apply(add_wall(170, OnConflict::Shorten));
    let p = fx.position(1, 2);
    assert_eq!(p.arrival, mv(30, Curve::EaseInOut));
    assert_eq!(p.flags, vec![Flag::MoveShortened]);
    let shortened = changes_with(&planned, "move_shortened");
    assert_eq!(shortened.len(), 1);
    assert_eq!(shortened[0].detail, json!({"plan": 1, "position": 2, "frames": [60, 30], "wall": 170}));
    assert_eq!(planned.plans, vec![PlanDelta { plan: 1, added: vec![], updated: vec![2], removed: vec![], changed_frames: vec![[141, 200]] }]);
    assert_eq!(fx.set().walls.stored, vec![stored(170, WallOrigin::Manual, None)]);
    // a wall one frame before the arrival leaves a one-frame piece, a cut; a wall on the arrival makes it a cut too
    for wall in [199, 200] {
        let mut fx = moving();
        fx.apply(add_wall(wall, OnConflict::Shorten));
        let p = fx.position(1, 2);
        assert_eq!((p.arrival, p.flags.clone()), (Arrival::Cut, vec![Flag::MoveShortened]), "wall {wall}");
    }
}

#[test]
fn w5_a_locked_move_is_never_shortened() {
    let mut fx = moving();
    fx.plan_mut(1).positions[1].locked = true;
    let e = fx.refuse(add_wall(170, OnConflict::Shorten));
    assert_eq!(e.code, "locked");
    assert_eq!(e.detail["ids"], json!([2]));
    assert_eq!(fx.refuse(add_wall(170, OnConflict::Refuse)).code, "would_split_move");
}

#[test]
fn w7_window_changes_one_or_two_frames_off_a_wall_are_found_with_the_callers_threshold() {
    // edit walls at 100 and 249
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 149, 5_000), (249, 151, 9_000)]);
    fx.install(one_plan(vec![
        at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 102, win(1100.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 300, win(800.0, 540.0, 1080.0), mv(50, Curve::Linear)),
    ]));
    let near = |n: Option<i64>| {
        let opts = MeasureOpts { thresholds: Thresholds { near_wall_frames: n, ..Default::default() }, ..Default::default() };
        let m = measure(&fx.project, fx.seq(), 1, &opts).expect("measured");
        let mut found: Vec<serde_json::Value> = with_code(&m.findings, "near_wall_change").iter().map(|i| i.detail.clone()).collect();
        found.sort_by_key(|d| d["frame"].as_i64());
        found
    };
    assert_eq!(
        near(Some(2)),
        vec![json!({"position": 2, "frame": 102, "wall": 100, "distance": 2}), json!({"position": 3, "frame": 251, "wall": 249, "distance": 2})],
        "a jump 2 frames after a wall; a move's first moving frame 2 frames after one"
    );
    assert!(near(Some(1)).is_empty());
    assert!(near(None).is_empty(), "no threshold, no finding");
}

/// A detected wall at 100 (score 0.5) with an opener on it, a move starting on it into frame 150, and a cut at 300.
fn wall_plan() -> Fx {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(vec![
        at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 100, win(1100.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 150, win(800.0, 540.0, 1080.0), mv(50, Curve::Linear)),
        at25(4, 300, win(900.0, 540.0, 1080.0), Arrival::Cut),
    ]);
    set.walls.stored = vec![stored(100, WallOrigin::Detected, Some(0.5))];
    fx.install(set);
    fx
}

fn move_wall(from: i64, to: i64) -> Write {
    Write::SetWalls(SetWalls { moves: vec![WallMove { from, to }], ..Default::default() })
}

#[test]
fn w8_a_moved_wall_carries_its_positions_and_takes_the_movers_origin() {
    let mut fx = wall_plan();
    let planned = fx.apply(move_wall(100, 110));
    assert_eq!(fx.set().walls.stored, vec![stored(110, WallOrigin::Manual, Some(0.5))], "the mover's origin, the score kept");
    assert_eq!(fx.frames(1), vec![0, 110, 150, 300], "the opener moved with the wall");
    assert_eq!(fx.position(1, 3).arrival, mv(40, Curve::Linear), "the move starting on the wall starts at its new frame");
    assert_eq!(changes_with(&planned, "positions_carried")[0].detail, json!({"plan": 1, "positions": [2], "from": 100, "to": 110}));
    assert_eq!(changes_with(&planned, "move_start_moved")[0].detail, json!({"plan": 1, "position": 3, "frames": [50, 40]}));
    // a retried move is a no-op
    let again = accepted(fx.write(move_wall(100, 110)));
    assert!(!again.changed);
    assert_eq!(again.set.as_deref(), Some(fx.set()));
    // moving it back earlier lengthens the move again
    fx.apply(move_wall(110, 90));
    assert_eq!(fx.frames(1), vec![0, 90, 150, 300]);
    assert_eq!(fx.position(1, 3).arrival, mv(60, Curve::Linear));
}

#[test]
fn w8_refusals() {
    let fx = wall_plan();
    let e = fx.refuse(move_wall(100, 160));
    assert_eq!((e.code, e.detail["frames"].clone()), ("wall_blocked", json!([150])), "a position strictly between");
    let e = fx.refuse(move_wall(100, 149));
    assert_eq!(e.code, "move_too_short");
    assert_eq!(e.detail, json!({"position": 3, "frames": 1, "minimum": 2}));
    let e = fx.refuse(move_wall(101, 105));
    assert_eq!((e.code, e.detail["frame"].clone()), ("not_found", json!(101)), "no wall stands there");
    let mut locked = wall_plan();
    locked.plan_mut(1).positions[1].locked = true;
    let e = locked.refuse(move_wall(100, 110));
    assert_eq!((e.code, e.detail["ids"].clone()), ("locked", json!([2])), "carrying a locked position");
    let mut locked = wall_plan();
    locked.plan_mut(1).positions[2].locked = true;
    let e = locked.refuse(move_wall(100, 110));
    assert_eq!((e.code, e.detail["ids"].clone()), ("locked", json!([3])), "shortening a locked move");
    // an edit wall is the edit itself: it does not move
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 900, 5_000)]);
    fx.install(one_plan(Vec::new()));
    let e = fx.refuse(move_wall(100, 120));
    assert_eq!((e.code, e.detail["frame"].clone()), ("bad_wall", json!(100)));
}

#[test]
fn w8_a_wall_never_carries_a_position_onto_another_one() {
    // a detected wall at 100 with an opener on it, and a cut at 120
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(vec![
        at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 100, win(1100.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 120, win(800.0, 540.0, 1080.0), Arrival::Cut),
    ]);
    set.walls.stored = vec![stored(100, WallOrigin::Detected, Some(0.5))];
    fx.install(set);
    let e = fx.refuse(move_wall(100, 120));
    assert_eq!((e.code, e.detail["frames"].clone()), ("wall_blocked", json!([120])), "the opener would land on the cut at 120 (I1)");
    // with no position on the wall, it moves onto a position's frame, which then opens its shot
    fx.plan_mut(1).positions.remove(1);
    fx.apply(move_wall(100, 120));
    assert_eq!((fx.wall_frames(), fx.frames(1)), (vec![120], vec![0, 120]));
}

#[test]
fn w8_a_suppressed_wall_does_not_move_and_no_wall_moves_onto_a_suppressed_frame() {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(vec![at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut)]);
    set.walls.stored = vec![stored(100, WallOrigin::Detected, Some(0.5)), stored(300, WallOrigin::Manual, None)];
    set.walls.suppressed = vec![tk(100), tk(500)];
    fx.install(set);
    assert_eq!(fx.wall_frames(), vec![300]);
    let e = fx.refuse(move_wall(100, 150));
    assert_eq!((e.code, e.detail["frame"].clone()), ("not_found", json!(100)), "a detected wall a person deleted is no wall");
    let e = fx.refuse(move_wall(300, 500));
    assert_eq!((e.code, e.detail["frame"].clone()), ("bad_wall", json!(500)), "it would be suppressed there");
}

#[test]
fn adding_a_wall_on_a_suppressed_frame_makes_it_a_wall_again() {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(Vec::new());
    set.walls.stored = vec![stored(100, WallOrigin::Detected, Some(0.5))];
    fx.install(set);
    fx.apply(Write::SetWalls(SetWalls { remove: vec![100], ..Default::default() }));
    assert_eq!(fx.wall_frames(), Vec::<i64>::new(), "deleting a detected wall suppresses it");
    let planned = fx.apply(add_wall(100, OnConflict::Refuse));
    assert!(planned.changed, "Alt+B on it again brings it back");
    assert_eq!(fx.set().walls.stored, vec![stored(100, WallOrigin::Manual, Some(0.5))], "as the person's wall, so a new measurement keeps it");
    assert!(fx.set().walls.suppressed.is_empty());
    assert_eq!(fx.wall_frames(), vec![100]);
    assert!(!accepted(fx.write(add_wall(100, OnConflict::Refuse))).changed, "adding it again changes nothing");
    // a suppressed edit point is a wall again too
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 900, 5_000)]);
    let mut set = one_plan(Vec::new());
    set.walls.suppressed = vec![tk(100)];
    fx.install(set);
    assert_eq!(fx.wall_frames(), Vec::<i64>::new());
    fx.apply(add_wall(100, OnConflict::Refuse));
    assert_eq!(wall_at(&fx, 100).expect("a wall again").sources, vec![WallSource::Edit, WallSource::Manual]);
}

#[test]
fn a_stored_walls_origin_is_its_callers() {
    // a person's added and moved walls are manual (above); the planner takes the origin from the caller (W8)
    let fx = wall_plan();
    let agent = Caller { writer: Writer::Agent, overwrite_allowed: false };
    let added = plan_write(&fx.project, fx.seq(), &add_wall(500, OnConflict::Refuse), agent).expect("planned");
    assert!(added.set.expect("a set").walls.stored.contains(&stored(500, WallOrigin::Agent, None)));
    let moved = plan_write(&fx.project, fx.seq(), &move_wall(100, 110), agent).expect("planned");
    let set = moved.set.expect("a set");
    assert_eq!(set.walls.stored, vec![stored(110, WallOrigin::Agent, Some(0.5))], "the mover's origin");
    let mut after = fx.seq().clone();
    after.vertical = Some(set);
    assert!(!plan_write(&fx.project, &after, &move_wall(100, 110), agent).expect("planned").changed, "a retried move is a no-op");
}

#[test]
fn removing_a_detected_wall_suppresses_it_and_other_stored_walls_are_removed() {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(Vec::new());
    set.walls.stored = vec![stored(100, WallOrigin::Detected, Some(0.5)), stored(200, WallOrigin::Manual, None), stored(300, WallOrigin::Reanchored, None)];
    fx.install(set);
    let remove = |frames: Vec<i64>| Write::SetWalls(SetWalls { remove: frames, ..Default::default() });
    fx.apply(remove(vec![100, 200, 300]));
    assert_eq!(fx.set().walls.stored, vec![stored(100, WallOrigin::Detected, Some(0.5))], "the detected wall stays stored");
    assert_eq!(fx.set().walls.suppressed, vec![tk(100)], "and is suppressed, so a new measurement does not bring it back");
    assert_eq!(fx.wall_frames(), Vec::<i64>::new());
    let again = accepted(fx.write(remove(vec![100, 200, 300])));
    assert!(!again.changed, "removing again changes nothing");
}

#[test]
fn w9_a_candidate_inside_a_move_is_reported_and_promoting_it_makes_a_manual_wall() {
    let mut fx = moving();
    fx.set_mut().walls.candidates = vec![Candidate { at: tk(170), score: 0.19 }];
    let found = issues(&fx.project, fx.seq(), 1, 0, 1_000);
    let spans = with_code(&found, "move_spans_candidate");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].level, Level::Warning);
    assert_eq!(spans[0].detail["position"], json!(2));
    assert_eq!(spans[0].detail["frame"], json!(170));
    assert_eq!(spans[0].detail["score"].as_f64(), Some(f64::from(0.19_f32)));
    // promoting it is a new wall inside the move (W5); with shorten the move starts on it
    let promote = Write::SetWalls(SetWalls { promote: vec![170], on_conflict: OnConflict::Shorten, ..Default::default() });
    fx.apply(promote);
    assert_eq!(fx.set().walls.stored, vec![stored(170, WallOrigin::Manual, Some(0.19))], "a manual wall with the candidate's score");
    assert!(fx.set().walls.candidates.is_empty());
    assert_eq!(fx.position(1, 2).arrival, mv(30, Curve::EaseInOut));
    assert!(with_code(&issues(&fx.project, fx.seq(), 1, 0, 1_000), "move_spans_candidate").is_empty());
}

/// A random valid plan on a 2000-frame sequence without edit walls: positions 2 to 120 frames apart, each after the
/// first reached by a cut or by a move that fits its room.
fn random_plan(rng: &mut Rng, r: &Raster) -> Vec<Position> {
    let mut positions = Vec::new();
    let mut frame = rng.int(0, 20);
    let mut id = 1;
    while frame < 1_990 {
        let arrival = if id > 1 && rng.chance(0.6) {
            let room = frame - RATE.frame_at(positions.last().map_or(Tick(0), |p: &Position| p.at));
            mv(rng.int(2, room.max(2)) as u32, valid_curve(rng))
        } else {
            Arrival::Cut
        };
        positions.push(at25(id, frame, random_window(rng, r, OUT), arrival));
        id += 1;
        frame += rng.int(2, 120);
    }
    positions
}

/// A random curve that passes I7 (`random_curve` can draw a custom curve steeper than 12 times its mean speed).
fn valid_curve(rng: &mut Rng) -> Curve {
    for _ in 0..1_000 {
        let curve = random_curve(rng);
        if check_curve(&curve).is_ok() {
            return curve;
        }
    }
    Curve::Linear
}

/// The arrivals W5's shorten gives `positions` for the new walls `walls`: a move containing one starts at the latest of
/// them (a cut when fewer than 2 frames are left).
fn shortened(positions: &[Position], walls: &[i64]) -> Vec<(Arrival, Vec<Flag>)> {
    positions
        .iter()
        .map(|p| {
            let f = RATE.frame_at(p.at);
            match p.arrival {
                Arrival::Move { move_frames, curve, .. } => {
                    let s = f - i64::from(move_frames);
                    match walls.iter().copied().filter(|&c| s < c && c <= f).max() {
                        Some(c) if f - c >= 2 => (mv((f - c) as u32, curve), vec![Flag::MoveShortened]),
                        Some(_) => (Arrival::Cut, vec![Flag::MoveShortened]),
                        None => (p.arrival, Vec::new()),
                    }
                }
                Arrival::Cut => (p.arrival, Vec::new()),
            }
        })
        .collect()
}

/// Whether some move of `positions` contains one of `walls`.
fn splits_a_move(positions: &[Position], walls: &[i64]) -> bool {
    shortened(positions, walls).iter().zip(positions).any(|((arrival, _), p)| *arrival != p.arrival)
}

#[test]
fn t_m7_random_walls_and_parts_are_refused_or_shorten_moves_as_w5_says() {
    let mut rng = Rng::new(0x5eed_0007);
    for case in 0..60 {
        let mut fx = Fx::new(2_000);
        let r = fx.r();
        let positions = random_plan(&mut rng, &r);
        let mut set = one_plan(positions.clone());
        set.walls.use_edit_points = false;
        fx.install(set);
        let c = rng.int(1, 1_999);
        let (a, b) = {
            let x = rng.int(0, 1_999);
            (x, rng.int(x + 1, 2_000))
        };
        let part = |on_conflict: OnConflict| {
            let add = vec![PartAdd { name: "P01".into(), from_frame: a, to_frame: b, ..Default::default() }];
            Write::SetParts(SetParts { add, on_conflict, ..Default::default() })
        };
        let part_walls: Vec<i64> = [a, b].into_iter().filter(|&f| f > 0 && f < 2_000).collect();
        for (write, shorten, new_walls) in [
            (add_wall(c, OnConflict::Refuse), false, vec![c]),
            (add_wall(c, OnConflict::Shorten), true, vec![c]),
            (part(OnConflict::Refuse), false, part_walls.clone()),
            (part(OnConflict::Shorten), true, part_walls.clone()),
        ] {
            let result = fx.write(write);
            if !shorten && splits_a_move(&positions, &new_walls) {
                assert_eq!(refused(result).code, "would_split_move", "case {case}, walls {new_walls:?}");
                continue;
            }
            let planned = accepted(result);
            let set = planned.set.expect("a set");
            let got: Vec<(Arrival, Vec<Flag>)> = set.plans[0].positions.iter().map(|p| (p.arrival, p.flags.clone())).collect();
            assert_eq!(got, shortened(&positions, &new_walls), "case {case}, walls {new_walls:?}");
            // and no move of the result contains a wall
            let mut after = fx.seq().clone();
            after.vertical = Some(set);
            let walls: Vec<i64> = walls(&fx.project, &after).iter().map(|w| RATE.frame_at(w.at)).collect();
            for p in &after.vertical.as_deref().expect("a set").plans[0].positions {
                if let Arrival::Move { move_frames, .. } = p.arrival {
                    let f = RATE.frame_at(p.at);
                    let s = f - i64::from(move_frames);
                    assert!(!walls.iter().any(|&w| s < w && w <= f), "case {case}: position {} crosses a wall", p.id);
                }
            }
            assert!(validate(&fx.project, &after).is_empty(), "case {case}: the result is valid");
        }
    }
}
