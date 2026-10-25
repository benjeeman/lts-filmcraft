//! The vertical frame (Vertical Edit, lane L3b; spec §6.1–§6.4, §9 C8–C11, C15 and C20; §11 T-R6–T-R9). Every picture
//! is synthetic: a coordinate ramp (each pixel's colour names the sequence point it shows), flat colours, and white
//! subtitles on black.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use filmcraft_captions::burn::PartRange;
use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{FrameRequest, Generator, MediaInfo, MediaSource, SharedSource};
use filmcraft_project::vertical::{
    Arrival, CaptionAnchor, CueEdge, Curve, DeliveryInfo, Filter, FitBackground, Layout, Origin, OutputSize, PartAudio, Position, VerticalCaptions,
    VerticalPlan, VerticalSet, WallSet, Window, Writer, centred_largest, window_at,
};
use filmcraft_project::{
    Caption, CaptionFormat, CaptionTrack, ClipId, ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackId, TrackKind,
};
use filmcraft_render::scale::{SCALE_BUCKETS, quantize_scale};
use filmcraft_render::vertical::{caption_boxes, caption_layout, cue_anchor, kernel_of, raster_of};
use filmcraft_render::window::resample_window;
use filmcraft_render::{
    FrameCache16x9, FrameIdentity, FrameKey16x9, Image, RenderOptions, SourceMap, SourceProvider, VerticalRenderInputs, VerticalRenderOptions, render_sequence,
    render_vertical,
};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const RATE: FrameRate = FrameRate::FPS_25;

/// A source that shows one picture at every time, and notes when it is asked for a frame in draft quality.
struct Still {
    info: MediaInfo,
    frame: Arc<VideoFrame>,
    draft: Arc<AtomicBool>,
}

impl MediaSource for Still {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, _req: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        if filmcraft_media::cancel::draft() {
            self.draft.store(true, Ordering::SeqCst);
        }
        Ok(self.frame.clone())
    }
    fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        Ok(AudioBuffer::silence(sample_rate, 2, frames))
    }
}

/// An opaque `w`×`h` picture, premultiplied linear-light RGBA f32, with the colour `f(x, y)` at pixel (x, y).
fn picture(w: u32, h: u32, f: impl Fn(usize, usize) -> [f32; 3]) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = f(x, y);
            px.extend_from_slice(&[r, g, b, 1.0]);
        }
    }
    px
}

/// T-R1's coordinate ramp: R = (x + ½)/W and G = (y + ½)/H, so R·W and G·H name the sequence point a pixel shows.
fn ramp(w: u32, h: u32) -> Vec<f32> {
    picture(w, h, |x, y| [((x as f64 + 0.5) / f64::from(w)) as f32, ((y as f64 + 0.5) / f64::from(h)) as f32, 0.0])
}

fn flat(w: u32, h: u32, c: [f32; 3]) -> Vec<f32> {
    picture(w, h, |_, _| c)
}

/// A vertical set with no plan.
fn empty_set() -> VerticalSet {
    VerticalSet {
        plans: Vec::new(),
        walls: WallSet { use_edit_points: true, stored: Vec::new(), suppressed: Vec::new(), candidates: Vec::new(), analysed: Vec::new() },
        parts: Vec::new(),
        regions: Vec::new(),
        annotations: Vec::new(),
        box_tracks: Vec::new(),
        active_area: Vec::new(),
        agent_overwrite: None,
        part_audio: PartAudio::default(),
        delivery: DeliveryInfo::default(),
        reference: None,
        deliveries: Vec::new(),
        next_plan_id: 1,
        next_part_id: 1,
        next_region_id: 1,
        next_annotation_id: 1,
    }
}

/// A `w`×`h` sequence at 25p whose one clip shows a picture for 10 s, with an empty vertical set.
struct Fixture {
    project: Project,
    seq: ItemId,
    sources: SourceMap,
    draft: Arc<AtomicBool>,
}

