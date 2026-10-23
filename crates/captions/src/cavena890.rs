//! Cavena 890 (`.890`): binary broadcast subtitles. Read only.
//!
//! No public specification exists. This reader follows public field-level descriptions of the
//! format and byte analysis of broadcast reference files (Hebrew and Russian); the crate README
//! lists the sources and how sure each field is.
//!
//! - **File:** a 388-byte header, then 128-byte blocks. Block 0 is a title block (zero times;
//!   title, episode and file name in the Latin set), skipped. Every later block is a subtitle; a
//!   block repeating the previous block's number and times adds two more rows to it. The last
//!   block may stop 4 bytes short (after its bottom row).
//! - **Header:** tape number at 2 (20 bytes), title at 40 (28), translator at 68 (28), episode at
//!   105 (11), character-set codes at 146 (the second set: 0x09) and 147 (the rows' set: 0x8F
//!   Hebrew, 0x56 Russian, 0x09 Latin), comment at 148 (24), block count at 179 (u16
//!   little-endian, title block included), the primary font name in 181–194 (the reference files
//!   start it at 187), original title at 218 (28), the second font name at 246 (8),
//!   start-of-message text `HH:MM:SS:FF` at 256 (kept, not used for times).
//! - **Block:** number at 0 (u16 big-endian), in and out at 2 and 5 (u24 big-endian frame counts),
//!   alignment at 8 (0x50 left, 0x52 right; 0x54 and 0x14 centred), byte 15 (0x16 in every
//!   reference cue: probably the row), top row at 16 and bottom row at 73 (51 bytes each, padded
//!   with 0x7F). A row whose flag byte (14 for the top row, 72 for the bottom row) has bit 0x80
//!   set is in the Latin set (the title block's rows are).
//! - **Hebrew rows:** letters 0x40–0x5A in Unicode order (U+05D0 + byte − 0x40, final forms
//!   included), ASCII digits and punctuation. A row holds the screen read from right to left:
//!   Hebrew words, dialogue dashes, quotes and end punctuation are in reading order, but every
//!   left-to-right run (digits, with a separator between digits: `2,500`, `14:30`, `24/7`) is
//!   reversed. The reader turns those runs back with the run rules of the Unicode bidirectional
//!   algorithm for a right-to-left line.
//! - **Russian rows:** a phonetic 7-bit layout in which Latin letters stand for Cyrillic ones
//!   (`a` а, `v` в, `w` ю, `q` я, `x` ы, `y` ь, `` ` `` ж, `{` ш, `}` щ, `|` э, `~` ч, 0x1D ъ;
//!   capitals alike at 0x40–0x5E); digits and punctuation are ASCII.
//! - **Latin rows** (code 0x09, 0x00, and codes with no table here): ASCII.
//! - Bytes with no established meaning read as U+FFFD with a [`Warning::Undecoded`] that keeps the
//!   row's bytes. A Hebrew row with characters whose stored order the reference files do not show
//!   (Latin letters, brackets, `# $ % & * + ; < = >`) is read by the same rule and flagged with
//!   [`Warning::UnverifiedOrder`].
//! - **Times:** frame counts at a rate the file does not store ([`RATE`], 25 fps, unless told
//!   otherwise). They are the source's absolute timecode (the reference programmes start at
//!   01:00:00:00) and are never shifted: [`Document::absolute_timecode`] is set and a note gives the
//!   first and last times, so an importer can map the source's start timecode to its first frame.

use filmcraft_time::{FrameRate, frames_to_fields};

use crate::{Cue, Document, Error, Result};

/// The frame rate times are counted at when none is given (the format does not store one).
pub const RATE: FrameRate = FrameRate::FPS_25;

const HEADER: usize = 388;
const BLOCK: usize = 128;
/// Where a short last block ends (after its bottom row).
const SHORT: usize = 124;
const ROW: usize = 51;
/// (row offset, flag byte) of the top and bottom rows.
const ROWS: [(usize, usize); 2] = [(16, 14), (73, 72)];
const PAD: u8 = 0x7f;
const LATIN_ROW: u8 = 0x80;
const NAME: &str = "Cavena 890";

