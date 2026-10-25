//! T-M5: an exact split (spec section 3.6) changes no frame's `cx`, `cy` or `h` by more than its bound (`split_bound`):
//! the rounding of the stored split window plus, in a move that zooms, a travel term. `k` nested splits stay within the
//! sum of their bounds; an edge-anchored zoom stays on its edge; fixed worst cases on HD and UHD pass the spec's flat
//! 0.0005 and 0.00125 px, which leave out the travel term and the clamp's aspect term, and stay within the bound; split
//! moves read "part k/n".

use super::support::*;
use crate::vertical::*;

/// Float noise allowed on top of a bound.
const NOISE: f64 = 1e-9;

/// Half a grid step: the most rounding moves a coordinate of an unclamped split window, and the spec's flat bound.
const HALF_STEP: f64 = 0.0005;

/// The spec's flat bound for a split window that directed rounding clamps.
const CLAMPED_FLAT: f64 = 0.00125;

/// A 4:5 output.
const FOUR_FIVE: OutputSize = OutputSize { width: 1080, height: 1350 };

/// A square output.
const SQUARE: OutputSize = OutputSize { width: 1080, height: 1080 };

/// A landscape 16:9 output of a UHD sequence.
const LANDSCAPE: OutputSize = OutputSize { width: 1920, height: 1080 };

/// A landscape 16:9 output of an HD sequence.
const HD_LANDSCAPE: OutputSize = OutputSize { width: 1280, height: 720 };

/// The largest change of `cx`, of `cy` and of `h` between two plans over the frames `from..=to`.
fn largest_changes(a: &VerticalPlan, b: &VerticalPlan, r: &Raster, from: i64, to: i64) -> [f64; 3] {
    let mut worst = [0.0_f64; 3];
    for f in from..=to {
        let (wa, wb) = (window_at(a, r, tick(r, f)).window, window_at(b, r, tick(r, f)).window);
        for (w, change) in worst.iter_mut().zip([wa.cx - wb.cx, wa.cy - wb.cy, wa.h - wb.h]) {
            *w = w.max(change.abs());
        }
    }
    worst
}

/// The largest change of `cx`, `cy` or `h` between two plans over the frames `from..=to`.
fn largest_change(a: &VerticalPlan, b: &VerticalPlan, r: &Raster, from: i64, to: i64) -> f64 {
    largest_changes(a, b, r, from, to).into_iter().fold(0.0, f64::max)
}

/// The bound of one split of the move from `from` to `to`, and whether directed rounding clamped the split window.
///
/// The stored window differs from the window evaluated at the split by at most `delta` in each coordinate: half a grid
/// step, or where the clamp moves it, the inward step of 0.001 px plus the shift of the centre bounds, at most half the
/// height's rounding times `max(1, OW / OH)`. Both pieces run from or to the stored window, and its height `h'` also
/// changes their zoom schedule `G(e) = expm1(e L) / expm1(L)` (spec section 3.4) through `L` by `|ln(h' / h)|`, at most
/// `delta_h / h_lo`: `delta_h` is half a grid step (0.001 px where the limits clamp the height) and `h_lo` the move's
/// smaller end height. `|dG/dL|` never exceeds 1/8, so that moves a frame by at most an eighth of it times the move's
/// largest change of `cx`, `cy` or `h`: the travel term. A pure pan has none, since its split height is exact.
fn split_bound(split: &Split, exact: &Window, from: &Window, to: &Window, out: OutputSize) -> (f64, bool) {
    let rounded = win(tq(exact.cx), tq(exact.cy), tq(exact.h));
    let clamped = !same_bits(&split.window, &rounded);
    let aspect = f64::from(out.width) / f64::from(out.height);
    let delta = if clamped { 0.001 + 0.00025 * aspect.max(1.0) } else { HALF_STEP };
    let delta_h = if split.window.h.to_bits() == rounded.h.to_bits() { HALF_STEP } else { 0.001 };
    let travel = (to.cx - from.cx).abs().max((to.cy - from.cy).abs()).max((to.h - from.h).abs());
    let tilt = if from.h == to.h { 0.0 } else { delta_h / from.h.min(to.h) };
    (delta + travel * tilt / 8.0, clamped)
}

