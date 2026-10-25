//! T-M9 (spec sections 7.1 and 7.6): every refusal code `plan_write` returns in M2, each from a minimal case and with its
//! `detail`; and hostile values (NaN, ±infinity, 1e309, negative and huge frames, huge counts, 10^6-item arrays) refused
//! under `catch_unwind` with no panic. The engine's own codes (`use_frame`, unknown keys, `stale_measurement`) are lane
//! L2b's; `bad_time` comes from the planner's helper `frame_of_time`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use filmcraft_time::Tick;
use serde_json::{Value, json};

use super::fixtures::*;
use super::support::mv;
use crate::vertical::*;

const A: Window = Window { cx: 700.0, cy: 540.0, h: 1080.0 };
const B: Window = Window { cx: 1100.0, cy: 540.0, h: 1080.0 };

/// A cut at 100 and a 60-frame ease in-out move into 200 (from frame 140) on a 1000-frame sequence without edit walls.
fn base() -> Fx {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 100, A, Arrival::Cut), at25(2, 200, B, mv(60, Curve::EaseInOut))]));
    fx
}

/// The refusal of `w` on `fx`, which must be `code`; its detail.
fn code(fx: &Fx, w: Write, code: &str) -> Value {
    let e = fx.refuse(w);
    assert_eq!(e.code, code, "{}: {}", e.message, e.detail);
    assert!(!e.message.is_empty(), "{code} carries a message");
    e.detail
}

const ZERO: &str = "fnv1a64:0000000000000000";

#[test]
fn t_m9_values_and_time() {
    let fx = base();
    let d = code(&fx, Write::SetPosition(SetPosition { plan: 9, frame: 300, ..Default::default() }), "not_found");
    assert_eq!(d["plan"], json!(9));
    let d = code(&fx, set_at(300, PositionFields { cx: Some(f64::NAN), ..Default::default() }), "bad_number");
    assert_eq!(d["key"], json!("cx"));
    for frame in [-1, 1_000] {
        let d = code(&fx, set_at(frame, PositionFields::default()), "out_of_range");
        assert_eq!((d["key"].clone(), d["value"].clone()), (json!("frame"), json!(frame)), "frame {frame}: the sequence has frames 0 to 999");
    }
    assert_eq!(frame_of_time(RATE, tk(10)).ok(), Some(10));
    for (t, floor, nearest) in [(Tick(tk(10).0 + 5), 10, 10), (Tick(tk(11).0 - 5), 10, 11)] {
        let e = match frame_of_time(RATE, t) {
            Ok(f) => panic!("tick {} is not a frame start, read as {f}", t.0),
            Err(e) => e,
        };
        assert_eq!(e.code, "bad_time");
        assert_eq!((e.detail["floorFrame"].clone(), e.detail["nearestFrame"].clone()), (json!(floor), json!(nearest)));
    }
}

#[test]
fn t_m9_geometry_and_size() {
    let fx = base();
    let d = code(&fx, set_at(300, keys(2_000.0, 540.0, 1080.0)), "outside_picture");
    assert_eq!((d["frame"].clone(), d["cx"].clone(), d["cy"].clone()), (json!(300), json!([303.75, 1616.25]), json!([540.0, 540.0])));
    let d = code(&fx, set_at(300, keys(900.0, 540.0, 400.0)), "size_limit");
    assert_eq!((d["minHeight"].clone(), d["maxHeight"].clone()), (json!(480.0), json!(1080.0)));
    let d = code(&fx, Write::Update(UpdatePlan { plan: 1, width: Some(1080), height: Some(1080), ..Default::default() }), "aspect_change");
    assert_eq!((d["width"].clone(), d["height"].clone()), (json!(1080), json!(1080)));
    let d = code(&fx, Write::Create(CreatePlan { width: Some(1081), height: Some(1920), ..Default::default() }), "invalid_output_size");
    assert_eq!((d["width"].clone(), d["height"].clone()), (json!(1081), json!(1920)));
    let d = code(&fx, Write::Create(CreatePlan { width: Some(1080), height: Some(4400), ..Default::default() }), "size_limit");
    assert_eq!((d["minHeight"].clone(), d["maxHeight"].clone()), (json!(1100.0), json!(1080.0)), "more than 4x upscale");
    let mut square = Fx::new(100);
    square.seq_mut().settings.par = (4, 3);
    let d = code(&square, Write::Create(CreatePlan::default()), "unsupported_pixel_aspect");
    assert_eq!(d["par"], json!([4, 3]));
}

