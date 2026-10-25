//! The motion measures (spec section 3.9). T-M22: measures use the arriving speed, and the reductions give the same bits on
//! 1 and 8 threads. T-M28 (all but the subject-box eye trace and look room, which need box tracks, M5): jump shift and
//! scale with `jump_similar` only for the caller's thresholds, hold after a wall, small moves, eye-trace jumps and
//! directions across every window break (a CUT and a JUMP) from focus points, framing spread per label, jump rhythm, fit
//! share and flips, caption-anchor flips, `cue_straddles_cut` and the raw cue distances, sustained speed, settle and still
//! share; reversals ignore the jumps of a live-switched two-shot. T-M26 (its centre-crop baseline clause): the baseline
//! returns its own numbers beside the plan's.

use std::thread;

use serde_json::json;

use super::fixtures::*;
use super::support::{mv, win};
use crate::vertical::*;

fn opts(thresholds: Thresholds) -> MeasureOpts {
    MeasureOpts { thresholds, ..Default::default() }
}

fn measured(fx: &Fx, opts: &MeasureOpts) -> Measures {
    measure(&fx.project, fx.seq(), 1, opts).expect("measured")
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() < 1e-9, "{what}: {a} against {b}");
}

#[test]
fn t_m22_measures_use_the_arriving_speed() {
    // a linear pan of 100 sequence px over 10 frames: frames 101-110 arrive at 10 px per frame, 17.78 output px
    let mut fx = Fx::new(300);
    fx.install(one_plan(vec![at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut), at25(2, 110, win(800.0, 540.0, 1080.0), mv(10, Curve::Linear))]));
    let m = measured(&fx, &MeasureOpts { from_frame: Some(95), to_frame: Some(115), series: true, ..Default::default() });
    let series = m.series.expect("the per-frame series");
    assert_eq!(series.frames, (95..115).collect::<Vec<i64>>());
    for (f, v) in series.frames.iter().zip(&series.speed) {
        let expected = if (101..=110).contains(f) { 1_920.0 / 108.0 } else { 0.0 };
        close(*v, expected, &format!("frame {f}"));
    }
    let mv0 = m.moves[0];
    assert_eq!((mv0.position, mv0.from, mv0.start, mv0.end, mv0.frames), (2, 1, 100, 110, 10));
    close(mv0.distance, 100.0 * 1_920.0 / 1_080.0, "distance");
    close(mv0.mean_speed, 1_920.0 / 108.0, "mean speed");
    close(mv0.peak_speed, 1_920.0 / 108.0, "peak speed");
    close(mv0.start_accel, 1_920.0 / 108.0, "leaving rest");
    close(mv0.end_accel, 1_920.0 / 108.0, "coming to rest");
    close(mv0.peak_zoom_rate, 0.0, "a pan does not zoom");
}

/// Every threshold, for the determinism checks.
fn all_thresholds() -> Thresholds {
    Thresholds {
        max_upscale: Some(1.5),
        max_speed: Some(0.5),
        max_accel: Some(0.1),
        min_hold_frames: Some(40),
        ping_pong_frames: Some(50),
        near_wall_frames: Some(2),
        max_speed_step: Some(0.1),
        jump_similar: Some(JumpSimilar { max_shift: 0.15, max_scale: 1.10 }),
        min_move_distance: Some(32.4),
        min_hold_after_wall: Some(40),
        max_hold_frames: Some(150),
        straddle_frames: Some(3),
        max_eye_trace_jump: Some(0.18),
    }
}

#[test]
fn t_m22_and_t_m28_reductions_give_the_same_bits_on_1_and_8_threads() {
    let fx = holds();
    let opts = MeasureOpts { thresholds: all_thresholds(), series: true, baseline: true, protected: true, ..Default::default() };
    let single = measured(&fx, &opts);
    assert!(!single.moves.is_empty() && !single.findings.is_empty(), "there is something to reduce");
    let once = format!("{single:?}");
    let results: Vec<String> = thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(scope.spawn(|| format!("{:?}", measured(&fx, &opts))));
        }
        handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    for (n, r) in results.iter().enumerate() {
        assert_eq!(r, &once, "thread {n}");
    }
}

