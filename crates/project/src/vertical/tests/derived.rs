//! Derived values (spec section 2.7; plan 63 L1b): the raster, tags with the FIT badge, moving and static ranges, spans,
//! hold lengths, the governing position (lane L3b's caption-anchor rule: `Before` gives the first position, `Hold` the
//! sample's position, a move or the starting window none), shots with own, carried and starting, events, the per-frame
//! rows of `vertical.windows`, `vertical.goTo`, and the issues that need no threshold.

use serde_json::json;

use super::fixtures::*;
use super::support::{mv, plan_of, win};
use crate::vertical::*;

const RAMP: Curve = Curve::Ramp { r_in: 0.2, r_out: 0.2 };

/// Walls at 100 and 300; START at 50, a CUT on 100, a SAME cut at 150, a Ramp move into 200 (from 160), a fit JUMP at 250,
/// a JUMP at 350 (the shot from 300 opens carried from the fit position).
fn derived_fx() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 200, 5_000), (300, 700, 9_000)]);
    fx.install(one_plan(vec![
        at25(1, 50, win(700.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 100, win(1000.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 150, win(1000.0, 540.0, 1080.0), Arrival::Cut),
        at25(4, 200, win(900.0, 540.0, 1080.0), mv(40, RAMP)),
        Position { layout: Layout::Fit, ..at25(5, 250, win(960.0, 540.0, 1080.0), Arrival::Cut) },
        at25(6, 350, win(800.0, 540.0, 1080.0), Arrival::Cut),
    ]));
    fx
}

#[test]
fn the_raster_is_the_sequences_frame_rate_and_content() {
    let fx = derived_fx();
    assert_eq!(raster(fx.seq()), Raster { width: 1920, height: 1080, rate: RATE, end_frame: 1_000 });
    assert_eq!(raster(Fx::with_clips(&[]).seq()).end_frame, 0, "an empty sequence has no frame");
}

#[test]
fn tags_ranges_spans_and_holds() {
    let fx = derived_fx();
    let d = derive_plan(&fx.project, fx.seq(), 1).expect("the plan");
    let tags: Vec<(u32, Tag, bool)> = d.positions.iter().map(|p| (p.id, p.tag, p.fit)).collect();
    assert_eq!(
        tags,
        vec![(1, Tag::Start, false), (2, Tag::Cut, false), (3, Tag::Same, false), (4, Tag::Move, false), (5, Tag::Jump, true), (6, Tag::Jump, false)]
    );
    let ranges: Vec<([i64; 2], [i64; 2], [i64; 2], i64)> = d.positions.iter().map(|p| (p.moving, p.still, p.span, p.hold_frames)).collect();
    assert_eq!(
        ranges,
        vec![
            ([50, 50], [50, 100], [0, 100], 50),
            ([100, 100], [100, 150], [100, 150], 50),
            ([150, 150], [150, 161], [150, 161], 11),
            ([161, 200], [200, 250], [161, 250], 50),
            ([250, 250], [250, 350], [250, 350], 100),
            ([350, 350], [350, 1_000], [350, 1_000], 650),
        ],
        "a cut's next change is its own frame, a move's is its start plus one; the first span covers the frames before it"
    );
    let shots: Vec<usize> = d.positions.iter().map(|p| p.shot).collect();
    assert_eq!(shots, vec![0, 1, 1, 1, 1, 2]);
    let move_info = &d.positions[3];
    assert!((move_info.peak_speed - 5.555_555_555_555_555).abs() < 1e-9, "{}", move_info.peak_speed);
    assert_eq!(move_info.curve_label.as_deref(), Some("Ramp 20/20"));
    assert_eq!((d.positions[0].peak_speed, d.positions[0].curve_label.clone()), (0.0, None));
    assert_eq!(d.limits, Limits { min_height: 480.0, max_height: 1080.0 });
}

#[test]
fn the_governing_position_follows_window_at() {
    let fx = derived_fx();
    let (plan, r) = (fx.plan(1), fx.r());
    for (frame, expected) in [(10, Some(1)), (50, Some(1)), (120, Some(2)), (160, Some(3)), (170, None), (199, None), (200, Some(4)), (999, Some(6))] {
        assert_eq!(governing(plan, &r, frame), expected, "frame {frame}");
    }
    assert_eq!(governing(&plan_of(Vec::new()), &r, 10), None, "the starting window");
}

#[test]
fn shots_are_own_carried_or_starting() {
    let fx = derived_fx();
    let all = shots(&fx.project, fx.seq(), 1);
    let rows: Vec<(u32, i64, i64, ShotStatus, Option<u32>, Option<u32>, Vec<u32>)> =
        all.iter().map(|s| (s.number, s.from, s.to, s.status, s.opener, s.carried_from, s.positions.clone())).collect();
    assert_eq!(
        rows,
        vec![
            (1, 0, 100, ShotStatus::Carried, None, Some(1), vec![1]),
            (2, 100, 300, ShotStatus::Own, Some(2), None, vec![2, 3, 4, 5]),
            (3, 300, 1_000, ShotStatus::Carried, None, Some(5), vec![6]),
        ]
    );
    assert_eq!((all[0].sources.clone(), all[1].sources.clone()), (vec![], vec![WallSource::Edit]));
    let mut empty = derived_fx();
    empty.plan_mut(1).positions.clear();
    assert!(shots(&empty.project, empty.seq(), 1).iter().all(|s| s.status == ShotStatus::Starting));
}

#[test]
fn events_are_the_history_as_rows() {
    let fx = derived_fx();
    let events = events(&fx.project, fx.seq(), 1);
    let row = |e: &Event| {
        let mut v = serde_json::to_value(e).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.remove("distance");
            o.remove("peakSpeed");
            o.remove("curve");
        }
        v
    };
    let kinds: Vec<serde_json::Value> = events.iter().map(row).collect();
    assert_eq!(
        kinds,
        vec![
            json!({"kind": "hold", "from": 0, "to": 100, "position": 1}),
            json!({"kind": "cut", "frame": 100, "position": 2}),
            json!({"kind": "hold", "from": 100, "to": 150, "position": 2}),
            json!({"kind": "hold", "from": 150, "to": 161, "position": 3}),
            json!({"kind": "move", "from": 160, "to": 200, "position": 4, "frames": 40}),
            json!({"kind": "hold", "from": 200, "to": 250, "position": 4}),
            json!({"kind": "jump", "frame": 250, "position": 5}),
            json!({"kind": "fit", "frame": 250, "position": 5, "fit": true}),
            json!({"kind": "hold", "from": 250, "to": 350, "position": 5}),
            json!({"kind": "jump", "frame": 350, "position": 6}),
            json!({"kind": "fit", "frame": 350, "position": 6, "fit": false}),
            json!({"kind": "hold", "from": 350, "to": 1_000, "position": 6}),
        ]
    );
    let Some(Event::Move { curve, distance, peak_speed, .. }) = events.iter().find(|e| matches!(e, Event::Move { .. })) else { panic!("a move event") };
    assert_eq!(*curve, RAMP);
    assert!((distance - 1_920.0 * 100.0 / 1_080.0).abs() < 1e-9 && (peak_speed - 5.555_555_555_555_555).abs() < 1e-9);
}

