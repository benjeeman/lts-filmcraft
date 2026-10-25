//! T-M20 and the bounds half of T-M24 (spec sections 2.5 and 2.8). `validate` names the invariant every damaged datum
//! breaks (I1-I21 and I23, box tracks and overwritten ranges included); the planner refuses out-of-bounds writes with
//! `too_many` or a `bad_*` code; repair sorts, merges duplicates keeping the first, clamps, shortens or cuts moves, resets
//! invalid curves and drops what cannot be repaired, deterministically, reporting each change, and never changes a locked
//! position.

use filmcraft_time::Tick;
use serde_json::json;

use super::fixtures::*;
use super::support::{mv, mv_split, win};
use crate::vertical::*;

const A: Window = Window { cx: 700.0, cy: 540.0, h: 1080.0 };
const B: Window = Window { cx: 1100.0, cy: 540.0, h: 1080.0 };

/// A valid set that holds every stored type: a plan with a cut, a move, an attention mark, an approval and an overwritten
/// range; walls of each list; a part; a region; an annotation; a box track; an active area. 1000 frames at 25p.
fn valid() -> Fx {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(vec![at25(1, 100, A, Arrival::Cut), at25(2, 200, B, mv(60, Curve::EaseInOut))]);
    let plan = &mut set.plans[0];
    plan.attention = vec![AttentionMark { at: tk(150), by: "Dana Editor".into(), when: 1_790_000_000, note: "eyeline".into() }];
    plan.part_approvals = vec![PartApproval {
        part: 1,
        framing_digest: "fnv1a64:0123456789abcdef".into(),
        output_digest: "fnv1a64:fedcba9876543210".into(),
        by: "Dana Editor".into(),
        when: 1_790_000_000,
        note: String::new(),
    }];
    plan.overwritten = vec![OverwrittenRange {
        id: 1,
        from: tk(100),
        to: tk(200),
        positions: vec![at25(1, 100, A, Arrival::Cut)],
        label: "Vertical: replace 1 position (agent)".into(),
        run_id: None,
        when: 1_790_000_100,
    }];
    plan.next_record_id = 2;
    set.walls.stored = vec![stored(300, WallOrigin::Detected, Some(0.5))];
    set.walls.suppressed = vec![tk(350)];
    set.walls.candidates = vec![Candidate { at: tk(400), score: 0.2 }];
    set.walls.analysed = vec![FrameRange { from: tk(0), to: tk(1_000) }];
    set.parts = vec![part25(1, "P01", 0, 500)];
    set.regions = vec![ProtectedRegion {
        id: 1,
        label: "logo".into(),
        from: tk(0),
        to: tk(1_000),
        rect: [100.0, 100.0, 200.0, 200.0],
        mode: RegionMode::KeepIn,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    }];
    set.annotations = vec![Annotation {
        id: 1,
        kind: "speech".into(),
        from: tk(10),
        to: tk(50),
        label: "char:dana".into(),
        score: Some(0.5),
        origin: Origin::Agent,
        modified_by: Writer::Agent,
    }];
    set.box_tracks = vec![BoxTrack {
        id: "char:dana".into(),
        label: "Dana".into(),
        kind: "face".into(),
        keys: vec![key(10, [100.0, 100.0, 50.0, 50.0]), key(20, [110.0, 100.0, 50.0, 50.0])],
        origin: Origin::Agent,
        modified_by: Writer::Agent,
    }];
    set.active_area = vec![ActiveArea { from: tk(0), to: tk(500), rect: [0.0, 132.0, 1920.0, 816.0], origin: Origin::Manual, modified_by: Writer::Human }];
    set.next_part_id = 2;
    set.next_region_id = 2;
    set.next_annotation_id = 2;
    fx.install(set);
    fx
}

fn key(frame: i64, rect: [f64; 4]) -> KeyBox {
    KeyBox { at: tk(frame), rect, facing: None, gone: false }
}