/// The character set of a file's subtitle rows (header byte 147).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charset {
    Hebrew,
    Russian,
    Latin,
}

/// The header fields this reader knows (strings trimmed; non-ASCII bytes read as U+FFFD).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub tape_number: String,
    pub title: String,
    pub translator: String,
    pub episode: String,
    pub comment: String,
    pub original_title: String,
    /// Byte 147: the character set of the subtitle rows.
    pub language_code: u8,
    /// Byte 146: the second character set (0x09, Latin, in every reference file).
    pub second_language_code: u8,
    /// The font of the primary set (`HEBNOA.V`, `KYRIL2.V`).
    pub primary_font: String,
    /// The font of the second set (`VFONTL.V`).
    pub second_font: String,
    /// Blocks in the file, the title block included.
    pub block_count: u16,
    /// The start-of-message timecode text, verbatim. Not used: the cue times are absolute.
    pub start_of_message: String,
}

/// One subtitle (one block, or a run of blocks with the same number and times).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subtitle {
    /// Index of its first block (block 0 is the title block).
    pub block: usize,
    pub number: u16,
    /// In and out, in frames (absolute timecode).
    pub start: i64,
    pub end: i64,
    /// Block byte 8.
    pub alignment: u8,
    /// Block byte 15 (probably the vertical position).
    pub row: u8,
    /// Its non-empty rows, top first, as Unicode text in reading order.
    pub lines: Vec<String>,
}

/// A problem found while reading. `subtitle` indexes [`File::subtitles`]; `line` counts the
/// subtitle's rows from 1 (top row of its first block = 1, bottom row = 2, next block = 3…).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Warning {
    /// Bytes with no established meaning in a row, read as U+FFFD; `raw` is the row as stored.
    Undecoded { subtitle: usize, line: usize, raw: Vec<u8> },
    /// A Hebrew row with characters whose stored order the reference files do not show (Latin
    /// letters, brackets, other symbols), read by the digit-run rule; `raw` is the row as stored.
    UnverifiedOrder { subtitle: usize, line: usize, raw: Vec<u8> },
    /// The header's block count differs from the blocks in the file.
    CountMismatch { header: u16, blocks: usize },
    /// Bytes after the last block that make no block.
    TrailingBytes(usize),
    /// A block whose out is not after its in (skipped).
    BadTiming { block: usize, start: i64, end: i64 },
    /// A character-set code with no table here; its rows are read as ASCII.
    UnknownLanguage(u8),
}

/// A Cavena 890 file as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub header: Header,
    pub charset: Charset,
    /// BCP-47 language of the rows (`he`, `ru`, `en`), when the code says.
    pub language: Option<&'static str>,
    pub subtitles: Vec<Subtitle>,
    pub warnings: Vec<Warning>,
}

/// Whether bytes look like a Cavena 890 file: the sizes the blocks make, the block count at 179,
/// ASCII font names at 181 and 246, in-times that never decrease, rows padded with 0x7F, and no
/// EBU STL signature.
pub fn sniff(bytes: &[u8]) -> bool {
    if bytes.len() < HEADER + SHORT || is_stl(bytes) {
        return false;
    }
    let body = bytes.len() - HEADER;
    let blocks = match body % BLOCK {
        0 => body / BLOCK,
        SHORT => body / BLOCK + 1,
        _ => return false,
    };
    if usize::from(u16::from_le_bytes([bytes[179], bytes[180]])) != blocks || !font_name(&bytes[181..195]) || !font_name(&bytes[246..254]) {
        return false;
    }
    let (mut previous, mut padded) = (0, false);
    for b in bytes[HEADER..].chunks(BLOCK) {
        let (start, end) = (u24(&b[2..5]), u24(&b[5..8]));
        if end < start || start < previous {
            return false;
        }
        previous = start;
        for (at, _) in ROWS {
            let row = &b[at..at + ROW];
            if let Some(p) = row.iter().position(|&c| c == PAD) {
                if row[p..].iter().any(|&c| c != PAD) {
                    return false;
                }
                padded = true;
            }
        }
    }
    padded
}

