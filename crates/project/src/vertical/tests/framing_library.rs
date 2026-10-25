//! The framing library (spec section 5.11): derived from labels, never stored. `fromLabel` takes the window and layout of
//! the last position with the label before the target frame's shot (the nearest earlier occurrence, which follows a
//! camera setup that drifts between takes), else of the first one at or after the shot's end; `label_not_found`
//! otherwise. The position at the frame takes that window, that layout and the label.

use serde_json::json;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;

/// Walls at 100, 200 and 300; "Dana CU" at 0 and, drifted, at 150; "Wide" (fit) at 100; "Noa" at 300.
fn labelled() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 5_000), (200, 100, 9_000), (300, 700, 13_000)]);
    let named = |id: u32, frame: i64, w: Window, label: &str| Position { label: label.into(), ..at25(id, frame, w, Arrival::Cut) };
    fx.install(one_plan(vec![
        named(1, 0, win(700.0, 540.0, 1080.0), "Dana CU"),
        Position { layout: Layout::Fit, ..named(2, 100, win(960.0, 540.0, 1080.0), "Wide") },
        named(3, 150, win(720.0, 530.0, 1000.0), "Dana CU"),
        named(4, 300, win(1200.0, 540.0, 1080.0), "Noa"),
    ]));
    fx
}

fn from_label_at(frame: i64, label: &str) -> Write {
    Write::SetPosition(SetPosition { plan: 1, frame, from_label: Some(label.into()), ..Default::default() })
}

#[test]
fn the_library_lists_each_label_with_its_positions_in_time_order() {
    let fx = labelled();
    let entries: Vec<(String, Vec<u32>)> = library(fx.plan(1)).into_iter().map(|e| (e.label, e.positions)).collect();
    assert_eq!(entries, vec![("Dana CU".to_string(), vec![1, 3]), ("Wide".to_string(), vec![2]), ("Noa".to_string(), vec![4])]);
}

#[test]
fn from_label_takes_the_nearest_earlier_occurrence_before_the_shot() {
    let mut fx = labelled();
    let source = from_label(&fx.project, fx.seq(), 1, 200, "Dana CU").expect("a Dana CU before shot 3");
    assert_eq!((source.position, source.window, source.layout), (3, win(720.0, 530.0, 1000.0), Layout::Fill), "the drifted take at 150");
    let planned = fx.apply(from_label_at(200, "Dana CU"));
    assert_eq!(planned.added, vec![5]);
    let opener = fx.position(1, 5);
    assert_eq!((opener.window, opener.layout, opener.label.as_str(), opener.arrival), (win(720.0, 530.0, 1000.0), Layout::Fill, "Dana CU", Arrival::Cut));
    // inside shot 2 the occurrence at 150 is the shot's own: the one before the shot is taken
    let source = from_label(&fx.project, fx.seq(), 1, 120, "Dana CU").expect("a Dana CU before shot 2");
    assert_eq!(source.position, 1);
    // the layout comes too
    let source = from_label(&fx.project, fx.seq(), 1, 250, "Wide").expect("Wide");
    assert_eq!((source.position, source.layout), (2, Layout::Fit));
}

#[test]
fn without_an_earlier_occurrence_the_first_after_the_shot_is_taken_else_label_not_found() {
    let fx = labelled();
    let source = from_label(&fx.project, fx.seq(), 1, 50, "Noa").expect("a Noa after shot 1");
    assert_eq!((source.position, source.window), (4, win(1200.0, 540.0, 1080.0)));
    let e = fx.refuse(from_label_at(50, "Nobody"));
    assert_eq!((e.code, e.detail["label"].clone()), ("label_not_found", json!("Nobody")));
    let e = from_label(&fx.project, fx.seq(), 1, 50, "Nobody").expect_err("no such label");
    assert_eq!(e.code, "label_not_found");
}

#[test]
fn from_label_updates_the_position_on_the_frame() {
    let mut fx = labelled();
    let planned = fx.apply(from_label_at(300, "Dana CU"));
    assert_eq!(planned.updated, vec![4]);
    let p = fx.position(1, 4);
    assert_eq!((p.window, p.label.as_str()), (win(720.0, 530.0, 1000.0), "Dana CU"), "the position on the frame takes the window and the label");
    // the library is derived from the labels: with position 4 relabelled, no Noa is left
    let explicit = Write::SetPosition(SetPosition {
        plan: 1,
        frame: 300,
        from_label: Some("Noa".into()),
        fields: PositionFields { label: Some("Noa 2".into()), ..Default::default() },
        ..Default::default()
    });
    assert_eq!(fx.refuse(explicit).code, "label_not_found", "no Noa is left: position 4 is Dana CU now");
}
