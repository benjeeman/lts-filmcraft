//! The vertical frame (Vertical Edit, spec §6.1–§6.4): [`render_vertical`] makes the exact output frame of a plan for
//! every path that needs its pixels (the EXACT preview, frame reads, cover frames and export). It renders the captionless
//! 16:9 frame (from the engine's 16:9 frame cache when one is given), resamples the plan's window of it with
//! [`resample_window`](crate::window::resample_window) (or, for a `fit` position, the whole frame into the output's
//! width over a black or blurred background), then lays the plan's subtitles out again for the output. The window is
//! the one [`window_at`](filmcraft_project::vertical::window_at) gives: there is no second implementation.
//!
//! The inputs are the same on every exact path ([`VerticalRenderInputs`]): full-resolution sources (never proxies),
//! never a render-preview frame, never draft decoding, and no 16:9 caption layer (the 16:9 layout must not be cropped
//! into the vertical).

use std::sync::Arc;

use filmcraft_captions::burn::{self, CaptionBox, PartRange};
use filmcraft_project::vertical::{CaptionAnchor, Filter, FitBackground, Layout, OutputSize, Phase, Raster, VerticalPlan, centred_largest, window_at};
use filmcraft_project::{ItemId, Project, Sequence, validate_frame_size};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use rayon::prelude::*;

use crate::scale::quantize_scale;
use crate::window::{Kernel, resample_window};
use crate::{Image, RenderOptions, SourceProvider, effects, render_sequence};

/// Which picture a 16:9 frame belongs to in the caller's session (spec §6.3, §8.5; plan 63 C-7): the project epoch
/// (advanced, with the cache cleared, whenever the whole project is replaced), the picture revision (advanced on every
/// change but plan-only ones and their undo and redo) and the media pool generation (advanced when a source is opened,
/// indexed, relinked or switched between proxy and full resolution). A caller without a session passes the default and
/// no cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FrameIdentity {
    pub epoch: u64,
    pub picture_revision: u64,
    pub media_generation: u64,
}

/// The key of a captionless 16:9 frame in the engine's frame cache (spec §6.3, §9 C20): the identity of the picture,
/// the sequence and frame, the scale it was rendered at (`f32::to_bits`) and whether it is in the working space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameKey16x9 {
    pub epoch: u64,
    pub sequence: ItemId,
    pub frame: i64,
    pub picture_revision: u64,
    pub scale_bits: u32,
    pub working_output: bool,
    pub media_generation: u64,
}

impl FrameKey16x9 {
    /// The key of frame `frame` of sequence `sequence`, rendered at `scale` (in the working space or not), in the
    /// picture `identity`.
    pub fn new(identity: FrameIdentity, sequence: ItemId, frame: i64, scale: f32, working_output: bool) -> Self {
        let FrameIdentity { epoch, picture_revision, media_generation } = identity;
        Self { epoch, sequence, frame, picture_revision, scale_bits: scale.to_bits(), working_output, media_generation }
    }
}

/// The engine's 16:9 frame cache, seen from the renderer (spec §6.3). `crates/render` cannot depend on the engine; the
/// engine's `VerticalFrameCache` implements this.
pub trait FrameCache16x9: Sync {
    fn get(&self, key: &FrameKey16x9) -> Option<Arc<Image>>;
    fn put(&self, key: FrameKey16x9, frame: Arc<Image>);
}

/// What every exact vertical path renders with (spec §6.1). By construction never a render-preview frame and never draft
/// decoding; the caller passes the full-resolution provider, so never a proxy.
#[derive(Clone, Copy)]
pub struct VerticalRenderInputs<'a> {
    /// Always full resolution (the engine's `full_res_provider`): never proxies.
    pub sources: &'a dyn SourceProvider,
    /// Burn the plan's vertical subtitle layout (spec §6.4). The 16:9 caption layer is always off.
    pub captions: bool,
    /// Keep the sequence's working space instead of the display transform (HDR exports only).
    pub working_output: bool,
    /// The engine's 16:9 frame cache; None renders the 16:9 frame directly.
    pub cache: Option<&'a dyn FrameCache16x9>,
    /// The picture's identity in the caller's session, for the cache key.
    pub identity: FrameIdentity,
}

