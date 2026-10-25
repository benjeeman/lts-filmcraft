//! Caption layout and burn-in.
//!
//! [`overlay`] lays out the caption showing on a track at a time with the track's
//! [`CaptionStyle`] (size relative to a 1080-line frame, colour, background box, outline,
//! alignment, anchor and margin; WebVTT `line:N%` and `align:` cue settings override the style)
//! and rasterises it into a small premultiplied linear-light RGBA [`Overlay`] positioned in the
//! frame. Renderers composite overlays over the finished picture. Lines read in the track's
//! direction: right to left on a Hebrew, Arabic, Persian, Urdu or Yiddish track
//! ([`rtl_language`]), else from each line's first strong character.
//!
//! A vertical (or any other) output lays its subtitles out again with a [`Layout`] (Vertical Edit, spec §6.4): inside
//! an area of the output, at a size in output pixels that shrinks to fit a number of lines, with WebVTT `align:start`
//! and `align:end` resolved against each line's direction, and a part export's edge rule ([`crate::cue_in_range`]).
//! [`cue_box`] gives where a cue lands without drawing it. The 16:9 path above is unchanged.

use filmcraft_color::srgb_to_linear;
use filmcraft_project::vertical::{self, CueEdge};
use filmcraft_project::{Caption, CaptionAlign, CaptionAnchor, CaptionStyle, CaptionTrack, Sequence};
use filmcraft_time::{FrameRate, Tick, TimeRange};

use filmcraft_text::{ParagraphStyle, TextStyle, bidi, layout, render};

/// A rendered caption: premultiplied linear-light RGBA f32 pixels at `(x, y)` in the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Overlay {
    /// Composite over a premultiplied RGBA f32 canvas of `cw`×`ch` pixels.
    pub fn composite_onto(&self, canvas: &mut [f32], cw: usize, ch: usize) {
        for y in 0..self.h {
            let cy = self.y + y as i32;
            if cy < 0 || cy >= ch as i32 {
                continue;
            }
            for x in 0..self.w {
                let cx = self.x + x as i32;
                if cx < 0 || cx >= cw as i32 {
                    continue;
                }
                let s = &self.px[(y * self.w + x) * 4..(y * self.w + x) * 4 + 4];
                if s[3] <= 0.0 {
                    continue;
                }
                let i = (cy as usize * cw + cx as usize) * 4;
                let k = 1.0 - s[3];
                for c in 0..4 {
                    canvas[i + c] = s[c] + canvas[i + c] * k;
                }
            }
        }
    }
}

/// The caption face: the track style's font (Inter when none is named), SemiBold, at `px`. A font
/// that is not installed falls back to Inter; characters the font lacks (Hebrew in a Latin-only
/// font) come from a fallback face.
fn caption_style(font: &str, px: f32) -> TextStyle {
    let family = match font.trim() {
        "" => filmcraft_text::fonts::DEFAULT_FAMILY,
        f => f,
    };
    TextStyle { family: family.into(), style: "SemiBold".into(), size: px, ..Default::default() }
}

fn measure(text: &str, font: &str, px: f32) -> f32 {
    filmcraft_text::measure(text, &caption_style(font, px))
}

fn lin(c: [u8; 4]) -> [f32; 4] {
    let a = c[3] as f32 / 255.0;
    [srgb_to_linear(c[0] as f32 / 255.0) * a, srgb_to_linear(c[1] as f32 / 255.0) * a, srgb_to_linear(c[2] as f32 / 255.0) * a, a]
}

/// Word-wrap one line to `max_w` pixels.
fn wrap(line: &str, font: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in line.split(' ') {
        let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if !cur.is_empty() && measure(&cand, font, px) > max_w {
            out.push(std::mem::take(&mut cur));
            cur = word.to_string();
        } else {
            cur = cand;
        }
    }
    out.push(cur);
    out
}