/// The invariants `validate` names after `damage` changes the valid set.
fn broken(damage: impl FnOnce(&mut Fx)) -> Vec<&'static str> {
    let mut fx = valid();
    damage(&mut fx);
    let mut found: Vec<&'static str> = validate(&fx.project, fx.seq()).iter().map(|v| v.invariant).collect();
    found.dedup();
    found
}

fn names(invariant: &'static str, damage: impl FnOnce(&mut Fx)) {
    let found = broken(damage);
    assert!(found.contains(&invariant), "{invariant} not among {found:?}");
}

#[test]
fn t_m20_the_valid_set_has_no_violation() {
    let fx = valid();
    assert_eq!(validate(&fx.project, fx.seq()), Vec::new());
    assert_eq!(validate(&fx.project, Fx::new(10).seq()), Vec::new(), "a sequence without a set");
    // and the check is live: the same set with its positions out of order breaks I1
    names("I1", |fx| fx.plan_mut(1).positions.reverse());
}

#[test]
fn t_m20_positions_and_plans() {
    names("I1", |fx| fx.plan_mut(1).positions.reverse());
    names("I1", |fx| fx.plan_mut(1).positions[1].at = tk(100));
    names("I1", |fx| fx.plan_mut(1).positions[1].at = tk(1_000));
    names("I1", |fx| fx.plan_mut(1).positions[1].at = Tick(tk(200).0 + 1));
    names("I2", |fx| fx.plan_mut(1).positions[0].window.cx = f64::NAN);
    names("I2", |fx| fx.plan_mut(1).positions[0].window.h = 70_000.0);
    names("I3", |fx| fx.plan_mut(1).positions[0].window.cx = 2_000.0);
    names("I4", |fx| fx.plan_mut(1).positions[0].window.h = 400.0);
    names("I5", |fx| fx.plan_mut(1).positions[1].arrival = mv(150, Curve::EaseInOut));
    names("I5", |fx| fx.plan_mut(1).positions[1].arrival = mv(1, Curve::EaseInOut));
    names("I6", |fx| fx.set_mut().walls.stored.insert(0, stored(170, WallOrigin::Manual, None)));
    names("I7", |fx| fx.plan_mut(1).positions[1].arrival = mv(60, Curve::Ramp { r_in: 0.7, r_out: 0.7 }));
    names("I7", |fx| fx.plan_mut(1).positions[1].arrival = mv(60, Curve::Bezier { x1: 1.0, y1: 0.0, x2: 0.0, y2: 1.0 }));
    names("I8", |fx| fx.plan_mut(1).positions[1].arrival = mv_split(60, Curve::EaseInOut, 10, 50));
    names("I9", |fx| fx.plan_mut(1).positions[0].layout = Layout::Fit);
    names("I11", |fx| fx.plan_mut(1).output = OutputSize { width: 1081, height: 1920 });
    names("I11", |fx| {
        let copy = VerticalPlan { id: 2, ..fx.plan(1).clone() };
        fx.set_mut().plans.push(copy);
    });
    names("I11", |fx| {
        for id in 2..=17 {
            let copy = VerticalPlan { id, name: format!("plan {id}"), ..fx.plan(1).clone() };
            fx.set_mut().plans.push(copy);
        }
    });
    names("I12", |fx| fx.seq_mut().settings.par = (4, 3));
    names("I13", |fx| fx.plan_mut(1).positions[0].label = "x".repeat(41));
    names("I13", |fx| fx.plan_mut(1).positions[0].tags = (0..17).map(|i| format!("t{i}")).collect());
    names("I13", |fx| fx.plan_mut(1).positions[0].note = "x".repeat(1_001));
    names("I13", |fx| fx.plan_mut(1).name = String::new());
    names("I20", |fx| fx.plan_mut(1).positions[0].confidence = Some(1.5));
    names("I20", |fx| fx.plan_mut(1).positions[0].focus = Some([2_000.0, 10.0]));
    names("I20", |fx| fx.plan_mut(1).positions[0].subjects = (0..9).map(|i| format!("face:{i}")).collect());
    names("I20", |fx| fx.plan_mut(1).positions[0].risk = "x".repeat(201));
    names("I21", |fx| fx.plan_mut(1).attention.push(AttentionMark { at: tk(120), by: "x".into(), when: 0, note: String::new() }));
    names("I21", |fx| fx.plan_mut(1).attention[0].at = tk(1_000));
    names("I21", |fx| {
        let again = fx.plan(1).part_approvals[0].clone();
        fx.plan_mut(1).part_approvals.push(again);
    });
    names("I23", |fx| fx.plan_mut(1).overwritten[0].to = tk(100));
    names("I23", |fx| fx.plan_mut(1).overwritten[0].positions[0].window.cx = 2_000.0);
}