/// The arrival of a piece of `frames` frames at `offset` of a `total`-frame move: a cut when it is one frame long.
fn piece(frames: i64, curve: Curve, offset: i64, total: i64) -> Arrival {
    if frames == 1 { Arrival::Cut } else { mv_split(frames as u32, curve, offset as u32, total as u32) }
}

/// A plan of `out` holding `positions`.
fn plan_for(out: OutputSize, positions: Vec<Position>) -> VerticalPlan {
    VerticalPlan { output: out, ..plan_of(positions) }
}

/// Splits the move that contains `frame` and gives the new position the next id; returns the split's bound and whether
/// directed rounding clamped its window.
fn split_at(plan: &mut VerticalPlan, r: &Raster, frame: i64) -> (f64, bool) {
    let exact = window_at(plan, r, tick(r, frame)).window;
    let Some(split) = split_move(plan, r, frame) else { panic!("frame {frame} lies inside a move") };
    let (from, to) = (plan.positions[split.index - 1].window, plan.positions[split.index].window);
    let bound = split_bound(&split, &exact, &from, &to, plan.output);
    let id = plan.next_position_id;
    insert_split(plan, &split, id, Origin::Manual, Writer::Human);
    plan.next_position_id += 1;
    bound
}

/// Splits the `frames`-frame move from `from` to `to` on `r` for `out` once at each frame of `splits`, checks that each
/// split stays within its bound, and returns each split's frame, its largest changes of `cx`, `cy` and `h`, and whether
/// directed rounding clamped it.
fn each_split(r: &Raster, out: OutputSize, from: Window, to: Window, curve: Curve, frames: i64, splits: &[i64]) -> Vec<(i64, [f64; 3], bool)> {
    let plan = plan_for(out, vec![pos(r, 1, 0, from, Arrival::Cut), pos(r, 2, frames, to, mv(frames as u32, curve))]);
    let mut rows = Vec::with_capacity(splits.len());
    for &m in splits {
        let mut cut = plan.clone();
        let (allowed, clamped) = split_at(&mut cut, r, m);
        let changes = largest_changes(&plan, &cut, r, 0, frames);
        let change = changes.into_iter().fold(0.0, f64::max);
        assert!(change <= allowed + NOISE, "{from:?} -> {to:?} on {}x{}, split at {m}: {change} px against {allowed}", r.width, r.height);
        rows.push((m, changes, clamped));
    }
    rows
}

/// The label of every position's move ("" for none).
fn labels(plan: &VerticalPlan) -> Vec<String> {
    (0..plan.positions.len()).map(|i| move_label(plan, i).unwrap_or_default()).collect()
}

#[test]
fn t_m5_one_split_moves_no_frame_more_than_its_bound() {
    // the client's HD master and UHD for 9:16, and the other aspects a plan may have
    let setups = [
        (hd(), OUT, 0x7e57_0005, 400),
        (uhd(), OUT, 0x7e57_0015, 200),
        (hd(), FOUR_FIVE, 0x7e57_0025, 100),
        (uhd(), SQUARE, 0x7e57_0035, 100),
        (uhd(), LANDSCAPE, 0x7e57_0045, 100),
    ];
    for (r, out, seed, cases) in setups {
        let mut rng = Rng::new(seed);
        for case in 0..cases {
            let d = rng.int(3, 150);
            let (a, b) = (random_window(&mut rng, &r, out), random_window(&mut rng, &r, out));
            let curve = random_curve(&mut rng);
            let plan = plan_for(out, vec![pos(&r, 1, 20, a, Arrival::Cut), pos(&r, 2, 20 + d, b, mv(d as u32, curve))]);
            let m = 20 + rng.int(1, d - 1);
            let exact = window_at(&plan, &r, tick(&r, m)).window;
            let Some(split) = split_move(&plan, &r, m) else { panic!("{out:?} case {case}: frame {m} lies inside the move") };
            assert_eq!((split.index, split.at), (1, tick(&r, m)), "{out:?} case {case}");
            assert_eq!(split.arrival, piece(m - 20, curve, 0, d), "{out:?} case {case}: the first piece");
            assert_eq!(split.next, piece(20 + d - m, curve, m - 20, d), "{out:?} case {case}: the second piece");
            assert!([split.window.cx, split.window.cy, split.window.h].iter().all(|&v| tgrid(v)), "{out:?} case {case}: {:?}", split.window);
            assert!(inside(&split.window, &r, out, 1e-9), "{out:?} case {case}: {:?}", split.window);
            let mut cut = plan.clone();
            insert_split(&mut cut, &split, 3, Origin::Manual, Writer::Human);
            let added = &cut.positions[1];
            assert_eq!((added.id, added.at, added.layout, added.arrival), (3, tick(&r, m), Layout::Fill, split.arrival), "{out:?} case {case}");
            assert!(same_bits(&added.window, &split.window) && cut.positions[2].arrival == split.next, "{out:?} case {case}");
            let change = largest_change(&plan, &cut, &r, 20, 20 + d);
            let allowed = split_bound(&split, &exact, &a, &b, out).0 + NOISE;
            assert!(change <= allowed, "{out:?} case {case}: {curve:?}, split at {m} of a {d}-frame move: {change} px against {allowed}");
        }
    }
}

