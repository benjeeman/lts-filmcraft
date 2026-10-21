//! Text-based Extract honours Markers › Ripple Sequence Markers exactly as Sequence › Extract
//! does (`commands.rs` `sequence.extract`): with the toggle on, a sequence marker inside the
//! removed range is deleted and the later ones move left by the cut; with it off (the default)
//! the markers stay where they are. `transcript.rs` `extract_or_lift` never rippled them, so the
//! markers after a text cut pointed at the wrong words.

use std::sync::Arc;

use filmcraft_engine::Session;
use filmcraft_project::{ItemId, Transcript, Word};
use filmcraft_speech::FixedTranscriber;
use filmcraft_time::Tick;
use serde_json::json;

/// The demo project with a fixed transcript on the first A1 clip's media: "Hello um world."
/// (Speaker 1), a pause, "Second speaker here." (Speaker 2), transcribed.
fn session() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let a = &q.audio_tracks[0].items[0];
    let (item, sin): (ItemId, Tick) = (a.item, a.source_in);
    let sec = |x: f64| sin + Tick::from_seconds_f64(x);
    let mut t = Transcript { language: "en".into(), ..Default::default() };
    for (text, a, b, sp) in
        [("Hello", 0.2, 0.5, 0), ("um", 0.6, 0.9, 0), ("world.", 1.0, 1.4, 0), ("Second", 2.6, 3.0, 1), ("speaker", 3.0, 3.5, 1), ("here.", 3.5, 4.0, 1)]
    {
        let mut w = Word::new(text, sec(a), sec(b));
        w.speaker = Some(sp);
        t.words.push(w);
    }
    t.normalize();
    s.transcriber = Some(Arc::new(FixedTranscriber { transcript: t, id: "fixed".into() }));
    s.execute("transcript.generate", json!({"items": [item.0]})).unwrap();
    s.execute("markers.clearAll", json!({})).unwrap();
    s
}

fn starts(s: &Session) -> Vec<i64> {
    s.active_sequence().unwrap().markers.iter().map(|m| m.start.0).collect()
}

#[test]
fn text_extract_ripples_sequence_markers_like_sequence_extract() {
    let mut s = session();
    // The word "um": a marker on it and one two seconds (48 frames) after it.
    let r = s.execute("transcript.select", json!({"from": 1})).unwrap();
    let (a, b) = (r["start"].as_i64().unwrap(), r["end"].as_i64().unwrap());
    let later = b + s.sequence_rate().tick_of(48).0;
    s.execute("markers.add", json!({"time": a})).unwrap();
    s.execute("markers.add", json!({"time": later})).unwrap();
    assert_eq!(starts(&s), vec![a, later], "fixture markers");

    // Off (the default): the markers stay where they are.
    s.execute("transcript.extract", json!({"from": 1})).unwrap();
    assert_eq!(starts(&s), vec![a, later], "Ripple Sequence Markers off");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(starts(&s), vec![a, later]);

    // On: the marker on the removed word goes, the later one moves left by the cut.
    s.execute("markers.rippleSequenceMarkers", json!({"on": true})).unwrap();
    let r = s.execute("transcript.extract", json!({"from": 1})).unwrap();
    let cut = r["end"].as_i64().unwrap() - r["start"].as_i64().unwrap();
    assert!(cut > 0, "{r}");
    assert_eq!(starts(&s), vec![later - cut], "Ripple Sequence Markers on: the inside marker is deleted, the later one follows the cut");

    // One undo step brings both markers back where they were.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(starts(&s), vec![a, later]);
}

#[test]
fn text_lift_leaves_sequence_markers_alone() {
    let mut s = session();
    let r = s.execute("transcript.select", json!({"from": 1})).unwrap();
    let (a, b) = (r["start"].as_i64().unwrap(), r["end"].as_i64().unwrap());
    let later = b + s.sequence_rate().tick_of(48).0;
    s.execute("markers.add", json!({"time": a})).unwrap();
    s.execute("markers.add", json!({"time": later})).unwrap();
    s.execute("markers.rippleSequenceMarkers", json!({"on": true})).unwrap();
    s.execute("transcript.lift", json!({"from": 1})).unwrap();
    assert_eq!(starts(&s), vec![a, later], "Lift leaves a gap: nothing ripples");
}
