//! The planner for a person (spec sections 3.6, 5.10 and 7.2-7.5). T-M8c: `replaceRange` with the plan's own read is a
//! no-op. T-M17 (the person's clauses): omitted keeps, null clears, unchanged positions keep everything, a direct edit of
//! one position marks it reviewed while bulk edits keep review states, a write to a position clears its flags. T-M23
//! (the planner's half): a move written without a curve is Ramp 20/20. And the position writes: upsert by frame, the exact
//! split inside a move (`split_conflict`; a split into a locked position is `locked`), retime, delete, swap, setArrival.

use serde_json::json;

use super::fixtures::*;
use super::support::{mv, mv_split, win};
use crate::Label;
use crate::vertical::*;

const A: Window = Window { cx: 700.0, cy: 540.0, h: 1080.0 };
const B: Window = Window { cx: 1100.0, cy: 540.0, h: 1080.0 };

/// A cut at 100 (window A) and a 60-frame ease in-out move into 200 (window B), which starts on frame 140; 1000 frames,
/// no edit wall.
fn two() -> Fx {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 100, A, Arrival::Cut), at25(2, 200, B, mv(60, Curve::EaseInOut))]));
    fx
}

/// The keys a read of `p` returns, as a `replaceRange` item.
fn read_back(p: &Position) -> ReplaceItem {
    let (arrival, move_frames, curve) = match p.arrival {
        Arrival::Cut => (ArrivalKind::Cut, None, None),
        Arrival::Move { move_frames, curve, .. } => (ArrivalKind::Move, Some(i64::from(move_frames)), Some(curve)),
    };
    let fields = PositionFields {
        cx: Some(p.window.cx),
        cy: Some(p.window.cy),
        h: Some(p.window.h),
        layout: Some(p.layout),
        arrival: Some(arrival),
        move_frames,
        curve,
        caption_anchor: Some(p.caption_anchor),
        label: Some(p.label.clone()),
        color: Some(p.color),
        tags: Some(p.tags.clone()),
        note: Some(p.note.clone()),
        focus: Some(p.focus),
        ..Default::default()
    };
    ReplaceItem { frame: RATE.frame_at(p.at), fields }
}

/// A plan using every position field a read returns: metadata, split pieces, a fit position and an agent's position.
fn rich() -> Fx {
    let ramp = DEFAULT_CURVE;
    let opener = Position {
        label: "Dana".into(),
        tags: vec!["speaker:Dana".into()],
        note: "opener".into(),
        color: Some(Label::Rose),
        caption_anchor: Some(CaptionAnchor::Top),
        focus: Some([700.0, 400.5]),
        ..at25(1, 100, win(700.25, 540.0, 1080.0), Arrival::Cut)
    };
    let walk = Position { label: "Dana".into(), ..at25(2, 200, win(1100.5, 520.25, 1000.0), mv(60, Curve::EaseInOut)) };
    let agent = Position {
        origin: Origin::Agent,
        modified_by: Writer::Agent,
        reviewed: true,
        flags: vec![Flag::PictureChanged],
        ..at25(6, 500, win(900.0, 540.0, 1080.0), Arrival::Cut)
    };
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![
        opener,
        walk,
        at25(3, 260, win(1000.0, 500.0, 900.0), mv_split(20, ramp, 0, 50)),
        at25(4, 290, win(800.0, 560.0, 950.0), mv_split(30, ramp, 20, 50)),
        Position { layout: Layout::Fit, ..at25(5, 400, win(960.0, 540.0, 1080.0), Arrival::Cut) },
        agent,
    ]));
    fx
}

fn replace(from: i64, to: i64, items: Vec<ReplaceItem>) -> Write {
    Write::ReplaceRange(ReplaceRange { plan: 1, from_frame: from, to_frame: to, items, ..Default::default() })
}