/// The options of one vertical frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerticalRenderOptions {
    /// The output's scale: 1 is the plan's output size, the only exact size (spec §9 C8, C10); below 1 a preview or a
    /// `maxSide` read (C15).
    pub scale: f32,
    /// A part export: the subtitles follow the part's edge rule (spec §6.4 "At part edges"). The picture does not
    /// depend on it.
    pub part: Option<PartRange>,
}

impl Default for VerticalRenderOptions {
    fn default() -> Self {
        Self { scale: 1.0, part: None }
    }
}

/// The frame of plan `plan` (its id) of sequence `seq` at tick `t` (spec §6.1, normative). None when the sequence, its
/// vertical set or the plan does not exist, when the sequence's frame, the plan's output or the output at `opts.scale`
/// is not a valid frame size, when `opts.scale` is not finite and positive, when the render was cancelled
/// (`filmcraft_media::cancel`) and when an image of the output's size cannot be allocated.
///
/// 1. `smp = window_at(plan, raster_of(seq), t)`.
/// 2. The 16:9 scale `s_h`: 1 when `opts.scale` is 1; else `min(1, s · OH / smp.window.h)` for a `fill` sample or
///    `min(1, s · OW / W)` for a `fit` one (1 when that is not finite and positive), rounded up to a bucket by
///    [`quantize_scale`](crate::scale::quantize_scale).
/// 3. The 16:9 frame: [`render_sequence`](crate::render_sequence) at `s_h`, captions off, `working_output` from the
///    inputs, never in draft quality. With a cache, and `t` on a frame start, it is looked up under [`FrameKey16x9`]
///    `{identity, seq, frame_at(t), s_h bits, working_output}` and stored there after a render that was not cancelled;
///    a tick inside a frame is rendered directly and never cached.
/// 4. The output: `(round(OW · s), round(OH · s))` with `s = opts.scale as f64`.
/// 5. `fill`: [`resample_window`](crate::window::resample_window) of the window's rectangle times `s_h` (as f64) into
///    the output with the plan's kernel ([`kernel_of`]), so the window lands exactly on the output's edges with the
///    per-axis scales `k_x = w · s_h / out_w`, `k_y = h · s_h / out_h`. `fit`: the background (opaque black, or the
///    centred largest window resampled the same way, blurred by [`gaussian`](crate::effects::gaussian) with
///    `σ = OW · s / 45` and repeated edges, its colour channels times 0.7, its alpha kept), then the whole 16:9 frame
///    (`[0, 0, W · s_h, H · s_h]`) resampled into `[0, y0, out_w, out_w · H / W]`, `y0 = (out_h − out_w · H / W) / 2`,
///    composited over it (premultiplied: `f + b · (1 − f.α)`); partial edge rows come out weighted by their coverage.
/// 6. With `inputs.captions`: the plan's subtitles ([`caption_layout`] at `s`, with `opts.part`), each cue burned on
///    [`burn::cues_at`] laid out with its own anchor ([`cue_anchor`] of the first tick it is burned on: its start, or
///    the part's first frame when the part clips it) and composited last, bottom track first.
pub fn render_vertical(project: &Project, seq: ItemId, plan: u32, t: Tick, opts: VerticalRenderOptions, inputs: &VerticalRenderInputs) -> Option<Image> {
    let sequence = project.sequence(seq)?;
    let plan = sequence.vertical.as_deref()?.plans.iter().find(|p| p.id == plan)?;
    validate_frame_size(sequence.settings.width, sequence.settings.height).ok()?;
    validate_frame_size(plan.output.width, plan.output.height).ok()?;
    let s = opts.scale;
    if !s.is_finite() || s <= 0.0 {
        return None;
    }
    let out = scaled_output(plan.output, s)?;
    let raster = raster_of(sequence);
    let smp = window_at(plan, &raster, t);
    let s_h = scale_16x9(plan, &raster, smp.layout, smp.window.h, s);
    let frame = frame_16x9(project, seq, raster.rate, t, s_h, inputs)?;
    let k = f64::from(s_h);
    let mut img = match smp.layout {
        Layout::Fill => {
            let r = smp.window.rect(plan.output);
            resample_window(&frame, [r.x * k, r.y * k, r.w * k, r.h * k], out, None, kernel_of(plan.filter))
        }
        Layout::Fit => fit_frame(plan, &raster, &frame, k, s, out)?,
    };
    // a buffer that could not be allocated comes back as a smaller image: never hand out a frame of another size
    if (img.w, img.h) != out {
        return None;
    }
    if inputs.captions {
        burn_captions(&mut img, sequence, plan, &raster, t, s, opts.part);
    }
    Some(img)
}