/// Placement overrides from WebVTT cue settings.
fn vtt_overrides(settings: &str) -> (Option<f32>, Option<CaptionAlign>) {
    let mut line = None;
    let mut align = None;
    for kv in settings.split_whitespace() {
        let Some((k, v)) = kv.split_once(':') else { continue };
        match k {
            "line" => {
                let v = v.split(',').next().unwrap_or("");
                if let Some(p) = v.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()) {
                    line = Some((p / 100.0).clamp(0.0, 1.0));
                }
            }
            "align" => {
                align = match v {
                    "start" | "left" => Some(CaptionAlign::Left),
                    "end" | "right" => Some(CaptionAlign::Right),
                    "center" | "middle" => Some(CaptionAlign::Center),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    (line, align)
}

/// Whether captions in `language` (a BCP-47 tag such as `he` or `ar-EG`; ISO 639-2 codes and the
/// old `iw` / `ji` too) are written right to left: Hebrew, Arabic, Persian, Urdu and Yiddish.
pub fn rtl_language(language: &str) -> bool {
    let primary = language.trim().split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase();
    matches!(primary.as_str(), "he" | "iw" | "heb" | "ar" | "ara" | "fa" | "fas" | "per" | "ur" | "urd" | "yi" | "ji" | "yid")
}

/// The paragraph style of a caption line on a track in `language`: right to left for a
/// right-to-left language (a Hebrew line that starts with a Latin word still reads from the right),
/// else from the line's first strong character.
pub fn caption_paragraph(language: &str) -> ParagraphStyle {
    ParagraphStyle { rtl: rtl_language(language).then_some(true), ..Default::default() }
}

/// Lay out and rasterise one caption for a `w`×`h` frame, each line in the direction of its first
/// strong character.
pub fn render_caption(c: &Caption, style: &CaptionStyle, w: usize, h: usize) -> Option<Overlay> {
    render_caption_in(c, style, "", w, h)
}

/// [`render_caption`] for a track in `language`: its lines are laid out in the track's direction
/// ([`caption_paragraph`]).
pub fn render_caption_in(c: &Caption, style: &CaptionStyle, language: &str, w: usize, h: usize) -> Option<Overlay> {
    if w == 0 || h == 0 {
        return None;
    }
    let scale = h as f32 / 1080.0;
    let px = (style.size * scale).max(4.0);
    let max_w = w as f32 * 0.9;
    let mut lines: Vec<String> = Vec::new();
    // (speaker names are metadata; like Premiere, they are not burned in)
    for l in c.plain_lines() {
        lines.extend(wrap(l.trim(), &style.font, px, max_w));
    }
    lines.retain(|l| !l.is_empty());
    if lines.is_empty() {
        return None;
    }
    let face = caption_style(&style.font, px);
    let vm = filmcraft_text::fonts::face(filmcraft_text::resolve(&face.family, &face.style).face).metrics(px);
    let (asc, desc) = (vm.ascent, vm.descent);
    let lh = px * style.line_spacing.max(0.8);
    let pad_x = (px * 0.3).round();
    let block_h = lh * lines.len() as f32;
    let (line_override, align_override) = vtt_overrides(&c.settings);
    let align = align_override.unwrap_or(style.align);
    let margin = style.margin.clamp(0.0, 0.45) * h as f32;
    let top = match (line_override, style.anchor) {
        (Some(f), _) => (f * h as f32).min(h as f32 - block_h),
        (None, CaptionAnchor::Top) => margin,
        (None, CaptionAnchor::Middle) => (h as f32 - block_h) / 2.0,
        (None, CaptionAnchor::Bottom) => h as f32 - margin - block_h,
    }
    .max(0.0)
    .round();
    let side = w as f32 * 0.05;
    let widths: Vec<f32> = lines.iter().map(|l| measure(l, &style.font, px)).collect();
    let xs: Vec<f32> = widths
        .iter()
        .map(|&lw| match align {
            CaptionAlign::Left => side + pad_x,
            CaptionAlign::Right => w as f32 - side - pad_x - lw,
            CaptionAlign::Center => (w as f32 - lw) / 2.0,
        })
        .collect();
    let outline = (style.outline * scale).max(0.0);
    let grow = outline.ceil() + 1.0;
    let bx0 = xs.iter().zip(&widths).map(|(x, _)| x - pad_x - grow).fold(f32::MAX, f32::min).floor();
    let bx1 = xs.iter().zip(&widths).map(|(x, lw)| x + lw + pad_x + grow).fold(f32::MIN, f32::max).ceil();
    let by0 = (top - grow).floor();
    let by1 = (top + block_h + grow).ceil();
    let (ox, oy) = (bx0 as i32, by0 as i32);
    let ow = (bx1 - bx0).max(1.0) as usize;
    let oh = (by1 - by0).max(1.0) as usize;
    let mut cover = vec![0.0f32; ow * oh];
    let mut bg = vec![0.0f32; ow * oh];
    let paragraph = caption_paragraph(language);
    for (i, line) in lines.iter().enumerate() {
        let lt = top + lh * i as f32;
        if style.background && style.background_color[3] > 0 {
            let (x0, x1) = (xs[i] - pad_x, xs[i] + widths[i] + pad_x);
            let (y0, y1) = (lt, lt + lh);
            for y in (y0.floor() as i32).max(oy)..(y1.ceil() as i32).min(oy + oh as i32) {
                let fy = ((y as f32 + 1.0).min(y1) - (y as f32).max(y0)).clamp(0.0, 1.0);
                for x in (x0.floor() as i32).max(ox)..(x1.ceil() as i32).min(ox + ow as i32) {
                    let fx = ((x as f32 + 1.0).min(x1) - (x as f32).max(x0)).clamp(0.0, 1.0);
                    let j = (y - oy) as usize * ow + (x - ox) as usize;
                    bg[j] = (bg[j] + fx * fy).min(1.0);
                }
            }
        }
        let baseline = lt + (lh - (asc + desc)) / 2.0 + asc;
        let l = layout(line, &face, &paragraph);
        let mut m = filmcraft_text::Mask { w: ow, h: oh, a: std::mem::take(&mut cover) };
        render::draw(&l, &render::at(xs[i], baseline.round()), &mut m, (ox, oy));
        cover = m.a;
    }
    let stroke = if outline > 0.0 { dilate(&cover, ow, oh, outline) } else { Vec::new() };
    let (tc, bc, oc) = (lin(style.color), lin(style.background_color), lin(style.outline_color));
    let mut out = vec![0.0f32; ow * oh * 4];
    for j in 0..ow * oh {
        let mut p = [0.0f32; 4];
        let over = |p: &mut [f32; 4], c: [f32; 4], a: f32| {
            if a > 0.0 {
                for k in 0..4 {
                    p[k] = c[k] * a + p[k] * (1.0 - c[3] * a);
                }
            }
        };
        over(&mut p, bc, bg[j]);
        if !stroke.is_empty() {
            over(&mut p, oc, stroke[j]);
        }
        over(&mut p, tc, cover[j]);
        out[j * 4..j * 4 + 4].copy_from_slice(&p);
    }
    Some(Overlay { x: ox, y: oy, w: ow, h: oh, px: out })
}

/// Max-filter a coverage mask with a disc of radius `r`.
fn dilate(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let ri = r.ceil() as i32;
    let mut offs = Vec::new();
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d <= r + 0.5 {
                offs.push((dx, dy, (r + 0.5 - d).clamp(0.0, 1.0)));
            }
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut v = 0.0f32;
            for &(dx, dy, wgt) in &offs {
                let (sx, sy) = (x + dx, y + dy);
                if sx >= 0 && sy >= 0 && sx < w as i32 && sy < h as i32 {
                    v = v.max(m[sy as usize * w + sx as usize] * wgt);
                }
            }
            out[y as usize * w + x as usize] = v;
        }
    }
    out
}

/// The overlay for a caption track at timeline time `t` (None when nothing shows).
pub fn track_overlay(track: &CaptionTrack, t: Tick, w: usize, h: usize) -> Option<Overlay> {
    if !track.enabled {
        return None;
    }
    let c = track.caption_at(t)?;
    render_caption_in(c, &track.style, &track.language, w, h)
}

/// Overlays of every visible caption track of a sequence at `t`, bottom track first.
pub fn sequence_overlays(seq: &Sequence, t: Tick, w: usize, h: usize) -> Vec<Overlay> {
    seq.caption_tracks.iter().rev().filter_map(|tr| track_overlay(tr, t, w, h)).collect()
}

/// The subtitle layout of a vertical (or any other) output (Vertical Edit, spec §6.4). The renderer makes it from a
/// plan's captions settings (`filmcraft_render::vertical::caption_layout`). Lengths are in pixels of the frame the layout
/// is used on.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// `[x, y, w, h]`: the subtitles stay inside it, and WebVTT `line:` and `align:` settings apply relative to it.
    pub area: [f32; 4],
    /// The font size; None, and a size that is not finite and positive, is each track style's size × min(w, h) / 1080.
    pub size_px: Option<f32>,
    /// The auto-shrink floor; None, and a floor that is not finite and positive, is 40 × min(w, h) / 1080. A floor
    /// above the size is the size.
    pub min_px: Option<f32>,
    /// The most lines a cue may take: 1..=4, other values clamped.
    pub max_lines: u8,
    /// Keep the subtitler's line breaks (each source line wraps on its own); false joins the lines with spaces.
    pub keep_breaks: bool,
    /// Where the block sits in the area for every cue laid out with this layout (the renderer gives each cue its own).
    pub anchor: vertical::CaptionAnchor,
    /// One caption track (its id), burned whether or not its output is on; None is every enabled track.
    pub track: Option<u32>,
    /// A part export: each cue is burned only on the ticks [`crate::cue_in_range`] keeps of it.
    pub part: Option<PartRange>,
}