/// Read a file: header, subtitles in block order, warnings.
pub fn read(bytes: &[u8]) -> Result<File> {
    if bytes.len() < HEADER || is_stl(bytes) {
        return Err(Error::NotFormat(NAME));
    }
    let header = Header::read(&bytes[..HEADER]);
    let mut warnings = Vec::new();
    let (charset, language) = match header.language_code {
        0x8f => (Charset::Hebrew, Some("he")),
        0x56 => (Charset::Russian, Some("ru")),
        0x09 => (Charset::Latin, Some("en")),
        0x00 => (Charset::Latin, None),
        code => {
            warnings.push(Warning::UnknownLanguage(code));
            (Charset::Latin, None)
        }
    };
    let (whole, tail) = bytes[HEADER..].as_chunks::<BLOCK>();
    let mut blocks = whole.to_vec();
    if tail.len() >= SHORT {
        let mut last = [0u8; BLOCK];
        last[..tail.len()].copy_from_slice(tail);
        blocks.push(last);
    } else if !tail.is_empty() {
        warnings.push(Warning::TrailingBytes(tail.len()));
    }
    if usize::from(header.block_count) != blocks.len() {
        warnings.push(Warning::CountMismatch { header: header.block_count, blocks: blocks.len() });
    }
    let mut subtitles: Vec<Subtitle> = Vec::new();
    let mut rows_so_far = 0;
    for (k, b) in blocks.iter().enumerate() {
        let number = u16::from_be_bytes([b[0], b[1]]);
        let (start, end) = (u24(&b[2..5]), u24(&b[5..8]));
        if k == 0 && start == 0 && end == 0 {
            continue; // the title block
        }
        let more = subtitles.last().is_some_and(|s| s.number == number && s.start == start && s.end == end);
        if !more {
            if end <= start {
                warnings.push(Warning::BadTiming { block: k, start, end });
                continue;
            }
            subtitles.push(Subtitle { block: k, number, start, end, alignment: b[8], row: b[15], lines: Vec::new() });
            rows_so_far = 0;
        }
        let subtitle = subtitles.len().saturating_sub(1);
        for (at, flag) in ROWS {
            rows_so_far += 1;
            let raw = stored_row(&b[at..at + ROW]);
            let set = if b[flag] & LATIN_ROW != 0 { Charset::Latin } else { charset };
            let row = decode_row(raw, set);
            let line = rows_so_far;
            if row.undecoded {
                warnings.push(Warning::Undecoded { subtitle, line, raw: raw.to_vec() });
            }
            if row.unverified {
                warnings.push(Warning::UnverifiedOrder { subtitle, line, raw: raw.to_vec() });
            }
            if row.text.is_empty() {
                continue;
            }
            if let Some(s) = subtitles.last_mut() {
                s.lines.push(row.text);
            }
        }
    }
    Ok(File { header, charset, language, subtitles, warnings })
}

/// Parse into a [`Document`] at [`RATE`] (25 fps).
pub fn parse(bytes: &[u8]) -> Result<Document> {
    parse_with_rate(bytes, RATE)
}

/// Parse into a [`Document`], counting frames at `rate`.
pub fn parse_with_rate(bytes: &[u8], rate: FrameRate) -> Result<Document> {
    read(bytes).map(|f| f.to_document(rate))
}