impl Fixture {
    fn new(w: u32, h: u32, px: Vec<f32>) -> Fixture {
        let mut project = Project::new("vertical");
        let seq = project.new_sequence("s", SequenceSettings { width: w, height: h, frame_rate: RATE, ..Default::default() }, 1, 0, None);
        let generator = GeneratorSource::new(Generator::BlackVideo, w, h, RATE, Tick(10 * TICKS_PER_SECOND));
        let info = generator.info().clone();
        let media = MediaClip {
            media: MediaRef::Generator(generator.generator.clone()),
            info: info.clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        };
        let item = project.add_item("picture", Label::Iris, ItemKind::Media(media), None);
        let draft = Arc::new(AtomicBool::new(false));
        let still = Still { info, frame: Arc::new(VideoFrame::rgba_f32(w, h, px)), draft: draft.clone() };
        let mut sources = SourceMap::default();
        sources.0.insert(item, Arc::new(still) as SharedSource);
        let mut clip = project.make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, RATE.tick_of(250)), RATE).unwrap();
        clip.scale_to_frame = true;
        let s = project.sequence_mut(seq).unwrap();
        s.video_tracks[0].items.push(clip);
        s.vertical = Some(Box::new(empty_set()));
        Fixture { project, seq, sources, draft }
    }

    fn set_mut(&mut self) -> &mut VerticalSet {
        self.project.sequence_mut(self.seq).and_then(|s| s.vertical.as_deref_mut()).unwrap()
    }

    /// Add a plan of output `out` with `filter` and `positions`; its id.
    fn add_plan(&mut self, out: (u32, u32), filter: Filter, positions: Vec<Position>) -> u32 {
        let set = self.set_mut();
        let id = set.next_plan_id;
        set.next_plan_id += 1;
        let mut plan = VerticalPlan::new(id, &format!("plan {id}"), OutputSize { width: out.0, height: out.1 });
        plan.filter = filter;
        plan.positions = positions;
        plan.next_position_id = 100;
        set.plans.push(plan);
        id
    }

    fn plan(&self, id: u32) -> &VerticalPlan {
        self.project.sequence(self.seq).and_then(|s| s.vertical.as_deref()).and_then(|v| v.plans.iter().find(|p| p.id == id)).unwrap()
    }

    fn plan_mut(&mut self, id: u32) -> &mut VerticalPlan {
        self.set_mut().plans.iter_mut().find(|p| p.id == id).unwrap()
    }

    fn add_captions(&mut self, track: CaptionTrack) {
        self.project.sequence_mut(self.seq).unwrap().caption_tracks.push(track);
    }

    /// No subtitles, no cache, no session.
    fn inputs(&self) -> VerticalRenderInputs<'_> {
        VerticalRenderInputs { sources: &self.sources, captions: false, working_output: false, cache: None, identity: FrameIdentity::default() }
    }

    /// The frame of plan `plan` at frame `frame` and output scale `scale` (no subtitles, no cache).
    fn render(&self, plan: u32, frame: i64, scale: f32) -> Image {
        let opts = VerticalRenderOptions { scale, part: None };
        render_vertical(&self.project, self.seq, plan, RATE.tick_of(frame), opts, &self.inputs()).expect("render_vertical gives a frame")
    }

    /// The frame with the plan's subtitles burned.
    fn render_captioned(&self, plan: u32, frame: i64) -> Image {
        let inputs = VerticalRenderInputs { captions: true, ..self.inputs() };
        render_vertical(&self.project, self.seq, plan, RATE.tick_of(frame), VerticalRenderOptions::default(), &inputs).expect("a subtitled frame")
    }
}

/// A position reached by a cut.
fn cut(id: u32, frame: i64, cx: f64, cy: f64, h: f64) -> Position {
    Position::new(id, RATE.tick_of(frame), Window { cx, cy, h }, Arrival::Cut, Origin::Manual, Writer::Human)
}

/// A position reached by a move of `frames` frames along `curve`.
fn moving(id: u32, frame: i64, cx: f64, cy: f64, h: f64, frames: u32, curve: Curve) -> Position {
    let arrival = Arrival::Move { move_frames: frames, curve, split_of: None };
    Position::new(id, RATE.tick_of(frame), Window { cx, cy, h }, arrival, Origin::Manual, Writer::Human)
}

/// A caption track with the cues `(first frame, end frame, text)`.
fn track(id: u64, language: &str, cues: &[(i64, i64, &str)]) -> CaptionTrack {
    let mut t = CaptionTrack::new(TrackId(id), format!("Subtitles {id}"), CaptionFormat::Subtitle);
    t.language = language.into();
    for (n, &(from, to, text)) in cues.iter().enumerate() {
        let (start, end) = (RATE.tick_of(from), RATE.tick_of(to));
        let id = ClipId(id * 1000 + n as u64);
        t.captions.push(Caption { id, start, duration: end - start, text: text.into(), speaker: None, cue_id: None, settings: String::new() });
    }
    t
}