/// The range of a part export and its edge rule, for the burned subtitles (spec §6.4 "At part edges"). The sequence's
/// frame rate counts the frames of a clipped remainder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartRange {
    pub range: TimeRange,
    pub edge: CueEdge,
    pub min_clipped_frames: u32,
}

/// A cue a [`Layout`] burns: its track, the caption, and the ticks `[from, to)` it is burned on (the cue's own span,
/// cut by the part's edge rule when the layout has a part).
#[derive(Clone, Copy, Debug)]
pub struct BurnedCue<'a> {
    pub track: &'a CaptionTrack,
    pub cue: &'a Caption,
    pub from: Tick,
    pub to: Tick,
}

/// Where a laid-out cue lands, in pixels of the output (`vertical.measure`'s `{source: "captions"}`).
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionBox {
    pub track: filmcraft_project::TrackId,
    pub cue: filmcraft_project::ClipId,
    /// The ticks it is burned on ([`BurnedCue`]).
    pub from: Tick,
    pub to: Tick,
    /// The anchor it was laid out with.
    pub anchor: vertical::CaptionAnchor,
    /// The font size it was laid out at.
    pub size_px: f32,
    /// The laid-out lines, top to bottom.
    pub text: Vec<String>,
    /// Each line's box, `[x, y, w, h]`: its text's width plus the box padding on both sides, one line height.
    pub lines: Vec<[f32; 4]>,
    /// The union of the line boxes.
    pub rect: [f32; 4],
    /// It needs more than `max_lines` lines at the minimum size, and is laid out at the minimum size with every line.
    pub overflow: bool,
}