/// The output of `out` at scale `s`: `(round(OW · s), round(OH · s))` in f64. None when it is not a valid frame size.
fn scaled_output(out: OutputSize, s: f32) -> Option<(usize, usize)> {
    let side = |n: u32| (f64::from(n) * f64::from(s)).round() as u32;
    let (w, h) = (side(out.width), side(out.height));
    validate_frame_size(w, h).ok()?;
    Some((w as usize, h as usize))
}

/// The scale of the 16:9 frame (spec §6.1 step 2) for a sample of `layout` whose window is `window_h` high, at output
/// scale `s`: 1 at full size; else the scale at which a pixel of the 16:9 frame covers at least one output pixel
/// (`s · OH / h` for `fill`, `s · OW / W` for `fit`), at most 1 (1 when that is not finite and positive), rounded up to a
/// preview bucket.
fn scale_16x9(plan: &VerticalPlan, raster: &Raster, layout: Layout, window_h: f64, s: f32) -> f32 {
    if s == 1.0 {
        return 1.0;
    }
    let need = match layout {
        Layout::Fill => f64::from(s) * f64::from(plan.output.height) / window_h,
        Layout::Fit => f64::from(s) * f64::from(plan.output.width) / f64::from(raster.width),
    };
    if need.is_finite() && need > 0.0 { quantize_scale(need.min(1.0) as f32) } else { 1.0 }
}

/// The captionless 16:9 frame of sequence `seq` (frames of `rate`) at `t` and scale `s_h` (spec §6.1 step 3, §6.3),
/// never in draft quality: from the cache when `t` is a frame start and the cache holds it, else rendered and, when `t`
/// is a frame start, stored. A tick inside a frame shows a picture a frame-keyed cache cannot hold, so it is rendered
/// and neither looked up nor stored. None when the render was cancelled; nothing is stored then.
fn frame_16x9(project: &Project, seq: ItemId, rate: FrameRate, t: Tick, s_h: f32, inputs: &VerticalRenderInputs) -> Option<Arc<Image>> {
    let frame = rate.frame_at(t);
    let key = FrameKey16x9::new(inputs.identity, seq, frame, s_h, inputs.working_output);
    let cache = inputs.cache.filter(|_| rate.tick_of(frame) == t);
    if let Some(hit) = cache.and_then(|c| c.get(&key)) {
        return Some(hit);
    }
    let opts = RenderOptions { scale: s_h, captions: false, working_output: inputs.working_output, ..RenderOptions::default() };
    let rendered = filmcraft_media::cancel::with_draft(false, || render_sequence(project, seq, t, opts, inputs.sources));
    if filmcraft_media::cancel::cancelled() {
        return None;
    }
    let rendered = Arc::new(rendered);
    if let Some(c) = cache {
        c.put(key, Arc::clone(&rendered));
    }
    Some(rendered)
}

/// A `fit` frame (spec §6.1 step 5): the whole 16:9 frame (`frame`, rendered at `k` = `s_h`) resampled into the
/// destination `[0, y0, out_w, out_w · H / W]` centred in `out`, composited over the plan's background (opaque black or
/// [`blurred_background`]); the destination's partial edge rows are weighted by their coverage. None when an image
/// cannot be allocated.
fn fit_frame(plan: &VerticalPlan, raster: &Raster, frame: &Image, k: f64, s: f32, out: (usize, usize)) -> Option<Image> {
    let (w, h) = (f64::from(raster.width), f64::from(raster.height));
    let dw = out.0 as f64;
    let dh = dw * h / w;
    let dest = [0.0, (out.1 as f64 - dh) * 0.5, dw, dh];
    let mut picture = match plan.fit_background {
        FitBackground::Black => opaque_black(out)?,
        FitBackground::Blur => blurred_background(plan, raster, frame, k, s, out),
    };
    let fitted = resample_window(frame, [0.0, 0.0, w * k, h * k], out, Some(dest), kernel_of(plan.filter));
    if (picture.w, picture.h) != out || (fitted.w, fitted.h) != out {
        return None;
    }
    composite_over(&mut picture, &fitted);
    Some(picture)
}