fn same_bits(a: &Image, b: &Image) -> bool {
    (a.w, a.h) == (b.w, b.h) && a.px.len() == b.px.len() && a.px.iter().zip(&b.px).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// The bounds `(x0, y0, x1, y1)` (inclusive) of the subtitle ink of a frame of a black picture: the pixels with a
/// channel above 0.5.
fn ink_bounds(img: &Image) -> Option<(usize, usize, usize, usize)> {
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for (n, p) in img.px.chunks_exact(4).enumerate() {
        if p[0] > 0.5 || p[1] > 0.5 || p[2] > 0.5 {
            let (x, y) = (n % img.w, n / img.w);
            b = Some(match b {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    b
}

/// The length of the overlap of `[a0, a1)` and `[b0, b1)`.
fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// T-R6 (spec §6.1, §9 C15): a scaled render puts the window exactly on the output's edges, with the per-axis scales
/// `k_x = w·s_h/out_w` and `k_y = h·s_h/out_h`. On the coordinate ramp each interior output pixel names the sequence
/// point it shows, `x + (i + ½)·w/out_w` and `y + (j + ½)·h/out_h`, within 0.01 px (one scale for both axes would be
/// up to half a pixel off at an edge). maxSide 1000, 640 and 480 of 1080×1920, the output `(round(OW·s), round(OH·s))`
/// with `s` as f64, and the 16:9 frame rendered at 1, 0.75 and 0.5; a full-height and a zoomed window.
#[test]
fn t_r6_scaled_renders_put_the_window_on_the_output_edges() {
    let mut f = Fixture::new(1920, 1080, ramp(1920, 1080));
    let plan = f.add_plan((1080, 1920), Filter::CatmullRom, vec![cut(1, 0, 700.25, 540.0, 1080.0), cut(2, 10, 1100.5, 420.75, 720.0)]);
    let raster = raster_of(f.project.sequence(f.seq).unwrap());
    let mut checked = 0usize;
    for frame in [0, 10] {
        let r = window_at(f.plan(plan), &raster, RATE.tick_of(frame)).window.rect(OutputSize { width: 1080, height: 1920 });
        for max_side in [1000.0f32, 640.0, 480.0] {
            let s = max_side / 1920.0;
            let img = f.render(plan, frame, s);
            let out = ((1080.0 * f64::from(s)).round() as usize, (1920.0 * f64::from(s)).round() as usize);
            assert_eq!((img.w, img.h), out, "maxSide {max_side}: the output size");
            let (kx, ky) = (r.w / out.0 as f64, r.h / out.1 as f64);
            for j in 0..out.1 {
                let y = r.y + (j as f64 + 0.5) * ky;
                if !(8.0..=1072.0).contains(&y) {
                    continue;
                }
                for i in 0..out.0 {
                    let x = r.x + (i as f64 + 0.5) * kx;
                    let p = img.get(i, j);
                    let (dx, dy) = (f64::from(p[0]) * 1920.0 - x, f64::from(p[1]) * 1080.0 - y);
                    assert!(dx.abs() <= 0.01 && dy.abs() <= 0.01, "maxSide {max_side}, frame {frame}: pixel ({i}, {j}) is {dx:+.4} / {dy:+.4} px off");
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 1_000_000, "only {checked} pixels were checked");
}

/// Spec §6.6 (1, 2), §9 C8 and C9: at full size the frame is exactly the declared resample, `resample_window` with the
/// plan's kernel of the rectangle `window_at` gives, on the frame `render_sequence` makes at scale 1 without captions:
/// bit for bit in holds, at a cut and inside a Ramp move, with both filters.
#[test]
fn full_size_is_the_declared_resample() {
    let mut f = Fixture::new(640, 360, ramp(640, 360));
    let positions = vec![cut(1, 0, 200.0, 180.0, 360.0), moving(2, 20, 420.5, 150.25, 240.0, 16, Curve::Ramp { r_in: 0.2, r_out: 0.2 })];
    let plans = [f.add_plan((360, 640), Filter::CatmullRom, positions.clone()), f.add_plan((360, 640), Filter::Bilinear, positions)];
    let raster = raster_of(f.project.sequence(f.seq).unwrap());
    for plan in plans {
        for frame in [0, 4, 9, 20, 30] {
            let t = RATE.tick_of(frame);
            let p = f.plan(plan);
            let r = window_at(p, &raster, t).window.rect(p.output);
            let frame16 = render_sequence(&f.project, f.seq, t, RenderOptions::default(), &f.sources);
            let want = resample_window(&frame16, [r.x, r.y, r.w, r.h], (360, 640), None, kernel_of(p.filter));
            let got = f.render(plan, frame, 1.0);
            assert!(same_bits(&got, &want), "plan {plan}, frame {frame}: not the declared resample");
        }
    }
}

/// T-R7 (spec §6.1 step 5): a `fit` position puts the whole 16:9 frame at the output's width, centred, `[0, y0, out_w,
/// out_w·H/W]`. Over black, the rows it does not reach are opaque black, its rows show the frame, and its two edge rows
/// show it weighted by their coverage (640×360 into 360×640: rows 218 and 421 are a quarter covered). At a scaled
/// output the frame spans the output's whole width.
#[test]
fn t_r7_fit_over_black_weights_the_edge_rows_by_their_coverage() {
    let c = [0.2f32, 0.4, 0.6];
    let mut f = Fixture::new(640, 360, flat(640, 360, c));
    let mut fit = cut(1, 0, 320.0, 180.0, 360.0);
    fit.layout = Layout::Fit;
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![fit]);
    f.plan_mut(plan).fit_background = FitBackground::Black;
    for s in [1.0f32, 0.5, 1000.0 / 1920.0] {
        let img = f.render(plan, 3, s);
        let out = ((360.0 * f64::from(s)).round() as usize, (640.0 * f64::from(s)).round() as usize);
        assert_eq!((img.w, img.h), out, "scale {s}: the output size");
        let dh = out.0 as f64 * 360.0 / 640.0;
        let y0 = (out.1 as f64 - dh) * 0.5;
        for j in 0..out.1 {
            let cover = overlap(j as f64, j as f64 + 1.0, y0, y0 + dh) as f32;
            for i in 0..out.0 {
                let p = img.get(i, j);
                let want = [c[0] * cover, c[1] * cover, c[2] * cover, 1.0];
                let off = (0..4).map(|k| (p[k] - want[k]).abs()).fold(0.0f32, f32::max);
                assert!(off <= 1e-6, "scale {s}: pixel ({i}, {j}) is {p:?}, not {want:?} (row coverage {cover})");
            }
        }
    }
}

/// T-R7: over the blurred background a flat frame gives 0.7 of its colour above and below the frame, the colour itself
/// inside, and their coverage mix on the edge rows: 0.25 + 0.75·0.7 = 0.775 of it.
#[test]
fn t_r7_fit_over_blur_of_a_flat_frame() {
    let c = [0.2f32, 0.4, 0.6];
    let mut f = Fixture::new(640, 360, flat(640, 360, c));
    let mut fit = cut(1, 0, 320.0, 180.0, 360.0);
    fit.layout = Layout::Fit;
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![fit]);
    assert_eq!(f.plan(plan).fit_background, FitBackground::Blur, "a new plan's fit background");
    let img = f.render(plan, 3, 1.0);
    for j in 0..640 {
        let cover = overlap(j as f64, j as f64 + 1.0, 218.75, 421.25) as f32;
        let gain = cover + (1.0 - cover) * 0.7;
        for i in 0..360 {
            let p = img.get(i, j);
            let want = [c[0] * gain, c[1] * gain, c[2] * gain, 1.0];
            let off = (0..4).map(|k| (p[k] - want[k]).abs()).fold(0.0f32, f32::max);
            assert!(off <= 1e-4, "pixel ({i}, {j}) is {p:?}, not {want:?}");
        }
    }
}

/// T-R7: the rows the frame does not reach are, bit for bit, the background of spec §6.1: the centred largest window
/// resampled with the plan's kernel from the 16:9 frame at `s_h`, blurred by `effects::gaussian` with
/// `σ = (OW·s)/45` and repeated edges, colour channels times 0.7, alpha kept. The frame's own rows show the whole 16:9
/// frame at `k = W/out_w` on both axes. At full size and at half size (the 16:9 frame at 0.375).
#[test]
fn t_r7_fit_blur_background_is_the_blurred_centre_window() {
    let mut f = Fixture::new(640, 360, ramp(640, 360));
    let mut fit = cut(1, 0, 320.0, 180.0, 360.0);
    fit.layout = Layout::Fit;
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![fit]);
    let raster = raster_of(f.project.sequence(f.seq).unwrap());
    let largest = centred_largest(&raster, OutputSize { width: 360, height: 640 }).rect(OutputSize { width: 360, height: 640 });
    for (s, s_h) in [(1.0f32, 1.0f32), (0.5, 0.375)] {
        let img = f.render(plan, 0, s);
        let out = ((360.0 * f64::from(s)).round() as usize, (640.0 * f64::from(s)).round() as usize);
        assert_eq!((img.w, img.h), out, "scale {s}: the output size");
        let frame16 = render_sequence(&f.project, f.seq, Tick::ZERO, RenderOptions { scale: s_h, ..Default::default() }, &f.sources);
        let k = f64::from(s_h);
        let mut bg = resample_window(&frame16, [largest.x * k, largest.y * k, largest.w * k, largest.h * k], out, None, kernel_of(Filter::CatmullRom));
        let sigma = (360.0 * s) / 45.0;
        filmcraft_render::effects::gaussian(&mut bg, sigma, sigma, true);
        for p in bg.px.chunks_exact_mut(4) {
            p[0] *= 0.7;
            p[1] *= 0.7;
            p[2] *= 0.7;
        }
        let dh = out.0 as f64 * 360.0 / 640.0;
        let y0 = (out.1 as f64 - dh) * 0.5;
        let kf = 640.0 / out.0 as f64;
        let mut inside = 0;
        for j in 0..out.1 {
            let (top, bottom) = (j as f64, j as f64 + 1.0);
            for i in 0..out.0 {
                let (p, q) = (img.get(i, j), bg.get(i, j));
                if bottom <= y0 || top >= y0 + dh {
                    assert!(p.iter().zip(&q).all(|(a, b)| a.to_bits() == b.to_bits()), "scale {s}: pixel ({i}, {j}) is {p:?}, the background {q:?}");
                    continue;
                }
                let (x, y) = ((i as f64 + 0.5) * kf, (j as f64 + 0.5 - y0) * kf);
                if top >= y0 + 4.0 && bottom <= y0 + dh - 4.0 && (12.0..=628.0).contains(&x) {
                    let (dx, dy) = (f64::from(p[0]) * 640.0 - x, f64::from(p[1]) * 360.0 - y);
                    assert!(dx.abs() <= 0.01 && dy.abs() <= 0.01, "scale {s}: fit pixel ({i}, {j}) is {dx:+.4} / {dy:+.4} px off");
                    inside += 1;
                }
            }
        }
        assert!(inside > 1000, "scale {s}: only {inside} pixels of the frame were checked");
    }
}

/// A 16:9 frame cache that counts its hits, misses and stores, and keeps the last key stored (T-R8).
#[derive(Default)]
struct CountingCache {
    frames: Mutex<HashMap<FrameKey16x9, Arc<Image>>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
    puts: AtomicUsize,
    last: Mutex<Option<FrameKey16x9>>,
}

impl FrameCache16x9 for CountingCache {
    fn get(&self, key: &FrameKey16x9) -> Option<Arc<Image>> {
        let found = self.frames.lock().unwrap().get(key).cloned();
        let counter = if found.is_some() { &self.hits } else { &self.misses };
        counter.fetch_add(1, Ordering::SeqCst);
        found
    }
    fn put(&self, key: FrameKey16x9, frame: Arc<Image>) {
        self.puts.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some(key);
        self.frames.lock().unwrap().insert(key, frame);
    }
}

impl CountingCache {
    fn counts(&self) -> (usize, usize, usize) {
        (self.hits.load(Ordering::SeqCst), self.misses.load(Ordering::SeqCst), self.puts.load(Ordering::SeqCst))
    }
    fn last(&self) -> Option<FrameKey16x9> {
        *self.last.lock().unwrap()
    }
}

/// A source provider that counts the sources it hands out: a 16:9 render asks for one per clip it draws.
struct CountingSources<'a> {
    inner: &'a SourceMap,
    calls: AtomicUsize,
}

impl SourceProvider for CountingSources<'_> {
    fn source(&self, item: ItemId) -> Option<SharedSource> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.source(item)
    }
}

/// T-R8, the render half (spec §6.3, §9 C20): with a cache, a plan-only change (the window moves; the picture revision
/// stays) renders no 16:9 frame and asks for no source; a changed media generation, picture revision, epoch or
/// working-space flag misses and renders. The key carries the identity, the sequence, the frame and the scale of the
/// 16:9 frame (`s_h`), and a frame from the cache gives the bits of a direct render.
#[test]
fn t_r8_a_plan_only_change_renders_no_16x9_frame() {
    let mut f = Fixture::new(640, 360, ramp(640, 360));
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![cut(1, 0, 200.0, 180.0, 360.0)]);
    let cache = CountingCache::default();
    let id = FrameIdentity { epoch: 3, picture_revision: 41, media_generation: 7 };
    let t = RATE.tick_of(12);
    let first = {
        let sources = CountingSources { inner: &f.sources, calls: AtomicUsize::new(0) };
        let inputs = VerticalRenderInputs { sources: &sources, captions: false, working_output: false, cache: Some(&cache), identity: id };
        let img = render_vertical(&f.project, f.seq, plan, t, VerticalRenderOptions::default(), &inputs).expect("the first frame");
        assert!(sources.calls.load(Ordering::SeqCst) > 0, "the first frame renders the 16:9 frame");
        img
    };
    assert_eq!(cache.counts(), (0, 1, 1), "the first frame: one miss, one store");
    let key =
        FrameKey16x9 { epoch: 3, sequence: f.seq, frame: 12, picture_revision: 41, scale_bits: 1.0f32.to_bits(), working_output: false, media_generation: 7 };
    assert_eq!(cache.last(), Some(key), "the key of the stored frame");
    f.plan_mut(plan).positions[0].window.cx = 420.0;
    let sources = CountingSources { inner: &f.sources, calls: AtomicUsize::new(0) };
    let inputs = VerticalRenderInputs { sources: &sources, captions: false, working_output: false, cache: Some(&cache), identity: id };
    let moved = render_vertical(&f.project, f.seq, plan, t, VerticalRenderOptions::default(), &inputs).expect("the moved frame");
    assert_eq!(sources.calls.load(Ordering::SeqCst), 0, "a plan-only change asks for no source");
    assert_eq!(cache.counts(), (1, 1, 1), "a plan-only change hits the cache and stores nothing");
    assert!(!same_bits(&moved, &first), "the window moved");
    let direct = render_vertical(&f.project, f.seq, plan, t, VerticalRenderOptions::default(), &f.inputs()).expect("a direct render");
    assert!(same_bits(&moved, &direct), "a 16:9 frame from the cache gives the bits of a direct render");
    let changed = [FrameIdentity { media_generation: 8, ..id }, FrameIdentity { picture_revision: 42, ..id }, FrameIdentity { epoch: 4, ..id }];
    for (n, identity) in changed.into_iter().enumerate() {
        let before = sources.calls.load(Ordering::SeqCst);
        let inputs = VerticalRenderInputs { identity, ..inputs };
        render_vertical(&f.project, f.seq, plan, t, VerticalRenderOptions::default(), &inputs).expect("a frame");
        assert!(sources.calls.load(Ordering::SeqCst) > before, "{identity:?}: the 16:9 frame is rendered again");
        assert_eq!(cache.counts(), (1, 2 + n, 2 + n), "{identity:?}: a miss and a store");
        let stored = cache.last().map(|k| FrameIdentity { epoch: k.epoch, picture_revision: k.picture_revision, media_generation: k.media_generation });
        assert_eq!(stored, Some(identity), "the stored key carries the identity");
    }
    let working = VerticalRenderInputs { working_output: true, ..inputs };
    render_vertical(&f.project, f.seq, plan, t, VerticalRenderOptions::default(), &working).expect("a working-space frame");
    assert_eq!(cache.counts(), (1, 5, 5), "the working-space frame is another key");
    assert_eq!(cache.last().map(|k| k.working_output), Some(true));
    let quarter = VerticalRenderOptions { scale: 0.25, part: None };
    render_vertical(&f.project, f.seq, plan, t, quarter, &inputs).expect("a scaled frame");
    assert_eq!(cache.counts(), (1, 6, 6), "a scaled frame is another key");
    assert_eq!(cache.last().map(|k| k.scale_bits), Some(0.5f32.to_bits()), "the 16:9 frame of a quarter-size 9:16 frame is rendered at 0.5");
}

/// T-R8: a tick inside a frame shows the 16:9 picture of that tick, which a frame-keyed cache cannot hold. It is
/// rendered directly, neither looked up nor stored, and equals an uncached render.
#[test]
fn t_r8_a_tick_inside_a_frame_bypasses_the_cache() {
    let mut f = Fixture::new(640, 360, ramp(640, 360));
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![cut(1, 0, 200.0, 180.0, 360.0), moving(2, 30, 420.0, 180.0, 300.0, 20, Curve::Linear)]);
    let cache = CountingCache::default();
    let inputs = VerticalRenderInputs { cache: Some(&cache), identity: FrameIdentity { epoch: 1, picture_revision: 1, media_generation: 1 }, ..f.inputs() };
    let t = RATE.tick_of(17) + Tick(RATE.frame_duration().0 / 3);
    let img = render_vertical(&f.project, f.seq, plan, t, VerticalRenderOptions::default(), &inputs).expect("a frame");
    assert_eq!(cache.counts(), (0, 0, 0), "the cache is not used for a tick inside a frame");
    let direct = render_vertical(&f.project, f.seq, plan, t, VerticalRenderOptions::default(), &f.inputs()).expect("a direct render");
    assert!(same_bits(&img, &direct));
}