/// The warning `caption_overflow {cue, lines, frame}`: a cue that needs more than the layout's `max_lines` lines at the
/// minimum size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptionOverflow {
    pub cue: filmcraft_project::ClipId,
    /// The lines it is laid out with.
    pub lines: usize,
    /// The first frame it is burned on.
    pub frame: i64,
}

/// The caption tracks `layout` burns: its track (enabled or not), else every enabled track, bottom track first, as
/// [`sequence_overlays`] does.
pub fn layout_tracks<'a>(seq: &'a Sequence, layout: &Layout) -> Vec<&'a CaptionTrack> {
    match layout.track {
        Some(id) => seq.caption_tracks.iter().find(|t| t.id.0 == u64::from(id)).into_iter().collect(),
        None => seq.caption_tracks.iter().rev().filter(|t| t.enabled).collect(),
    }
}

/// The ticks `[from, to)` a cue is burned on: its own span, cut by the edge rule of `part` ([`crate::cue_in_range`],
/// frames of `rate`) when there is one. None when that is no tick. The end saturates.
fn burned_span(cue: &Caption, part: Option<&PartRange>, rate: FrameRate) -> Option<(Tick, Tick)> {
    let end = Tick(cue.start.0.saturating_add(cue.duration.0));
    match part {
        Some(p) => crate::cue_in_range(cue.start, end, p.range, p.edge, p.min_clipped_frames, rate),
        None => (end > cue.start).then_some((cue.start, end)),
    }
}

/// The cues `layout` burns at `t`: on each of its tracks the cue showing at `t`, when the part's edge rule keeps `t`
/// of it; bottom track first.
pub fn cues_at<'a>(seq: &'a Sequence, layout: &Layout, t: Tick) -> Vec<BurnedCue<'a>> {
    let rate = seq.settings.frame_rate;
    let mut out = Vec::new();
    for track in layout_tracks(seq, layout) {
        // the last cue that starts at or before `t` (a track's cues are sorted by start and do not overlap)
        let Some(cue) = track.captions.partition_point(|c| c.start <= t).checked_sub(1).and_then(|i| track.captions.get(i)) else { continue };
        let Some((from, to)) = burned_span(cue, layout.part.as_ref(), rate) else { continue };
        if (from..to).contains(&t) {
            out.push(BurnedCue { track, cue, from, to });
        }
    }
    out
}

/// The cues `layout` burns on at least one tick of `range`, bottom track first, each track's cues by start.
pub fn cues_in_range<'a>(seq: &'a Sequence, layout: &Layout, range: TimeRange) -> Vec<BurnedCue<'a>> {
    let rate = seq.settings.frame_rate;
    let end = Tick(range.start.0.saturating_add(range.duration.0));
    let mut out = Vec::new();
    if end <= range.start {
        return out;
    }
    for track in layout_tracks(seq, layout) {
        let mut cues = Vec::new();
        for cue in &track.captions {
            let Some((from, to)) = burned_span(cue, layout.part.as_ref(), rate) else { continue };
            if from < end && to > range.start {
                cues.push(BurnedCue { track, cue, from, to });
            }
        }
        cues.sort_by_key(|c| c.from);
        out.extend(cues);
    }
    out
}

/// A WebVTT `align:` value, before it is resolved against a line's direction.
#[derive(Clone, Copy)]
enum VttAlign {
    Start,
    End,
    Left,
    Right,
    Center,
}

