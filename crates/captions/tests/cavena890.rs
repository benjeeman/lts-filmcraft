//! Cavena 890 (`.890`) reading, on synthetic files built byte by byte from the layout in
//! `src/cavena890.rs`: a 388-byte header, then 128-byte blocks (the last one may stop 4 bytes
//! short), block 0 being a title block. Hebrew rows store letters in logical order but every run
//! of digits reversed (the screen read from right to left); Russian rows use Latin letters for
//! Cyrillic ones; times are absolute frame counts at 25 fps.

use filmcraft_captions::cavena890::{self, Charset, Warning};
use filmcraft_captions::{Document, Error, Format, WriteOptions, detect, parse, stl, track_from_document, try_write, write};
use filmcraft_project::CaptionFormat;
use filmcraft_project::TrackId;
use filmcraft_time::{FrameRate, Tick};
use proptest::prelude::*;

const HEBREW: u8 = 0x8f;
const RUSSIAN: u8 = 0x56;
const LATIN: u8 = 0x09;
/// 01:00:00:00 at 25 fps.
const HOUR: u32 = 90_000;

fn put(buf: &mut [u8], at: usize, s: &[u8]) {
    buf[at..at + s.len()].copy_from_slice(s);
}

/// Stored Hebrew text: Hebrew letters as 0x40 + (code point - U+05D0), ASCII as itself. The
/// string is given in stored order.
fn heb(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            'א'..='ת' => 0x40 + (c as u32 - 'א' as u32) as u8,
            c if c.is_ascii() => c as u8,
            c => panic!("no byte for {c}"),
        })
        .collect()
}

fn header(lang: u8, font: &[u8], blocks: usize, start_of_message: &[u8]) -> Vec<u8> {
    let mut h = vec![0u8; 388];
    put(&mut h, 2, b"TAPE000123");
    put(&mut h, 40, b"Synthetic Show");
    put(&mut h, 68, b"A. Translator");
    put(&mut h, 105, b"2-7");
    h[146] = LATIN;
    h[147] = lang;
    put(&mut h, 148, b"made by a test");
    let n = (blocks as u16).to_le_bytes();
    put(&mut h, 179, &n);
    // the primary font field is 181–194; the name starts 6 bytes in
    put(&mut h, 187, font);
    put(&mut h, 208, &n);
    put(&mut h, 210, &n);
    put(&mut h, 218, b"Synthetic Show Original");
    put(&mut h, 246, b"VFONTL.V");
    put(&mut h, 256, start_of_message);
    h
}

/// A subtitle block: number, in and out frames, two 51-byte rows padded with 0x7F.
fn block(number: u16, start: u32, end: u32, row1: &[u8], row2: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    put(&mut b, 0, &number.to_be_bytes());
    put(&mut b, 2, &start.to_be_bytes()[1..]);
    put(&mut b, 5, &end.to_be_bytes()[1..]);
    b[8] = 0x14;
    b[15] = 0x16;
    b[16..67].fill(0x7f);
    put(&mut b, 16, row1);
    b[73..124].fill(0x7f);
    put(&mut b, 73, row2);
    b
}

/// The title block: zero times, both rows in the Latin set (0x80 at bytes 14 and 72).
fn title_block(row1: &[u8], row2: &[u8]) -> Vec<u8> {
    let mut b = block(1, 0, 0, row1, row2);
    b[8] = 0x04;
    b[14] = 0x80;
    b[72] = 0x80;
    b
}

/// A whole file; `short` cuts the last block to 124 bytes, as the reference files do.
fn file(lang: u8, font: &[u8], cues: Vec<Vec<u8>>, short: bool) -> Vec<u8> {
    let mut blocks = vec![title_block(b"Synthetic Show 2-7", b"0000000Axx.890")];
    blocks.extend(cues);
    let mut out = header(lang, font, blocks.len(), b"00:00:00:00");
    for b in &blocks {
        out.extend_from_slice(b);
    }
    if short {
        out.truncate(out.len() - 4);
    }
    out
}

fn cue_block(k: u32, row1: &[u8], row2: &[u8]) -> Vec<u8> {
    block(16 * k as u16, HOUR + 100 * k, HOUR + 100 * k + 60, row1, row2)
}

fn hebrew_file(rows: &[(&str, &str)]) -> Vec<u8> {
    let cues = rows.iter().enumerate().map(|(k, (r1, r2))| cue_block(k as u32 + 1, &heb(r1), &heb(r2))).collect();
    file(HEBREW, b"HEBNOA.V", cues, true)
}