/// The blurred background of a `fit` frame at output scale `s` (spec §6.1 step 5): the centred largest window of the
/// 16:9 frame (rendered at `k` = `s_h`) resampled into `out` with the plan's kernel, blurred by
/// [`gaussian`](crate::effects::gaussian) with `σ = OW · s / 45` and repeated edges, its colour channels times 0.7 and
/// its alpha kept.
fn blurred_background(plan: &VerticalPlan, raster: &Raster, frame: &Image, k: f64, s: f32, out: (usize, usize)) -> Image {
    let r = centred_largest(raster, plan.output).rect(plan.output);
    let mut bg = resample_window(frame, [r.x * k, r.y * k, r.w * k, r.h * k], out, None, kernel_of(plan.filter));
    let sigma = plan.output.width as f32 * s / 45.0;
    effects::gaussian(&mut bg, sigma, sigma, true);
    bg.px.as_chunks_mut::<4>().0.par_iter_mut().for_each(|p| {
        p[0] *= 0.7;
        p[1] *= 0.7;
        p[2] *= 0.7;
    });
    bg
}

/// An opaque black image of `out`; None when it cannot be allocated.
fn opaque_black((w, h): (usize, usize)) -> Option<Image> {
    let n = w.checked_mul(h)?.checked_mul(4)?;
    let mut px: Vec<f32> = Vec::new();
    px.try_reserve_exact(n).ok()?;
    px.extend((0..n).map(|i| if i % 4 == 3 { 1.0 } else { 0.0 }));
    Some(Image { w, h, px })
}

/// Composite the premultiplied `top` over `bottom`, an image of the same size: `t + b · (1 − t.α)` per channel. A pixel
/// `top` leaves transparent keeps `bottom`'s bits.
fn composite_over(bottom: &mut Image, top: &Image) {
    let (b, t) = (bottom.px.as_chunks_mut::<4>().0, top.px.as_chunks::<4>().0);
    b.par_iter_mut().zip(t).for_each(|(b, t)| {
        if t[3] > 0.0 {
            let k = 1.0 - t[3];
            for (b, t) in b.iter_mut().zip(t) {
                *b = t + *b * k;
            }
        }
    });
}

/// Burn the plan's subtitles at `t` into `img`, the output at scale `s` (spec §6.1 step 6, §6.4): each cue
/// [`burn::cues_at`] gives, laid out with its own anchor ([`cue_anchor`] of the first tick it is burned on), bottom track
/// first.
fn burn_captions(img: &mut Image, seq: &Sequence, plan: &VerticalPlan, raster: &Raster, t: Tick, s: f32, part: Option<PartRange>) {
    let layout = caption_layout(plan, s, part);
    let (w, h) = (img.w, img.h);
    for cue in burn::cues_at(seq, &layout, t) {
        let anchored = burn::Layout { anchor: cue_anchor(plan, raster, cue.from), ..layout.clone() };
        if let Some(o) = burn::cue_overlay(&cue, w, h, &anchored) {
            o.composite_onto(&mut img.px, w, h);
        }
    }
}

/// The 16:9 frame a sequence's plans are evaluated on: its frame size and rate, and its end frame (the first frame shown
/// at or after the end of its last clip or caption; ends saturate).
pub fn raster_of(seq: &Sequence) -> Raster {
    let rate = seq.settings.frame_rate;
    let items = seq.video_tracks.iter().chain(&seq.audio_tracks).flat_map(|t| &t.items).map(|i| i.start.0.saturating_add(i.duration.0));
    let cues = seq.caption_tracks.iter().flat_map(|t| &t.captions).map(|c| c.start.0.saturating_add(c.duration.0));
    let end = items.chain(cues).fold(0i64, i64::max);
    let end_frame = if end > 0 { rate.frame_at(Tick(end.saturating_sub(1))).saturating_add(1) } else { 0 };
    Raster { width: seq.settings.width, height: seq.settings.height, rate, end_frame }
}