#[test]
fn t_m5_nested_splits_stay_within_the_sum_of_their_bounds() {
    for (r, seed, cases) in [(hd(), 0x7e57_0055, 300), (uhd(), 0x7e57_0065, 150)] {
        let mut rng = Rng::new(seed);
        for case in 0..cases {
            let d = rng.int(8, 200);
            let (a, b) = (random_window(&mut rng, &r, OUT), random_window(&mut rng, &r, OUT));
            let curve = random_curve(&mut rng);
            let plan = plan_of(vec![pos(&r, 1, 0, a, Arrival::Cut), pos(&r, 2, d, b, mv(d as u32, curve))]);
            let mut cut = plan.clone();
            let mut allowed = NOISE;
            for k in 1..=3 {
                let taken: Vec<i64> = cut.positions.iter().map(|p| r.rate.frame_at(p.at)).collect();
                let free: Vec<i64> = (1..d).filter(|f| !taken.contains(f)).collect();
                let m = free[rng.int(0, free.len() as i64 - 1) as usize];
                allowed += split_at(&mut cut, &r, m).0;
                let change = largest_change(&plan, &cut, &r, 0, d);
                assert!(change <= allowed, "{} wide, case {case}: {curve:?}, {k} splits of a {d}-frame move: {change} px against {allowed}", r.width);
            }
        }
    }
}

#[test]
fn t_m5_an_edge_anchored_zoom_stays_on_its_edge() {
    // x = 0 and y = 0 throughout: the window zooms in from the full height to 600 px about the frame's top-left corner
    let r = hd();
    let (a, b) = (win(303.75, 540.0, 1080.0), win(168.75, 300.0, 600.0));
    let mut rng = Rng::new(0x7e57_0555);
    for curve in every_curve() {
        let plan = plan_of(vec![pos(&r, 1, 0, a, Arrival::Cut), pos(&r, 2, 90, b, mv(90, curve))]);
        for f in 0..=90 {
            let [x, y, _, _] = rect_of(&window_at(&plan, &r, tick(&r, f)).window, OUT);
            assert!(x.abs() <= 1e-9 && y.abs() <= 1e-9, "{curve:?}: frame {f} has x = {x}, y = {y}");
        }
        let mut cut = plan.clone();
        let mut allowed = NOISE;
        for m in [rng.int(1, 29), rng.int(31, 59), rng.int(61, 89)] {
            allowed += split_at(&mut cut, &r, m).0;
        }
        let change = largest_change(&plan, &cut, &r, 0, 90);
        assert!(change <= allowed, "{curve:?}: {change} px against {allowed}");
        for f in 0..=90 {
            let w = window_at(&cut, &r, tick(&r, f)).window;
            let [x, y, _, _] = rect_of(&w, OUT);
            assert!(inside(&w, &r, OUT, 1e-9), "{curve:?}: frame {f} leaves the frame: {w:?}");
            assert!(x <= allowed && y <= allowed, "{curve:?}: frame {f} left the edge: x = {x}, y = {y}");
        }
    }
}