fn texts(doc: &Document) -> Vec<&str> {
    doc.cues.iter().map(|c| c.text.as_str()).collect()
}

#[test]
fn hebrew_digit_runs_are_reversed_back_and_letters_and_punctuation_kept() {
    // (stored row, logical text)
    let cases = [
        ("שילמתי 005 שקל.", "שילמתי 500 שקל."),
        ("-מה השעה?", "-מה השעה?"),
        ("-בדיוק 03:41.", "-בדיוק 14:30."),
        ("נולדתי ב-5891.", "נולדתי ב-1985."),
        ("הגיעו 000,2 איש", "הגיעו 2,000 איש"),
        ("1, 2, 3, 01.", "1, 2, 3, 10."),
        ("קיבלתי 5.9 במבחן", "קיבלתי 9.5 במבחן"),
        ("פתוח 7/42", "פתוח 24/7"),
        ("הוא אמר \"05 אחוז\".", "הוא אמר \"50 אחוז\"."),
        ("בעוד 5 דקות, צה\"ל וג'ירפה!", "בעוד 5 דקות, צה\"ל וג'ירפה!"),
        ("-04, 06 או 001?", "-40, 60 או 100?"),
    ];
    let rows: Vec<(&str, &str)> = cases.iter().map(|(stored, _)| ("", *stored)).collect();
    let doc = parse(&hebrew_file(&rows), Format::Cavena890).unwrap();
    let want: Vec<&str> = cases.iter().map(|(_, logical)| *logical).collect();
    assert_eq!(texts(&doc), want);
    assert!(doc.warnings.iter().all(|w| !w.contains("U+FFFD")), "{:?}", doc.warnings);
    assert_eq!(doc.language.as_deref(), Some("he"));
}

#[test]
fn hebrew_two_line_cues_join_with_a_line_break() {
    let doc = parse(&hebrew_file(&[("-איפה היית?", "-בבית, עם 3 חברים."), ("שורה עליונה בלבד", "")]), Format::Cavena890).unwrap();
    assert_eq!(texts(&doc), vec!["-איפה היית?\n-בבית, עם 3 חברים.", "שורה עליונה בלבד"]);
}

#[test]
fn russian_latin_letters_stand_for_cyrillic() {
    let cues = vec![
        cue_block(1, b"Privet, kak dela?", b"V 1985 godu - 25 let."),
        cue_block(2, b"abvgde`zijklmnoprstufhc~{}\x1dxy|wq", b"ABVGDE@ZIJKLMNOPRSTUFHC^[]XY\\WQ"),
    ];
    let doc = parse(&file(RUSSIAN, b"KYRIL2.V", cues, true), Format::Cavena890).unwrap();
    assert_eq!(texts(&doc), vec!["Привет, как дела?\nВ 1985 году - 25 лет.", "абвгдежзийклмнопрстуфхцчшщъыьэюя\nАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЫЬЭЮЯ"]);
    assert_eq!(doc.language.as_deref(), Some("ru"));
    assert!(doc.warnings.iter().all(|w| !w.contains("U+FFFD")), "{:?}", doc.warnings);
}

#[test]
fn latin_files_read_as_ascii() {
    let cues = vec![cue_block(1, b"Hello, World!", b"See you in 2026 (or 2027).")];
    let doc = parse(&file(LATIN, b"VFONTL.V", cues, false), Format::Cavena890).unwrap();
    assert_eq!(texts(&doc), vec!["Hello, World!\nSee you in 2026 (or 2027)."]);
    assert_eq!(doc.language.as_deref(), Some("en"));
}

#[test]
fn a_row_flagged_latin_reads_as_ascii_in_a_hebrew_file() {
    let mut b = cue_block(1, b"Hello", &heb("שלום"));
    b[14] = 0x80;
    let doc = parse(&file(HEBREW, b"HEBNOA.V", vec![b], true), Format::Cavena890).unwrap();
    assert_eq!(texts(&doc), vec!["Hello\nשלום"]);
}

#[test]
fn title_block_is_skipped_and_the_header_is_read() {
    let bytes = hebrew_file(&[("", "אחת"), ("", "שתיים")]);
    let f = cavena890::read(&bytes).unwrap();
    assert_eq!(f.subtitles.len(), 2, "the title block is not a subtitle");
    assert_eq!(f.subtitles[0].block, 1);
    assert_eq!(f.subtitles[0].number, 16);
    assert_eq!((f.subtitles[0].alignment, f.subtitles[0].row), (0x14, 0x16));
    let h = &f.header;
    assert_eq!(
        (h.tape_number.as_str(), h.title.as_str(), h.translator.as_str(), h.episode.as_str(), h.comment.as_str()),
        ("TAPE000123", "Synthetic Show", "A. Translator", "2-7", "made by a test")
    );
    assert_eq!(h.original_title, "Synthetic Show Original");
    assert_eq!((h.language_code, h.second_language_code), (HEBREW, LATIN));
    assert_eq!((h.primary_font.as_str(), h.second_font.as_str()), ("HEBNOA.V", "VFONTL.V"));
    assert_eq!(h.block_count, 3);
    assert_eq!(h.start_of_message, "00:00:00:00");
    assert_eq!(f.charset, Charset::Hebrew);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
}

