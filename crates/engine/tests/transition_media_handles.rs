//! A transition longer than the media on either side of the cut still applies, but the editor
//! says so, as Premiere does ("Insufficient media. This transition will contain repeated
//! frames."): the result carries a warning with how many frames repeat. `commands.rs`
//! `apply_transition` never compared the transition with the clips' handles, so the dissolve
//! silently showed frozen frames.

use filmcraft_engine::Session;
use serde_json::{Value, json};

/// The demo project; returns the session and the second V1 clip (its In point is a few seconds
/// into its media, the first clip's media runs on for several seconds after the cut).
fn session() -> (Session, u64, i64) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let rate = s.sequence_rate();
    let q = s.active_sequence().unwrap();
    let incoming = &q.video_tracks[0].items[1];
    assert_eq!(q.video_tracks[0].items[0].end(), incoming.start, "fixture: a straight cut");
    let (clip, head) = (incoming.id.0, rate.frame_at(incoming.source_in));
    assert!(head > 10, "fixture: the incoming clip has some media before its In point ({head} frames)");
    (s, clip, head)
}

fn warnings(r: &Value) -> Vec<String> {
    r.get("warnings").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

#[test]
fn a_dissolve_longer_than_the_handles_warns_about_repeated_frames() {
    let (mut s, clip, head) = session();
    // Centred on the cut, each half needs half the frames: 10 more than the incoming clip has.
    let frames = 2 * head + 20;
    let r = s.execute("sequence.applyVideoTransition", json!({"clip": clip, "edge": "in", "frames": frames})).unwrap();
    assert!(r.get("transition").is_some(), "the transition is still applied: {r}");
    let w = warnings(&r);
    assert!(w.iter().any(|w| w.to_lowercase().contains("insufficient media") && w.contains("10")), "a warning names the 10 repeated frames: {r}");
}

#[test]
fn a_dissolve_within_the_handles_has_no_warning() {
    let (mut s, clip, head) = session();
    let r = s.execute("sequence.applyVideoTransition", json!({"clip": clip, "edge": "in", "frames": head.min(24)})).unwrap();
    assert!(r.get("transition").is_some(), "{r}");
    assert!(warnings(&r).is_empty(), "no warning when the media covers the transition: {r}");
}
