//! T-M6: the geometric zoom schedule, and `window_at` bit for bit against an independent implementation of the normative
//! operation order of spec sections 3.1 to 3.4, over plans with every curve, zooms, splits and `fit` positions (C4).

use filmcraft_time::Tick;

use super::support::*;
use crate::vertical::*;

/// `E(u)` in the operation order of spec sections 3.2 and 3.3, written again from the spec. The ramp ends with
/// `e.max(0.0).min(1.0)`, the normative order.
#[allow(clippy::manual_clamp)]
fn reference_ease(curve: &Curve, u: f64) -> f64 {
    if u <= 0.0 {
        return 0.0;
    }
    if u >= 1.0 {
        return 1.0;
    }
    match *curve {
        Curve::Linear => u,
        Curve::EaseIn => u * u * (2.0 - u),
        Curve::EaseOut => u * (1.0 + u * (1.0 - u)),
        Curve::EaseInOut => u * u * (3.0 - 2.0 * u),
        Curve::Speeds { start: a, end: b } => {
            let v = 1.0 - u;
            u * (v * v * a + u * (v * (3.0 - b) + u))
        }
        Curve::Bezier { x1, y1, x2, y2 } => {
            let cx = 3.0 * x1;
            let bx = 3.0 * (x2 - x1) - cx;
            let ax = 1.0 - cx - bx;
            let cy = 3.0 * y1;
            let by = 3.0 * (y2 - y1) - cy;
            let ay = 1.0 - cy - by;
            let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
            for _ in 0..64 {
                let mid = 0.5 * (lo + hi);
                if ((ax * mid + bx) * mid + cx) * mid < u { lo = mid } else { hi = mid }
            }
            let s = 0.5 * (lo + hi);
            ((ay * s + by) * s + cy) * s
        }
        Curve::Ramp { r_in: a, r_out: b } => {
            let v = 1.0 / (1.0 - 0.5 * (a + b));
            let e = if u < a {
                0.5 * v * u * u / a
            } else if u <= 1.0 - b {
                v * (u - 0.5 * a)
            } else {
                let w = 1.0 - u;
                1.0 - 0.5 * v * w * w / b
            };
            e.max(0.0).min(1.0)
        }
    }
}

/// The path of spec section 3.4, written again from the spec.
fn reference_path(a: &Window, b: &Window, e: f64) -> Window {
    let g = if a.h == b.h {
        e
    } else {
        let l = libm::log(b.h / a.h);
        libm::expm1(e * l) / libm::expm1(l)
    };
    Window { cx: a.cx + (b.cx - a.cx) * g, cy: a.cy + (b.cy - a.cy) * g, h: a.h + (b.h - a.h) * g }
}

/// `window_at` in the operation order of spec section 3.1, written again from the spec (with a linear scan, not a search).
fn reference_window(plan: &VerticalPlan, r: &Raster, t: Tick) -> (Window, Layout) {
    let ps = &plan.positions;
    let Some(first) = ps.first() else {
        let (w, h) = (f64::from(r.width), f64::from(r.height));
        let (ow, oh) = (f64::from(plan.output.width), f64::from(plan.output.height));
        let largest = Window { cx: w / 2.0, cy: h / 2.0, h: tfloor(h.min(w * oh / ow)) };
        return (plan.starting_window.unwrap_or(largest), Layout::Fill);
    };
    if t < first.at {
        return (first.window, first.layout);
    }
    let i = ps.iter().rposition(|p| p.at <= t).unwrap_or(0);
    let pi = &ps[i];
    let Some(pj) = ps.get(i + 1) else { return (pi.window, pi.layout) };
    let Arrival::Move { move_frames: d, curve, split_of } = pj.arrival else { return (pi.window, pi.layout) };
    if pi.layout != Layout::Fill || pj.layout != Layout::Fill {
        return (pi.window, pi.layout);
    }
    let fj = r.rate.frame_at(pj.at);
    let s = r.rate.tick_of(fj - i64::from(d)).max(pi.at);
    if s >= t {
        return (pi.window, pi.layout);
    }
    let (o, total) = split_of.map_or((0, d), |p| (p.offset, p.total));
    let ub = if t == r.rate.tick_of(r.rate.frame_at(t)) {
        (i64::from(o) + (r.rate.frame_at(t) - r.rate.frame_at(s))) as f64 / f64::from(total)
    } else {
        (f64::from(o) + ((t - s).0 as f64 / (pj.at - s).0 as f64) * f64::from(d)) / f64::from(total)
    };
    let e = match split_of {
        None => reference_ease(&curve, ub),
        Some(_) => {
            let e0 = reference_ease(&curve, f64::from(o) / f64::from(total));
            let e1 = reference_ease(&curve, f64::from(o + d) / f64::from(total));
            (reference_ease(&curve, ub) - e0) / (e1 - e0)
        }
    };
    (reference_path(&pi.window, &pj.window, e), Layout::Fill)
}