/// The resampling kernel of a plan's filter.
pub fn kernel_of(filter: Filter) -> Kernel {
    match filter {
        Filter::CatmullRom => Kernel::CatmullRom,
        Filter::Bilinear => Kernel::Bilinear,
    }
}

/// The anchor of a cue whose first burned tick is `cue_start` (spec §6.4 "Anchor per shot"; plan 63 §3.3): the sample of
/// [`window_at`](filmcraft_project::vertical::window_at) at the cue's first frame (the first frame shown at or after
/// `cue_start`, `rate.frame_at(cue_start − 1) + 1`) decides. Before the first position, that position's
/// `caption_anchor`; in a hold, the governing position's; inside a move, and in a plan with no position, none. A position
/// without a `caption_anchor`, and none, give the plan's anchor. The anchor then holds for the whole cue.
pub fn cue_anchor(plan: &VerticalPlan, raster: &Raster, cue_start: Tick) -> CaptionAnchor {
    let rate = raster.rate;
    let first = rate.frame_at(Tick(cue_start.0.saturating_sub(1))).saturating_add(1);
    let smp = window_at(plan, raster, rate.tick_of(first));
    let governing = match smp.phase {
        Phase::Before => plan.positions.first(),
        Phase::Hold => smp.position.and_then(|id| plan.positions.iter().find(|p| p.id == id)),
        Phase::Move | Phase::Starting => None,
    };
    governing.and_then(|p| p.caption_anchor).unwrap_or(plan.captions.anchor)
}

/// The subtitle layout of `plan` for its output at `scale` (spec §6.4 "Defaults"), with a part export's edge rule. The
/// area is the plan's, else, for a 9:16 output (`OW · 16 = OH · 9`), the Union safe area `[120, 288, 768, 960]` of
/// 1080×1920 scaled to the output, else `[5 %, 8 %, 90 %, 84 %]` of the output (each computed in f64 as `n · side / d`).
/// The area and the plan's sizes are then multiplied by `scale`; a size the plan leaves out stays None (each track
/// style's size, and 40 px, per 1080 of the frame's shorter side). The lines, line breaks, anchor and track are the
/// plan's.
pub fn caption_layout(plan: &VerticalPlan, scale: f32, part: Option<PartRange>) -> burn::Layout {
    let c = &plan.captions;
    let (w, h) = (plan.output.width, plan.output.height);
    let of = |n: f64, side: u32, d: f64| (n * f64::from(side) / d) as f32;
    let area = match c.area {
        Some(a) => a.map(|v| v as f32),
        None if u64::from(w) * 16 == u64::from(h) * 9 => [of(120.0, w, 1080.0), of(288.0, h, 1920.0), of(768.0, w, 1080.0), of(960.0, h, 1920.0)],
        None => [of(5.0, w, 100.0), of(8.0, h, 100.0), of(90.0, w, 100.0), of(84.0, h, 100.0)],
    };
    let (size_px, min_px) = (c.size.map(|v| v * scale), c.min_size.map(|v| v * scale));
    let (max_lines, keep_breaks, anchor, track) = (c.max_lines, c.keep_line_breaks, c.anchor, c.track);
    burn::Layout { area: area.map(|v| v * scale), size_px, min_px, max_lines, keep_breaks, anchor, track, part }
}

/// Where the plan's subtitles land in its full-size output over `range` (`vertical.measure`'s `{source: "captions"}`,
/// the `caption_collision` check): one box per cue [`burn::cues_in_range`] burns, laid out with its own anchor
/// ([`cue_anchor`] of the first tick it is burned on), in the order of [`burn::cues_in_range`].
pub fn caption_boxes(seq: &Sequence, plan: &VerticalPlan, range: TimeRange, part: Option<PartRange>) -> Vec<CaptionBox> {
    let raster = raster_of(seq);
    let layout = caption_layout(plan, 1.0, part);
    let (w, h) = (plan.output.width as usize, plan.output.height as usize);
    let mut boxes = Vec::new();
    for cue in burn::cues_in_range(seq, &layout, range) {
        let anchored = burn::Layout { anchor: cue_anchor(plan, &raster, cue.from), ..layout.clone() };
        boxes.extend(burn::cue_box(&cue, w, h, &anchored));
    }
    boxes
}
