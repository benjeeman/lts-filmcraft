//! T-M8a: the canonical form of spec section 2.6. `q` is idempotent and `on_grid` is `q(v) == v` bit for bit; directed
//! rounding at the four edges gives windows on the grid and inside (the `h = 1000.001` case included); off-grid caller
//! values store `q(v)`; `zoom` converts to `h`; the limits round inward; and the JSON text keeps every bit
//! (`float_roundtrip`), digests included.

use filmcraft_time::FrameRate;

use super::support::*;
use crate::vertical::*;

/// Every float a curve holds, as bits.
fn curve_bits(curve: &Curve) -> Vec<u64> {
    match *curve {
        Curve::Speeds { start, end } => vec![start.to_bits(), end.to_bits()],
        Curve::Bezier { x1, y1, x2, y2 } => vec![x1.to_bits(), y1.to_bits(), x2.to_bits(), y2.to_bits()],
        Curve::Ramp { r_in, r_out } => vec![r_in.to_bits(), r_out.to_bits()],
        Curve::Linear | Curve::EaseIn | Curve::EaseOut | Curve::EaseInOut => Vec::new(),
    }
}

#[test]
fn t_m8a_q_is_idempotent_and_on_grid_is_q_bit_for_bit() {
    let mut rng = Rng::new(0x7e57_008a);
    let mut checked = 0;
    for _ in 0..30_000 {
        let v = match rng.int(0, 2) {
            0 => rng.range(-2000.0, 4000.0),
            1 => tq(rng.range(0.0, 2000.0)),
            _ => f64::from_bits(rng.next_u64()),
        };
        // coordinates stay within 1e6 px; far beyond, v * 1000 has no fraction left to round
        if v.is_finite() && v.abs() <= 1e6 {
            assert_eq!(q(q(v)).to_bits(), q(v).to_bits(), "q is idempotent at {v:e}");
            assert_eq!(q(v).to_bits(), tq(v).to_bits(), "q({v:e})");
            assert_eq!(on_grid(v), q(v).to_bits() == v.to_bits(), "on_grid({v:e})");
            checked += 1;
        }
    }
    assert!(checked > 20_000, "{checked} values checked");
    assert!(on_grid(812.4) && on_grid(1080.0) && on_grid(-0.001) && on_grid(0.0));
    assert!(!on_grid(812.4004) && !on_grid(0.0005) && !on_grid(f64::NAN) && !on_grid(f64::INFINITY));
    // rounding half away from zero, as f64::round does
    assert_eq!((q(0.0005), q(-0.0005), q(812.4004), q(812.4006)), (0.001, -0.001, 812.4, 812.401));
    assert_eq!((floor_g(281.25028125), ceil_g(281.25028125)), (281.25, 281.251));
    // the 1e-12 tolerance keeps a value that float noise put just past a grid point on that point
    assert_eq!((floor_g(812.4 - 1e-13), ceil_g(812.4 + 1e-13)), (812.4, 812.4));
    assert_eq!((floor_micro(1.7499999999999996), floor_micro(1.7499985), floor_micro(0.5)), (1.75, 1.749998, 0.5));
}