#[test]
fn t_m8c_replace_range_with_the_plans_own_read_is_a_no_op() {
    let fx = rich();
    let before = fx.set().clone();
    let items: Vec<ReplaceItem> = fx.plan(1).positions.iter().map(read_back).collect();
    let planned = accepted(fx.write(replace(0, 1_000, items)));
    assert!(!planned.changed, "a restatement changes nothing");
    assert_eq!(planned.set.as_deref(), Some(&before), "split pieces keep their splitOf; metadata and states stay");
    assert!(planned.added.is_empty() && planned.updated.is_empty() && planned.removed.is_empty());
    assert!(planned.changed_frames.is_empty());
    assert_eq!(planned.label, "");
    // a sub-range restated alone is a no-op too
    let items: Vec<ReplaceItem> = fx.plan(1).positions.iter().filter(|p| (150..300).contains(&RATE.frame_at(p.at))).map(read_back).collect();
    assert_eq!(items.len(), 3);
    assert!(!accepted(fx.write(replace(150, 300, items))).changed);
}

#[test]
fn t_m17_omitted_metadata_keeps_its_value_and_explicit_null_clears_it() {
    let mut fx = rich();
    let before = fx.position(1, 1).clone();
    fx.apply(set_at(100, PositionFields { cx: Some(710.0), ..Default::default() }));
    let p = fx.position(1, 1);
    assert_eq!(p.window, Window { cx: 710.0, ..before.window });
    assert_eq!((p.id, &p.label, &p.tags, &p.note), (1, &before.label, &before.tags, &before.note));
    assert_eq!((p.color, p.caption_anchor, p.focus), (before.color, before.caption_anchor, before.focus));
    let cleared = PositionFields {
        label: Some(String::new()),
        color: Some(None),
        tags: Some(Vec::new()),
        note: Some(String::new()),
        caption_anchor: Some(None),
        focus: Some(None),
        ..Default::default()
    };
    fx.apply(set_at(100, cleared));
    let p = fx.position(1, 1);
    assert_eq!((p.label.as_str(), p.tags.len(), p.note.as_str()), ("", 0, ""));
    assert_eq!((p.color, p.caption_anchor, p.focus), (None, None, None));
    assert_eq!(p.window.cx, 710.0, "the window is not metadata");
}

#[test]
fn t_m17_unchanged_positions_keep_everything_and_a_changed_one_follows_the_change() {
    let fx = rich();
    let mut items: Vec<ReplaceItem> = fx.plan(1).positions.iter().map(read_back).collect();
    // change only the walk's window
    if let Some(walk) = items.iter_mut().find(|i| i.frame == 200) {
        walk.fields.cx = Some(1105.0);
    }
    let mut set = accepted(fx.write(replace(0, 1_000, items))).set.expect("a set");
    let after = std::mem::take(&mut set.plans[0].positions);
    let before = &fx.plan(1).positions;
    for (a, b) in after.iter().zip(before) {
        if a.id == 2 {
            continue;
        }
        assert_eq!(a, b, "position {} is unchanged and keeps everything", a.id);
    }
    let agent = after.iter().find(|p| p.id == 6).expect("the agent's position");
    assert_eq!((agent.origin, agent.modified_by, agent.reviewed, agent.flags.clone()), (Origin::Agent, Writer::Agent, true, vec![Flag::PictureChanged]));
    let walk = after.iter().find(|p| p.id == 2).expect("the walk");
    assert_eq!(walk.window.cx, 1105.0);
    assert_eq!((walk.origin, walk.modified_by), (Origin::Manual, Writer::Human));
    assert!(walk.reviewed, "a person's write that changes exactly one position marks it reviewed");
}