#[test]
fn frame_rows_give_the_window_state_and_speed_per_frame() {
    let fx = derived_fx();
    let rows = frame_rows(&fx.project, fx.seq(), 1, 0, 1_000, 1);
    assert_eq!(rows.len(), 1_000);
    let at = |frame: i64| rows.iter().find(|r| r.frame == frame).expect("a row");
    let states: Vec<(i64, char)> = [10, 50, 75, 100, 150, 160, 170, 200, 250, 300, 350, 999].iter().map(|&f| (f, at(f).state)).collect();
    assert_eq!(
        states,
        vec![(10, 'b'), (50, 's'), (75, 'h'), (100, 'c'), (150, 'h'), (160, 'h'), (170, 'm'), (200, 'h'), (250, 'f'), (300, 'f'), (350, 'j'), (999, 'h')]
    );
    let moving = at(170);
    assert_eq!((moving.position, moving.tick, moving.layout), (Some(3), tk(170), Layout::Fill));
    assert!(moving.speed > 0.0 && moving.u > 0.0 && moving.u < 1.0);
    assert_eq!((at(100).speed, at(250).speed), (0.0, 0.0), "a cut and a fit frame carry no speed");
    assert_eq!(at(100).rect, Rect { x: 1000.0 - 303.75, y: 0.0, w: 607.5, h: 1080.0 });
    assert!((at(100).upscale - 1_920.0 / 1_080.0).abs() < 1e-12);
    assert_eq!(at(100).active, [0.0, 0.0, 1920.0, 1080.0]);
    let sparse = frame_rows(&fx.project, fx.seq(), 1, 0, 100, 25);
    assert_eq!(sparse.iter().map(|r| r.frame).collect::<Vec<i64>>(), vec![0, 25, 50, 75]);
    let mut empty = derived_fx();
    empty.plan_mut(1).positions.clear();
    assert_eq!(frame_rows(&empty.project, empty.seq(), 1, 0, 1, 1)[0].state, 'd');
    assert!(frame_rows(&fx.project, fx.seq(), 9, 0, 10, 1).is_empty(), "no plan 9");
}