#[test]
fn t_m9_plans_and_positions() {
    let mut fx = base();
    let d = code(&fx, Write::Create(CreatePlan { name: Some("9:16".into()), ..Default::default() }), "plan_exists");
    assert_eq!(d["id"], json!(1));
    let twice = vec![ReplaceItem { frame: 300, fields: keys(900.0, 540.0, 1080.0) }, ReplaceItem { frame: 300, fields: keys(950.0, 540.0, 1080.0) }];
    let d = code(&fx, Write::ReplaceRange(ReplaceRange { plan: 1, from_frame: 250, to_frame: 400, items: twice, ..Default::default() }), "position_exists");
    assert_eq!(d["frame"], json!(300));
    let d = code(&fx, Write::RetimePosition(RetimePosition { plan: 1, position: 2, frame: 50, ..Default::default() }), "would_reorder");
    assert_eq!((d["position"].clone(), d["allowed"].clone()), (json!(2), json!([101, 999])));
    let d = code(&fx, set_at(300, PositionFields { move_frames: Some(1), ..keys(900.0, 540.0, 1080.0) }), "move_too_short");
    assert_eq!((d["frames"].clone(), d["minimum"].clone()), (json!(1), json!(2)));
    let long = Write::SetArrival(SetArrival { plan: 1, positions: vec![2], arrival: ArrivalKind::Move, move_frames: Some(150), ..Default::default() });
    let d = code(&fx, long, "move_overlaps");
    assert_eq!((d["position"].clone(), d["earliestStart"].clone(), d["maxMoveFrames"].clone()), (json!(2), json!(100), json!(100)));
    let d = code(&fx, set_at(170, PositionFields { arrival: Some(ArrivalKind::Cut), ..Default::default() }), "split_conflict");
    assert_eq!(d["arrival"]["splitOf"], json!({"offset": 0, "total": 60}));
    let snap = Curve::Bezier { x1: 1.0, y1: 0.0, x2: 0.0, y2: 1.0 };
    let d = code(&fx, set_at(300, PositionFields { curve: Some(snap), ..move_keys(40, None) }), "bad_curve");
    assert_eq!(d["axis"], json!("slope"));
    let d = code(&fx, set_at(200, PositionFields { layout: Some(Layout::Fit), ..Default::default() }), "layout_move");
    assert_eq!(d["ids"], json!([2]));
    let d = code(&fx, Write::SetPosition(SetPosition { plan: 1, frame: 300, from_label: Some("Nobody".into()), ..Default::default() }), "label_not_found");
    assert_eq!(d["label"], json!("Nobody"));
    let slow = Write::FitMove(FitMove { plan: 1, position: 2, max_speed: 0.5, ..Default::default() });
    let d = code(&fx, slow, "speed_limit_unmet");
    assert_eq!((d["position"].clone(), d["maxMoveFrames"].clone(), d["alternative"].clone()), (json!(2), json!(100), json!({"arrival": "cut"})));
    assert!(d["peak"].as_f64().is_some_and(|p| p > 0.5), "the peak at the longest move: {}", d["peak"]);
    // a wall the current move does not cross (at 250) is inside a new move into 300 of 60 frames
    fx.set_mut().walls.stored = vec![stored(250, WallOrigin::Manual, None)];
    let into = PositionFields { arrival: Some(ArrivalKind::Move), move_frames: Some(60), ..keys(900.0, 540.0, 1080.0) };
    let d = code(&fx, set_at(300, into), "move_crosses_wall");
    assert_eq!((d["wall"].clone(), d["sources"].clone(), d["maxMoveFrames"].clone()), (json!(250), json!(["manual"]), json!(50)));
    let d = code(&fx, Write::SetWalls(SetWalls { add: vec![WallAdd { frame: 170, score: None }], ..Default::default() }), "would_split_move");
    assert_eq!((d["plan"].clone(), d["position"].clone(), d["wall"].clone()), (json!(1), json!(2), json!(170)));
}