#[test]
fn t_m6_the_zoom_schedule_is_geometric() {
    for (from, to) in [(1080.0, 810.0), (810.0, 1080.0), (1080.0, 480.0), (500.25, 977.5)] {
        let (a, b) = (win(960.0, 540.0, from), win(900.0, 520.0, to));
        for k in 0..=200 {
            let e = f64::from(k) / 200.0;
            let h = path(&a, &b, e).h;
            let geometric = from * (to / from).powf(e);
            assert!((h - geometric).abs() <= 1e-9, "{from} to {to} at e = {e}: h = {h}, h_i r^e = {geometric}");
        }
    }
    let mid = path(&win(960.0, 540.0, 1080.0), &win(960.0, 540.0, 810.0), 0.5).h;
    assert!((mid - (1080.0_f64 * 810.0).sqrt()).abs() < 1e-9, "the geometric mean, not 945: {mid}");
    // the centre moves on the same line: the window is a scaling about one fixed point
    let (a, b) = (win(700.0, 500.0, 1000.0), win(1100.0, 600.0, 600.0));
    for k in 1..10 {
        let w = path(&a, &b, f64::from(k) / 10.0);
        let along = (w.h - a.h) / (b.h - a.h);
        assert!((w.cx - (a.cx + (b.cx - a.cx) * along)).abs() < 1e-9 && (w.cy - (a.cy + (b.cy - a.cy) * along)).abs() < 1e-9, "{w:?}");
    }
}

#[test]
fn t_m6_pure_pans_are_linear_in_e() {
    let (a, b) = (win(600.0, 540.0, 1080.0), win(1200.0, 540.0, 1080.0));
    for k in 0..=100 {
        let e = f64::from(k) / 100.0;
        let w = path(&a, &b, e);
        assert_eq!(w.cx.to_bits(), (600.0 + 600.0 * e).to_bits(), "e = {e}");
        assert_eq!((w.cy, w.h), (540.0, 1080.0));
        assert_eq!(path::progress(1080.0, 1080.0, e).to_bits(), e.to_bits());
    }
    assert_eq!((path::start_rate(1080.0, 1080.0), path::end_rate(1080.0, 1080.0)), (1.0, 1.0));
    // G'(0) and G'(1) of a zoom against finite differences of G
    for (from, to) in [(1080.0, 810.0), (600.0, 1000.0)] {
        let step = 1e-7;
        let start = (path::progress(from, to, step) - path::progress(from, to, 0.0)) / step;
        let end = (path::progress(from, to, 1.0) - path::progress(from, to, 1.0 - step)) / step;
        assert!((path::start_rate(from, to) - start).abs() < 1e-5, "{from} to {to}: G'(0) {start}");
        assert!((path::end_rate(from, to) - end).abs() < 1e-5, "{from} to {to}: G'(1) {end}");
    }
}

#[test]
fn t_m6_window_at_equals_the_reference_bit_for_bit() {
    for r in [hd(), uhd()] {
        let start = VerticalPlan { starting_window: Some(win(700.125, 600.5, 900.0)), ..plan_of(Vec::new()) };
        let plans = [mixed_plan(&r), plan_of(Vec::new()), start];
        let duration = r.rate.frame_duration().0;
        for (n, plan) in plans.iter().enumerate() {
            let last = plan.positions.last().map_or(10, |p| r.rate.frame_at(p.at)) + 5;
            for f in -5..=last {
                let t = tick(&r, f);
                for t in [t, Tick(t.0 + 1), Tick(t.0 + duration / 3), Tick(t.0 + duration - 1)] {
                    let sample = window_at(plan, &r, t);
                    let (window, layout) = reference_window(plan, &r, t);
                    let got = (sample.window.cx.to_bits(), sample.window.cy.to_bits(), sample.window.h.to_bits(), sample.layout);
                    let want = (window.cx.to_bits(), window.cy.to_bits(), window.h.to_bits(), layout);
                    assert_eq!(got, want, "plan {n}, tick {}: {:?} against the reference {window:?}", t.0, sample.window);
                }
            }
        }
    }
}

#[test]
fn t_m6_random_plans_equal_the_reference_bit_for_bit() {
    let r = hd();
    let mut rng = Rng::new(0x7e57_0006);
    for case in 0..60 {
        let (a, b, c) = (random_window(&mut rng, &r, OUT), random_window(&mut rng, &r, OUT), random_window(&mut rng, &r, OUT));
        let (d1, d2) = (rng.int(2, 60), rng.int(2, 60));
        let total = d2 + rng.int(1, 40);
        let (first, second) = (random_curve(&mut rng), random_curve(&mut rng));
        let positions = vec![
            pos(&r, 1, 0, a, Arrival::Cut),
            pos(&r, 2, d1 + 3, b, mv(d1 as u32, first)),
            pos(&r, 3, d1 + 3 + d2, c, mv_split(d2 as u32, second, (total - d2) as u32, total as u32)),
        ];
        let plan = plan_of(positions);
        for f in -1..=d1 + d2 + 5 {
            let t = tick(&r, f);
            let sample = window_at(&plan, &r, t);
            let (window, _) = reference_window(&plan, &r, t);
            assert!(same_bits(&sample.window, &window), "case {case}, frame {f}: {:?} against {window:?}", sample.window);
        }
    }
}