#[test]
fn t_m17_a_write_to_a_position_clears_its_flags() {
    let mut fx = two();
    fx.plan_mut(1).positions[1].flags = vec![Flag::MoveShortened, Flag::PictureChanged];
    fx.plan_mut(1).positions[0].flags = vec![Flag::TimeApproximate];
    fx.apply(set_at(200, PositionFields { cx: Some(1_000.0), ..Default::default() }));
    assert!(fx.position(1, 2).flags.is_empty(), "flags clear on the next write to the position");
    assert_eq!(fx.position(1, 1).flags, vec![Flag::TimeApproximate], "another position keeps its flags");
}

#[test]
fn t_m17_bulk_edits_keep_review_states() {
    let mut fx = two();
    fx.plan_mut(1).positions[0].reviewed = true;
    // two positions change: both keep their states
    let mut walk = read_back(fx.position(1, 2));
    walk.fields.cx = Some(1090.0);
    fx.apply(replace(0, 1_000, vec![ReplaceItem { frame: 100, fields: keys(710.0, 540.0, 1080.0) }, walk]));
    assert_eq!((fx.position(1, 1).reviewed, fx.position(1, 2).reviewed), (true, false));
    // setArrival changing two positions keeps them too
    let arrival = Write::SetArrival(SetArrival { plan: 1, positions: vec![1, 2], arrival: ArrivalKind::Move, move_frames: Some(30), ..Default::default() });
    fx.apply(arrival);
    assert_eq!((fx.position(1, 1).reviewed, fx.position(1, 2).reviewed), (true, false));
    // a direct edit of the unreviewed one marks it
    fx.apply(set_at(200, PositionFields { cx: Some(1000.0), ..Default::default() }));
    assert!(fx.position(1, 2).reviewed);
}

#[test]
fn t_m17_a_restatement_is_a_no_op_even_for_a_locked_position() {
    let mut fx = two();
    fx.plan_mut(1).positions[1].locked = true;
    let same = PositionFields { arrival: Some(ArrivalKind::Move), move_frames: Some(60), curve: Some(Curve::EaseInOut), ..keys(1100.0, 540.0, 1080.0) };
    let planned = accepted(fx.write(set_at(200, same)));
    assert!(!planned.changed);
    let e = fx.refuse(set_at(200, PositionFields { cx: Some(1000.0), ..Default::default() }));
    assert_eq!((e.code, e.detail["ids"].clone()), ("locked", json!([2])));
}

#[test]
fn t_m23_a_move_written_without_a_curve_is_ramp_20_20() {
    let mut fx = two();
    // a new move
    fx.apply(set_at(400, PositionFields { arrival: Some(ArrivalKind::Move), move_frames: Some(40), ..keys(900.0, 540.0, 1080.0) }));
    assert_eq!(fx.position(1, 3).arrival, mv(40, Curve::Ramp { r_in: 0.2, r_out: 0.2 }));
    // a cut that becomes a move
    let arrival = Write::SetArrival(SetArrival { plan: 1, positions: vec![1], arrival: ArrivalKind::Move, move_frames: Some(30), ..Default::default() });
    fx.apply(arrival);
    assert_eq!(fx.position(1, 1).arrival, mv(30, DEFAULT_CURVE));
    // an existing move given a new length keeps its curve
    let arrival = Write::SetArrival(SetArrival { plan: 1, positions: vec![2], arrival: ArrivalKind::Move, move_frames: Some(40), ..Default::default() });
    fx.apply(arrival);
    assert_eq!(fx.position(1, 2).arrival, mv(40, Curve::EaseInOut));
    // a new position of replaceRange
    let mut items: Vec<ReplaceItem> = fx.plan(1).positions.iter().map(read_back).collect();
    items.push(ReplaceItem { frame: 700, fields: PositionFields { arrival: Some(ArrivalKind::Move), move_frames: Some(50), ..keys(1200.0, 540.0, 1080.0) } });
    fx.apply(replace(0, 1_000, items));
    let last = fx.plan(1).positions.last().expect("a position").clone();
    assert_eq!((RATE.frame_at(last.at), last.arrival), (700, mv(50, DEFAULT_CURVE)));
}

