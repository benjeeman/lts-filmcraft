//! Right-to-left text in the interface.
//!
//! egui 0.36 shapes each run of one font with harfrust, which sets a Hebrew or Arabic run right to
//! left, but it lays a line's runs out left to right in their logical order and starts a new run
//! wherever the font changes (Inter's spaces between Hebrew words). A Hebrew sentence then shows
//! each word right to left but the words in reverse order, its final punctuation on the wrong side.
//! For text with right-to-left letters the helpers here build the layout job in display order
//! instead (UAX #9 through `filmcraft_text::bidi`): each line's directional runs left to right on
//! screen, each right-to-left run as one section in [`RTL_FAMILY`], whose first face is the
//! installed Hebrew / Arabic face, so egui shapes the run, spaces and punctuation included, right to
//! left as a whole. Text without right-to-left letters is laid out exactly as before.
//!
//! Used for read-only text (timeline clip and caption labels, [`paint_text`]) and for the captions
//! list while a caption is not being edited ([`edit_layouter`]). Text being edited, and other
//! labels, keep egui's own layout: their letters show (the theme installs the face) but a
//! multi-word right-to-left line reads in logical word order, and the caret moves in logical order.

use std::ops::Range;
use std::sync::Arc;

use egui::epaint::text::FontsView;
use egui::text::{Galley, LayoutJob, TextFormat};
use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect};

use crate::theme::RTL_FAMILY;

/// Whether `text` has a right-to-left letter (Hebrew, Arabic…).
pub fn has_rtl(text: &str) -> bool {
    filmcraft_text::bidi::has_rtl(text)
}

/// The mirror image of a paired punctuation mark, for marks turned by hand.
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        c => c,
    }
}

/// The formats of left-to-right and right-to-left runs.
struct Formats {
    ltr: TextFormat,
    rtl: TextFormat,
}

impl Formats {
    fn new(font: &FontId, color: Color32) -> Formats {
        let ltr = TextFormat::simple(font.clone(), color);
        let rtl = TextFormat { font_id: FontId::new(font.size, FontFamily::Name(RTL_FAMILY.into())), ..ltr.clone() };
        Formats { ltr, rtl }
    }
}

/// Append line `line` (a byte range) of `paragraph` in display order: its directional runs left to
/// right, each right-to-left run as one section in the right-to-left family (egui sets it right to
/// left as a whole).
fn push_line(job: &mut LayoutJob, paragraph: &str, line: Range<usize>, f: &Formats) {
    for (run, rtl) in filmcraft_text::bidi::visual_runs(paragraph, None, line) {
        let text = &paragraph[run];
        if !rtl {
            job.append(text, 0.0, f.ltr.clone());
        } else if text.chars().any(filmcraft_text::bidi::is_rtl_char) {
            job.append(text, 0.0, f.rtl.clone());
        } else {
            // neutrals alone (the "!" closing a Hebrew line after a Latin word): egui would set
            // them left to right, so they are turned here
            let turned: String = text.chars().rev().map(mirror).collect();
            job.append(&turned, 0.0, f.ltr.clone());
        }
    }
}

/// The layout job for `text` (lines split at `\n`, no wrapping) in display order; text without
/// right-to-left letters gets the job `Painter::text` lays out.
pub fn job(text: &str, font: FontId, color: Color32) -> LayoutJob {
    if !has_rtl(text) {
        return LayoutJob::simple(text.to_owned(), font, color, f32::INFINITY);
    }
    let f = Formats::new(&font, color);
    let mut job = LayoutJob::default();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            job.append("\n", 0.0, f.ltr.clone());
        }
        push_line(&mut job, line, 0..line.len(), &f);
    }
    job
}

/// The rows of `paragraph` wrapped at `wrap_width` (as egui wraps: after a space, a word longer
/// than a row overflows it), as byte ranges without the space that closes each row.
fn rows(fonts: &mut FontsView<'_>, paragraph: &str, f: &Formats, wrap_width: f32) -> Vec<Range<usize>> {
    let mut width = |line: Range<usize>| {
        let mut job = LayoutJob::default();
        push_line(&mut job, paragraph, line, f);
        fonts.layout_job(job).size().x
    };
    let mut rows = Vec::new();
    // the current row starts at `start`; `end` is the space that would close it
    let (mut start, mut end) = (0, 0);
    for (space, _) in paragraph.match_indices(' ').chain([(paragraph.len(), "")]) {
        if end > start && width(start..space) > wrap_width {
            rows.push(start..end);
            start = end + 1;
        }
        end = space;
    }
    rows.push(start..paragraph.len());
    rows
}

/// A text edit's layout job for `text` wrapped at `wrap_width`, in display order: rows break
/// between words where they would pass `wrap_width`, each row is ordered on its own (UAX #9, in its
/// paragraph's direction) and the first words are on the first row. The space that closes a row
/// becomes its line break, so the job holds exactly the text's characters (an edit maps its cursor
/// through them). Text without right-to-left letters gets egui's own job.
pub fn wrapped_job(fonts: &mut FontsView<'_>, text: &str, font: FontId, color: Color32, wrap_width: f32) -> LayoutJob {
    if !has_rtl(text) {
        return LayoutJob::simple(text.to_owned(), font, color, wrap_width);
    }
    let f = Formats::new(&font, color);
    let mut job = LayoutJob::default();
    for (i, paragraph) in text.split('\n').enumerate() {
        if i > 0 {
            job.append("\n", 0.0, f.ltr.clone());
        }
        for (k, row) in rows(fonts, paragraph, &f, wrap_width).into_iter().enumerate() {
            if k > 0 {
                job.append("\n", 0.0, f.ltr.clone());
            }
            push_line(&mut job, paragraph, row, &f);
        }
    }
    job
}

