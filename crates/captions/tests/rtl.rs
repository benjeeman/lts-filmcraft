//! Right-to-left caption lines (Hebrew, Arabic) are written to SRT and WebVTT sidecars with a
//! leading RIGHT-TO-LEFT MARK (U+200F). A player lays a line out from its first strong character,
//! so without the mark a Hebrew line that starts with a Latin word reads left to right and its
//! final punctuation lands on the wrong side. A line is right to left when its Hebrew or Arabic
//! letters outnumber its Latin ones (markup such as `<i>` does not count); left-to-right lines,
//! lines with no letters and lines that already start with a direction mark are written as they
//! were. `srt.rs` and `vtt.rs` wrote every line verbatim.

use filmcraft_captions::{Cue, Document, Format, WriteOptions, parse, write};
use filmcraft_time::{TICKS_PER_SECOND, Tick};

const RLM: &str = "\u{200F}";

/// (as entered, as written back).
fn cases() -> Vec<(&'static str, String)> {
    vec![
        ("שלום עולם!", format!("{RLM}שלום עולם!")),
        ("iPhone חדש בחנות הגדולה.", format!("{RLM}iPhone חדש בחנות הגדולה.")),
        ("10 שקלים בלבד", format!("{RLM}10 שקלים בלבד")),
        ("مرحبا بالعالم", format!("{RLM}مرحبا بالعالم")),
        ("<i>שלום</i>", format!("{RLM}<i>שלום</i>")),
        ("Hello world.", "Hello world.".to_string()),
        ("2026", "2026".to_string()),
        ("Hello\nשלום לכולם", format!("Hello\n{RLM}שלום לכולם")),
        ("\u{200F}כבר מסומן", "\u{200F}כבר מסומן".to_string()),
    ]
}

fn doc(speaker: Option<&str>) -> Document {
    let cues = cases()
        .iter()
        .enumerate()
        .map(|(i, (text, _))| {
            let i = i as i64;
            let speaker = speaker.filter(|_| i == 0).map(str::to_string);
            Cue { start: Tick(2 * i * TICKS_PER_SECOND), end: Tick((2 * i + 1) * TICKS_PER_SECOND), text: (*text).to_string(), speaker, ..Default::default() }
        })
        .collect();
    Document { cues, ..Default::default() }
}

fn check(format: Format, speaker: Option<&str>) {
    let written = write(&doc(speaker), format, WriteOptions::default());
    let back = parse(&written, format).unwrap();
    let got: Vec<&str> = back.cues.iter().map(|c| c.text.as_str()).collect();
    let want: Vec<String> = cases().into_iter().map(|(_, w)| w).collect();
    assert_eq!(got, want, "{format:?}: right-to-left lines start with U+200F, the others are unchanged");
    assert_eq!(back.cues[0].speaker.as_deref(), speaker, "{format:?}: the voice span still reads back as the speaker");
    let text = String::from_utf8(written.clone()).unwrap();
    assert!(!text.contains("\u{200F}\u{200F}"), "{format:?}: a line already marked is not marked twice");
    assert_eq!(write(&back, format, WriteOptions::default()), written, "{format:?}: writing what was read back changes nothing");
}

#[test]
fn srt_marks_right_to_left_lines() {
    check(Format::Srt, None);
}

#[test]
fn webvtt_marks_right_to_left_lines_after_the_voice_span() {
    check(Format::WebVtt, None);
    check(Format::WebVtt, Some("Dana"));
}