impl VttAlign {
    /// The alignment of a line that reads right to left (`rtl`) or left to right: `start` and `end` follow its
    /// direction; `left`, `right` and `center` do not.
    fn resolve(self, rtl: bool) -> CaptionAlign {
        match self {
            VttAlign::Start if rtl => CaptionAlign::Right,
            VttAlign::End if rtl => CaptionAlign::Left,
            VttAlign::Start | VttAlign::Left => CaptionAlign::Left,
            VttAlign::End | VttAlign::Right => CaptionAlign::Right,
            VttAlign::Center => CaptionAlign::Center,
        }
    }
}

/// The WebVTT cue settings the vertical layout applies (spec §6.4): `line:N%` as a fraction of the area's height (a
/// finite percentage, held to 0–100 %; line numbers are ignored, as on the 16:9 frame) and `align:`. A later setting
/// replaces an earlier one, and an `align:` value that is not known clears it. The 16:9 [`vtt_overrides`] is unchanged.
fn vtt_placement(settings: &str) -> (Option<f32>, Option<VttAlign>) {
    let (mut line, mut align) = (None, None);
    for (k, v) in settings.split_whitespace().filter_map(|kv| kv.split_once(':')) {
        match k {
            "line" => {
                let percent = v.split(',').next().and_then(|p| p.strip_suffix('%')).and_then(|p| p.parse::<f32>().ok());
                if let Some(p) = percent.filter(|p| p.is_finite()) {
                    line = Some((p / 100.0).clamp(0.0, 1.0));
                }
            }
            "align" => {
                align = match v {
                    "start" => Some(VttAlign::Start),
                    "end" => Some(VttAlign::End),
                    "left" => Some(VttAlign::Left),
                    "right" => Some(VttAlign::Right),
                    "center" | "middle" => Some(VttAlign::Center),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    (line, align)
}

/// Whether a line reads right to left by its first strong character (the Unicode bidi rule P2, without embeddings): a
/// right-to-left letter (or U+200F) before any letter of a left-to-right script (or U+200E).
fn first_strong_rtl(line: &str) -> bool {
    line.chars().find(|&c| bidi::is_rtl_char(c) || c.is_alphabetic() || c == '\u{200E}').is_some_and(bidi::is_rtl_char)
}

/// `v` when it is a finite size above zero.
fn positive(v: Option<f32>) -> Option<f32> {
    v.filter(|v| v.is_finite() && *v > 0.0)
}

/// The padding of a line's box on each side of its text at the size `px`, as on the 16:9 frame.
fn box_pad(px: f32) -> f32 {
    (px * 0.3).round()
}

/// A style's line spacing in the vertical layout: at least 0.8 (as on the 16:9 frame) and at most 4; a spacing that is
/// not a number is 0.8.
fn line_spacing(style: &CaptionStyle) -> f32 {
    if style.line_spacing.is_nan() { 0.8 } else { style.line_spacing.clamp(0.8, 4.0) }
}

/// One line of a laid-out cue: its text, where the text starts and how wide it is, and the line's top.
struct PlacedLine {
    text: String,
    x: f32,
    width: f32,
    top: f32,
}

/// A cue laid out by a [`Layout`] in a frame: what [`cue_box`] reports and [`cue_overlay`] draws.
struct Placed {
    /// The font size, the padding of a line's box on each side of its text, and the line height.
    px: f32,
    pad: f32,
    lh: f32,
    /// Top to bottom; never empty.
    lines: Vec<PlacedLine>,
    /// More than the layout's `max_lines` lines at the minimum size.
    overflow: bool,
}

impl Placed {
    /// Each line's box `[x, y, w, h]`: its text and the padding on both sides, one line height.
    fn boxes(&self) -> Vec<[f32; 4]> {
        self.lines.iter().map(|l| [l.x - self.pad, l.top, l.width + 2.0 * self.pad, self.lh]).collect()
    }
}

/// The smallest rectangle `[x, y, w, h]` that holds every one of `boxes`.
fn enclosing(boxes: &[[f32; 4]]) -> [f32; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for &[x, y, w, h] in boxes {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + w), y1.max(y + h));
    }
    [x0, y0, x1 - x0, y1 - y0]
}

/// Lay one cue out in a `w`×`h` frame (the rules of [`cue_box`]).
fn place(cue: &BurnedCue<'_>, w: usize, h: usize, layout: &Layout) -> Option<Placed> {
    filmcraft_project::validate_frame_size(u32::try_from(w).ok()?, u32::try_from(h).ok()?).ok()?;
    let [ax, ay, aw, ah] = layout.area;
    if layout.area.iter().any(|v| !v.is_finite()) || aw <= 0.0 || ah <= 0.0 {
        return None;
    }
    let mut source: Vec<String> = cue.cue.plain_lines().iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    if source.is_empty() {
        return None;
    }
    if !layout.keep_breaks {
        source = vec![source.join(" ")];
    }
    let style = &cue.track.style;
    let unit = w.min(h) as f32 / 1080.0;
    let base = positive(layout.size_px).or_else(|| positive(Some(style.size * unit)))?.min(h as f32).max(1.0);
    let floor = positive(layout.min_px).unwrap_or(40.0 * unit).max(1.0).min(base);
    let max_lines = usize::from(layout.max_lines.clamp(1, 4));
    // each source line wrapped so that its box, with the padding, fits the area's width
    let wrapped = |px: f32| {
        let max_w = aw - 2.0 * box_pad(px);
        source.iter().flat_map(|l| wrap(l, &style.font, px, max_w)).filter(|l| !l.is_empty()).collect::<Vec<_>>()
    };
    // 2 % smaller per step while the lines do not fit, down to the floor (from at most `h` to at least 1 px: at most
    // 516 steps); a cue with more source lines than `max_lines` never fits, and goes to the floor at once
    let mut px = if source.len() > max_lines { floor } else { base };
    let rows = loop {
        let rows = wrapped(px);
        if rows.len() <= max_lines || px <= floor {
            break rows;
        }
        px = (px * 0.98).max(floor);
    };
    let overflow = rows.len() > max_lines;
    let (pad, lh) = (box_pad(px), px * line_spacing(style));
    let block = lh * rows.len() as f32;
    let (line_at, align) = vtt_placement(&cue.cue.settings);
    let top = match (line_at, layout.anchor) {
        (Some(f), _) => (ay + f * ah).min(ay + ah - block),
        (None, vertical::CaptionAnchor::Top) => ay,
        (None, vertical::CaptionAnchor::Bottom) => ay + ah - block,
    }
    .max(0.0)
    .round();
    let rtl_track = rtl_language(&cue.track.language);
    let mut lines = Vec::with_capacity(rows.len());
    for (i, text) in rows.into_iter().enumerate() {
        let width = measure(&text, &style.font, px);
        let rtl = rtl_track || first_strong_rtl(&text);
        let x = match align.map_or(style.align, |a| a.resolve(rtl)) {
            CaptionAlign::Left => ax + pad,
            CaptionAlign::Right => ax + aw - pad - width,
            CaptionAlign::Center => ax + (aw - width) / 2.0,
        };
        lines.push(PlacedLine { text, x, width, top: top + lh * i as f32 });
    }
    Some(Placed { px, pad, lh, lines, overflow })
}

/// Lay one cue out in the area of a `w`×`h` frame with `layout` (its anchor included), without drawing it (spec §6.4
/// "Layout", normative). None when the cue has no text, when the frame is not a valid frame size, when a value of the
/// area is not finite or its width or height is not positive, and when neither the layout nor the track style gives a
/// finite, positive size.
///
/// - **Lines.** The cue's plain lines ([`Caption::plain_lines`]: markup removed), each trimmed, empty ones left out;
///   joined with single spaces when `keep_breaks` is false. Each line is word-wrapped so that its box (its text and a
///   padding of round(0.3 × size) on each side) fits the area's width; a single word wider than that stays whole.
/// - **Size.** It starts at the layout's size (at most `h`, at least 1 px) and shrinks in 2 % steps (× 0.98) while the
///   lines number more than `max_lines`, down to the minimum size (at least 1 px, at most the size). When they still
///   number more at the minimum, the cue is laid out at exactly the minimum with every line, and `overflow` is set.
/// - **Alignment.** Each line by the track style's alignment, or by the cue's WebVTT `align:` setting, with `start` and
///   `end` resolved against the line's direction (right to left on a right-to-left track, [`rtl_language`], else from
///   its first strong character): on a right-to-left line `start` is the area's right edge. Left puts the line's box
///   on the area's left edge, right on its right edge, centre in its middle.
/// - **Block.** Lines are one line height apart (size × the style's line spacing, held to 0.8–4). The block's top is the
///   area's top (`Top`) or its bottom less the block's height (`Bottom`); a WebVTT `line:N%` setting puts it N % down
///   the area instead, kept so that the block ends inside it. Then at least 0, rounded to a whole pixel.
/// - Font, colours, background box and outline come from the track style ([`cue_overlay`]); lines are set with
///   [`caption_paragraph`] of the track's language.
pub fn cue_box(cue: &BurnedCue<'_>, w: usize, h: usize, layout: &Layout) -> Option<CaptionBox> {
    let p = place(cue, w, h, layout)?;
    let lines = p.boxes();
    let rect = enclosing(&lines);
    let (track, id, anchor, overflow) = (cue.track.id, cue.cue.id, layout.anchor, p.overflow);
    let text = p.lines.into_iter().map(|l| l.text).collect();
    Some(CaptionBox { track, cue: id, from: cue.from, to: cue.to, anchor, size_px: p.px, text, lines, rect, overflow })
}

/// A pixel edge `v` (a whole number) of a frame side of `max` pixels, as an index in `0..=max`; a value below zero or
/// not a number is 0.
fn pixel(v: f32, max: usize) -> usize {
    if v > 0.0 { (v as usize).min(max) } else { 0 }
}

/// `n` zeros, or None when they cannot be allocated.
fn zeros(n: usize) -> Option<Vec<f32>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, 0.0);
    Some(v)
}