#[test]
fn t_m5_long_pans_with_a_slight_zoom_pass_half_a_grid_step() {
    // near-full-width pans at h_min with a zoom under 1 % (found by search on a model of this evaluator): rounding the
    // split height tilts the zoom schedule, which moves a pan of 1 600 px (HD) or 3 500 px (UHD) by up to an eighth of
    // the tilt times its travel; where that adds to the rounding of cx, a split passes 0.0005 px and stays in its bound
    let ramp = Curve::Ramp { r_in: 0.2, r_out: 0.2 };
    let cases: [(Raster, Window, Window, Curve, i64, &[i64]); 5] = [
        (hd(), win(1765.92, 540.0, 480.0), win(152.132, 540.0, 482.013), Curve::EaseInOut, 115, &[108]),
        (hd(), win(137.452, 540.0, 482.161), win(1777.868, 540.0, 480.0), Curve::EaseOut, 80, &[71]),
        (hd(), win(1782.147, 540.0, 484.1), win(152.898, 540.0, 480.0), Curve::EaseInOut, 53, &[7]),
        (hd(), win(1776.703, 540.0, 480.0), win(140.922, 540.0, 481.019), Curve::Ramp { r_in: 0.5, r_out: 0.5 }, 57, &[2]),
        (uhd(), win(3696.285, 540.0, 505.707), win(154.237, 540.0, 480.0), ramp, 188, &[181, 157, 184, 146]),
    ];
    for (r, from, to, curve, frames, splits) in cases {
        for (m, changes, clamped) in each_split(&r, OUT, from, to, curve, frames, splits) {
            assert!(!clamped && changes[0] > HALF_STEP, "{from:?} -> {to:?}, split at {m}: {changes:?} must pass half a grid step in cx, unclamped");
        }
    }
}

#[test]
fn t_m5_a_deep_zoom_moves_h_by_more_than_half_a_grid_step() {
    // a zoom about the centre from the full UHD height to near h_min: in a piece whose end heights differ more than
    // e = 2.718 times, rounding the split height moves h itself by more than that rounding; cx and cy do not move
    let (from, to) = (win(1920.0, 1080.0, 2160.0), win(1920.0, 1080.0, 481.221));
    for (m, changes, clamped) in each_split(&uhd(), OUT, from, to, Curve::EaseOut, 115, &[113, 109, 92]) {
        assert!(!clamped && changes[2] > HALF_STEP && changes[0] == 0.0 && changes[1] == 0.0, "split at {m}: {changes:?}");
    }
}

#[test]
fn t_m5_nested_splits_of_a_long_pan_pass_their_flat_sum() {
    // two splits of the UHD pan above move a frame by more than twice half a grid step, within the sum of their bounds
    let r = uhd();
    let (from, to) = (win(3696.285, 540.0, 505.707), win(154.237, 540.0, 480.0));
    let plan = plan_of(vec![pos(&r, 1, 0, from, Arrival::Cut), pos(&r, 2, 188, to, mv(188, Curve::Ramp { r_in: 0.2, r_out: 0.2 }))]);
    let mut cut = plan.clone();
    let mut allowed = NOISE;
    for m in [181, 50] {
        let (bound, clamped) = split_at(&mut cut, &r, m);
        assert!(!clamped, "split at {m}");
        allowed += bound;
    }
    let change = largest_change(&plan, &cut, &r, 0, 188);
    assert!(change > 2.0 * HALF_STEP && change <= allowed, "{change} px against {allowed}");
}

#[test]
fn t_m5_a_landscape_output_clamps_by_more_than_0_00125_px() {
    // zooms about an edge for 16:9 outputs of UHD and HD: where directed rounding clamps the split window, the bounds of
    // cx move by up to half the height's rounding times OW / OH, 0.00044 px here, more than the 0.00025 px of cy's
    // bounds that the spec's flat 0.00125 px counts; a split passes it and stays within 0.001 + 0.00025 * OW / OH px
    // plus its travel term
    let cases: [(Raster, OutputSize, Window, Window, i64, &[i64]); 2] = [
        (uhd(), LANDSCAPE, win(1920.0, 1080.0, 2160.0), win(1539.224, 865.814, 1731.627), 114, &[106, 96, 91]),
        (hd(), HD_LANDSCAPE, win(802.4, 540.0, 902.7), win(960.0, 540.0, 1080.0), 108, &[40, 87]),
    ];
    for (r, out, from, to, frames, splits) in cases {
        for (m, changes, clamped) in each_split(&r, out, from, to, Curve::Linear, frames, splits) {
            assert!(clamped && changes[0] > CLAMPED_FLAT, "{out:?}, split at {m}: {changes:?} must pass 0.00125 px in cx, clamped");
        }
    }
}

