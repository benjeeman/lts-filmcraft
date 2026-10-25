//! T-M21 (spec section 7.3, `vertical.update`): a larger output raises `h_min = OH / 4`. Positions below it are refused
//! `size_limit {minHeight, maxHeight, positions}`, or with `clamp: true` enlarged about their centres (then clamped with
//! directed rounding) and each reported; a locked one is refused `locked`. Size changes keep the aspect.

use serde_json::json;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;

/// A 9:16 1080x1920 plan (h_min 480) with windows of height 500, 1080 and 600 (the last near the left edge).
fn small_windows() -> Fx {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![
        at25(1, 100, win(700.0, 540.0, 500.0), Arrival::Cut),
        at25(2, 200, win(1100.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 300, win(200.0, 540.0, 600.0), Arrival::Cut),
    ]));
    fx
}

fn resize(width: u32, height: u32, clamp: bool) -> Write {
    Write::Update(UpdatePlan { plan: 1, width: Some(width), height: Some(height), clamp, ..Default::default() })
}

#[test]
fn t_m21_a_larger_output_refuses_windows_below_its_minimum_without_clamp() {
    let fx = small_windows();
    let e = fx.refuse(resize(2160, 3840, false));
    assert_eq!(e.code, "size_limit");
    assert_eq!(e.detail["minHeight"], json!(960.0));
    assert_eq!(e.detail["maxHeight"], json!(1080.0));
    assert_eq!(e.detail["positions"], json!([1, 3]));
}

#[test]
fn t_m21_with_clamp_windows_are_enlarged_about_their_centres_and_reported() {
    let mut fx = small_windows();
    let planned = fx.apply(resize(2160, 3840, true));
    assert_eq!(fx.plan(1).output, OutputSize { width: 2160, height: 3840 });
    assert_eq!(fx.position(1, 1).window, win(700.0, 540.0, 960.0), "about its centre");
    assert_eq!(fx.position(1, 2).window, win(1100.0, 540.0, 1080.0), "a window above the new minimum is untouched");
    assert_eq!(fx.position(1, 3).window, win(270.0, 540.0, 960.0), "then clamped: w = 540 needs cx >= 270");
    assert_eq!(planned.clamped, vec![Clamped { position: 1, dx: 0.0, dy: 0.0, dh: 460.0 }, Clamped { position: 3, dx: 70.0, dy: 0.0, dh: 360.0 }]);
    assert_eq!(planned.updated, vec![1, 3]);
}

#[test]
fn t_m21_a_locked_window_is_never_enlarged() {
    let mut fx = small_windows();
    fx.plan_mut(1).positions[2].locked = true;
    let e = fx.refuse(resize(2160, 3840, true));
    assert_eq!((e.code, e.detail["ids"].clone()), ("locked", json!([3])));
}

#[test]
fn t_m21_sizes_keep_the_aspect() {
    let mut fx = small_windows();
    let e = fx.refuse(resize(1080, 1080, false));
    assert_eq!(e.code, "aspect_change");
    let planned = fx.apply(resize(720, 1280, false));
    assert!(planned.changed);
    assert_eq!(fx.plan(1).output, OutputSize { width: 720, height: 1280 });
    assert_eq!(fx.position(1, 1).window, win(700.0, 540.0, 500.0), "h_min 320: nothing moves");
    assert!(!accepted(fx.write(resize(720, 1280, false))).changed, "the same size again is a no-op");
    assert_eq!(fx.refuse(resize(721, 1280, false)).code, "invalid_output_size");
}
