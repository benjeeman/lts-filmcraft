//! Display order for hosts that shape text themselves (the egui interface): the Unicode
//! bidirectional algorithm (UAX #9) over one paragraph, as directional runs in visual order.
//! [`crate::layout`] does the same internally for the text it sets.

use std::ops::Range;

use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo};

/// Whether `c` is a strong right-to-left character (bidi class R or AL: Hebrew, Arabic, Syriac,
/// Thaana, N'Ko…).
pub fn is_rtl_char(c: char) -> bool {
    matches!(unicode_bidi::bidi_class(c), BidiClass::R | BidiClass::AL)
}

/// Whether `text` has a strong right-to-left character.
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_char)
}

/// One line of a paragraph in display order (UAX #9 rules L1–L2). `paragraph` is one paragraph
/// (no line break), `rtl` its direction (None = from its first strong character) and `line` a byte
/// range of it (the whole paragraph for a single line; one row of it after wrapping). Returns the
/// line's directional runs from left to right on screen: a byte range of `paragraph` and whether
/// the run reads right to left. A right-to-left run keeps its characters in logical order, for a
/// shaper that sets it right to left as a whole.
pub fn visual_runs(paragraph: &str, rtl: Option<bool>, line: Range<usize>) -> Vec<(Range<usize>, bool)> {
    let line = line.start.min(paragraph.len())..line.end.min(paragraph.len());
    if line.is_empty() {
        return Vec::new();
    }
    let info = ParagraphBidiInfo::new(paragraph, rtl.map(|r| if r { Level::rtl() } else { Level::ltr() }));
    let (levels, runs) = info.visual_runs(line);
    let mut out = Vec::with_capacity(runs.len());
    for r in runs.into_iter().filter(|r| !r.is_empty()) {
        let rtl = levels[r.start].is_rtl();
        out.push((r, rtl));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The runs of `text` as one line: (text, right to left), left to right on screen.
    fn runs(text: &str, rtl: Option<bool>) -> Vec<(&str, bool)> {
        visual_runs(text, rtl, 0..text.len()).into_iter().map(|(r, rtl)| (&text[r], rtl)).collect()
    }

    #[test]
    fn right_to_left_letters() {
        assert!("שלום".chars().all(is_rtl_char) && "مرحبا".chars().all(is_rtl_char));
        assert!(!"Az09 .,!?()".chars().any(is_rtl_char));
        assert!(has_rtl("iPhone חדש") && has_rtl("سلام") && !has_rtl("Hello, world 2026"));
    }

    #[test]
    fn a_hebrew_line_is_one_right_to_left_run_with_its_punctuation() {
        assert_eq!(runs("שלום עולם!", None), vec![("שלום עולם!", true)]);
        assert_eq!(runs("مرحبا بالعالم", None), vec![("مرحبا بالعالم", true)]);
        assert_eq!(runs("Hello world.", None), vec![("Hello world.", false)]);
    }

    /// A Hebrew line that starts with a Latin word: in a right-to-left paragraph the Latin word sits
    /// at the right end and the full stop at the left; from its first strong character the line is
    /// left to right.
    #[test]
    fn a_latin_first_hebrew_line() {
        let line = "iPhone חדש בחנות.";
        assert_eq!(runs(line, Some(true)), vec![(" חדש בחנות.", true), ("iPhone", false)]);
        assert_eq!(runs(line, None), vec![("iPhone ", false), ("חדש בחנות", true), (".", false)]);
        assert_eq!(runs("Hello!", Some(true)), vec![("!", true), ("Hello", false)]);
    }

    /// Numbers in a Hebrew line read left to right in their place.
    #[test]
    fn numbers_in_a_hebrew_line() {
        assert_eq!(runs("פרק 12 מתוך 20", None), vec![("20", false), (" מתוך ", true), ("12", false), ("פרק ", true)]);
    }

    /// Each row of a wrapped paragraph is ordered on its own, in the paragraph's direction.
    #[test]
    fn rows_of_a_wrapped_paragraph() {
        let p = "שלום עולם iPhone חדש";
        let cut = "שלום עולם ".len();
        let row = |r: Range<usize>| visual_runs(p, None, r).into_iter().map(|(r, rtl)| (&p[r], rtl)).collect::<Vec<_>>();
        assert_eq!(row(0..cut), vec![("שלום עולם ", true)]);
        assert_eq!(row(cut..p.len()), vec![(" חדש", true), ("iPhone", false)]);
    }
}
