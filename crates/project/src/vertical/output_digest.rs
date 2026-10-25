//! The output digest of a part in a plan (spec section 2.6): everything else, besides its framing, that makes the part's
//! delivered file. (1) The plan's `output`, `filter`, `captions` and `fitBackground`; (2) the part's resolved fades and its
//! metadata; (3) for the plan's caption tracks: their language, style and enabled flag, and every cue overlapping the part
//! (times, text, settings); (4) every enabled item on any video or audio track overlapping the part, in full, and the
//! identity of its media (path, file size, frame count, rate, frame size); (5) the video and audio transitions overlapping
//! the part; (6) the sequence's frame size, rate, colour pipeline, sample rate and mix (master volume, master effects and
//! automation, submixes, and the mixer settings of the tracks with items in the part); (7) recursively, the same for the
//! nested and multicam sequences those items show, to the renderer's nest depth (8). A nested sequence enters as the
//! digest of its own view, computed once per project item and depth, so a sequence that many clips show (or a hostile
//! project nesting one sequence many times over) costs its items once per depth.
//!
//! Excluded: positions (they are in the framing digest), annotations, box tracks, active areas, attention marks, markers,
//! names and labels, other plans, the view. It is conservative: an input change in range changes it even when the pixels
//! happen not to change. FNV-1a 64 over canonical bytes, printed `fnv1a64:` and 16 hex digits, as every digest.
//!
//! The plan's caption tracks are its pinned track, or every caption track with its enabled flag (so turning a track's
//! output off or on changes the digest). A nested or multicam sequence is digested whole, every enabled item of it.

use std::collections::BTreeMap;

use filmcraft_color::ColorPipeline;
use filmcraft_time::{FrameRate, Tick};
use serde::Serialize;

use super::derive::Scene;
use super::digest::digest_of;
use super::types::{Filter, FitBackground, OutputSize, Part, PartAudio, PartMeta, VerticalCaptions, VerticalPlan};
use crate::{
    AudioChannels, Caption, CaptionStyle, CaptionTrack, EffectInstance, EssentialSound, FieldOptions, GraphicMeta, Interpretation, ItemId, ItemKind, MediaRef,
    MixerStrip, MulticamSel, Project, Sequence, TimeInterpolation, Track, TrackItem, TrackKind, Transition,
};

/// The deepest nesting the digest follows, as the renderer's `MAX_NEST_DEPTH`.
pub const NEST_DEPTH: u32 = 8;

/// The digests of the nested sequences already viewed, by project item and nesting depth.
type Nests = BTreeMap<(ItemId, u32), String>;

/// The output digest of `part` in `plan` of `seq`.
pub fn output_digest(project: &Project, seq: &Sequence, plan: &VerticalPlan, part: &Part) -> String {
    output_digest_in(&Scene::of(project, seq), plan, part)
}

/// The output digest of a part in a plan of a scene.
pub(crate) fn output_digest_in(scene: &Scene, plan: &VerticalPlan, part: &Part) -> String {
    let fades = part.audio.or(scene.set.map(|s| s.part_audio)).unwrap_or_default();
    let range = (part.from, part.to);
    let pinned = plan.captions.track;
    let tracks = scene.seq.caption_tracks.iter().filter(|t| pinned.is_none_or(|id| t.id.0 == u64::from(id)));
    let captions = tracks.map(|t| captions_view(t, range)).collect();
    let content = OutputView {
        plan: PlanView { output: plan.output, filter: plan.filter, captions: &plan.captions, fit_background: plan.fit_background },
        part: PartView { fades, meta: &part.meta },
        captions,
        sequence: sequence_view(scene.project, scene.seq, Some(range), 0, &mut Nests::new()),
    };
    digest_of(&content)
}