#[test]
fn t_m28_a_punch_in_is_never_too_similar_and_a_nudge_is() {
    let similar = Thresholds { jump_similar: Some(JumpSimilar { max_shift: 0.15, max_scale: 1.10 }), ..Default::default() };
    let mut fx = Fx::new(300);
    fx.install(one_plan(vec![at25(1, 0, win(960.0, 540.0, 1080.0), Arrival::Cut), at25(2, 100, win(960.0, 540.0, 939.13), Arrival::Cut)]));
    let m = measured(&fx, &opts(similar));
    close(m.jumps[0].shift, 0.0, "a punch-in about the centre");
    close(m.jumps[0].scale, 1.150_000_532_407_653_9, "1.15x");
    assert!(with_code(&m.findings, "jump_similar").is_empty(), "a punch-in (scale >= 1.10) is never too similar");
    let mut fx = Fx::new(300);
    fx.install(one_plan(vec![at25(1, 0, win(960.0, 540.0, 1080.0), Arrival::Cut), at25(2, 100, win(931.071, 540.0, 1028.571), Arrival::Cut)]));
    let m = measured(&fx, &opts(similar));
    close(m.jumps[0].shift, 0.050_000_761_574_391_36, "0.05 output widths");
    close(m.jumps[0].scale, 1.050_000_437_500_182_3, "1.05x");
    let found = with_code(&m.findings, "jump_similar");
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].detail["position"].clone(), found[0].detail["frame"].clone()), (json!(2), json!(100)));
    assert!(with_code(&measured(&fx, &MeasureOpts::default()).findings, "jump_similar").is_empty(), "no threshold, no finding");
}

/// A wall at 100; a cut on it; a 20-frame linear move of 10 sequence px into 150 (a small move, from frame 130); jumps at
/// 250, 300 and 330.
fn holds() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 300, 5_000)]);
    let mut set = one_plan(vec![
        at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 100, win(1000.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 150, win(1010.0, 540.0, 1080.0), mv(20, Curve::Linear)),
        at25(4, 250, win(900.0, 540.0, 1080.0), Arrival::Cut),
        at25(5, 300, win(1100.0, 540.0, 1080.0), Arrival::Cut),
        at25(6, 330, win(800.0, 540.0, 1080.0), Arrival::Cut),
    ]);
    set.regions = vec![ProtectedRegion {
        id: 1,
        label: "logo".into(),
        from: tk(0),
        to: tk(400),
        rect: [1700.0, 40.0, 180.0, 90.0],
        mode: RegionMode::AllOrNothing,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    }];
    fx.install(set);
    fx
}

#[test]
fn t_m28_hold_after_a_wall_small_moves_settle_still_share_jump_rhythm_and_sustained_speed() {
    let fx = holds();
    let thresholds = Thresholds { min_move_distance: Some(32.4), min_hold_after_wall: Some(40), max_speed: Some(0.5), ..Default::default() };
    let m = measured(&fx, &opts(thresholds));
    let shot = m.shots.iter().find(|s| s.number == 2).expect("shot 2");
    assert_eq!((shot.from, shot.to, shot.status), (100, 400, ShotStatus::Own));
    assert_eq!((shot.positions, shot.moves, shot.jumps), (5, 1, 3));
    assert_eq!(shot.hold_after_wall, Some(31), "frame 131 is the shot's first moving frame");
    assert_eq!(shot.settle, 50, "the window stops moving on frame 150");
    close(shot.still_share, 280.0 / 300.0, "20 of its 300 frames move");
    assert_eq!(shot.jump_rhythm, Some([30, 50]));
    let first = m.shots.iter().find(|s| s.number == 1).expect("shot 1");
    assert_eq!((first.hold_after_wall, first.jump_rhythm, first.settle), (None, None, 0));
    close(m.moves[0].distance, 1_920.0 / 108.0, "10 sequence px at full height");
    let small = with_code(&m.findings, "small_move");
    assert_eq!(small.len(), 1);
    assert_eq!(small[0].detail["position"], json!(3));
    let early = with_code(&m.findings, "early_move");
    assert_eq!(early.len(), 1);
    assert_eq!(early[0].detail["frames"], json!(31));
    assert_eq!(m.totals.sustained_speed, Some(20), "frames 131-150 arrive at 0.89 output px per frame");
    assert_eq!(measured(&fx, &MeasureOpts::default()).totals.sustained_speed, None, "only with maxSpeed");
    assert!(with_code(&measured(&fx, &MeasureOpts::default()).findings, "small_move").is_empty());
}