#[test]
fn the_last_block_may_be_four_bytes_short_or_whole() {
    let cues = || vec![cue_block(1, b"", &heb("ראשונה")), cue_block(2, &heb("אחרונה"), &heb("שורה 2"))];
    let short = file(HEBREW, b"HEBNOA.V", cues(), true);
    let whole = file(HEBREW, b"HEBNOA.V", cues(), false);
    assert_eq!(short.len(), 388 + 3 * 128 - 4);
    assert_eq!(whole.len(), 388 + 3 * 128);
    let a = parse(&short, Format::Cavena890).unwrap();
    let b = parse(&whole, Format::Cavena890).unwrap();
    assert_eq!(texts(&a), vec!["ראשונה", "אחרונה\nשורה 2"]);
    assert_eq!(a.cues, b.cues);
    assert!(a.warnings.iter().all(|w| !w.contains("trailing")), "{:?}", a.warnings);
    // a cut anywhere else is reported, and the whole blocks before it are still read
    let cut = &whole[..whole.len() - 50];
    let c = parse(cut, Format::Cavena890).unwrap();
    assert_eq!(texts(&c), vec!["ראשונה"]);
    assert!(cavena890::read(cut).unwrap().warnings.contains(&Warning::TrailingBytes(78)));
}

#[test]
fn times_are_exact_frames_at_25_fps_unless_told_otherwise() {
    let cues = vec![block(16, HOUR + 237, HOUR + 312, b"", &heb("זמן")), block(32, HOUR + 1_000, HOUR + 1_001, b"", &heb("קצר"))];
    let bytes = file(HEBREW, b"HEBNOA.V", cues, true);
    let doc = parse(&bytes, Format::Cavena890).unwrap();
    let r = FrameRate::FPS_25;
    assert_eq!((doc.cues[0].start, doc.cues[0].end), (r.tick_of(90_237), r.tick_of(90_312)));
    assert_eq!(doc.cues[0].start, Tick(r.frame_duration().0 * 90_237), "frame 90237 at 25 fps");
    assert_eq!((doc.cues[1].start, doc.cues[1].end), (r.tick_of(91_000), r.tick_of(91_001)));
    let at24 = cavena890::parse_with_rate(&bytes, FrameRate::FPS_24).unwrap();
    assert_eq!(at24.cues[0].start, FrameRate::FPS_24.tick_of(90_237));
}

#[test]
fn absolute_timecode_is_reported_not_subtracted() {
    let cues = vec![block(16, HOUR + 237, HOUR + 312, b"", &heb("ראשון")), block(32, HOUR + 2_500, HOUR + 2_560, b"", &heb("אחרון"))];
    let mut bytes = file(HEBREW, b"HEBNOA.V", cues, true);
    put(&mut bytes, 256, b"01:00:00:00");
    let doc = parse(&bytes, Format::Cavena890).unwrap();
    assert!(doc.absolute_timecode);
    assert_eq!(doc.cues[0].start, FrameRate::FPS_25.tick_of(90_237), "nothing is subtracted");
    let note = doc.warnings.iter().find(|w| w.contains("absolute")).expect("a note on the absolute times");
    assert!(note.contains("01:00:09:12") && note.contains("01:01:42:10"), "first in and last out: {note}");
    assert!(note.contains("Synthetic Show") && note.contains("2-7"), "title and episode: {note}");
}

#[test]
fn language_reaches_the_caption_track() {
    let doc = parse(&hebrew_file(&[("", "שלום")]), Format::Cavena890).unwrap();
    let mut next = 1;
    let mut alloc = || {
        next += 1;
        next
    };
    let track = track_from_document(&doc, TrackId(1), "he", CaptionFormat::Subtitle, &mut alloc);
    assert_eq!(track.language, "he");
    assert_eq!(track.captions[0].text, "שלום");
}