#[test]
fn set_position_is_an_upsert_by_frame() {
    let mut fx = two();
    let planned = fx.apply(set_at(400, keys(900.0, 540.0, 1080.0)));
    assert_eq!((planned.plan, planned.added.clone(), planned.updated.clone()), (Some(1), vec![3], vec![]));
    assert_eq!(planned.label, "Vertical: new position (#3)");
    assert_eq!(planned.changed_frames, vec![[400, 1_000]]);
    let p = fx.position(1, 3).clone();
    let want = (win(900.0, 540.0, 1080.0), Layout::Fill, Arrival::Cut, Origin::Manual, Writer::Human);
    assert_eq!((p.window, p.layout, p.arrival, p.origin, p.modified_by), want);
    assert!(p.reviewed, "the one position a person's write changes");
    let planned = fx.apply(set_at(400, PositionFields { cx: Some(950.0), ..Default::default() }));
    assert_eq!((planned.added.clone(), planned.updated.clone()), (vec![], vec![3]));
    assert_eq!(fx.position(1, 3).window, win(950.0, 540.0, 1080.0), "omitted components keep the window shown");
    assert_eq!(fx.plan(1).positions.len(), 3);
    assert_eq!(fx.plan(1).next_position_id, 4, "ids come from the plan's counter");
}

#[test]
fn a_new_position_in_a_hold_or_at_a_moves_start_takes_the_window_shown() {
    let mut fx = two();
    let planned = fx.apply(set_at(500, PositionFields::default()));
    assert!(planned.changed);
    assert!(planned.changed_frames.is_empty(), "nothing visible changes");
    assert_eq!(fx.position(1, 3).window, B);
    let mut fx = two();
    fx.apply(set_at(140, PositionFields::default()));
    assert_eq!((fx.frames(1), fx.position(1, 3).window), (vec![100, 140, 200], A), "a split at s itself is a SAME position");
    assert_eq!(fx.position(1, 2).arrival, mv(60, Curve::EaseInOut), "the move needs no split");
    let tags: Vec<Tag> = derive_plan(&fx.project, fx.seq(), 1).expect("the plan").positions.iter().map(|p| p.tag).collect();
    assert_eq!(tags, vec![Tag::Start, Tag::Same, Tag::Move]);
}

#[test]
fn inside_a_move_set_position_makes_the_exact_split_first() {
    let mut fx = two();
    let split = split_move(fx.plan(1), &fx.r(), 170).expect("170 is inside the move");
    let planned = fx.apply(set_at(170, PositionFields::default()));
    assert_eq!(planned.label, "Vertical: waypoint (#3)");
    assert_eq!((planned.added.clone(), planned.updated.clone()), (vec![3], vec![2]));
    assert_eq!(fx.frames(1), vec![100, 170, 200]);
    let waypoint = fx.position(1, 3);
    assert_eq!((waypoint.window, waypoint.arrival), (split.window, split.arrival));
    assert_eq!(split.arrival, mv_split(30, Curve::EaseInOut, 0, 60));
    assert!(waypoint.reviewed, "the waypoint is the one position the person edits");
    assert!(!fx.position(1, 2).reviewed, "the arriving piece changes only as a by-product and keeps its state");
    assert_eq!(fx.position(1, 2).arrival, mv_split(30, Curve::EaseInOut, 30, 60));
    // with a window, the split happens first and the new position takes the window; both halves keep their splits
    let mut fx = two();
    fx.apply(set_at(170, keys(900.0, 500.0, 1000.0)));
    assert_eq!((fx.position(1, 3).window, fx.position(1, 3).arrival), (win(900.0, 500.0, 1000.0), mv_split(30, Curve::EaseInOut, 0, 60)));
    assert_eq!(fx.position(1, 2).arrival, mv_split(30, Curve::EaseInOut, 30, 60));
}