#[test]
fn t_m9_peoples_work() {
    let mut fx = base();
    fx.plan_mut(1).positions[1].locked = true;
    let d = code(&fx, set_at(200, PositionFields { cx: Some(1_000.0), ..Default::default() }), "locked");
    assert_eq!(d["ids"], json!([2]));
    let mut fx = base();
    fx.set_mut().parts = vec![part25(1, "P01", 100, 300)];
    let approve = Write::ApprovePart(ApprovePart { plan: 1, part: 1, note: String::new(), by: "Dana Editor".into(), when: 1_790_000_000 });
    let d = code(&fx, approve, "not_reviewed");
    assert_eq!(d["shots"], json!([2]), "shot 2 ([100, 300)) holds unreviewed positions");
}

#[test]
fn t_m9_concurrency_and_counts() {
    let fx = base();
    let current = plan_digest(fx.seq(), fx.plan(1));
    let d = code(&fx, Write::SetPosition(SetPosition { plan: 1, frame: 300, expect: Some(ZERO.into()), ..Default::default() }), "stale_plan");
    assert_eq!(d["digest"], json!(current));
    let guard = RangeGuard { from_frame: 0, to_frame: 1_000, digest: ZERO.into() };
    let range = range_digest(&fx.project, fx.seq(), fx.plan(1), 0, 1_000);
    let d = code(&fx, Write::SetPosition(SetPosition { plan: 1, frame: 300, expect_range: Some(guard), ..Default::default() }), "stale_range");
    assert_eq!((d["fromFrame"].clone(), d["toFrame"].clone(), d["digest"].clone()), (json!(0), json!(1_000), json!(range)));
    let walls_now = effective_walls_digest(&fx.project, fx.seq());
    let stale = Write::ReplaceRange(ReplaceRange { plan: 1, from_frame: 0, to_frame: 50, expect_walls: Some(ZERO.into()), ..Default::default() });
    let d = code(&fx, stale, "stale_walls");
    assert_eq!(d["digest"], json!(walls_now));
    // replacing the move into 200 by a cut changes frames 141-199, outside [150, 250)
    let cut = vec![ReplaceItem { frame: 200, fields: PositionFields { arrival: Some(ArrivalKind::Cut), ..keys(1100.0, 540.0, 1080.0) } }];
    let strict = Write::ReplaceRange(ReplaceRange { plan: 1, from_frame: 150, to_frame: 250, items: cut, strict: true, ..Default::default() });
    let d = code(&fx, strict, "range_exceeded");
    assert_eq!(d["changedFrames"], json!([[141, 200]]));
    let ids: Vec<u32> = (1..=10_001).collect();
    let d = code(&fx, Write::DeletePositions(DeletePositions { plan: 1, positions: ids, ..Default::default() }), "too_many");
    assert_eq!((d["key"].clone(), d["count"].clone(), d["bound"].clone()), (json!("positions"), json!(10_001), json!(10_000)));
}

#[test]
fn t_m9_walls_parts_and_data() {
    let mut fx = base();
    for frame in [0, 1_000] {
        let d = code(&fx, Write::SetWalls(SetWalls { add: vec![WallAdd { frame, score: None }], ..Default::default() }), "bad_wall");
        assert_eq!(d["frame"], json!(frame), "a wall lies inside (0, end)");
    }
    let part = |from_frame: i64, to_frame: i64| {
        Write::SetParts(SetParts { add: vec![PartAdd { name: "P01".into(), from_frame, to_frame, ..Default::default() }], ..Default::default() })
    };
    let d = code(&fx, part(500, 500), "bad_part");
    assert_eq!(d["index"], json!(0));
    fx.apply(part(0, 100));
    let d = code(&fx, part(0, 90), "part_exists");
    assert_eq!(d["id"], json!(1));
    let shout = vec![AnnotationAdd { kind: "Speech".into(), from_frame: 0, to_frame: 10, ..Default::default() }];
    let d = code(&fx, Write::SetAnnotations(SetAnnotations { add: shout, ..Default::default() }), "bad_annotation");
    assert_eq!(d["index"], json!(0));
    let wide = vec![AreaSet { from_frame: 0, to_frame: 100, rect: [0.0, 0.0, 2_000.0, 1_080.0] }];
    let d = code(&fx, Write::SetActiveArea(SetActiveArea { set: wide, ..Default::default() }), "bad_area");
    assert_eq!(d["index"], json!(0));
    // a wall move blocked by a position strictly between its frames
    let mut fx = base();
    fx.set_mut().walls.stored = vec![stored(300, WallOrigin::Detected, Some(0.4))];
    fx.plan_mut(1).positions.push(at25(3, 350, A, Arrival::Cut));
    fx.plan_mut(1).next_position_id = 4;
    let d = code(&fx, Write::SetWalls(SetWalls { moves: vec![WallMove { from: 300, to: 400 }], ..Default::default() }), "wall_blocked");
    assert_eq!(d["frames"], json!([350]));
}