/// T-R9 (spec §6.4 "Anchor per shot", plan 63 §3.3): a cue's anchor is decided by `window_at` at its first frame: before
/// the first position, that position's anchor; in a hold, the governing position's; inside a move, and with no
/// position, the plan's; a position without an anchor gives the plan's. A cue that starts inside a frame is first shown
/// on the next frame.
#[test]
fn t_r9_a_cue_takes_the_anchor_of_the_position_governing_its_first_frame() {
    let mut f = Fixture::new(640, 360, flat(640, 360, [0.0; 3]));
    let mut first = cut(1, 10, 320.0, 180.0, 360.0);
    first.caption_anchor = Some(CaptionAnchor::Top);
    let mut last = moving(3, 40, 440.0, 180.0, 360.0, 10, Curve::Linear);
    last.caption_anchor = Some(CaptionAnchor::Top);
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![first, cut(2, 20, 200.0, 180.0, 360.0), last]);
    let empty = f.add_plan((360, 640), Filter::CatmullRom, Vec::new());
    f.plan_mut(empty).captions.anchor = CaptionAnchor::Top;
    let raster = raster_of(f.project.sequence(f.seq).unwrap());
    let p = f.plan(plan);
    assert_eq!(p.captions.anchor, CaptionAnchor::Bottom, "the plan's anchor");
    let at = |frame: i64| cue_anchor(p, &raster, RATE.tick_of(frame));
    assert_eq!(at(2), CaptionAnchor::Top, "before the first position: its anchor");
    assert_eq!(at(10), CaptionAnchor::Top, "on the first position");
    assert_eq!(at(15), CaptionAnchor::Top, "in its hold");
    assert_eq!(at(20), CaptionAnchor::Bottom, "a position without an anchor: the plan's");
    assert_eq!(at(30), CaptionAnchor::Bottom, "the move's first frame still shows the departing position");
    assert_eq!(at(33), CaptionAnchor::Bottom, "inside a move: the plan's, not the arriving position's");
    assert_eq!(at(40), CaptionAnchor::Top, "on the arriving position");
    assert_eq!(cue_anchor(p, &raster, RATE.tick_of(19) + Tick(1)), CaptionAnchor::Bottom, "a cue starting inside frame 19 is first shown on frame 20");
    assert_eq!(cue_anchor(f.plan(empty), &raster, RATE.tick_of(5)), CaptionAnchor::Top, "no position: the plan's anchor");
}