#[test]
fn alignment_reaches_the_cue_settings_only_when_it_is_left_or_right() {
    let mut left = cue_block(1, b"", &heb("שמאל"));
    left[8] = 0x50;
    let mut right = cue_block(2, b"", &heb("ימין"));
    right[8] = 0x52;
    let centre = cue_block(3, b"", &heb("מרכז"));
    let doc = parse(&file(HEBREW, b"HEBNOA.V", vec![left, right, centre], true), Format::Cavena890).unwrap();
    let settings: Vec<&str> = doc.cues.iter().map(|c| c.settings.as_str()).collect();
    assert_eq!(settings, vec!["align:left", "align:right", ""]);
}

#[test]
fn repeated_block_numbers_add_rows_to_one_subtitle() {
    let first = block(16, HOUR, HOUR + 50, &heb("שורה 1"), &heb("שורה 2"));
    let more = block(16, HOUR, HOUR + 50, &heb("שורה 3"), b"");
    let next = block(32, HOUR + 60, HOUR + 90, b"", &heb("הבא"));
    let doc = parse(&file(HEBREW, b"HEBNOA.V", vec![first, more, next], true), Format::Cavena890).unwrap();
    assert_eq!(texts(&doc), vec!["שורה 1\nשורה 2\nשורה 3", "הבא"]);
}

#[test]
fn bytes_without_an_established_meaning_are_kept_and_warned() {
    let mut row = heb("שלום ");
    row.push(0x9b);
    let bytes = file(HEBREW, b"HEBNOA.V", vec![cue_block(1, b"", &row)], true);
    let f = cavena890::read(&bytes).unwrap();
    assert_eq!(f.subtitles[0].lines, vec!["שלום \u{FFFD}"]);
    assert_eq!(f.warnings, vec![Warning::Undecoded { subtitle: 0, line: 2, raw: row }]);
    let doc = parse(&bytes, Format::Cavena890).unwrap();
    let w = doc.warnings.iter().find(|w| w.contains("U+FFFD")).expect("a warning on the cue");
    assert!(w.contains("01:00:04:00") && w.contains("9b"), "{w}");
}

/// No reference file has Latin letters in a Hebrew row (ASCII capitals are Hebrew letters there).
/// Small Latin letters are read as the screen would show them (a Latin stretch between Hebrew
/// words, with the digits around it, reversed as one run) and flagged: "21 iphone" and
/// "iphone 21" look the same on screen.
#[test]
fn latin_letters_in_a_hebrew_row_are_read_but_flagged() {
    let mut row = heb("קניתי ");
    row.extend_from_slice(b"12 enohpi");
    row.extend_from_slice(&heb(" ב-005."));
    let bytes = file(HEBREW, b"HEBNOA.V", vec![cue_block(1, b"", &row)], true);
    let f = cavena890::read(&bytes).unwrap();
    assert_eq!(f.subtitles[0].lines, vec!["קניתי iphone 21 ב-500."]);
    assert_eq!(f.warnings, vec![Warning::UnverifiedOrder { subtitle: 0, line: 2, raw: row }]);
}

#[test]
fn header_count_mismatch_is_read_with_a_warning() {
    let mut bytes = hebrew_file(&[("", "אחת")]);
    put(&mut bytes, 179, &7u16.to_le_bytes());
    assert!(!cavena890::sniff(&bytes), "detection needs the count");
    let f = cavena890::read(&bytes).unwrap();
    assert_eq!(f.subtitles.len(), 1);
    assert!(f.warnings.contains(&Warning::CountMismatch { header: 7, blocks: 2 }));
}

#[test]
fn detection() {
    let he = hebrew_file(&[("", "אחת"), ("", "שתיים")]);
    assert_eq!(detect(&he, Some("890")), Some(Format::Cavena890));
    assert_eq!(detect(&he, None), Some(Format::Cavena890));
    let whole = file(RUSSIAN, b"KYRIL2.V", vec![cue_block(1, b"", b"Da.")], false);
    assert_eq!(detect(&whole, Some("890")), Some(Format::Cavena890), "a whole last block too");
    let latin = file(LATIN, b"VFONTL.V", vec![cue_block(1, b"", b"Yes.")], true);
    assert!(cavena890::sniff(&latin));

    // not a size the blocks make
    assert!(!cavena890::sniff(&he[..he.len() - 1]));
    // decreasing times
    let mut back = he.clone();
    put(&mut back, 388 + 2 * 128 + 2, &(HOUR - 1).to_be_bytes()[1..]);
    assert!(!cavena890::sniff(&back));
    // the font name may also start at the field's start; it must be there and ASCII
    let mut left = he.clone();
    left[181..195].fill(0);
    put(&mut left, 181, b"HEBNOA.V");
    assert!(cavena890::sniff(&left));
    let mut nofont = he.clone();
    nofont[181..195].fill(0);
    assert!(!cavena890::sniff(&nofont));
    let mut binary = he.clone();
    put(&mut binary, 187, &[0x48, 0x9b, 0x45]);
    assert!(!cavena890::sniff(&binary));
    // rows not padded with 0x7F
    let mut nopad = he.clone();
    for k in 0..3 {
        for row in [16, 73] {
            nopad[388 + k * 128 + row..388 + k * 128 + row + 51].fill(b' ');
        }
    }
    assert!(!cavena890::sniff(&nopad));
    // a text file named .890 is nothing
    assert_eq!(detect(b"1\n00:00:01,000 --> 00:00:02,000\nhi\n", Some("890")), None);
}