/// Add the coverage of the rectangle `[x0, y0, x1, y1]` (frame pixels) to `mask`, which holds the `w`×`h` pixels at
/// `(ox, oy)` of the frame; each pixel's coverage stays at most 1.
fn add_rect(mask: &mut [f32], (ox, oy, w, h): (usize, usize, usize, usize), [x0, y0, x1, y1]: [f32; 4]) {
    let cols = pixel(x0.floor(), ox + w).max(ox)..pixel(x1.ceil(), ox + w);
    let rows = pixel(y0.floor(), oy + h).max(oy)..pixel(y1.ceil(), oy + h);
    for y in rows {
        let fy = ((y as f32 + 1.0).min(y1) - (y as f32).max(y0)).clamp(0.0, 1.0);
        for x in cols.clone() {
            let fx = ((x as f32 + 1.0).min(x1) - (x as f32).max(x0)).clamp(0.0, 1.0);
            if let Some(m) = mask.get_mut((y - oy) * w + (x - ox)) {
                *m = (*m + fx * fy).min(1.0);
            }
        }
    }
}

/// Composite the colour `c` at coverage `a` over the premultiplied pixel `p`, as on the 16:9 frame.
fn over(p: &mut [f32; 4], c: [f32; 4], a: f32) {
    if a > 0.0 {
        let k = 1.0 - c[3] * a;
        for (v, c) in p.iter_mut().zip(c) {
            *v = c * a + *v * k;
        }
    }
}