#[test]
fn a_given_arrival_that_disagrees_with_the_split_is_split_conflict() {
    let fx = two();
    let e = fx.refuse(set_at(170, PositionFields { arrival: Some(ArrivalKind::Cut), ..Default::default() }));
    assert_eq!(e.code, "split_conflict");
    assert_eq!(e.detail["arrival"], json!({"kind": "move", "moveFrames": 30, "curve": {"type": "easeInOut"}, "splitOf": {"offset": 0, "total": 60}}));
    assert_eq!(fx.refuse(set_at(170, move_keys(20, None))).code, "split_conflict", "another length");
    assert_eq!(fx.refuse(set_at(170, move_keys(30, Some(Curve::Linear)))).code, "split_conflict", "another curve");
    assert!(accepted(fx.write(set_at(170, move_keys(30, Some(Curve::EaseInOut))))).changed, "the split's own arrival agrees");
}

#[test]
fn a_split_into_a_locked_position_is_refused_locked() {
    let mut fx = two();
    fx.plan_mut(1).positions[1].locked = true;
    let e = fx.refuse(set_at(170, PositionFields::default()));
    assert_eq!((e.code, e.detail["ids"].clone()), ("locked", json!([2])));
    let e = fx.refuse(set_at(170, keys(900.0, 540.0, 1080.0)));
    assert_eq!(e.code, "locked");
}

#[test]
fn clamp_moves_a_window_inward_and_reports_it_while_without_clamp_it_is_outside_picture() {
    let fx = two();
    let e = fx.refuse(set_at(400, keys(2_000.0, 540.0, 1080.0)));
    assert_eq!(e.code, "outside_picture");
    assert_eq!(e.detail["cx"], json!([303.75, 1616.25]), "the legal centres at that height, on the grid");
    assert_eq!(e.detail["cy"], json!([540.0, 540.0]));
    assert_eq!(e.detail["frame"], json!(400));
    let clamped = Write::SetPosition(SetPosition { plan: 1, frame: 400, fields: keys(2_000.0, 540.0, 1080.0), clamp: true, ..Default::default() });
    let planned = accepted(fx.write(clamped));
    let set = planned.set.as_deref().expect("a set");
    let p = set.plans[0].positions.iter().find(|p| p.id == 3).expect("the new position");
    assert_eq!(p.window, win(1616.25, 540.0, 1080.0));
    assert_eq!(planned.clamped, vec![Clamped { position: 3, dx: 1616.25 - 2_000.0, dy: 0.0, dh: 0.0 }]);
    // off-grid values are stored as q(v), never refused for it
    let planned = accepted(fx.write(set_at(400, keys(900.0004, 540.0006, 1000.0001))));
    let set = planned.set.as_deref().expect("a set");
    assert_eq!(set.plans[0].positions[2].window, win(900.0, 540.001, 1000.0));
}

#[test]
fn zoom_and_from_position_give_the_window() {
    let mut fx = two();
    fx.apply(set_at(400, PositionFields { cx: Some(900.0), cy: Some(540.0), zoom: Some(125.0), ..Default::default() }));
    assert_eq!(fx.position(1, 3).window, win(900.0, 540.0, 864.0), "h = q(1080 * 100 / 125)");
    let copy = Write::SetPosition(SetPosition { plan: 1, frame: 600, from_position: Some(2), ..Default::default() });
    fx.apply(copy);
    assert_eq!(fx.position(1, 4).window, B);
    let e = fx.refuse(Write::SetPosition(SetPosition { plan: 1, frame: 650, from_position: Some(99), ..Default::default() }));
    assert_eq!((e.code, e.detail["position"].clone()), ("not_found", json!(99)));
}