#[test]
fn t_m5_split_moves_read_part_k_of_n() {
    let r = hd();
    let (a, b) = (win(600.0, 540.0, 1080.0), win(1200.0, 540.0, 1080.0));
    let whole = plan_of(vec![pos(&r, 1, 0, a, Arrival::Cut), pos(&r, 2, 50, b, mv(50, Curve::EaseInOut))]);
    assert_eq!(labels(&whole), ["", "Ease in-out"]);
    let mut plan = whole.clone();
    split_at(&mut plan, &r, 20);
    assert_eq!(labels(&plan), ["", "Ease in-out · part 1/2", "Ease in-out · part 2/2"]);
    split_at(&mut plan, &r, 35);
    assert_eq!(labels(&plan), ["", "Ease in-out · part 1/3", "Ease in-out · part 2/3", "Ease in-out · part 3/3"]);
    // a one-frame piece becomes a cut, so the rest no longer tiles the move
    let mut plan = whole.clone();
    split_at(&mut plan, &r, 1);
    assert_eq!(plan.positions[1].arrival, Arrival::Cut);
    assert_eq!(labels(&plan), ["", "", "Ease in-out · frames 1–50 of 50"]);
    let mut plan = whole;
    split_at(&mut plan, &r, 30);
    plan.positions[1].arrival = Arrival::Cut;
    assert_eq!(labels(&plan), ["", "", "Ease in-out · frames 30–50 of 50"]);
    // the names of whole moves
    let names = [
        (Curve::Linear, "Linear"),
        (Curve::EaseIn, "Ease in"),
        (Curve::EaseOut, "Ease out"),
        (Curve::Speeds { start: 0.0, end: 1.75 }, "Speeds"),
        (Curve::Bezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 }, "Custom"),
        (Curve::Ramp { r_in: 0.2, r_out: 0.2 }, "Ramp 20/20"),
        (Curve::Ramp { r_in: 0.3, r_out: 0.1 }, "Ramp 30/10"),
    ];
    for (curve, name) in names {
        let plan = plan_of(vec![pos(&r, 1, 0, a, Arrival::Cut), pos(&r, 2, 50, b, mv(50, curve))]);
        assert_eq!(labels(&plan), ["", name], "{curve:?}");
    }
}

#[test]
fn t_m5_frames_that_are_not_inside_a_move_do_not_split() {
    let r = hd();
    let plan = plan_of(vec![pos(&r, 1, 10, win(600.0, 540.0, 1080.0), Arrival::Cut), pos(&r, 2, 60, win(900.0, 540.0, 1080.0), mv(30, Curve::Linear))]);
    // the hold before the move, its start frame (a SAME position needs no split), the arrival, a cut and outside the plan
    for frame in [0, 10, 25, 30, 60, 61, 500] {
        assert_eq!(split_move(&plan, &r, frame), None, "frame {frame}");
    }
    assert!(split_move(&plan, &r, 31).is_some() && split_move(&plan, &r, 59).is_some());
}

#[test]
fn t_m5_splits_leave_the_lock_check_to_their_caller() {
    // I16 is the caller's: the planner refuses a split into a locked position, while the edit hook may split a locked
    // position's move at a junction (spec section 4.8), so the building blocks split a locked arriving position too
    let r = hd();
    let mut plan = plan_of(vec![pos(&r, 1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut), pos(&r, 2, 50, win(900.0, 540.0, 1080.0), mv(50, Curve::Linear))]);
    plan.positions[1].locked = true;
    let Some(split) = split_move(&plan, &r, 20) else { panic!("frame 20 lies inside the move") };
    insert_split(&mut plan, &split, 3, Origin::Reanchored, Writer::Human);
    assert_eq!(plan.positions[2].arrival, mv_split(30, Curve::Linear, 20, 50));
    assert!(plan.positions[2].locked && !plan.positions[1].locked);
}