/// A caption track's view: its language, style and enabled flag, and its cues overlapping `range`.
fn captions_view(t: &CaptionTrack, range: (Tick, Tick)) -> CaptionsView<'_> {
    let cues = t.captions.iter().filter(|c| overlaps(c.start, c.duration, range)).collect();
    CaptionsView { track: t.id.0, language: &t.language, style: &t.style, enabled: t.enabled, cues }
}

/// Whether `[start, start + duration)` overlaps the half-open `range`.
fn overlaps(start: Tick, duration: Tick, (from, to): (Tick, Tick)) -> bool {
    start < to && Tick(start.0.saturating_add(duration.0)) > from
}

/// What the output digest covers.
#[derive(Serialize)]
struct OutputView<'a> {
    plan: PlanView<'a>,
    part: PartView<'a>,
    captions: Vec<CaptionsView<'a>>,
    sequence: SequenceView<'a>,
}

/// The plan's output settings.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanView<'a> {
    output: OutputSize,
    filter: Filter,
    captions: &'a VerticalCaptions,
    fit_background: FitBackground,
}

/// The part's delivery settings: its resolved fades and its metadata.
#[derive(Serialize)]
struct PartView<'a> {
    fades: PartAudio,
    meta: &'a PartMeta,
}

/// One caption track of the plan and its cues overlapping the part.
#[derive(Serialize)]
struct CaptionsView<'a> {
    track: u64,
    language: &'a str,
    style: &'a CaptionStyle,
    enabled: bool,
    cues: Vec<&'a Caption>,
}

/// A sequence as its rendered output sees it: settings, mix, and its tracks with items (or transitions) in range.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SequenceView<'a> {
    width: u32,
    height: u32,
    rate: FrameRate,
    color: &'a ColorPipeline,
    sample_rate: u32,
    master_volume_db: f64,
    master_effects: &'a [EffectInstance],
    master_mixer: &'a MixerStrip,
    submix_tracks: &'a [Track],
    tracks: Vec<TrackView<'a>>,
}

/// One video or audio track with items (or transitions) in range: its output and mixer settings and those items.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackView<'a> {
    kind: TrackKind,
    index: usize,
    enabled: bool,
    muted: bool,
    solo: bool,
    channels: AudioChannels,
    volume_db: f64,
    pan: f64,
    effects: &'a [EffectInstance],
    mixer: &'a MixerStrip,
    items: Vec<ItemView<'a>>,
    transitions: Vec<&'a Transition>,
}

/// One enabled track item in full, without its name, label, id, links, group and markers, and its media's identity.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemView<'a> {
    item: ItemId,
    start: Tick,
    duration: Tick,
    source_in: Tick,
    speed: f64,
    reverse: bool,
    effects: &'a [EffectInstance],
    gain_db: f64,
    frame_hold: Option<Tick>,
    scale_to_frame: bool,
    essential: Option<&'a EssentialSound>,
    multicam: Option<MulticamSel>,
    time_interpolation: TimeInterpolation,
    hold_filters: bool,
    field_options: Option<FieldOptions>,
    source_channels: &'a [u16],
    graphic: Option<&'a GraphicMeta>,
    source: SourceView<'a>,
}