/// An EBU STL file is never taken for Cavena 890, even when its sizes, the bytes at 179 and the
/// font fields happen to fit.
#[test]
fn detection_never_claims_ebu_stl() {
    let doc = Document {
        cues: vec![filmcraft_captions::Cue { start: Tick(0), end: FrameRate::FPS_25.tick_of(50), text: "Hello".into(), ..Default::default() }],
        ..Default::default()
    };
    let mut s = stl::write(&doc, None);
    assert_eq!((s.len() - 388) % 128, 124, "an STL file has the 890 size arithmetic");
    let blocks = (s.len() - 388).div_ceil(128) as u16;
    put(&mut s, 179, &blocks.to_le_bytes());
    s[181..195].fill(0);
    put(&mut s, 187, b"HEBNOA.V");
    put(&mut s, 246, b"VFONTL.V");
    assert!(!cavena890::sniff(&s));
    assert_eq!(detect(&s, Some("890")), Some(Format::Stl));
    assert!(cavena890::read(&s).is_err(), "the reader refuses STL too");
}

#[test]
fn writing_is_unsupported() {
    let doc = parse(&hebrew_file(&[("", "שלום")]), Format::Cavena890).unwrap();
    assert!(!Format::Cavena890.can_write());
    assert!(Format::ALL.iter().filter(|f| **f != Format::Cavena890).all(|f| f.can_write()));
    assert_eq!(try_write(&doc, Format::Cavena890, WriteOptions::default()), Err(Error::WriteUnsupported("Cavena 890")));
    assert!(write(&doc, Format::Cavena890, WriteOptions::default()).is_empty());
    assert!(try_write(&doc, Format::Srt, WriteOptions::default()).is_ok_and(|b| !b.is_empty()));
    assert!(Format::Cavena890.is_binary());
    assert_eq!(Format::from_name(".890"), Some(Format::Cavena890));
}

#[test]
fn short_input_is_not_a_file() {
    assert!(matches!(cavena890::read(&[0u8; 100]), Err(Error::NotFormat(_))));
    assert!(!cavena890::sniff(&[0u8; 100]));
    assert!(!cavena890::sniff(&[]));
}

proptest! {
    /// Random bytes of any size the blocks make are never detected.
    #[test]
    fn random_bytes_are_never_detected(n in 1usize..8, short in any::<bool>(), seed in proptest::collection::vec(any::<u8>(), 1..64)) {
        let len = 388 + 128 * n - if short { 4 } else { 0 };
        let bytes: Vec<u8> = (0..len).map(|i| seed[i % seed.len()].wrapping_mul(31).wrapping_add((i * 7) as u8)).collect();
        prop_assert!(!cavena890::sniff(&bytes));
        prop_assert_ne!(detect(&bytes, Some("890")), Some(Format::Cavena890));
    }

    /// The reader never panics on a valid header followed by arbitrary blocks, nor on arbitrary
    /// rows inside well-formed blocks.
    #[test]
    fn reader_is_total(
        tail in proptest::collection::vec(any::<u8>(), 0..1200),
        row in proptest::collection::vec(any::<u8>(), 0..51),
        lang in prop_oneof![Just(HEBREW), Just(RUSSIAN), Just(LATIN), any::<u8>()],
    ) {
        let mut bytes = header(lang, b"HEBNOA.V", 3, b"01:00:00:00");
        bytes.extend(tail);
        let _ = cavena890::read(&bytes);
        let _ = parse(&bytes, Format::Cavena890);
        let mut b = cue_block(1, &row, &row);
        b[14] = row.first().copied().unwrap_or(0);
        let f = file(lang, b"HEBNOA.V", vec![b], true);
        let doc = parse(&f, Format::Cavena890).unwrap();
        prop_assert!(doc.cues.len() <= 1);
    }
}