/// [`cue_box`], drawn: the cue's lines in its track's style and direction, as a premultiplied linear-light overlay cut
/// to the frame (never larger than it). The outline is the style's × min(w, h) / 1080, at most a quarter of the font
/// size. None also when the cue lies outside the frame.
pub fn cue_overlay(cue: &BurnedCue<'_>, w: usize, h: usize, layout: &Layout) -> Option<Overlay> {
    let p = place(cue, w, h, layout)?;
    let style = &cue.track.style;
    let outline = (style.outline * w.min(h) as f32 / 1080.0).max(0.0).min(p.px * 0.25);
    let grow = outline.ceil() + 1.0;
    // the overlay: the lines' boxes grown by the outline, cut to the frame
    let boxes = p.boxes();
    let [bx, by, bw, bh] = enclosing(&boxes);
    let (x0, x1) = (pixel((bx - grow).floor(), w), pixel((bx + bw + grow).ceil(), w));
    let (y0, y1) = (pixel((by - grow).floor(), h), pixel((by + bh + grow).ceil(), h));
    let (ow, oh) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    if ow == 0 || oh == 0 {
        return None;
    }
    let origin = (i32::try_from(x0).ok()?, i32::try_from(y0).ok()?);
    let n = ow.checked_mul(oh)?;
    let mut bg = zeros(n)?;
    if style.background && style.background_color[3] > 0 {
        for &[x, y, lw, lh] in &boxes {
            add_rect(&mut bg, (x0, y0, ow, oh), [x, y, x + lw, y + lh]);
        }
    }
    let face = caption_style(&style.font, p.px);
    let vm = filmcraft_text::fonts::face(filmcraft_text::resolve(&face.family, &face.style).face).metrics(p.px);
    let paragraph = caption_paragraph(&cue.track.language);
    let mut text = filmcraft_text::Mask { w: ow, h: oh, a: zeros(n)? };
    let (fx0, fy0, fx1, fy1) = (x0 as f32, y0 as f32, x1 as f32, y1 as f32);
    for l in p.lines.iter().filter(|l| l.x < fx1 && l.x + l.width > fx0 && l.top < fy1 && l.top + p.lh > fy0) {
        let baseline = l.top + (p.lh - (vm.ascent + vm.descent)) / 2.0 + vm.ascent;
        let set = filmcraft_text::layout(&l.text, &face, &paragraph);
        render::draw(&set, &render::at(l.x, baseline.round()), &mut text, origin);
    }
    let stroke = if outline > 0.0 { dilate(&text.a, ow, oh, outline) } else { Vec::new() };
    let (tc, bc, oc) = (lin(style.color), lin(style.background_color), lin(style.outline_color));
    let mut px = zeros(n.checked_mul(4)?)?;
    for (j, out) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        over(out, bc, bg.get(j).copied().unwrap_or(0.0));
        over(out, oc, stroke.get(j).copied().unwrap_or(0.0));
        over(out, tc, text.a.get(j).copied().unwrap_or(0.0));
    }
    Some(Overlay { x: origin.0, y: origin.1, w: ow, h: oh, px })
}