#[test]
fn t_m20_walls_parts_regions_and_data() {
    names("I10", |fx| fx.set_mut().walls.stored.insert(0, stored(0, WallOrigin::Manual, None)));
    names("I10", |fx| fx.set_mut().walls.suppressed = vec![tk(360), tk(350)]);
    names("I10", |fx| fx.set_mut().walls.candidates = vec![Candidate { at: tk(1_000), score: 0.2 }]);
    names("I10", |fx| fx.set_mut().walls.analysed = vec![FrameRange { from: tk(0), to: tk(600) }, FrameRange { from: tk(500), to: tk(1_000) }]);
    names("I14", |fx| fx.set_mut().parts[0].to = tk(0));
    names("I14", |fx| fx.set_mut().parts.push(part25(2, "P01", 600, 700)));
    names("I14", |fx| fx.set_mut().parts[0].audio = Some(PartAudio { fade_in_frames: 251, fade_out_frames: 4 }));
    names("I15", |fx| fx.set_mut().regions[0].rect = [1_900.0, 0.0, 100.0, 100.0]);
    names("I15", |fx| fx.set_mut().regions[0].to = tk(0));
    names("I17", |fx| fx.set_mut().annotations[0].kind = "Speech".into());
    names("I17", |fx| fx.set_mut().annotations[0].score = Some(0.000_5));
    names("I17", |fx| {
        let again = fx.set().annotations[0].clone();
        fx.set_mut().annotations.push(again);
    });
    names("I18", |fx| fx.set_mut().box_tracks[0].id = "bad id!".into());
    names("I18", |fx| fx.set_mut().box_tracks[0].keys.reverse());
    names("I18", |fx| fx.set_mut().box_tracks[0].keys[0].rect = [1_900.0, 0.0, 50.0, 50.0]);
    names("I19", |fx| {
        let more = ActiveArea { from: tk(400), to: tk(600), rect: [0.0, 0.0, 1920.0, 1080.0], origin: Origin::Manual, modified_by: Writer::Human };
        fx.set_mut().active_area.push(more);
    });
    names("I19", |fx| fx.set_mut().active_area[0].rect = [0.0, 132.0, 0.0, 816.0]);
}