/// A [`egui::TextEdit`] layouter that shows the edit's text in display order ([`wrapped_job`]) in
/// `font`, with the edit's row height, painted in the edit's own text colour.
pub fn edit_layouter(font: FontId) -> impl FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> Arc<Galley> {
    move |ui, text, wrap_width| {
        let line_height = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
        let mut job = ui.fonts_mut(|f| wrapped_job(f, text.as_str(), font.clone(), Color32::PLACEHOLDER, wrap_width));
        for section in &mut job.sections {
            section.format.line_height = Some(line_height);
        }
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

/// [`Painter::text`] in display order: right-to-left text is laid out by [`job`], other text as
/// `Painter::text` lays it out. Returns where the text ended up.
pub fn paint_text(painter: &Painter, pos: Pos2, anchor: Align2, text: impl ToString, font: FontId, color: Color32) -> Rect {
    let text = text.to_string();
    if !has_rtl(&text) {
        return painter.text(pos, anchor, text, font, color);
    }
    let galley = painter.layout_job(job(&text, font, color));
    let rect = anchor.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ThemeKind, Tokens, install};
    use egui::FontFamily;

    fn ctx() -> Option<egui::Context> {
        let ctx = egui::Context::default();
        install(&ctx, &Tokens::for_kind(ThemeKind::default()));
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        if !crate::theme::tests::rtl_font_installed() {
            eprintln!("SKIPPED: no installed Segoe UI, Arial or Tahoma face covers Hebrew and Arabic");
            return None;
        }
        Some(ctx)
    }

    fn font() -> FontId {
        FontId::new(13.0, FontFamily::Proportional)
    }

    /// Each row of a laid-out job: its characters from left to right on screen. (egui follows a
    /// right-to-left run with zero-width copies of all but one of its characters; they draw
    /// nothing and are left out.)
    fn rows_on_screen(ctx: &egui::Context, job: LayoutJob) -> Vec<String> {
        let galley = ctx.fonts_mut(|f| f.layout_job(job));
        galley
            .rows
            .iter()
            .map(|row| {
                let mut glyphs: Vec<(f32, char)> = row.glyphs.iter().filter(|g| g.advance_width > 0.0).map(|g| (g.pos.x, g.chr)).collect();
                glyphs.sort_by(|a, b| a.0.total_cmp(&b.0));
                glyphs.into_iter().map(|(_, c)| c).collect()
            })
            .collect()
    }

    /// A Hebrew or Arabic label reads right to left as a whole: the first word at the right end,
    /// the final punctuation at the left, Latin words and numbers left to right in their place.
    /// egui alone sets each word right to left but lays the words out left to right.
    #[test]
    fn right_to_left_labels_are_laid_out_in_display_order() {
        let Some(ctx) = ctx() else { return };
        let cases = [
            ("שלום עולם!", "!םלוע םולש"),
            ("iPhone חדש", "iPhone שדח"),
            ("שלום world!", "!world םולש"),
            ("פרק 12 מתוך 20", "20 ךותמ 12 קרפ"),
            ("Hello world", "Hello world"),
        ];
        for (text, screen) in cases {
            assert_eq!(rows_on_screen(&ctx, job(text, font(), Color32::WHITE)), vec![screen.to_string()], "{text}");
        }
        // Arabic: the first letter of the first word is the rightmost glyph
        let rows = rows_on_screen(&ctx, job("مرحبا بالعالم", font(), Color32::WHITE));
        assert_eq!(rows[0].chars().last(), Some('م'), "{rows:?}");
        assert_eq!(rows[0].chars().next(), Some('م'), "the last letter of the second word is the leftmost: {rows:?}");
        // two lines, each in its own direction
        assert_eq!(rows_on_screen(&ctx, job("Hello\nשלום עולם", font(), Color32::WHITE)), vec!["Hello".to_string(), "םלוע םולש".to_string()]);
    }

    /// Text without right-to-left letters is laid out exactly as before.
    #[test]
    fn left_to_right_text_is_unchanged() {
        let j = job("Hello world", font(), Color32::WHITE);
        assert_eq!(j.text, "Hello world");
        assert_eq!(j.sections.len(), 1);
        assert_eq!(j.sections[0].format.font_id, font());
    }

    /// A text edit's wrapped Hebrew caption: each row in display order, the rows top to bottom in
    /// reading order (the first words on the first row), and every character of the text kept once
    /// (the edit maps its cursor through them).
    #[test]
    fn wrapped_rows_are_each_in_display_order() {
        let Some(ctx) = ctx() else { return };
        let text = "שלום עולם יפה מאוד";
        let one_row = ctx.fonts_mut(|f| f.layout_no_wrap("שלום עולם ".into(), font(), Color32::WHITE)).size().x;
        let job = ctx.fonts_mut(|f| wrapped_job(f, text, font(), Color32::WHITE, one_row + 2.0));
        assert_eq!(job.text.chars().count(), text.chars().count(), "every character kept: {:?}", job.text);
        let rows = rows_on_screen(&ctx, job);
        assert_eq!(rows.first().map(String::as_str), Some("םלוע םולש"), "{rows:?}");
        assert_eq!(rows.last().map(String::as_str), Some("דואמ הפי"), "{rows:?}");
        // short text: one row, as a label
        let short = ctx.fonts_mut(|f| wrapped_job(f, "שלום עולם!", font(), Color32::WHITE, 500.0));
        assert_eq!(rows_on_screen(&ctx, short), vec!["!םלוע םולש".to_string()]);
    }
}