#[test]
fn t_m28_eye_trace_across_a_cut_and_a_jump_from_focus_points() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 300, 5_000)]);
    fx.install(one_plan(vec![
        Position { focus: Some([700.0, 400.0]), ..at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut) },
        Position { focus: Some([1100.0, 450.0]), ..at25(2, 100, win(1000.0, 540.0, 1080.0), Arrival::Cut) },
        Position { focus: Some([900.0, 500.0]), ..at25(3, 200, win(900.0, 540.0, 900.0), Arrival::Cut) },
        at25(4, 250, win(950.0, 540.0, 900.0), Arrival::Cut),
    ]));
    let m = measured(&fx, &opts(Thresholds { max_eye_trace_jump: Some(0.18), ..Default::default() }));
    let at = |frame: i64| m.breaks.iter().find(|b| b.frame == frame).copied().expect("a break");
    let cut = at(100);
    assert_eq!(cut.kind, BreakKind::Cut);
    let trace = cut.eye_trace.expect("both frames have a focus");
    close(trace.before[0], 540.0, "x before");
    close(trace.before[1], 711.111_111_111_111_1, "y before");
    close(trace.after[0], 717.777_777_777_777_8, "x after");
    close(trace.after[1], 800.0, "y after");
    close(trace.distance, 0.184_038_516_666_649_4, "distance");
    let jump = at(200);
    assert_eq!(jump.kind, BreakKind::Jump);
    close(jump.eye_trace.expect("measured").distance, 0.178_538_230_294_074_06, "the JUMP is measured too");
    assert!(at(250).eye_trace.is_none(), "no focus governs frame 250: not measured");
    let found = with_code(&m.findings, "eye_trace_jump");
    assert_eq!(found.len(), 1, "only the cut exceeds 0.18");
    assert_eq!(found[0].detail["frame"], json!(100));
}

#[test]
fn t_m28_the_direction_of_motion_across_a_break() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 300, 5_000)]);
    fx.install(one_plan(vec![
        at25(1, 0, win(500.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 99, win(800.0, 540.0, 1080.0), mv(20, Curve::Linear)),
        at25(3, 100, win(1000.0, 540.0, 1080.0), Arrival::Cut),
        at25(4, 130, win(900.0, 540.0, 1080.0), mv(30, Curve::Linear)),
    ]));
    let m = measured(&fx, &MeasureOpts::default());
    let cut = m.breaks.iter().find(|b| b.frame == 100).expect("the cut");
    let [before, after] = cut.direction.expect("the window moves on both sides");
    close(before, 0.0, "moving right before the cut");
    close(after.abs(), 180.0, "moving left after it");
}

#[test]
fn t_m28_framing_spread_per_label() {
    let mut fx = Fx::new(400);
    let dana = |id: u32, frame: i64, w: Window| Position { label: "Dana".into(), ..at25(id, frame, w, Arrival::Cut) };
    fx.install(one_plan(vec![
        dana(1, 0, win(700.0, 540.0, 1080.0)),
        at25(2, 50, win(1200.0, 540.0, 1080.0), Arrival::Cut),
        dana(3, 100, win(720.0, 530.0, 1000.0)),
        dana(5, 200, win(680.0, 550.0, 900.0)),
    ]));
    let m = measured(&fx, &MeasureOpts::default());
    let spread = m.labels.iter().find(|l| l.label == "Dana").expect("the label");
    assert_eq!(spread.positions, 3);
    close(spread.cx, 16.329_931_618_554_52, "cx");
    close(spread.cy, 8.164_965_809_277_26, "cy");
    close(spread.h, 73.635_740_114_581_74, "h");
    assert_eq!(spread.focus, None, "no focus point");
    assert_eq!(m.labels.len(), 1, "an unlabelled position is in no label");
}

#[test]
fn t_m28_fit_share_and_flips_and_caption_anchor_flips_per_part() {
    let mut fx = Fx::new(400);
    let mut set = one_plan(vec![
        at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut),
        Position { layout: Layout::Fit, ..at25(2, 100, win(960.0, 540.0, 1080.0), Arrival::Cut) },
        at25(3, 200, win(800.0, 540.0, 1080.0), Arrival::Cut),
        Position { layout: Layout::Fit, ..at25(4, 300, win(960.0, 540.0, 1080.0), Arrival::Cut) },
    ]);
    set.parts = vec![part25(1, "P01", 0, 400)];
    fx.install(set);
    let part = measured(&fx, &MeasureOpts::default()).parts[0];
    close(part.fit_share, 0.5, "frames 100-199 and 300-399");
    assert_eq!(part.fit_changes, 3);
    // a minute of 25p: cues governed by the plan's anchor (bottom), a top opener, the plan's again, an explicit bottom
    let mut fx = Fx::new(1_500);
    let mut set = one_plan(vec![
        at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut),
        Position { caption_anchor: Some(CaptionAnchor::Top), ..at25(2, 150, win(800.0, 540.0, 1080.0), Arrival::Cut) },
        at25(3, 450, win(900.0, 540.0, 1080.0), Arrival::Cut),
        Position { caption_anchor: Some(CaptionAnchor::Bottom), ..at25(4, 850, win(1000.0, 540.0, 1080.0), Arrival::Cut) },
    ]);
    set.parts = vec![part25(1, "P01", 0, 1_500)];
    fx.install(set);
    add_cues(&mut fx, &[(10, 40), (200, 230), (500, 530), (900, 930)]);
    close(measured(&fx, &MeasureOpts::default()).parts[0].anchor_flips_per_minute, 2.0, "bottom, top, bottom, bottom");
}