#[test]
fn retime_keeps_the_id_window_and_move_length_and_refuses_with_the_allowed_range() {
    let mut fx = two();
    let retime = |frame: i64| Write::RetimePosition(RetimePosition { plan: 1, position: 2, frame, ..Default::default() });
    let e = fx.refuse(retime(90));
    assert_eq!(e.code, "would_reorder");
    assert_eq!((e.detail["position"].clone(), e.detail["frame"].clone(), e.detail["allowed"].clone()), (json!(2), json!(90), json!([101, 999])));
    let e = fx.refuse(retime(150));
    assert_eq!(e.code, "move_overlaps");
    assert_eq!((e.detail["position"].clone(), e.detail["earliestStart"].clone(), e.detail["maxMoveFrames"].clone()), (json!(2), json!(100), json!(50)));
    let planned = fx.apply(retime(220));
    assert_eq!(planned.updated, vec![2]);
    assert_eq!(fx.frames(1), vec![100, 220]);
    let p = fx.position(1, 2);
    assert_eq!((p.window, p.arrival), (B, mv(60, Curve::EaseInOut)));
    // a stored wall at 230 is inside the move a retime to 240 would make (it would start on 180)
    fx.set_mut().walls.stored = vec![stored(230, WallOrigin::Manual, None)];
    let e = fx.refuse(retime(240));
    assert_eq!(e.code, "move_crosses_wall");
    assert_eq!((e.detail["wall"].clone(), e.detail["sources"].clone(), e.detail["maxMoveFrames"].clone()), (json!(230), json!(["manual"]), json!(10)));
    // retiming the end of a split piece makes it a whole move
    let mut fx = two();
    fx.apply(set_at(170, PositionFields::default()));
    fx.apply(Write::RetimePosition(RetimePosition { plan: 1, position: 3, frame: 165, ..Default::default() }));
    assert_eq!(fx.position(1, 3).arrival, mv(30, Curve::EaseInOut));
    assert_eq!(fx.frames(1), vec![100, 165, 200]);
    // a locked position does not move
    let mut fx = two();
    fx.plan_mut(1).positions[1].locked = true;
    assert_eq!(fx.refuse(retime(220)).code, "locked");
}

#[test]
fn retiming_a_split_piece_marks_it_reviewed_and_the_next_piece_keeps_its_state() {
    let mut fx = Fx::new(1_000);
    let mid = win(900.0, 540.0, 1080.0);
    fx.install(one_plan(vec![
        at25(1, 100, A, Arrival::Cut),
        at25(2, 150, mid, mv_split(30, Curve::EaseInOut, 0, 60)),
        at25(3, 180, B, mv_split(30, Curve::EaseInOut, 30, 60)),
    ]));
    let planned = fx.apply(Write::RetimePosition(RetimePosition { plan: 1, position: 2, frame: 145, ..Default::default() }));
    assert_eq!(planned.updated, vec![2, 3]);
    assert_eq!((fx.position(1, 2).arrival, fx.position(1, 3).arrival), (mv(30, Curve::EaseInOut), mv(30, Curve::EaseInOut)), "both whole moves");
    assert!(fx.position(1, 2).reviewed, "the retimed position is the one the person edits");
    assert!(!fx.position(1, 3).reviewed, "the next piece became a whole move as a by-product");
}

#[test]
fn delete_positions_lists_the_missing_ids_and_refuses_locked_ones() {
    let mut fx = two();
    let delete = |ids: Vec<u32>| Write::DeletePositions(DeletePositions { plan: 1, positions: ids, ..Default::default() });
    let planned = fx.apply(delete(vec![2, 99]));
    assert_eq!((planned.removed.clone(), planned.missing.clone()), (vec![2], vec![99]));
    assert_eq!(fx.frames(1), vec![100]);
    let again = accepted(fx.write(delete(vec![2])));
    assert!(!again.changed, "deleting what is gone changes nothing");
    assert_eq!(again.missing, vec![2]);
    let mut fx = two();
    fx.plan_mut(1).positions[0].locked = true;
    let e = fx.refuse(delete(vec![1]));
    assert_eq!((e.code, e.detail["ids"].clone()), ("locked", json!([1])));
}