#[test]
fn t_m8a_directed_rounding_at_the_four_edges() {
    let mut rng = Rng::new(0x7e57_0e0e);
    for (r, cases) in [(hd(), 2000), (uhd(), 1000)] {
        let (h_min, h_max) = height_range(&r, OUT);
        let (fw, fh) = (f64::from(r.width), f64::from(r.height));
        for case in 0..cases {
            let h = tq(rng.range(h_min, h_max)).max(h_min).min(h_max);
            let w = h * f64::from(OUT.width) / f64::from(OUT.height);
            for (cx, cy) in [(w * 0.5, h * 0.5), (fw - w * 0.5, h * 0.5), (w * 0.5, fh - h * 0.5), (fw - w * 0.5, fh - h * 0.5)] {
                let stored = clamp_window(win(cx, cy, h), &r, OUT);
                assert!([stored.cx, stored.cy, stored.h].iter().all(|&v| tgrid(v)), "case {case}: {stored:?} is off the grid");
                assert_eq!(stored.h.to_bits(), h.to_bits(), "case {case}: a legal height on the grid stays");
                assert!(inside(&stored, &r, OUT, 1e-9), "case {case}: {stored:?} leaves the frame");
                let moved = (stored.cx - cx).abs().max((stored.cy - cy).abs());
                assert!(moved <= 0.001 + 1e-9, "case {case}: {stored:?} moved {moved} px from ({cx}, {cy})");
                let bounds = centre_bounds(&r, OUT, h);
                assert!((bounds.cx[0]..=bounds.cx[1]).contains(&stored.cx), "case {case}: {stored:?} outside {bounds:?}");
                assert!((bounds.cy[0]..=bounds.cy[1]).contains(&stored.cy), "case {case}: {stored:?} outside {bounds:?}");
            }
        }
    }
}

#[test]
fn t_m8a_the_1000_001_case() {
    // w / 2 = 281.25028125: q would store 281.25, outside by 0.00028 px; directed rounding stores 281.251
    let r = hd();
    let bounds = centre_bounds(&r, OUT, 1000.001);
    assert_eq!(bounds, CentreBounds { cx: [281.251, 1638.749], cy: [500.001, 579.999] });
    assert_eq!(clamp_window(win(281.25, 540.0, 1000.001), &r, OUT), win(281.251, 540.0, 1000.001));
    assert_eq!(clamp_window(win(2000.0, -5.0, 1000.001), &r, OUT), win(1638.749, 500.001, 1000.001));
    assert!(rect_of(&win(281.25, 540.0, 1000.001), OUT)[0] < 0.0, "the plain grid value would leave the frame");
    assert!(rect_of(&win(281.251, 540.0, 1000.001), OUT)[0] >= 0.0);
}

#[test]
fn t_m8a_clamping_writes_round_h_first() {
    let r = hd();
    // heights are clamped to the limits on the grid, then the centre to what that height allows
    assert_eq!(clamp_window(win(960.0, 540.0, 2000.0), &r, OUT), win(960.0, 540.0, 1080.0));
    assert_eq!(clamp_window(win(960.0, 540.0, 100.0), &r, OUT), win(960.0, 540.0, 480.0));
    assert_eq!(clamp_window(win(-50.0, 2000.0, 1000.0), &r, OUT), win(281.25, 580.0, 1000.0));
    assert_eq!(clamp_window(win(812.4004, 540.0002, 1080.0004), &r, OUT), win(812.4, 540.0, 1080.0));
}

#[test]
fn t_m8a_off_grid_caller_values_store_q() {
    assert_eq!(quantize_window(win(812.40004, 540.0002, 1000.0006)), win(812.4, 540.0, 1000.001));
    assert_eq!(quantize_window(win(-0.0004, 1e-9, 480.0)), win(-0.0, 0.0, 480.0));
    // a window that is already on the grid is stored as it is, bit for bit
    let w = win(281.251, 579.999, 1000.001);
    assert!(same_bits(&quantize_window(w), &w));
}

#[test]
fn t_m8a_zoom_converts_to_h() {
    assert_eq!(h_from_zoom(1080.0, 100.0), Some(1080.0));
    assert_eq!(h_from_zoom(1080.0, 150.0), Some(720.0));
    assert_eq!(h_from_zoom(1080.0, 133.0), Some(812.03));
    assert_eq!(h_from_zoom(1080.0, 300.0), Some(360.0), "below the limit: the write then checks I4 or clamps");
    for zoom in [0.0, -100.0, f64::NAN, f64::INFINITY] {
        assert_eq!(h_from_zoom(1080.0, zoom), None, "zoom {zoom}");
    }
    assert_eq!(zoom_percent(1080.0, 720.0), 150.0);
    assert_eq!(zoom_percent(1080.0, 1080.0), 100.0);
}