/// T-R9: the anchor a cue starts with holds for the whole cue (a subtitle never jumps mid-display): a cue that starts
/// while a position without an anchor governs stays at the bottom of the area after a top-anchored position takes
/// over; the next cue, which starts under it, is at the top.
#[test]
fn t_r9_the_anchor_holds_for_the_whole_cue() {
    let mut f = Fixture::new(640, 360, flat(640, 360, [0.0; 3]));
    let mut top = cut(2, 10, 300.0, 180.0, 360.0);
    top.caption_anchor = Some(CaptionAnchor::Top);
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![cut(1, 0, 320.0, 180.0, 360.0), top]);
    f.plan_mut(plan).captions.size = Some(24.0);
    f.plan_mut(plan).captions.min_size = Some(16.0);
    f.add_captions(track(1, "en", &[(6, 16, "Holds"), (18, 24, "Next")]));
    let area = caption_layout(f.plan(plan), 1.0, None).area;
    assert_eq!(area, [40.0, 96.0, 256.0, 320.0], "the default 9:16 area of 360×640");
    let middle = f64::from(area[1] + area[3] * 0.5);
    for frame in [6, 9, 10, 15, 18, 23] {
        let img = f.render_captioned(plan, frame);
        let (_, y0, _, y1) = ink_bounds(&img).unwrap_or_else(|| panic!("frame {frame}: no subtitle burned"));
        if frame < 16 {
            assert!(y0 as f64 > middle, "frame {frame}: the first cue's ink (rows {y0}–{y1}) is not at the bottom of the area");
        } else {
            assert!((y1 as f64) < middle, "frame {frame}: the second cue's ink (rows {y0}–{y1}) is not at the top of the area");
        }
    }
    assert_eq!(ink_bounds(&f.render_captioned(plan, 16)), None, "frame 16 shows no cue");
}

