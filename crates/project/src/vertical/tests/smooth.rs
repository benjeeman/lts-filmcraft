//! T-M12: Smooth through (spec section 3.7). Junction speeds equal within the 1e-6 rounding, ramps monotone where the
//! floors apply, every curve valid and every window inside, the values of the section's example, and moving ends.

use super::support::*;
use crate::vertical::*;

/// `V(delta; at)` of spec section 3.7, written again from the spec: the largest corner speed in output pixels.
fn speed(delta: [f64; 3], at: &Window) -> f64 {
    let (ow, oh) = (f64::from(OUT.width), f64::from(OUT.height));
    let [dcx, dcy, dh] = delta;
    let gx = (dcx - dh * ow / (2.0 * oh)) * oh / at.h;
    let gy = (dcy - dh / 2.0) * oh / at.h;
    let mut largest: f64 = 0.0;
    for (ox, oy) in [(0.0, 0.0), (ow, 0.0), (0.0, oh), (ow, oh)] {
        let (dx, dy) = (-(ox * dh / at.h) - gx, -(oy * dh / at.h) - gy);
        largest = largest.max((dx * dx + dy * dy).sqrt());
    }
    largest
}

/// `G'(0)` and `G'(1)` of the path from `a` to `b`: 1 for a pan.
fn rates(a: &Window, b: &Window) -> (f64, f64) {
    if a.h == b.h {
        return (1.0, 1.0);
    }
    let l = libm::log(b.h / a.h);
    let m = libm::expm1(l);
    (l / m, l * (1.0 + m) / m)
}

/// The change from `a` to `b`.
fn delta(a: &Window, b: &Window) -> [f64; 3] {
    [b.cx - a.cx, b.cy - a.cy, b.h - a.h]
}

/// The start and end speeds of a whole `speeds` move; None for anything else.
fn speeds_of(arrival: &Arrival) -> Option<(f64, f64)> {
    match *arrival {
        Arrival::Move { curve: Curve::Speeds { start, end }, split_of: None, .. } => Some((start, end)),
        _ => None,
    }
}

/// One junction of a smoothed run: the screen speed leaving the move into the waypoint, the one entering the move out of
/// it, and the factors `A_k` and `B_k+1` (output pixels per frame per unit of curve speed).
#[derive(Clone, Copy, Debug)]
struct Junction {
    leaving: f64,
    entering: f64,
    a: f64,
    b: f64,
}

/// Every junction of a smoothed run, computed from the spec's formulas.
fn junctions(waypoints: &[Position], arrivals: &[Arrival], r: &Raster) -> Vec<Junction> {
    let mut out = Vec::new();
    for k in 1..waypoints.len().saturating_sub(1) {
        let (w0, w1, w2) = (&waypoints[k - 1].window, &waypoints[k].window, &waypoints[k + 1].window);
        let d1 = (r.rate.frame_at(waypoints[k].at) - r.rate.frame_at(waypoints[k - 1].at)) as f64;
        let d2 = (r.rate.frame_at(waypoints[k + 1].at) - r.rate.frame_at(waypoints[k].at)) as f64;
        let a = speed(delta(w0, w1), w1) * rates(w0, w1).1 / d1;
        let b = speed(delta(w1, w2), w1) * rates(w1, w2).0 / d2;
        let beta = arrivals.get(k - 1).and_then(speeds_of).map_or(f64::NAN, |s| s.1);
        let alpha = arrivals.get(k).and_then(speeds_of).map_or(f64::NAN, |s| s.0);
        out.push(Junction { leaving: beta * a, entering: alpha * b, a, b });
    }
    out
}

/// The plan with `arrivals` written into the waypoints after the first.
fn with_arrivals(plan: &VerticalPlan, arrivals: &[Arrival]) -> VerticalPlan {
    let mut smoothed = plan.clone();
    for (p, arrival) in smoothed.positions.iter_mut().skip(1).zip(arrivals) {
        p.arrival = *arrival;
    }
    smoothed
}

/// A random run of `n` waypoints at least 2 frames apart, with pans and zooms.
fn random_run(rng: &mut Rng, r: &Raster, n: usize) -> VerticalPlan {
    let mut frame = rng.int(0, 20);
    let mut positions = Vec::new();
    for id in 1..=n {
        positions.push(pos(r, id as u32, frame, random_window(rng, r, OUT), Arrival::Cut));
        frame += rng.int(2, 80);
    }
    plan_of(positions)
}

fn ids(plan: &VerticalPlan) -> Vec<u32> {
    plan.positions.iter().map(|p| p.id).collect()
}

fn code(result: Result<Vec<Arrival>, Refusal>) -> Option<&'static str> {
    result.err().map(|refusal| refusal.code)
}

/// The curve's speed on 201 points of `[0, 1]`.
fn profile(curve: &Curve) -> Vec<f64> {
    (0..=200).map(|k| curve::slope(curve, f64::from(k) / 200.0)).collect()
}