#[test]
fn t_m24_writes_out_of_bounds_are_refused_with_too_many_or_a_bad_code() {
    let fx = valid();
    let too_many = |w: Write, key: &str, count: usize, bound: usize| {
        let e = fx.refuse(w);
        assert_eq!(e.code, "too_many", "{key}: {}", e.detail);
        assert_eq!((e.detail["key"].clone(), e.detail["count"].clone(), e.detail["bound"].clone()), (json!(key), json!(count), json!(bound)));
    };
    too_many(set_at(300, PositionFields { label: Some("x".repeat(41)), ..Default::default() }), "label", 41, 40);
    too_many(set_at(300, PositionFields { note: Some("x".repeat(1_001)), ..Default::default() }), "note", 1_001, 1_000);
    too_many(set_at(300, PositionFields { tags: Some((0..17).map(|i| format!("t{i}")).collect()), ..Default::default() }), "tags", 17, 16);
    too_many(set_at(300, PositionFields { subjects: Some((0..9).map(|i| format!("face:{i}")).collect()), ..Default::default() }), "subjects", 9, 8);
    too_many(set_at(300, PositionFields { risk: Some("x".repeat(201)), ..Default::default() }), "risk", 201, 200);
    too_many(Write::Create(CreatePlan { name: Some("x".repeat(121)), ..Default::default() }), "name", 121, 120);
    let long = PartAdd { name: "x".repeat(65), from_frame: 600, to_frame: 700, ..Default::default() };
    too_many(Write::SetParts(SetParts { add: vec![long], ..Default::default() }), "name", 65, 64);
    let flag = Write::FlagAttention(FlagAttention { plan: 1, frame: 300, note: "x".repeat(1_001), by: "Dana Editor".into(), when: 1 });
    too_many(flag, "note", 1_001, 1_000);
    let e = fx.refuse(set_at(300, PositionFields { confidence: Some(Some(1.5)), ..Default::default() }));
    assert_eq!((e.code, e.detail["key"].clone()), ("out_of_range", json!("confidence")));
    let e = fx.refuse(set_at(300, PositionFields { focus: Some(Some([2_000.0, 10.0])), ..Default::default() }));
    assert_eq!((e.code, e.detail["key"].clone()), ("outside_picture", json!("focus")));
    let e = fx.refuse(set_at(300, PositionFields { subjects: Some(vec!["bad id!".into()]), ..Default::default() }));
    assert_eq!(e.code, "bad_box_track", "a subject is a box-track id");
    let bad = AnnotationAdd { kind: "speech".into(), from_frame: 50, to_frame: 50, ..Default::default() };
    assert_eq!(fx.refuse(Write::SetAnnotations(SetAnnotations { add: vec![bad], ..Default::default() })).code, "bad_annotation");
    let off = AnnotationAdd { kind: "music".into(), from_frame: 0, to_frame: 2_000, ..Default::default() };
    assert_eq!(fx.refuse(Write::SetAnnotations(SetAnnotations { add: vec![off], ..Default::default() })).code, "bad_annotation", "inside [0, end]");
    let speech = |i: i64| AnnotationAdd { kind: "speech".into(), from_frame: i % 900, to_frame: i % 900 + 1, label: format!("{i}"), ..Default::default() };
    let adds: Vec<AnnotationAdd> = (0..20_000).map(speech).collect();
    let e = fx.refuse(Write::SetAnnotations(SetAnnotations { add: adds, ..Default::default() }));
    assert_eq!((e.code, e.detail["bound"].clone()), ("too_many", json!(20_000)), "one annotation is there already");
    let areas: Vec<AreaSet> = (0..2_001).map(|i| AreaSet { from_frame: i % 1_000, to_frame: i % 1_000 + 1, rect: [0.0, 0.0, 100.0, 100.0] }).collect();
    assert_eq!(fx.refuse(Write::SetActiveArea(SetActiveArea { set: areas, ..Default::default() })).code, "too_many");
    let e = fx.refuse(Write::FlagAttention(FlagAttention { plan: 1, frame: 1_000, note: String::new(), by: "Dana Editor".into(), when: 1 }));
    assert_eq!((e.code, e.detail["key"].clone()), ("out_of_range", json!("frame")));
}

/// A plan damaged in every way repair handles: unsorted, a duplicate frame, a window outside the frame, an invalid
/// curve, a move longer than its room, a move departing from a fit position, a move across a wall, a position past the end.
fn damaged() -> Fx {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(vec![
        at25(3, 300, A, Arrival::Cut),
        at25(1, 100, A, Arrival::Cut),
        at25(2, 200, B, mv(150, Curve::EaseInOut)),
        at25(4, 300, B, Arrival::Cut),
        at25(5, 400, win(2_000.0, 540.0, 1080.0), Arrival::Cut),
        at25(6, 500, B, mv(50, Curve::Ramp { r_in: 0.7, r_out: 0.7 })),
        Position { layout: Layout::Fit, ..at25(7, 600, A, Arrival::Cut) },
        at25(8, 650, B, mv(20, Curve::Linear)),
        at25(9, 720, A, mv(40, Curve::Linear)),
        at25(10, 1_500, A, Arrival::Cut),
    ]);
    set.walls.stored = vec![stored(700, WallOrigin::Manual, None)];
    set.plans[0].next_position_id = 11;
    fx.install(set);
    fx
}