impl File {
    /// The caption document, with frames counted at `rate`: one cue per subtitle with text (its
    /// rows joined by line breaks; left or right alignment as an `align:` setting), the
    /// language, the absolute-timecode flag and a note, and the warnings as text.
    pub fn to_document(&self, rate: FrameRate) -> Document {
        let mut doc = Document { language: self.language.map(str::to_string), absolute_timecode: true, ..Default::default() };
        for s in self.subtitles.iter().filter(|s| !s.lines.is_empty()) {
            let align = match s.alignment {
                0x50 => "align:left",
                0x52 => "align:right",
                _ => "",
            };
            let (start, end) = (rate.tick_of(s.start), rate.tick_of(s.end));
            doc.cues.push(Cue { start, end, text: s.lines.join("\n"), settings: align.into(), ..Default::default() });
        }
        let first = self.subtitles.iter().map(|s| s.start).min();
        let last = self.subtitles.iter().map(|s| s.end).max();
        if let (Some(first), Some(last)) = (first, last) {
            let (title, episode, som) = (&self.header.title, &self.header.episode, &self.header.start_of_message);
            let (first, last) = (timecode(first, rate), timecode(last, rate));
            let fps = if rate.den == 1 { rate.num.to_string() } else { format!("{:.3}", rate.as_f64()) };
            doc.warnings.push(format!(
                "{NAME} \"{title}\" episode \"{episode}\": cue times are absolute timecode at {fps} fps (first in {first}, last out {last}); \
                 the header's start of message reads \"{som}\" and is not used. Nothing was subtracted: place the cues by mapping the \
                 source's start timecode (01:00:00:00 on broadcast masters) to its first frame."
            ));
        }
        doc.warnings.extend(self.warnings.iter().map(|w| self.describe(w, rate)));
        doc
    }

    fn describe(&self, w: &Warning, rate: FrameRate) -> String {
        let at = |subtitle: &usize| self.subtitles.get(*subtitle).map(|s| timecode(s.start, rate)).unwrap_or_default();
        let hex = |raw: &[u8]| raw.iter().map(|b| format!("{b:02x}")).collect::<String>();
        match w {
            Warning::Undecoded { subtitle, line, raw } => {
                let (at, raw) = (at(subtitle), hex(raw.as_slice()));
                format!("{NAME}: subtitle at {at}, line {line}: bytes with no established meaning read as U+FFFD (row as stored: {raw})")
            }
            Warning::UnverifiedOrder { subtitle, line, raw } => {
                let (at, raw) = (at(subtitle), hex(raw.as_slice()));
                format!("{NAME}: subtitle at {at}, line {line}: Latin or symbols in a Hebrew row, stored order not established (row as stored: {raw})")
            }
            Warning::CountMismatch { header, blocks } => format!("{NAME}: the header counts {header} blocks, the file has {blocks}"),
            Warning::TrailingBytes(n) => format!("{NAME}: {n} trailing bytes after the last block"),
            Warning::BadTiming { block, start, end } => {
                let (start, end) = (timecode(*start, rate), timecode(*end, rate));
                format!("{NAME}: block {block} skipped: its out {end} is not after its in {start}")
            }
            Warning::UnknownLanguage(code) => format!("{NAME}: character set 0x{code:02X} has no table here; its rows are read as ASCII"),
        }
    }
}

impl Header {
    fn read(h: &[u8]) -> Header {
        Header {
            tape_number: field_text(&h[2..22]),
            title: field_text(&h[40..68]),
            translator: field_text(&h[68..96]),
            episode: field_text(&h[105..116]),
            comment: field_text(&h[148..172]),
            original_title: field_text(&h[218..246]),
            language_code: h[147],
            second_language_code: h[146],
            primary_font: field_text(&h[181..195]),
            second_font: field_text(&h[246..254]),
            block_count: u16::from_le_bytes([h[179], h[180]]),
            start_of_message: field_text(&h[256..267]),
        }
    }
}

fn timecode(frame: i64, rate: FrameRate) -> String {
    let (_, h, m, s, f) = frames_to_fields(frame, rate, false);
    format!("{h:02}:{m:02}:{s:02}:{f:02}")
}

fn is_stl(bytes: &[u8]) -> bool {
    bytes.get(3..6).is_some_and(|s| s == b"STL") || crate::stl::sniff(bytes)
}

fn u24(b: &[u8]) -> i64 {
    (i64::from(b[0]) << 16) | (i64::from(b[1]) << 8) | i64::from(b[2])
}