#[test]
fn t_m12_the_example_of_section_3_7() {
    // a pan at full height, k = 0.5625: cx 600, 900 and 1250 on frames 0, 50 and 100
    let r = hd();
    let plan = plan_of(vec![
        pos(&r, 1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        pos(&r, 2, 50, win(900.0, 540.0, 1080.0), Arrival::Cut),
        pos(&r, 3, 100, win(1250.0, 540.0, 1080.0), Arrival::Cut),
    ]);
    let arrivals = smooth_through(&plan, &r, &[1, 2, 3], SmoothOpts::default()).expect("a valid run");
    assert_eq!(arrivals.len(), 2, "{arrivals:?}");
    for arrival in &arrivals {
        assert!(matches!(arrival, Arrival::Move { move_frames: 50, split_of: None, .. }), "{arrival:?}");
    }
    let ((a1, b1), (a2, b2)) = (speeds_of(&arrivals[0]).unwrap_or_default(), speeds_of(&arrivals[1]).unwrap_or_default());
    assert_eq!((a1, b2), (0.0, 0.0), "from rest and to rest");
    assert!((b1 - 1.75).abs() <= 1e-6 && (a2 - 1.5).abs() <= 1e-6, "beta_1 = {b1}, alpha_2 = {a2}");
    let j = junctions(&plan.positions, &arrivals, &r)[0];
    assert!((j.a - 300.0 / 0.5625 / 50.0).abs() < 1e-9 && (j.b - 350.0 / 0.5625 / 50.0).abs() < 1e-9, "{j:?}");
    let v1 = 1.5 * 350.0 / 0.5625 / 50.0;
    assert!((j.leaving - v1).abs() < 1e-4 && (j.entering - v1).abs() < 1e-4, "18.67 px per frame on both sides: {j:?}");
}

#[test]
fn t_m12_speeds_are_continuous_at_every_junction() {
    let r = hd();
    let mut rng = Rng::new(0x7e57_0012);
    for case in 0..300 {
        let n = rng.int(3, 7) as usize;
        let plan = random_run(&mut rng, &r, n);
        let arrivals = smooth_through(&plan, &r, &ids(&plan), SmoothOpts::default()).expect("a valid run");
        assert_eq!(arrivals.len(), n - 1, "case {case}");
        for (k, j) in junctions(&plan.positions, &arrivals, &r).into_iter().enumerate() {
            // each curve speed is rounded down onto the 1e-6 grid: a speed of `A` per unit moves by less than 1e-6 A
            let allowed = (1e-6 + 1e-9) * j.a.max(j.b) + 1e-12;
            assert!((j.leaving - j.entering).abs() <= allowed, "case {case}, junction {}: {j:?}", k + 1);
        }
        // every move takes its whole gap, and the run starts and ends at rest
        for (k, arrival) in arrivals.iter().enumerate() {
            let gap = r.rate.frame_at(plan.positions[k + 1].at) - r.rate.frame_at(plan.positions[k].at);
            assert!(matches!(*arrival, Arrival::Move { move_frames, split_of: None, .. } if i64::from(move_frames) == gap), "case {case}");
        }
        assert_eq!(speeds_of(&arrivals[0]).map(|s| s.0), Some(0.0), "case {case}");
        assert_eq!(speeds_of(&arrivals[n - 2]).map(|s| s.1), Some(0.0), "case {case}");
    }
}

#[test]
fn t_m12_every_curve_is_valid_and_every_window_inside() {
    let r = hd();
    let mut rng = Rng::new(0x7e57_0121);
    for case in 0..200 {
        let n = rng.int(3, 6) as usize;
        let plan = random_run(&mut rng, &r, n);
        let opts = SmoothOpts { start_from_rest: rng.chance(0.5), end_at_rest: rng.chance(0.5) };
        let arrivals = smooth_through(&plan, &r, &ids(&plan), opts).expect("a valid run");
        assert_eq!(arrivals.len(), n - 1, "case {case}");
        for arrival in &arrivals {
            let Some((a, b)) = speeds_of(arrival) else { panic!("case {case}: {arrival:?} is not a speeds move") };
            assert!((0.0..=3.0).contains(&a) && (0.0..=3.0).contains(&b), "case {case}: speeds [{a}, {b}]");
            assert!(3.0 - a - b + (a * b).sqrt() >= 0.0, "case {case}: speeds [{a}, {b}] break the monotone rule");
            for v in [a, b] {
                assert!(((v * 1e6).round() - v * 1e6).abs() < 1e-6, "case {case}: {v} is off the 1e-6 grid");
            }
            if let Arrival::Move { curve, .. } = arrival {
                assert!(check_curve(curve).is_ok(), "case {case}: {curve:?}");
            }
        }
        let smoothed = with_arrivals(&plan, &arrivals);
        let last = r.rate.frame_at(smoothed.positions[n - 1].at);
        for f in 0..=last {
            let w = window_at(&smoothed, &r, tick(&r, f)).window;
            assert!(inside(&w, &r, OUT, 1e-6), "case {case}, frame {f}: {w:?}");
        }
    }
}

#[test]
fn t_m12_ramps_are_monotone_where_the_floors_apply() {
    let r = hd();
    let mut rng = Rng::new(0x7e57_0122);
    let mut checked = 0;
    for case in 0..300 {
        let n = rng.int(3, 6) as usize;
        let segments = n - 1;
        let plan = random_run(&mut rng, &r, n);
        let arrivals = smooth_through(&plan, &r, &ids(&plan), SmoothOpts::default()).expect("a valid run");
        assert_eq!(arrivals.len(), segments, "case {case}");
        let all = junctions(&plan.positions, &arrivals, &r);
        let (first, last) = (all[0], all[all.len() - 1]);
        let two = segments == 2;
        let inner_cap = if two { 3.0 } else { 1.5 };
        // the first junction: floor 1.5 A (the larger of 1.5 A and 1.5 B with two segments), caps 3 A and 3 B or 1.5 B
        let floor = if two { 1.5 * first.a.max(first.b) } else { 1.5 * first.a };
        let cap = (3.0 * first.a).min(inner_cap * first.b);
        if first.a > 0.0 && first.b > 0.0 && floor <= cap {
            let (start, end) = speeds_of(&arrivals[0]).unwrap_or_default();
            assert!(start == 0.0 && end >= 1.5 - 1e-6, "case {case}: the first move is speeds [{start}, {end}]");
            let speeds = profile(&Curve::Speeds { start, end });
            assert!(speeds.windows(2).all(|w| w[1] >= w[0] - 1e-9), "case {case}: the first move slows down: [{start}, {end}]");
            checked += 1;
        }
        // the last junction, the mirror image
        let floor = if two { 1.5 * last.a.max(last.b) } else { 1.5 * last.b };
        let cap = (inner_cap * last.a).min(3.0 * last.b);
        if last.a > 0.0 && last.b > 0.0 && floor <= cap {
            let (start, end) = speeds_of(&arrivals[segments - 1]).unwrap_or_default();
            assert!(end == 0.0 && start >= 1.5 - 1e-6, "case {case}: the last move is speeds [{start}, {end}]");
            let speeds = profile(&Curve::Speeds { start, end });
            assert!(speeds.windows(2).all(|w| w[1] <= w[0] + 1e-9), "case {case}: the last move speeds up: [{start}, {end}]");
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked} ramps checked");
}

#[test]
fn t_m12_moving_ends_enter_and_leave_at_the_mean_speed() {
    let r = hd();
    let mut rng = Rng::new(0x7e57_0123);
    for case in 0..100 {
        let n = rng.int(3, 6) as usize;
        let plan = random_run(&mut rng, &r, n);
        for (start_from_rest, end_at_rest) in [(false, true), (true, false), (false, false)] {
            let opts = SmoothOpts { start_from_rest, end_at_rest };
            let arrivals = smooth_through(&plan, &r, &ids(&plan), opts).expect("a valid run");
            assert_eq!(arrivals.len(), n - 1, "case {case}: {opts:?}");
            let first = speeds_of(&arrivals[0]).map(|s| s.0);
            let last = speeds_of(&arrivals[n - 2]).map(|s| s.1);
            let start = if start_from_rest { 0.0 } else { 1.0 };
            let end = if end_at_rest { 0.0 } else { 1.0 };
            assert_eq!((first, last), (Some(start), Some(end)), "case {case}: {opts:?}");
            for arrival in &arrivals {
                if let Arrival::Move { curve, .. } = arrival {
                    assert!(check_curve(curve).is_ok(), "case {case}: {opts:?} gives {curve:?}");
                }
            }
        }
    }
}

#[test]
fn t_m12_refusals() {
    let r = hd();
    let w = |cx: f64| win(cx, 540.0, 1080.0);
    let plan = plan_of(vec![
        pos(&r, 1, 0, w(600.0), Arrival::Cut),
        pos(&r, 2, 50, w(900.0), Arrival::Cut),
        pos(&r, 3, 100, w(1250.0), Arrival::Cut),
        pos(&r, 4, 101, w(1260.0), Arrival::Cut),
    ]);
    let run = |plan: &VerticalPlan, chosen: &[u32]| code(smooth_through(plan, &r, chosen, SmoothOpts::default()));
    assert_eq!(run(&plan, &[1, 2]), Some("out_of_range"), "fewer than 3 positions");
    assert_eq!(run(&plan, &[1, 3, 2]), Some("out_of_range"), "not consecutive");
    assert_eq!(run(&plan, &[1, 2, 9]), Some("not_found"));
    assert_eq!(run(&plan, &[2, 3, 4]), Some("move_too_short"), "a gap of 1 frame");
    let mut locked = plan.clone();
    locked.positions[1].locked = true;
    assert_eq!(run(&locked, &[1, 2, 3]), Some("locked"));
    let mut with_fit = plan.clone();
    with_fit.positions[1].layout = Layout::Fit;
    assert_eq!(run(&with_fit, &[1, 2, 3]), Some("layout_move"));
    assert_eq!(run(&plan, &[1, 2, 3]), None);
}