/// What a track item shows.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum SourceView<'a> {
    /// A media file or generator: its identity.
    Media(MediaView<'a>),
    /// A subclip: its range and its parent's identity.
    Subclip { range: (Tick, Tick), parent: Box<SourceView<'a>> },
    /// A nested or multicam sequence, whole, to the nest depth: the digest of its view.
    Sequence(String),
    /// An adjustment layer or a graphic: its item kind.
    Other(&'a ItemKind),
    /// A missing item, or one past the nest depth.
    Missing,
}

/// The identity of a media item: what it is, its size, length, rate and frame size, and how it is interpreted.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaView<'a> {
    media: &'a MediaRef,
    file_size: Option<u64>,
    duration: Tick,
    rate: FrameRate,
    width: u32,
    height: u32,
    interpret: &'a Interpretation,
    offline: bool,
}

/// The view of `seq` over `range` (every item when None), at nesting `depth`.
fn sequence_view<'a>(project: &'a Project, seq: &'a Sequence, range: Option<(Tick, Tick)>, depth: u32, nests: &mut Nests) -> SequenceView<'a> {
    let shown = |start: Tick, duration: Tick| range.is_none_or(|r| overlaps(start, duration, r));
    let mut tracks = Vec::new();
    for (index, track) in seq.video_tracks.iter().chain(&seq.audio_tracks).enumerate() {
        let enabled = track.items.iter().filter(|i| i.enabled && shown(i.start, i.duration));
        let items: Vec<ItemView> = enabled.map(|i| item_view(project, i, depth, nests)).collect();
        let transitions: Vec<&Transition> = track.transitions.iter().filter(|t| shown(t.start, t.duration)).collect();
        if items.is_empty() && transitions.is_empty() {
            continue;
        }
        tracks.push(TrackView {
            kind: track.kind,
            index,
            enabled: track.enabled,
            muted: track.muted,
            solo: track.solo,
            channels: track.channels,
            volume_db: track.volume_db,
            pan: track.pan,
            effects: &track.effects,
            mixer: &track.mixer,
            items,
            transitions,
        });
    }
    let s = &seq.settings;
    SequenceView {
        width: s.width,
        height: s.height,
        rate: s.frame_rate,
        color: &s.color,
        sample_rate: s.sample_rate,
        master_volume_db: seq.master_volume_db,
        master_effects: &seq.master_effects,
        master_mixer: &seq.master_mixer,
        submix_tracks: &seq.submix_tracks,
        tracks,
    }
}

/// The view of one track item at nesting `depth`.
fn item_view<'a>(project: &'a Project, i: &'a TrackItem, depth: u32, nests: &mut Nests) -> ItemView<'a> {
    ItemView {
        item: i.item,
        start: i.start,
        duration: i.duration,
        source_in: i.source_in,
        speed: i.speed,
        reverse: i.reverse,
        effects: &i.effects,
        gain_db: i.gain_db,
        frame_hold: i.frame_hold,
        scale_to_frame: i.scale_to_frame,
        essential: i.essential.as_ref(),
        multicam: i.multicam,
        time_interpolation: i.time_interpolation,
        hold_filters: i.hold_filters,
        field_options: i.field_options,
        source_channels: &i.source_channels,
        graphic: i.graphic.as_deref(),
        source: source_view(project, i.item, depth, nests),
    }
}

/// The identity of what project item `id` shows, following subclips and nests to the nest depth.
fn source_view<'a>(project: &'a Project, id: ItemId, depth: u32, nests: &mut Nests) -> SourceView<'a> {
    let Some(item) = project.item(id) else { return SourceView::Missing };
    match &item.kind {
        ItemKind::Media(m) => {
            let (width, height) = m.info.video.as_ref().map_or((0, 0), |v| (v.width, v.height));
            let (file_size, duration, rate) = (m.info.file_size, m.info.duration, m.frame_rate());
            let (media, interpret, offline) = (&m.media, &m.interpret, m.offline);
            SourceView::Media(MediaView { media, file_size, duration, rate, width, height, interpret, offline })
        }
        ItemKind::Subclip { parent, range, .. } if depth < NEST_DEPTH => {
            let parent = Box::new(source_view(project, *parent, depth + 1, nests));
            SourceView::Subclip { range: (range.start, range.duration), parent }
        }
        ItemKind::Sequence(nested) if depth < NEST_DEPTH => {
            let key = (id, depth + 1);
            if let Some(digest) = nests.get(&key) {
                return SourceView::Sequence(digest.clone());
            }
            let digest = digest_of(&sequence_view(project, nested, None, depth + 1, nests));
            nests.insert(key, digest.clone());
            SourceView::Sequence(digest)
        }
        ItemKind::AdjustmentLayer { .. } | ItemKind::Graphic { .. } => SourceView::Other(&item.kind),
        _ => SourceView::Missing,
    }
}