/// A name of printable ASCII with only NUL or space padding around it (the reference files start
/// the primary font name 6 bytes into its field).
fn font_name(field: &[u8]) -> bool {
    let pad = |c: &u8| *c == 0 || *c == b' ';
    let from = field.iter().position(|c| !pad(c)).unwrap_or(field.len());
    let to = field.iter().rposition(|c| !pad(c)).map_or(from, |p| p + 1);
    from < to && field[from..to].iter().all(u8::is_ascii_graphic)
}

/// An ASCII header field with its NUL and space padding removed (NUL inside reads as a space).
fn field_text(field: &[u8]) -> String {
    let ascii = |c: &u8| match c {
        0 => ' ',
        0x20..=0x7e => char::from(*c),
        _ => '\u{FFFD}',
    };
    field.iter().map(ascii).collect::<String>().trim().to_string()
}

/// A row without its trailing padding (0x7F, or NUL).
fn stored_row(row: &[u8]) -> &[u8] {
    let end = row.iter().rposition(|&c| c != PAD && c != 0).map_or(0, |p| p + 1);
    &row[..end]
}

struct Row {
    text: String,
    undecoded: bool,
    unverified: bool,
}

fn decode_row(raw: &[u8], set: Charset) -> Row {
    let bytes = raw.iter().copied().filter(|&c| c != PAD && c != 0);
    let mut row = match set {
        Charset::Hebrew => hebrew(bytes),
        Charset::Russian => {
            let mut undecoded = false;
            let text = bytes.map(|b| russian(b).unwrap_or_else(|| replacement(&mut undecoded))).collect();
            Row { text, undecoded, unverified: false }
        }
        Charset::Latin => {
            let mut undecoded = false;
            let text = bytes.map(|b| if (0x20..0x7f).contains(&b) { char::from(b) } else { replacement(&mut undecoded) }).collect();
            Row { text, undecoded, unverified: false }
        }
    };
    row.text = row.text.trim().to_string();
    row
}

fn replacement(undecoded: &mut bool) -> char {
    *undecoded = true;
    '\u{FFFD}'
}

/// Cyrillic for 0x60–0x7E, in byte order (`` ` `` a b c … z { | } ~).
const RUSSIAN_SMALL: [char; 31] =
    ['ж', 'а', 'б', 'ц', 'д', 'е', 'ф', 'г', 'х', 'и', 'й', 'к', 'л', 'м', 'н', 'о', 'п', 'я', 'р', 'с', 'т', 'у', 'в', 'ю', 'ы', 'ь', 'з', 'ш', 'э', 'щ', 'ч'];
/// Cyrillic for 0x40–0x5E, in byte order (@ A B C … Z [ \ ] ^).
const RUSSIAN_CAPITAL: [char; 31] =
    ['Ж', 'А', 'Б', 'Ц', 'Д', 'Е', 'Ф', 'Г', 'Х', 'И', 'Й', 'К', 'Л', 'М', 'Н', 'О', 'П', 'Я', 'Р', 'С', 'Т', 'У', 'В', 'Ю', 'Ы', 'Ь', 'З', 'Ш', 'Э', 'Щ', 'Ч'];

fn russian(b: u8) -> Option<char> {
    match b {
        0x20..=0x3f => Some(char::from(b)),
        0x40..=0x5e => RUSSIAN_CAPITAL.get(usize::from(b - 0x40)).copied(),
        0x60..=0x7e => RUSSIAN_SMALL.get(usize::from(b - 0x60)).copied(),
        0x1d => Some('ъ'),
        _ => None,
    }
}

/// Bidirectional character types the reader needs (UAX #9 names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bidi {
    /// Hebrew letter.
    R,
    /// Latin letter.
    L,
    /// Digit.
    En,
    /// Common separator: `, . : /`.
    Cs,
    /// European separator: `+ -`.
    Es,
    /// European terminator: `# $ %`.
    Et,
    /// Space and other neutrals.
    On,
}