/// Spec §6.1 steps 3 and 6: the 16:9 caption layer is always off in the vertical frame, so an enabled caption track
/// draws nothing without `captions`; with `captions` its cue is laid out again inside the area, and nowhere else.
#[test]
fn only_the_vertical_subtitle_layout_is_burned() {
    let mut f = Fixture::new(640, 360, flat(640, 360, [0.0; 3]));
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![cut(1, 0, 320.0, 180.0, 360.0)]);
    f.plan_mut(plan).captions.size = Some(24.0);
    f.add_captions(track(1, "en", &[(0, 50, "Caption text")]));
    assert_eq!(ink_bounds(&f.render(plan, 10, 1.0)), None, "without captions nothing is burned, although the 16:9 track is on");
    let [ax, ay, aw, ah] = caption_layout(f.plan(plan), 1.0, None).area;
    let (x0, y0, x1, y1) = ink_bounds(&f.render_captioned(plan, 10)).expect("the cue is burned");
    let inside = x0 as f32 >= ax && (x1 as f32) < ax + aw && y0 as f32 >= ay && (y1 as f32) < ay + ah;
    assert!(inside, "ink ({x0}, {y0})–({x1}, {y1}) outside the area [{ax}, {ay}, {aw}, {ah}]");
}

/// Spec §6.4 "Defaults": a 9:16 output's area is the Union safe area `[120, 288, 768, 960]` of 1080×1920 scaled to the
/// output, other aspects take `[5 %, 8 %, 90 %, 84 %]`, a plan's own area and sizes are kept, everything scales with
/// the output, and the lines, breaks, anchor, track and part are the plan's.
#[test]
fn caption_layout_defaults_and_scale() {
    let plan = |w: u32, h: u32| VerticalPlan::new(1, "p", OutputSize { width: w, height: h });
    let area = |w: u32, h: u32, s: f32| caption_layout(&plan(w, h), s, None).area;
    assert_eq!(area(1080, 1920, 1.0), [120.0, 288.0, 768.0, 960.0]);
    assert_eq!(area(720, 1280, 1.0), [80.0, 192.0, 512.0, 640.0]);
    assert_eq!(area(1080, 1920, 0.5), [60.0, 144.0, 384.0, 480.0]);
    for (w, h, want) in [(1080, 1350, [54.0f32, 108.0, 972.0, 1134.0]), (1080, 1080, [54.0, 86.4, 972.0, 907.2])] {
        let got = area(w, h, 1.0);
        assert!(got.iter().zip(&want).all(|(a, b)| (a - b).abs() <= 1e-3), "{w}×{h}: {got:?}, not {want:?}");
    }
    let l = caption_layout(&plan(1080, 1920), 1.0, None);
    assert_eq!((l.size_px, l.min_px, l.max_lines, l.keep_breaks), (None, None, 2, true));
    assert_eq!((l.anchor, l.track, l.part), (CaptionAnchor::Bottom, None, None));
    let mut own = plan(1080, 1920);
    own.captions.area = Some([10, 20, 300, 400]);
    own.captions.size = Some(30.0);
    own.captions.min_size = Some(20.0);
    own.captions.max_lines = 3;
    own.captions.keep_line_breaks = false;
    own.captions.anchor = CaptionAnchor::Top;
    own.captions.track = Some(7);
    let part = PartRange { range: TimeRange::new(RATE.tick_of(10), RATE.tick_of(20)), edge: CueEdge::Drop, min_clipped_frames: 3 };
    let l = caption_layout(&own, 0.5, Some(part));
    assert_eq!(l.area, [5.0, 10.0, 150.0, 200.0]);
    assert_eq!((l.size_px, l.min_px, l.max_lines, l.keep_breaks), (Some(15.0), Some(10.0), 3, false));
    assert_eq!((l.anchor, l.track, l.part), (CaptionAnchor::Top, Some(7), Some(part)));
}