#[test]
fn t_m20_repair_yields_a_valid_plan_deterministically_and_reports_each_change() {
    let fx = damaged();
    assert!(!validate(&fx.project, fx.seq()).is_empty());
    let first = accepted(fx.write(Write::Repair(Repair { plan: 1 })));
    let second = accepted(fx.write(Write::Repair(Repair { plan: 1 })));
    assert_eq!(first, second, "repair is deterministic");
    assert!(first.changed);
    let set = first.set.clone().expect("a set");
    let mut repaired = fx.seq().clone();
    repaired.vertical = Some(set);
    assert_eq!(validate(&fx.project, &repaired), Vec::new(), "the repaired plan is valid");
    let plan = &repaired.vertical.as_deref().expect("a set").plans[0];
    let rows: Vec<(u32, i64, Arrival)> = plan.positions.iter().map(|p| (p.id, RATE.frame_at(p.at), p.arrival)).collect();
    assert_eq!(
        rows,
        vec![
            (1, 100, Arrival::Cut),
            (2, 200, mv(100, Curve::EaseInOut)),
            (3, 300, Arrival::Cut),
            (5, 400, Arrival::Cut),
            (6, 500, mv(50, DEFAULT_CURVE)),
            (7, 600, Arrival::Cut),
            (8, 650, Arrival::Cut),
            (9, 720, mv(20, Curve::Linear)),
        ],
        "sorted; the duplicate frame keeps the first; the move into 200 takes its room; the invalid curve is Ramp 20/20; the move \
         from a fit position is a cut; the move across the wall starts on it; the position past the end is dropped"
    );
    assert_eq!(plan.positions[3].window, win(1616.25, 540.0, 1080.0), "clamped with directed rounding");
    let codes: Vec<&str> = first.changes.iter().map(|c| c.code).collect();
    for code in ["sorted", "dropped", "clamped", "move_shortened", "move_cut", "curve_reset"] {
        assert!(codes.contains(&code), "{code} not reported: {codes:?}");
    }
    let dropped: Vec<serde_json::Value> = first.changes.iter().filter(|c| c.code == "dropped").map(|c| c.detail["id"].clone()).collect();
    assert!(dropped.contains(&json!(4)) && dropped.contains(&json!(10)), "{dropped:?}");
    // repairing a valid plan changes nothing
    let mut clean = Fx::new(1_000);
    clean.seq_mut().vertical = first.set;
    assert!(!accepted(clean.write(Write::Repair(Repair { plan: 1 }))).changed);
}

#[test]
fn t_m20_repair_never_changes_a_locked_position() {
    let mut fx = damaged();
    fx.plan_mut(1).positions[4].locked = true;
    let e = fx.refuse(Write::Repair(Repair { plan: 1 }));
    assert_eq!((e.code, e.detail["ids"].clone()), ("locked", json!([5])), "the locked position's window is outside the frame");
}