#[test]
fn go_to_moves_between_positions_walls_breaks_and_marks() {
    let mut fx = derived_fx();
    fx.plan_mut(1).attention = vec![AttentionMark { at: tk(500), by: "x".into(), when: 1, note: String::new() }];
    let go = |frame: i64, to: GoTo| go_to(&fx.project, fx.seq(), 1, frame, to);
    assert_eq!((go(160, GoTo::PrevPosition), go(160, GoTo::NextPosition)), (Some(150), Some(200)));
    assert_eq!((go(160, GoTo::PrevWall), go(160, GoTo::NextWall)), (Some(100), Some(300)));
    assert_eq!((go(160, GoTo::PrevBreak), go(160, GoTo::NextBreak)), (Some(100), Some(250)), "a SAME position is no break");
    assert_eq!((go(160, GoTo::NextAttention), go(600, GoTo::NextAttention)), (Some(500), None));
    assert_eq!(go(50, GoTo::PrevPosition), None);
}

#[test]
fn the_issues_that_need_no_threshold() {
    let mut fx = derived_fx();
    fx.plan_mut(1).positions[5].flags = vec![Flag::PictureChanged];
    let media = fx.media;
    if let Some(clip) = fx.project.item_mut(media).and_then(|i| i.as_media_mut()) {
        clip.interpret.par = Some((4, 3));
    }
    let found = issues(&fx.project, fx.seq(), 1, 0, 1_000);
    let not_own = with_code(&found, "shot_not_own");
    assert_eq!(not_own.len(), 2);
    assert_eq!(not_own[0].level, Level::Warning);
    assert_eq!(not_own[0].detail, json!({"shot": 1, "from": 0, "to": 100, "status": "carried", "carriedFrom": 1}));
    let redundant = with_code(&found, "redundant_position");
    assert_eq!(redundant.len(), 1, "a SAME position that opens no shot");
    assert_eq!((redundant[0].level, redundant[0].detail.clone()), (Level::Info, json!({"position": 3})));
    assert_eq!(with_code(&found, "picture_changed")[0].detail, json!({"position": 6}));
    let nonsquare = with_code(&found, "nonsquare_source");
    assert_eq!((nonsquare.len(), nonsquare[0].level), (1, Level::Error));
    // a range keeps the issues touching it and every error
    let late = issues(&fx.project, fx.seq(), 1, 300, 1_000);
    assert_eq!(with_code(&late, "shot_not_own").len(), 1, "only shot 3's");
    assert_eq!(with_code(&late, "nonsquare_source").len(), 1, "errors are always listed");
    assert!(with_code(&late, "redundant_position").is_empty());
    assert!(codes(&late).iter().all(|c| *c != "move_in_dissolve"));
}
