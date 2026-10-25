//! T-M1: every phase (starting, before, hold, move, cut), exactness at positions (C5), frame semantics, and layout changes
//! by cut only (spec section 3.1).

use filmcraft_time::Tick;

use super::support::*;
use crate::vertical::*;

#[test]
fn t_m1_a_plan_without_positions_shows_its_starting_window() {
    let r = hd();
    let largest = win(960.0, 540.0, 1080.0);
    assert!(same_bits(&centred_largest(&r, OUT), &largest), "the centred, largest window");
    let empty = plan_of(Vec::new());
    for f in [0, 1, 4805, 99_999] {
        let s = window_at(&empty, &r, tick(&r, f));
        assert!(same_bits(&s.window, &largest), "frame {f}: {:?}", s.window);
        assert_eq!((s.layout, s.phase, s.position, s.next), (Layout::Fill, Phase::Starting, None, None), "frame {f}");
    }
    let start = win(700.125, 600.5, 900.0);
    let with_start = VerticalPlan { starting_window: Some(start), ..plan_of(Vec::new()) };
    let s = window_at(&with_start, &r, tick(&r, 12));
    assert!(same_bits(&s.window, &start), "{:?}", s.window);
    assert_eq!((s.layout, s.phase), (Layout::Fill, Phase::Starting));
}

#[test]
fn t_m1_frames_before_the_first_position_show_it() {
    let r = hd();
    let first = fit(&r, 4, 100, win(500.0, 540.0, 1080.0));
    let plan = plan_of(vec![first.clone(), pos(&r, 5, 200, win(900.0, 540.0, 1080.0), Arrival::Cut)]);
    for t in [tick(&r, -20), tick(&r, 0), tick(&r, 50), tick(&r, 99), Tick(tick(&r, 100).0 - 1)] {
        let s = window_at(&plan, &r, t);
        assert!(same_bits(&s.window, &first.window), "tick {}: {:?}", t.0, s.window);
        assert_eq!((s.layout, s.phase, s.position, s.next), (Layout::Fit, Phase::Before, Some(4), None), "tick {}", t.0);
    }
}

#[test]
fn t_m1_on_its_frame_a_position_shows_its_own_window_bit_for_bit() {
    for r in [hd(), uhd()] {
        let plan = mixed_plan(&r);
        for p in &plan.positions {
            let s = window_at(&plan, &r, p.at);
            assert!(same_bits(&s.window, &p.window), "position {}: {:?} against {:?}", p.id, s.window, p.window);
            assert_eq!((s.layout, s.phase, s.position, s.next), (p.layout, Phase::Hold, Some(p.id), None), "position {}", p.id);
        }
    }
}

#[test]
fn t_m1_a_cut_changes_the_window_exactly_on_its_frame() {
    let r = hd();
    let (a, b) = (win(600.0, 540.0, 1080.0), win(1200.0, 540.0, 1080.0));
    let plan = plan_of(vec![pos(&r, 1, 10, a, Arrival::Cut), pos(&r, 2, 40, b, Arrival::Cut)]);
    for t in (10..40).map(|f| tick(&r, f)).chain([Tick(tick(&r, 40).0 - 1)]) {
        let s = window_at(&plan, &r, t);
        assert!(same_bits(&s.window, &a), "tick {} still shows the first window: {:?}", t.0, s.window);
        assert_eq!((s.phase, s.position), (Phase::Hold, Some(1)), "tick {}", t.0);
    }
    let s = window_at(&plan, &r, tick(&r, 40));
    assert!(same_bits(&s.window, &b), "{:?}", s.window);
    assert_eq!((s.phase, s.position), (Phase::Hold, Some(2)));
}

#[test]
fn t_m1_a_move_of_d_frames_changes_the_picture_on_exactly_d_frames() {
    let r = hd();
    let (a, b) = (win(600.0, 540.0, 1080.0), win(1000.5, 520.0, 900.0));
    for curve in every_curve() {
        for d in [2_u32, 3, 12, 50] {
            let arrive = 100;
            let start = arrive - i64::from(d);
            let plan = plan_of(vec![pos(&r, 1, 10, a, Arrival::Cut), pos(&r, 2, arrive, b, mv(d, curve))]);
            let samples: Vec<Sample> = (10..=arrive + 5).map(|f| window_at(&plan, &r, tick(&r, f))).collect();
            let mut changes = 0;
            for (k, s) in samples.iter().enumerate() {
                let f = 10 + k as i64;
                if f <= start {
                    assert!(same_bits(&s.window, &a), "{curve:?}, {d} frames: frame {f} shows the departing window");
                    assert_eq!((s.phase, s.position), (Phase::Hold, Some(1)), "{curve:?}, {d} frames: frame {f}");
                } else if f < arrive {
                    let u = (f - start) as f64 / f64::from(d);
                    assert_eq!((s.phase, s.position, s.next), (Phase::Move, Some(1), Some(2)), "{curve:?}, {d} frames: frame {f}");
                    assert_eq!(s.u.to_bits(), u.to_bits(), "{curve:?}, {d} frames: frame {f} is {u} of the move");
                    assert!(s.e > 0.0 && s.e < 1.0, "{curve:?}, {d} frames: frame {f} has e = {}", s.e);
                } else {
                    assert!(same_bits(&s.window, &b), "{curve:?}, {d} frames: frame {f} shows the arriving window");
                    assert_eq!((s.phase, s.position), (Phase::Hold, Some(2)), "{curve:?}, {d} frames: frame {f}");
                }
                if k > 0 && !same_bits(&s.window, &samples[k - 1].window) {
                    changes += 1;
                }
            }
            assert_eq!(changes, d, "{curve:?}: a {d}-frame move changes the picture on {changes} frames");
        }
    }
}