#[test]
fn deleting_the_position_between_a_fit_and_a_move_is_layout_move() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![
        Position { layout: Layout::Fit, ..at25(1, 100, A, Arrival::Cut) },
        at25(2, 150, A, Arrival::Cut),
        at25(3, 200, B, mv(30, Curve::Linear)),
    ]));
    let e = fx.refuse(Write::DeletePositions(DeletePositions { plan: 1, positions: vec![2], ..Default::default() }));
    assert_eq!((e.code, e.detail["ids"].clone()), ("layout_move", json!([3])), "the move would depart from a fit position");
}

#[test]
fn swap_windows_exchanges_windows_and_layouts_and_keeps_times_and_arrivals() {
    let mut fx = two();
    let swap = |ids: Vec<u32>| Write::SwapWindows(SwapWindows { plan: 1, positions: ids, ..Default::default() });
    fx.apply(swap(vec![1, 2]));
    let (p1, p2) = (fx.position(1, 1).clone(), fx.position(1, 2).clone());
    assert_eq!((p1.window, p1.arrival, RATE.frame_at(p1.at)), (B, Arrival::Cut, 100));
    assert_eq!((p2.window, p2.arrival, RATE.frame_at(p2.at)), (A, mv(60, Curve::EaseInOut), 200));
    let e = fx.refuse(swap(vec![1]));
    assert_eq!((e.code, e.detail["count"].clone()), ("out_of_range", json!(1)));
    let mut fx = two();
    fx.plan_mut(1).positions.push(Position { layout: Layout::Fit, ..at25(3, 300, A, Arrival::Cut) });
    fx.plan_mut(1).next_position_id = 4;
    let e = fx.refuse(swap(vec![2, 3]));
    assert_eq!((e.code, e.detail["ids"].clone()), ("layout_move", json!([2])), "the move would arrive at a fit position");
}

#[test]
fn set_arrival_checks_the_room_and_a_move_on_the_first_position_is_reported_ignored() {
    let mut fx = two();
    let arrival = |ids: Vec<u32>, kind: ArrivalKind, frames: Option<i64>| {
        Write::SetArrival(SetArrival { plan: 1, positions: ids, arrival: kind, move_frames: frames, ..Default::default() })
    };
    let e = fx.refuse(arrival(vec![2], ArrivalKind::Move, Some(150)));
    assert_eq!(e.code, "move_overlaps");
    assert_eq!((e.detail["position"].clone(), e.detail["earliestStart"].clone(), e.detail["maxMoveFrames"].clone()), (json!(2), json!(100), json!(100)));
    let planned = fx.apply(arrival(vec![2], ArrivalKind::Cut, None));
    assert_eq!(fx.position(1, 2).arrival, Arrival::Cut);
    assert_eq!(planned.changed_frames, vec![[141, 200]]);
    let planned = fx.apply(arrival(vec![1], ArrivalKind::Move, Some(30)));
    assert_eq!(fx.position(1, 1).arrival, mv(30, DEFAULT_CURVE));
    assert!(planned.changed_frames.is_empty(), "a move on the first position has no effect");
    let ignored = with_code(&planned.issues, "arrival_ignored");
    assert_eq!(ignored.len(), 1);
    assert_eq!((ignored[0].level, ignored[0].detail["position"].clone()), (Level::Info, json!(1)));
    assert_eq!(fx.refuse(arrival(vec![7], ArrivalKind::Cut, None)).code, "not_found");
}

#[test]
fn the_label_of_an_arrival_names_the_curve_and_the_length() {
    let fx = two();
    let arrival = Write::SetArrival(SetArrival {
        plan: 1,
        positions: vec![2],
        arrival: ArrivalKind::Move,
        move_frames: Some(50),
        curve: Some(Curve::EaseInOut),
        ..Default::default()
    });
    assert_eq!(accepted(fx.write(arrival)).label, "Vertical: arrival ease in-out 50 f (#2)");
}