#[test]
fn t_m9_a_plan_holds_at_most_10_000_positions() {
    let mut fx = Fx::new(20_010);
    fx.install(one_plan((0..10_000).map(|i| at25(i + 1, i64::from(i) * 2, A, Arrival::Cut)).collect()));
    let d = code(&fx, set_at(20_001, keys(900.0, 540.0, 1080.0)), "too_many");
    assert_eq!((d["key"].clone(), d["count"].clone(), d["bound"].clone()), (json!("positions"), json!(10_001), json!(10_000)));
    assert!(accepted(fx.write(set_at(2, PositionFields { cx: Some(710.0), ..Default::default() }))).changed, "editing one of them is fine");
    // seeding past the bound: an opener for frame 0 and for each of 10 001 walls
    let mut fx = Fx::new(20_010);
    let mut set = one_plan(Vec::new());
    set.walls.stored = (1..=10_001).map(|i| stored(i * 2, WallOrigin::Manual, None)).collect();
    fx.install(set);
    let d = code(&fx, Write::SeedShots(SeedShots { plan: 1, ..Default::default() }), "too_many");
    assert_eq!((d["key"].clone(), d["count"].clone()), (json!("positions"), json!(10_002)));
}

/// A framing write on a damaged plan is refused (a person's marks are not: `validation.rs` unlocks one, then repairs).
#[test]
fn t_m9_a_damaged_plan_is_invalid_plan_for_a_framing_write_and_repair_fixes_it() {
    let mut fx = base();
    fx.plan_mut(1).positions.reverse();
    let d = code(&fx, set_at(300, keys(900.0, 540.0, 1080.0)), "invalid_plan");
    let violations = d["violations"].as_array().expect("a list of violations");
    assert!(violations.iter().any(|v| v["invariant"] == json!("I1")), "{d}");
    assert!(accepted(fx.write(Write::Repair(Repair { plan: 1 }))).changed, "repair works on it");
}

/// Plans `w` under `catch_unwind`: it must not panic and must be refused `expected`.
fn hostile(fx: &Fx, w: Write, expected: &str, what: &str) {
    let result = catch_unwind(AssertUnwindSafe(|| fx.write(w)));
    let Ok(result) = result else { panic!("{what}: the planner panicked") };
    match result {
        Ok(planned) => panic!("{what}: accepted ({})", planned.label),
        Err(e) => assert_eq!(e.code, expected, "{what}: {} {}", e.message, e.detail),
    }
}