/// A Hebrew row: decode, then turn the reversed left-to-right runs back.
fn hebrew(bytes: impl Iterator<Item = u8>) -> Row {
    let (mut undecoded, mut unverified) = (false, false);
    let mut chars = Vec::new();
    let mut types = Vec::new();
    for b in bytes {
        // (character, type, whether the reference files show how it is stored)
        let (c, t, shown) = match b {
            0x40..=0x5a => (char::from_u32(0x5d0 + u32::from(b - 0x40)).unwrap_or('\u{FFFD}'), Bidi::R, true),
            b'0'..=b'9' => (char::from(b), Bidi::En, true),
            b',' | b'.' | b':' | b'/' => (char::from(b), Bidi::Cs, true),
            b'-' => (char::from(b), Bidi::Es, true),
            b' ' | b'!' | b'"' | b'\'' | b'?' => (char::from(b), Bidi::On, true),
            b'a'..=b'z' => (char::from(b), Bidi::L, false),
            b'#' | b'$' | b'%' => (char::from(b), Bidi::Et, false),
            b'+' => (char::from(b), Bidi::Es, false),
            b'&' | b'(' | b')' | b'*' | b';' | b'<' | b'=' | b'>' => (char::from(b), Bidi::On, false),
            _ => (replacement(&mut undecoded), Bidi::On, true),
        };
        unverified |= !shown;
        chars.push(c);
        types.push(t);
    }
    left_to_right_runs_back(&mut chars, &mut types);
    Row { text: chars.into_iter().collect(), undecoded, unverified }
}

/// Reverse the left-to-right runs of a right-to-left line stored as the screen reads from the
/// right (this undoes itself: the same runs are found in either order).
///
/// Within each stretch between Hebrew letters: a single separator between two digits joins them
/// (W4: `2,500`, `14:30`, `24/7`) and terminators next to digits join them (W5: `50%`); each
/// group of digits is one run, and the spaces and punctuation between groups stay where they are
/// (N1 treats digits as right-to-left, so `1, 2, 10` keeps its order). With Latin letters in the
/// stretch, everything from its first letter or digit to its last Latin letter is one run (a
/// Latin word takes the digits after it into its run, W7); on screen that looks the same as
/// digits before the word, which is why such rows are flagged.
fn left_to_right_runs_back(chars: &mut [char], types: &mut [Bidi]) {
    let n = types.len();
    for i in 1..n.saturating_sub(1) {
        if matches!(types[i], Bidi::Cs | Bidi::Es) && types[i - 1] == Bidi::En && types[i + 1] == Bidi::En {
            types[i] = Bidi::En;
        }
    }
    let mut i = 0;
    while i < n {
        if types[i] != Bidi::Et {
            i += 1;
            continue;
        }
        let from = i;
        while i < n && types[i] == Bidi::Et {
            i += 1;
        }
        if (from > 0 && types[from - 1] == Bidi::En) || (i < n && types[i] == Bidi::En) {
            types[from..i].fill(Bidi::En);
        }
    }
    let mut from = 0;
    while from < n {
        if types[from] == Bidi::R {
            from += 1;
            continue;
        }
        let to = types[from..].iter().position(|&t| t == Bidi::R).map_or(n, |p| from + p);
        stretch_back(&mut chars[from..to], &types[from..to]);
        from = to;
    }
}

/// One stretch between Hebrew letters (see [`left_to_right_runs_back`]).
fn stretch_back(chars: &mut [char], types: &[Bidi]) {
    let mut i = 0;
    if let Some(last) = types.iter().rposition(|&t| t == Bidi::L)
        && let Some(first) = types.iter().position(|&t| matches!(t, Bidi::L | Bidi::En))
    {
        chars[first..=last].reverse();
        i = last + 1;
    }
    while i < types.len() {
        if types[i] != Bidi::En {
            i += 1;
            continue;
        }
        let from = i;
        while i < types.len() && types[i] == Bidi::En {
            i += 1;
        }
        chars[from..i].reverse();
    }
}