/// The caption boxes of `vertical.measure`'s `{source: "captions"}`: one per cue burned over the range, in the plan's
/// full-size output, each laid out with its own anchor and inside the area; a part's edge rule clips or drops them.
#[test]
fn caption_boxes_follow_each_cue_anchor_and_the_part_rule() {
    let mut f = Fixture::new(640, 360, flat(640, 360, [0.0; 3]));
    let mut top = cut(2, 10, 300.0, 180.0, 360.0);
    top.caption_anchor = Some(CaptionAnchor::Top);
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![cut(1, 0, 320.0, 180.0, 360.0), top]);
    f.plan_mut(plan).captions.size = Some(24.0);
    f.add_captions(track(1, "en", &[(6, 16, "Holds"), (18, 24, "Next")]));
    let seq = f.project.sequence(f.seq).unwrap();
    let p = f.plan(plan);
    let [ax, ay, aw, ah] = caption_layout(p, 1.0, None).area;
    let range = |from: i64, to: i64| TimeRange::new(RATE.tick_of(from), RATE.tick_of(to) - RATE.tick_of(from));
    let boxes = caption_boxes(seq, p, range(0, 30), None);
    assert_eq!(boxes.len(), 2, "one box per cue");
    let (a, b) = (&boxes[0], &boxes[1]);
    assert_eq!((a.anchor, a.from, a.to), (CaptionAnchor::Bottom, RATE.tick_of(6), RATE.tick_of(16)));
    assert_eq!((b.anchor, b.from, b.to), (CaptionAnchor::Top, RATE.tick_of(18), RATE.tick_of(24)));
    assert_eq!(a.text, ["Holds"]);
    assert_eq!(b.text, ["Next"]);
    for bx in [a, b] {
        let [x, y, w, h] = bx.rect;
        assert!(x >= ax && x + w <= ax + aw && y >= ay - 0.5 && y + h <= ay + ah + 0.5, "{bx:?} is not inside the area");
    }
    assert!((a.rect[1] + a.rect[3] - (ay + ah)).abs() <= 1.0, "the bottom-anchored cue ends at the area's bottom: {a:?}");
    assert!((b.rect[1] - ay).abs() <= 1.0, "the top-anchored cue starts at the area's top: {b:?}");
    let clip = PartRange { range: range(8, 20), edge: CueEdge::Clip, min_clipped_frames: 0 };
    let clipped = caption_boxes(seq, p, range(0, 30), Some(clip));
    let spans: Vec<(Tick, Tick)> = clipped.iter().map(|bx| (bx.from, bx.to)).collect();
    assert_eq!(spans, [(RATE.tick_of(8), RATE.tick_of(16)), (RATE.tick_of(18), RATE.tick_of(20))], "clipped to the part");
    let dropping = PartRange { edge: CueEdge::Drop, ..clip };
    assert!(caption_boxes(seq, p, range(0, 30), Some(dropping)).is_empty(), "both cues straddle an edge of the part");
    assert!(caption_boxes(seq, p, range(16, 18), None).is_empty(), "no cue between the two");
}

/// Spec §6.1: the vertical frame never decodes in draft quality, even inside a draft frame job (a plain 16:9 render
/// there does, which shows the probe works).
#[test]
fn never_decodes_in_draft_quality() {
    let mut f = Fixture::new(640, 360, ramp(640, 360));
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![cut(1, 0, 200.0, 180.0, 360.0)]);
    let t = RATE.tick_of(5);
    let img = filmcraft_media::cancel::with_draft(true, || render_vertical(&f.project, f.seq, plan, t, VerticalRenderOptions::default(), &f.inputs()));
    assert!(img.is_some(), "a frame");
    assert!(!f.draft.load(Ordering::SeqCst), "the vertical frame asked a source for a draft-quality frame");
    filmcraft_media::cancel::with_draft(true, || render_sequence(&f.project, f.seq, t, RenderOptions::default(), &f.sources));
    assert!(f.draft.load(Ordering::SeqCst), "the probe sees a draft request");
}