/// A wall at 100, a cut on it and jumps at 200, 300 and 400; cues straddling the cut by 2 frames before it, ending 2
/// frames before the jump at 200, starting on the jump at 300, and straddling the jump at 400 by 2 frames after it.
fn cues() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 400, 5_000)]);
    fx.install(one_plan(vec![
        at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 100, win(1000.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 200, win(800.0, 540.0, 1080.0), Arrival::Cut),
        at25(4, 300, win(1100.0, 540.0, 1080.0), Arrival::Cut),
        at25(5, 400, win(900.0, 540.0, 1080.0), Arrival::Cut),
    ]));
    add_cues(&mut fx, &[(98, 130), (150, 198), (300, 330), (390, 402)]);
    fx
}

#[test]
fn t_m28_a_cue_straddling_a_cut_by_a_few_frames() {
    let fx = cues();
    let m = measured(&fx, &opts(Thresholds { straddle_frames: Some(3), ..Default::default() }));
    let mut found: Vec<serde_json::Value> = with_code(&m.findings, "cue_straddles_cut").iter().map(|i| i.detail.clone()).collect();
    found.sort_by_key(|d| d["frame"].as_i64());
    assert_eq!(
        found,
        vec![
            json!({"position": 2, "frame": 100, "cue": 1, "before": 2, "after": 30}),
            json!({"position": 5, "frame": 400, "cue": 4, "before": 10, "after": 2}),
        ],
        "not for the cue ending 2 frames before the jump at 200, nor for the jump on a cue start at 300"
    );
    let distances: Vec<(i64, Option<i64>, Option<i64>)> = m.breaks.iter().map(|b| (b.frame, b.cue_start, b.cue_end)).collect();
    assert_eq!(distances, vec![(100, Some(-2), Some(30)), (200, Some(-50), Some(-2)), (300, Some(0), Some(30)), (400, Some(-10), Some(2))]);
    assert!(with_code(&measured(&fx, &MeasureOpts::default()).findings, "cue_straddles_cut").is_empty(), "only with straddleFrames");
}

#[test]
fn t_m28_reversals_ignore_the_jumps_of_a_live_switched_two_shot() {
    let mut fx = Fx::new(400);
    let (left, right) = (win(600.0, 540.0, 1080.0), win(1300.0, 540.0, 1080.0));
    fx.install(one_plan(vec![
        at25(1, 0, left, Arrival::Cut),
        at25(2, 50, right, Arrival::Cut),
        at25(3, 100, left, Arrival::Cut),
        at25(4, 150, right, Arrival::Cut),
        at25(5, 200, win(1400.0, 540.0, 1080.0), mv(20, Curve::Linear)),
        at25(6, 230, right, mv(20, Curve::Linear)),
    ]));
    let m = measured(&fx, &opts(Thresholds { ping_pong_frames: Some(50), ..Default::default() }));
    assert_eq!(m.shots[0].reversals, vec![211], "the move back starts 11 frames after the move out ended; the jumps are not motion");
    let found = with_code(&m.findings, "ping_pong");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].detail["frame"], json!(211));
    assert!(measured(&fx, &MeasureOpts::default()).shots[0].reversals.is_empty(), "only with pingPongFrames");
}

#[test]
fn t_m26_the_centre_crop_baseline_returns_its_own_numbers_beside_the_plans() {
    let mut fx = Fx::new(100);
    let mut set = one_plan(vec![at25(1, 0, win(303.75, 540.0, 1080.0), Arrival::Cut)]);
    set.regions = vec![ProtectedRegion {
        id: 1,
        label: "face".into(),
        from: tk(0),
        to: tk(100),
        rect: [50.0, 100.0, 200.0, 200.0],
        mode: RegionMode::KeepIn,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    }];
    fx.install(set);
    let m = measured(&fx, &MeasureOpts { protected: true, baseline: true, ..Default::default() });
    assert_eq!(m.regions[0].frames_in, 100, "the plan frames the face");
    let baseline = m.baseline.as_deref().expect("the centre-crop baseline");
    assert_eq!((baseline.regions[0].frames_in, baseline.regions[0].frames_out), (0, 100), "the centre crop cuts it off");
    assert!(baseline.moves.is_empty() && baseline.jumps.is_empty() && baseline.baseline.is_none());
    close(baseline.totals.max_upscale, 1_920.0 / 1_080.0, "the centred largest window");
    assert_eq!((baseline.from_frame, baseline.to_frame), (m.from_frame, m.to_frame), "over the same frames");
    assert!(!with_code(&baseline.findings, "region_cut").is_empty());
    assert!(with_code(&m.findings, "region_cut").is_empty());
    assert!(measured(&fx, &MeasureOpts { protected: true, ..Default::default() }).baseline.is_none(), "only when asked");
}