#[test]
fn t_m1_a_tick_between_frames_takes_the_exact_ratio() {
    let r = hd();
    let plan = plan_of(vec![pos(&r, 1, 10, win(600.0, 540.0, 1080.0), Arrival::Cut), pos(&r, 2, 60, win(1000.0, 540.0, 1080.0), mv(40, Curve::Linear))]);
    // frame 30 is 10 frames into the move that starts on frame 20
    let on_frame = window_at(&plan, &r, tick(&r, 30));
    assert_eq!((on_frame.phase, on_frame.u), (Phase::Move, 0.25));
    assert_eq!(on_frame.window.cx, 700.0);
    let half = r.rate.frame_duration().0 / 2;
    let between = window_at(&plan, &r, Tick(tick(&r, 30).0 + half));
    assert_eq!(between.phase, Phase::Move);
    assert!((between.u - 10.5 / 40.0).abs() < 1e-15, "u = {}", between.u);
    assert!((between.window.cx - 705.0).abs() < 1e-9, "cx = {}", between.window.cx);
}

#[test]
fn t_m1_a_layout_changes_only_by_cut() {
    let r = hd();
    let (a, b) = (win(600.0, 540.0, 1080.0), win(900.0, 540.0, 1080.0));
    // a move written into or out of a `fit` position (bad data, I9) still shows a hold and then a cut
    let into_fit = Position { layout: Layout::Fit, ..pos(&r, 2, 60, b, mv(30, Curve::Linear)) };
    let out_of_fit = Position { layout: Layout::Fit, ..pos(&r, 1, 10, a, Arrival::Cut) };
    let cases = [(pos(&r, 1, 10, a, Arrival::Cut), into_fit), (out_of_fit, pos(&r, 2, 60, b, mv(30, Curve::Linear)))];
    for (first, second) in cases {
        let plan = plan_of(vec![first.clone(), second.clone()]);
        for f in 10..60 {
            let s = window_at(&plan, &r, tick(&r, f));
            assert!(same_bits(&s.window, &a), "frame {f}: {:?}", s.window);
            assert_eq!((s.layout, s.phase), (first.layout, Phase::Hold), "frame {f}");
        }
        let s = window_at(&plan, &r, tick(&r, 60));
        assert!(same_bits(&s.window, &b), "{:?}", s.window);
        assert_eq!((s.layout, s.phase), (second.layout, Phase::Hold));
    }
    // a valid plan: fill, fit and fill again, each by a cut
    let plan = plan_of(vec![pos(&r, 1, 10, a, Arrival::Cut), fit(&r, 2, 20, a), pos(&r, 3, 30, b, Arrival::Cut)]);
    let layouts: Vec<Layout> = (10..40).map(|f| window_at(&plan, &r, tick(&r, f)).layout).collect();
    let expected: Vec<Layout> = (10..40).map(|f| if (20..30).contains(&f) { Layout::Fit } else { Layout::Fill }).collect();
    assert_eq!(layouts, expected);
}

#[test]
fn t_m1_after_the_last_position_its_window_holds() {
    let r = hd();
    let plan = mixed_plan(&r);
    let last = plan.positions.last().expect("the plan has positions");
    let frame = r.rate.frame_at(last.at);
    for f in [frame, frame + 1, frame + 1000, r.end_frame] {
        let s = window_at(&plan, &r, tick(&r, f));
        assert!(same_bits(&s.window, &last.window), "frame {f}: {:?}", s.window);
        assert_eq!((s.layout, s.phase, s.position, s.next), (last.layout, Phase::Hold, Some(last.id), None), "frame {f}");
    }
}

#[test]
fn t_m1_window_rect_and_upscale() {
    let w = win(960.0, 540.0, 1080.0);
    assert_eq!(w.rect(OUT), Rect { x: 656.25, y: 0.0, w: 607.5, h: 1080.0 });
    assert_eq!(w.upscale(OUT), 1920.0 / 1080.0);
    let zoomed = win(1000.5, 500.25, 810.0);
    assert_eq!(zoomed.rect(OUT), Rect { x: 1000.5 - 227.8125, y: 500.25 - 405.0, w: 455.625, h: 810.0 });
}