#[test]
fn t_m8a_limits_round_inward() {
    assert_eq!(limits(&hd(), OUT), Limits { min_height: 480.0, max_height: 1080.0 });
    assert_eq!(limits(&uhd(), OUT), Limits { min_height: 480.0, max_height: 2160.0 });
    assert_eq!(limits(&hd(), OutputSize { width: 1080, height: 1350 }), Limits { min_height: 337.5, max_height: 1080.0 });
    // h_max = 1000 * 1000 / 1081 = 925.06938... rounds down onto the grid
    let square = Raster { width: 1000, height: 1000, rate: FrameRate::FPS_25, end_frame: 100 };
    assert_eq!(limits(&square, OutputSize { width: 1081, height: 1000 }), Limits { min_height: 250.0, max_height: 925.069 });
    assert_eq!(centred_largest(&square, OutputSize { width: 1081, height: 1000 }), win(500.0, 500.0, 925.069));
}

#[test]
fn t_m8a_json_text_keeps_every_bit() {
    let mut rng = Rng::new(0x7e57_0f10);
    let values: Vec<f64> = (0..8000).map(|_| f64::from_bits(rng.next_u64())).filter(|v| v.is_finite()).collect();
    assert!(values.len() > 7000);
    for chunk in values.chunks_exact(4) {
        for curve in [
            Curve::Bezier { x1: chunk[0], y1: chunk[1], x2: chunk[2], y2: chunk[3] },
            Curve::Speeds { start: chunk[0], end: chunk[1] },
            Curve::Ramp { r_in: chunk[2], r_out: chunk[3] },
        ] {
            let text = serde_json::to_string(&curve).expect("finite values print");
            let back: Curve = serde_json::from_str(&text).expect("and read back");
            assert_eq!(curve_bits(&back), curve_bits(&curve), "{text}");
        }
        let window = win(chunk[0], chunk[1], chunk[2]);
        let back: Window = serde_json::from_str(&serde_json::to_string(&window).expect("prints")).expect("reads");
        assert!(same_bits(&back, &window), "{window:?}");
    }
    // a plan with computed speeds, Bezier and ramp values and splits: identical text, bits and digest after a round trip
    let r = hd();
    let awkward = Curve::Speeds { start: 0.1 + 0.2, end: 1.0 / 3.0 * 5.0 };
    let bezier = Curve::Bezier { x1: 1.0 / 3.0, y1: 0.1 + 0.7, x2: 2.0 / 3.0, y2: 0.9999999999999999 };
    let plan = plan_of(vec![
        pos(&r, 1, 0, win(812.4, 540.0, 1080.0), Arrival::Cut),
        pos(&r, 2, 50, win(1000.001, 500.25, 1000.0), mv_split(20, awkward, 10, 77)),
        pos(&r, 3, 90, win(700.5, 540.0, 1080.0), mv(40, bezier)),
        pos(&r, 4, 130, win(960.0, 540.0, 1080.0), mv(40, Curve::Ramp { r_in: 0.2 + 0.1, r_out: 0.7 / 3.0 })),
    ]);
    let text = serde_json::to_string(&plan).expect("prints");
    let back: VerticalPlan = serde_json::from_str(&text).expect("reads");
    assert_eq!(serde_json::to_string(&back).expect("prints"), text);
    for (a, b) in plan.positions.iter().zip(&back.positions) {
        assert!(same_bits(&a.window, &b.window), "position {}", a.id);
        if let (Arrival::Move { curve: ca, .. }, Arrival::Move { curve: cb, .. }) = (a.arrival, b.arrival) {
            assert_eq!(curve_bits(&ca), curve_bits(&cb), "position {}", a.id);
        }
    }
    let before = sequence_with(VerticalSet { plans: vec![plan.clone()], ..empty_set() });
    let after = sequence_with(VerticalSet { plans: vec![back.clone()], ..empty_set() });
    let (d1, d2) = (plan_digest(&before, &plan), plan_digest(&after, &back));
    assert!(digest_ok(&d1) && d1 == d2, "{d1} against {d2}");
}