#[test]
fn t_m9_hostile_values_are_refused_without_a_panic() {
    let fx = base();
    let huge: f64 = "1e309".parse().expect("parses to infinity");
    assert!(huge.is_infinite());
    for (v, what) in [(f64::NAN, "NaN"), (f64::INFINITY, "+inf"), (f64::NEG_INFINITY, "-inf"), (huge, "1e309")] {
        hostile(&fx, set_at(300, PositionFields { cx: Some(v), ..Default::default() }), "bad_number", &format!("cx {what}"));
        hostile(&fx, set_at(300, PositionFields { cy: Some(v), ..Default::default() }), "bad_number", &format!("cy {what}"));
        hostile(&fx, set_at(300, PositionFields { h: Some(v), ..Default::default() }), "bad_number", &format!("h {what}"));
        hostile(&fx, set_at(300, PositionFields { zoom: Some(v), ..Default::default() }), "bad_number", &format!("zoom {what}"));
        hostile(&fx, set_at(300, PositionFields { focus: Some(Some([v, 10.0])), ..Default::default() }), "bad_number", &format!("focus {what}"));
        let fit = Write::FitMove(FitMove { plan: 1, position: 2, max_speed: v, ..Default::default() });
        hostile(&fx, fit, "bad_number", &format!("maxSpeed {what}"));
        let ramp = Curve::Ramp { r_in: v, r_out: 0.2 };
        hostile(&fx, set_at(300, PositionFields { curve: Some(ramp), ..move_keys(40, None) }), "bad_curve", &format!("ramp {what}"));
        let bezier = Curve::Bezier { x1: v, y1: 0.0, x2: 0.5, y2: 1.0 };
        hostile(&fx, set_at(300, PositionFields { curve: Some(bezier), ..move_keys(40, None) }), "bad_curve", &format!("bezier {what}"));
        let rect = Write::SetRegions(SetRegions {
            add: vec![RegionAdd { label: "logo".into(), from_frame: 0, to_frame: 10, rect: [v, 0.0, 10.0, 10.0], mode: RegionMode::KeepIn }],
            ..Default::default()
        });
        hostile(&fx, rect, "bad_number", &format!("region rect {what}"));
        let score = Write::SetWalls(SetWalls { add: vec![WallAdd { frame: 500, score: Some(v) }], ..Default::default() });
        hostile(&fx, score, "bad_number", &format!("wall score {what}"));
    }
    for frame in [-1, -1_000_000, i64::MIN, i64::MAX] {
        hostile(&fx, set_at(frame, PositionFields::default()), "out_of_range", &format!("frame {frame}"));
        let retime = Write::RetimePosition(RetimePosition { plan: 1, position: 2, frame, ..Default::default() });
        hostile(&fx, retime, "out_of_range", &format!("retime to {frame}"));
        let range = Write::ReplaceRange(ReplaceRange { plan: 1, from_frame: frame, to_frame: 10, ..Default::default() });
        hostile(&fx, range, "out_of_range", &format!("range from {frame}"));
        hostile(&fx, Write::SetWalls(SetWalls { suppress: vec![frame], ..Default::default() }), "bad_wall", &format!("suppress {frame}"));
    }
    for frames in [i64::MAX, i64::from(u32::MAX) + 1, 1_000_001] {
        hostile(&fx, set_at(300, PositionFields { move_frames: Some(frames), ..keys(900.0, 540.0, 1080.0) }), "out_of_range", &format!("moveFrames {frames}"));
    }
    for frames in [i64::MIN, -5, 0] {
        let fields = PositionFields { move_frames: Some(frames), ..keys(900.0, 540.0, 1080.0) };
        hostile(&fx, set_at(300, fields), "move_too_short", &format!("moveFrames {frames}"));
    }
    hostile(&fx, set_at(300, keys(1.0e9, 540.0, 1080.0)), "out_of_range", "cx beyond MAX_COORDINATE");
    let clamped = Write::SetPosition(SetPosition { plan: 1, frame: 300, fields: keys(-1.0e9, 540.0, 1080.0), clamp: true, ..Default::default() });
    hostile(&fx, clamped, "out_of_range", "cx beyond MAX_COORDINATE, even with clamp");
    hostile(&fx, Write::SetPosition(SetPosition { plan: u32::MAX, frame: 300, ..Default::default() }), "not_found", "plan u32::MAX");
    // a million ids or frames; a hundred thousand of the larger items (each over a cap of at most 20 000)
    let (million, many) = (1_000_000_usize, 100_000_usize);
    let ids: Vec<u32> = (0..million as u32).collect();
    hostile(&fx, Write::DeletePositions(DeletePositions { plan: 1, positions: ids, ..Default::default() }), "too_many", "10^6 ids");
    let frames: Vec<i64> = (1..=million as i64).collect();
    hostile(&fx, Write::SetWalls(SetWalls { suppress: frames, ..Default::default() }), "too_many", "10^6 frames");
    let items: Vec<ReplaceItem> = (0..many as i64).map(|f| ReplaceItem { frame: f, fields: PositionFields::default() }).collect();
    hostile(&fx, Write::ReplaceRange(ReplaceRange { plan: 1, from_frame: 0, to_frame: 1_000, items, ..Default::default() }), "too_many", "10^5 items");
    let speech = |f: i64| AnnotationAdd { kind: "speech".into(), from_frame: f, to_frame: f + 1, ..Default::default() };
    let adds: Vec<AnnotationAdd> = (0..many as i64).map(speech).collect();
    hostile(&fx, Write::SetAnnotations(SetAnnotations { add: adds, ..Default::default() }), "too_many", "10^5 annotations");
    let tags: Vec<String> = (0..many).map(|i| format!("t{i}")).collect();
    hostile(&fx, set_at(300, PositionFields { tags: Some(tags), ..Default::default() }), "too_many", "10^5 tags");
    hostile(&fx, set_at(300, PositionFields { label: Some("x".repeat(million)), ..Default::default() }), "too_many", "a label of 10^6 characters");
    let width = Write::Create(CreatePlan { width: Some(u32::MAX), height: Some(u32::MAX), ..Default::default() });
    hostile(&fx, width, "invalid_output_size", "a size of u32::MAX");
    let zero = Write::Create(CreatePlan { width: Some(0), height: Some(0), ..Default::default() });
    hostile(&fx, zero, "invalid_output_size", "a size of 0");
    let fit = Write::FitMove(FitMove { plan: 1, position: 2, max_speed: -1.0, ..Default::default() });
    hostile(&fx, fit, "bad_number", "a negative maxSpeed");
    let fit = Write::FitMove(FitMove { plan: 1, position: 2, max_speed: 6.0, max_accel: Some(f64::NAN), ..Default::default() });
    hostile(&fx, fit, "bad_number", "maxAccel NaN");
    assert_eq!(fx.plan(1).positions.len(), 2, "nothing changed");
}