#[test]
fn t_m20_a_locked_position_that_breaks_an_invariant_is_unlocked_and_then_repaired() {
    let mut fx = damaged();
    fx.plan_mut(1).positions[4].locked = true;
    assert_eq!(fx.refuse(Write::Repair(Repair { plan: 1 })).code, "locked", "repair refuses, and the person unlocks it");
    // a person's marks change no framing, so they work on the damaged plan, which they leave unevaluated
    let unlock = fx.apply(Write::SetLock(SetLock { plan: 1, positions: vec![5], locked: false }));
    assert!(unlock.changed && !fx.position(1, 5).locked);
    assert_eq!(codes(&unlock.issues), vec!["invalid_plan"], "a plan that still breaks an invariant is not evaluated");
    assert!(unlock.changed_frames.is_empty());
    fx.apply(Write::SetReview(SetReview { plan: 1, positions: vec![5], reviewed: true, ..Default::default() }));
    assert!(fx.position(1, 5).reviewed);
    fx.apply(Write::FlagAttention(FlagAttention { plan: 1, frame: 450, note: "the window".into(), by: "Dana Editor".into(), when: 1 }));
    assert_eq!(fx.plan(1).attention.len(), 1);
    fx.apply(Write::ClearAttention(ClearAttention { plan: 1, frames: vec![450] }));
    assert!(fx.plan(1).attention.is_empty());
    // a framing write is still refused, and so is approving a shot, whose opener evaluates the plan
    assert_eq!(fx.refuse(set_at(450, keys(900.0, 540.0, 1080.0))).code, "invalid_plan");
    assert_eq!(fx.refuse(Write::SetReview(SetReview { plan: 1, shot_frame: Some(450), reviewed: true, ..Default::default() })).code, "invalid_plan");
    // unlocked, the plan repairs
    assert!(fx.apply(Write::Repair(Repair { plan: 1 })).changed);
    assert_eq!(validate(&fx.project, fx.seq()), Vec::new());
}

#[test]
fn t_m24_repair_sorts_merges_and_drops_annotations_keys_areas_and_marks() {
    let mut fx = valid();
    {
        let set = fx.set_mut();
        let speech = set.annotations[0].clone();
        set.annotations = vec![
            Annotation { id: 2, from: tk(60), to: tk(80), ..speech.clone() },
            speech.clone(),
            Annotation { id: 3, kind: "Not a kind".into(), ..speech.clone() },
            Annotation { id: 2, from: tk(90), to: tk(95), ..speech },
        ];
        set.next_annotation_id = 4;
        set.box_tracks[0].keys = vec![key(20, [110.0, 100.0, 50.0, 50.0]), key(10, [100.0, 100.0, 50.0, 50.0]), key(10, [120.0, 100.0, 50.0, 50.0])];
        let area = set.active_area[0];
        set.active_area = vec![ActiveArea { from: tk(400), to: tk(600), ..area }, area];
        set.plans[0].attention.insert(0, AttentionMark { at: tk(180), by: "x".into(), when: 0, note: String::new() });
    }
    let planned = accepted(fx.write(Write::Repair(Repair { plan: 1 })));
    let set = planned.set.as_deref().expect("a set");
    let annotations: Vec<(u32, i64)> = set.annotations.iter().map(|a| (a.id, RATE.frame_at(a.from))).collect();
    assert_eq!(annotations, vec![(1, 10), (2, 60)], "sorted by (from, id); the invalid kind and the second id 2 dropped");
    let keys: Vec<(i64, f64)> = set.box_tracks[0].keys.iter().map(|k| (RATE.frame_at(k.at), k.rect[0])).collect();
    assert_eq!(keys, vec![(10, 100.0), (20, 110.0)], "sorted, the duplicate frame keeps the first in time order");
    let areas: Vec<(i64, i64)> = set.active_area.iter().map(|a| (RATE.frame_at(a.from), RATE.frame_at(a.to))).collect();
    assert_eq!(areas, vec![(0, 500), (500, 600)], "sorted; the overlap is trimmed off the later area");
    let marks: Vec<i64> = set.plans[0].attention.iter().map(|m| RATE.frame_at(m.at)).collect();
    assert_eq!(marks, vec![150, 180]);
    assert!(planned.changes.len() >= 4, "each change is reported: {:?}", planned.changes);
    let mut repaired = fx.seq().clone();
    repaired.vertical = planned.set.clone();
    assert_eq!(validate(&fx.project, &repaired), Vec::new());
}