/// The overlays of the cues `layout` burns at `t` on a `w`×`h` frame, bottom track first, every cue laid out with
/// `layout.anchor`: the vertical variant of [`sequence_overlays`]. The renderer lays each cue out with its own anchor
/// (`filmcraft_render::vertical`), from [`cues_at`] and [`cue_overlay`].
pub fn sequence_overlays_in(seq: &Sequence, t: Tick, w: usize, h: usize, layout: &Layout) -> Vec<Overlay> {
    cues_at(seq, layout, t).iter().filter_map(|c| cue_overlay(c, w, h, layout)).collect()
}

/// The `caption_overflow` warnings of the cues `layout` burns on `range` of a `w`×`h` frame, in the order of
/// [`cues_in_range`], each at the first frame the cue is burned on. The anchor does not change them.
pub fn caption_overflow(seq: &Sequence, range: TimeRange, w: usize, h: usize, layout: &Layout) -> Vec<CaptionOverflow> {
    let rate = seq.settings.frame_rate;
    let mut out = Vec::new();
    for c in cues_in_range(seq, layout, range) {
        if let Some(b) = cue_box(&c, w, h, layout).filter(|b| b.overflow) {
            out.push(CaptionOverflow { cue: c.cue.id, lines: b.lines.len(), frame: crate::shown_frame(rate, c.from) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{CaptionFormat, ClipId, TrackId};

    fn cap(text: &str) -> Caption {
        Caption { id: ClipId(1), start: Tick(0), duration: Tick(100), text: text.into(), speaker: None, cue_id: None, settings: String::new() }
    }

    #[test]
    fn bottom_centre_with_box() {
        let st = CaptionStyle::default();
        let o = render_caption(&cap("Hello world"), &st, 1920, 1080).unwrap();
        // centred horizontally, in the lower part of the frame
        let cx = o.x as f32 + o.w as f32 / 2.0;
        assert!((cx - 960.0).abs() < 4.0, "centre {cx}");
        assert!(o.y > 800 && (o.y as usize + o.h) < 1080, "y {} h {}", o.y, o.h);
        // white text pixels exist and the box is dark translucent
        let mut white = 0;
        let mut boxy = 0;
        for p in o.px.chunks(4) {
            if p[3] > 0.99 && p[0] > 0.9 {
                white += 1;
            } else if p[3] > 0.7 && p[0] < 0.01 {
                boxy += 1;
            }
        }
        assert!(white > 500, "{white}");
        assert!(boxy > 1000, "{boxy}");
    }

    #[test]
    fn top_anchor_and_vtt_line() {
        let st = CaptionStyle { anchor: CaptionAnchor::Top, ..Default::default() };
        let o = render_caption(&cap("Top"), &st, 1280, 720).unwrap();
        assert!(o.y < 100);
        let mut c = cap("Line");
        c.settings = "line:50% align:start".into();
        let o = render_caption(&c, &CaptionStyle::default(), 1280, 720).unwrap();
        assert!((o.y - 360).abs() < 4, "{}", o.y);
        assert!(o.x < 120, "{}", o.x);
    }

    #[test]
    fn wraps_and_composites() {
        let long = "word ".repeat(60);
        let o = render_caption(&cap(&long), &CaptionStyle::default(), 640, 360).unwrap();
        assert!(o.w <= 640 && o.h > 40, "{}x{}", o.w, o.h);
        let mut canvas = vec![0.0f32; 640 * 360 * 4];
        o.composite_onto(&mut canvas, 640, 360);
        assert!(canvas.chunks(4).any(|p| p[0] > 0.9));
    }

    #[test]
    fn outline_draws_around_text() {
        let st = CaptionStyle { background: false, outline: 4.0, outline_color: [255, 0, 0, 255], ..Default::default() };
        let o = render_caption(&cap("O"), &st, 1920, 1080).unwrap();
        assert!(o.px.chunks(4).any(|p| p[0] > 0.9 && p[1] < 0.05 && p[3] > 0.9), "red outline pixels");
    }

    #[test]
    fn hidden_track_draws_nothing() {
        let mut t = CaptionTrack::new(TrackId(1), "S".into(), CaptionFormat::Subtitle);
        t.captions.push(cap("x"));
        assert!(track_overlay(&t, Tick(5), 100, 100).is_some());
        t.enabled = false;
        assert!(track_overlay(&t, Tick(5), 100, 100).is_none());
    }
}