#[test]
fn t_m9_an_annotation_score_whose_grid_value_is_not_finite_is_bad_number() {
    // q(score) is stored (spec section 2.6) and I17 asks it to be finite: q of a huge finite score overflows to infinity
    let mut fx = base();
    let speech = Annotation {
        id: 1,
        kind: "speech".into(),
        from: tk(10),
        to: tk(20),
        label: String::new(),
        score: Some(0.5),
        origin: Origin::Manual,
        modified_by: Writer::Human,
    };
    fx.set_mut().annotations = vec![speech];
    fx.set_mut().next_annotation_id = 2;
    let huge: f64 = "1e309".parse().expect("parses to infinity");
    for (v, what) in [(f64::NAN, "NaN"), (f64::INFINITY, "+inf"), (huge, "1e309"), (1e306, "1e306"), (-1e306, "-1e306"), (f64::MAX, "f64::MAX")] {
        let add = AnnotationAdd { kind: "speech".into(), from_frame: 30, to_frame: 40, score: Some(v), ..Default::default() };
        hostile(&fx, Write::SetAnnotations(SetAnnotations { add: vec![add], ..Default::default() }), "bad_number", &format!("add {what}"));
        let update = AnnotationUpdate { id: 1, score: Some(Some(v)), ..Default::default() };
        hostile(&fx, Write::SetAnnotations(SetAnnotations { update: vec![update], ..Default::default() }), "bad_number", &format!("update {what}"));
        let item = AnnotationItem { from_frame: 30, to_frame: 40, label: String::new(), score: Some(v) };
        let replace = Some(AnnotationReplace { kind: "speech".into(), from_frame: 0, to_frame: 100, set: vec![item] });
        hostile(&fx, Write::SetAnnotations(SetAnnotations { replace, ..Default::default() }), "bad_number", &format!("replace {what}"));
    }
    // an ordinary score is stored on the grid, and the data stays valid
    let add = AnnotationAdd { kind: "speech".into(), from_frame: 30, to_frame: 40, score: Some(0.123_4), ..Default::default() };
    fx.apply(Write::SetAnnotations(SetAnnotations { add: vec![add], ..Default::default() }));
    assert_eq!(fx.set().annotations.iter().map(|a| a.score).collect::<Vec<_>>(), vec![Some(0.5), Some(0.123)]);
    assert!(validate(&fx.project, fx.seq()).is_empty());
}