/// The preview scale buckets (spec §6.1), pinned: the Program monitor's list; the smallest bucket at or above the need
/// within 1e-4, and 1 above the last; NaN gives 1, nothing positive gives the smallest.
#[test]
fn quantize_scale_buckets_are_pinned() {
    assert_eq!(SCALE_BUCKETS, [1.0 / 32.0, 1.0 / 16.0, 1.0 / 8.0, 0.1875, 0.25, 0.375, 0.5, 0.75, 1.0]);
    for b in SCALE_BUCKETS {
        assert_eq!(quantize_scale(b), b, "a bucket is its own scale");
    }
    let cases = [
        (0.0f32, 1.0 / 32.0),
        (-3.0, 1.0 / 32.0),
        (f32::NEG_INFINITY, 1.0 / 32.0),
        (0.01, 1.0 / 32.0),
        (0.0313, 1.0 / 32.0),
        (0.0314, 1.0 / 16.0),
        (0.1, 0.125),
        (0.13, 0.1875),
        (0.2, 0.25),
        (0.25005, 0.25),
        (0.26, 0.375),
        (0.3, 0.375),
        (0.444, 0.5),
        (0.6, 0.75),
        (0.75009, 0.75),
        (0.7502, 1.0),
        (0.9259, 1.0),
        (2.0, 1.0),
        (f32::INFINITY, 1.0),
        (f32::NAN, 1.0),
    ];
    for (s, want) in cases {
        assert_eq!(quantize_scale(s), want, "quantize_scale({s})");
    }
}

/// Spec §6.2 "Deterministic", §9 C11: a frame has the same bits with 1, 2 and 8 threads: a zoomed `fill` frame, a
/// `fit` frame over the blur and a subtitled frame.
#[test]
fn same_bits_with_any_thread_count() {
    let mut f = Fixture::new(640, 360, ramp(640, 360));
    let mut fit = cut(2, 10, 320.0, 180.0, 360.0);
    fit.layout = Layout::Fit;
    let plan = f.add_plan((360, 640), Filter::CatmullRom, vec![cut(1, 0, 230.5, 150.25, 200.0), fit]);
    f.plan_mut(plan).captions.size = Some(24.0);
    f.add_captions(track(1, "en", &[(15, 25, "Same bits")]));
    for frame in [5, 12, 20] {
        let run = |threads: usize| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            pool.install(|| f.render_captioned(plan, frame))
        };
        let one = run(1);
        for threads in [2, 8] {
            assert!(same_bits(&one, &run(threads)), "frame {frame}: {threads} threads give other bits than 1");
        }
    }
}

/// Never crash (FilmCraft AGENTS.md §0): a missing sequence or plan, a scale that is not finite and positive or gives no
/// valid output size, and a plan output that is not a frame size give no frame; windows that are not finite, far
/// frames and hostile subtitle settings give a frame of the right size with finite pixels.
#[test]
fn hostile_requests_never_panic() {
    let mut f = Fixture::new(64, 36, ramp(64, 36));
    let windows = vec![cut(1, 0, 32.0, 18.0, 36.0), cut(2, 5, f64::NAN, f64::INFINITY, -4.0), cut(3, 9, 1e300, -1e300, 1e-300)];
    let plan = f.add_plan((36, 64), Filter::CatmullRom, windows);
    let no_size = f.add_plan((0, 0), Filter::Bilinear, vec![cut(1, 0, 32.0, 18.0, 36.0)]);
    let odd = f.add_plan((36, 64), Filter::Bilinear, vec![cut(1, 0, 32.0, 18.0, 36.0)]);
    {
        let c = &mut f.plan_mut(odd).captions;
        c.area = Some([u32::MAX, 3, u32::MAX, u32::MAX]);
        c.size = Some(f32::NAN);
        c.min_size = Some(-1.0);
        c.max_lines = 0;
    }
    f.add_captions(track(1, "he", &[(0, 40, "שלום\n\n<i>x</i>"), (40, 41, "")]));
    let inputs = VerticalRenderInputs { captions: true, ..f.inputs() };
    let frame = |seq: ItemId, id: u32, t: Tick, scale: f32| render_vertical(&f.project, seq, id, t, VerticalRenderOptions { scale, part: None }, &inputs);
    assert!(frame(ItemId(424_242), plan, Tick::ZERO, 1.0).is_none(), "no such sequence");
    assert!(frame(f.seq, 999, Tick::ZERO, 1.0).is_none(), "no such plan");
    assert!(frame(f.seq, no_size, Tick::ZERO, 1.0).is_none(), "a plan output that is not a frame size");
    for s in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e-9, 1e9] {
        assert!(frame(f.seq, plan, Tick::ZERO, s).is_none(), "scale {s}");
    }
    for p in [plan, odd] {
        for n in [-100, 0, 3, 5, 7, 9, 40, 100_000] {
            let img = frame(f.seq, p, RATE.tick_of(n), 1.0).unwrap_or_else(|| panic!("plan {p}, frame {n}: no frame"));
            assert_eq!((img.w, img.h, img.px.len()), (36, 64, 36 * 64 * 4), "plan {p}, frame {n}");
            assert!(img.px.iter().all(|v| v.is_finite()), "plan {p}, frame {n}: a pixel is not finite");
        }
    }
    f.plan_mut(odd).captions = VerticalCaptions { size: Some(1e30), area: Some([0, 0, 36, 64]), ..Default::default() };
    let inputs = VerticalRenderInputs { captions: true, ..f.inputs() };
    let img = render_vertical(&f.project, f.seq, odd, RATE.tick_of(3), VerticalRenderOptions::default(), &inputs).expect("a frame with a huge subtitle size");
    assert_eq!((img.w, img.h), (36, 64));
}
