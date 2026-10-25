//! Fixtures of the M2 model tests (lane L1b): a synthetic 25p 1920x1080 project whose first video track holds clips of one
//! synthetic matte, a vertical set installed into its sequence, and the planner called as a person. The fixtures compute
//! frames and ticks themselves (at 25p) and never lean on the code under test. Synthetic data only.

use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_time::{FrameRate, Tick, TimeRange};

use super::support::{empty_set, plan_of};
use crate::vertical::*;
use crate::{
    Caption, CaptionFormat, CaptionTrack, ClipId, ItemId, ItemKind, Label, MediaClip, MediaRef, Project, Sequence, SequenceSettings, TrackId, TrackItem,
    TrackKind,
};

/// The fixtures' rate.
pub const RATE: FrameRate = FrameRate::FPS_25;

/// The start tick of `frame` at 25p.
pub fn tk(frame: i64) -> Tick {
    RATE.tick_of(frame)
}

/// A project with one 1920x1080 25p sequence (two video tracks, one audio track) and one long synthetic matte.
pub struct Fx {
    pub project: Project,
    pub seq: ItemId,
    pub media: ItemId,
}

impl Fx {
    /// A sequence whose first video track holds one clip of `frames` frames from source frame 0: no edit wall.
    pub fn new(frames: i64) -> Self {
        Self::with_clips(&[(0, frames, 0)])
    }

    /// A sequence whose first video track holds the clips `(start, frames, source_in)`, in frames, of the matte.
    pub fn with_clips(clips: &[(i64, i64, i64)]) -> Self {
        let mut project = Project::new("Vertical Edit M2 tests");
        let generator = Generator::ColorMatte { color: [0.2, 0.3, 0.4, 1.0] };
        let source = GeneratorSource::new(generator.clone(), 1920, 1080, RATE, tk(400_000));
        let clip = MediaClip {
            media: MediaRef::Generator(generator),
            info: source.info().clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: Vec::new(),
            offline: false,
            proxy: None,
            identity: None,
        };
        let media = project.add_item("matte", Label::Iris, ItemKind::Media(clip), None);
        let settings = SequenceSettings { frame_rate: RATE, ..SequenceSettings::default() };
        let seq = project.new_sequence("Episode 12", settings, 2, 1, None);
        let mut fx = Self { project, seq, media };
        for &(start, frames, source_in) in clips {
            fx.add_clip(0, start, frames, source_in);
        }
        fx
    }

    /// A clip of the matte on video track `track`, starting at frame `start`, `frames` long, from source frame `source_in`.
    pub fn add_clip(&mut self, track: usize, start: i64, frames: i64, source_in: i64) -> ClipId {
        self.add_clip_at(track, tk(start), frames, source_in)
    }

    /// A clip of the matte starting at tick `start` (anywhere, on the frame grid or not).
    pub fn add_clip_at(&mut self, track: usize, start: Tick, frames: i64, source_in: i64) -> ClipId {
        let item = self.make_item(self.media, start, frames, source_in);
        self.put(track, item)
    }

    /// A track item of project item `item` (not placed).
    pub fn make_item(&mut self, item: ItemId, start: Tick, frames: i64, source_in: i64) -> TrackItem {
        let range = TimeRange::new(tk(source_in), tk(frames));
        self.project.make_track_item(item, TrackKind::Video, start, range, RATE).expect("the project item exists")
    }

    /// Places `item` on video track `track` (kept sorted) and returns its id.
    pub fn put(&mut self, track: usize, item: TrackItem) -> ClipId {
        let id = item.id;
        let track = self.seq_mut().video_tracks.get_mut(track).expect("the video track exists");
        track.items.push(item);
        track.sort();
        id
    }

    /// The video track item `id`.
    pub fn item_mut(&mut self, id: ClipId) -> &mut TrackItem {
        self.seq_mut().video_tracks.iter_mut().find_map(|t| t.item_mut(id)).expect("the track item exists")
    }

    pub fn seq(&self) -> &Sequence {
        self.project.sequence(self.seq).expect("the sequence exists")
    }

    pub fn seq_mut(&mut self) -> &mut Sequence {
        let id = self.seq;
        self.project.sequence_mut(id).expect("the sequence exists")
    }

    /// The sequence's raster, computed here: its settings and the frames its content covers.
    pub fn r(&self) -> Raster {
        let seq = self.seq();
        let end = seq.duration();
        let end_frame = if end.0 > 0 { RATE.frame_at(Tick(end.0 - 1)) + 1 } else { 0 };
        Raster { width: seq.settings.width, height: seq.settings.height, rate: seq.settings.frame_rate, end_frame }
    }

    /// Installs `set` as the sequence's vertical data.
    pub fn install(&mut self, set: VerticalSet) {
        self.seq_mut().vertical = Some(Box::new(set));
    }

    pub fn set(&self) -> &VerticalSet {
        self.seq().vertical.as_deref().expect("the sequence has a vertical set")
    }

    pub fn set_mut(&mut self) -> &mut VerticalSet {
        self.seq_mut().vertical.as_deref_mut().expect("the sequence has a vertical set")
    }

    pub fn plan(&self, id: u32) -> &VerticalPlan {
        self.set().plans.iter().find(|p| p.id == id).expect("the plan exists")
    }

    pub fn plan_mut(&mut self, id: u32) -> &mut VerticalPlan {
        self.set_mut().plans.iter_mut().find(|p| p.id == id).expect("the plan exists")
    }

    /// The position `id` of plan `plan`.
    pub fn position(&self, plan: u32, id: u32) -> &Position {
        self.plan(plan).positions.iter().find(|p| p.id == id).expect("the position exists")
    }

    /// `w` planned as a person.
    pub fn write(&self, w: Write) -> Result<Planned, Refusal> {
        plan_write(&self.project, self.seq(), &w, PERSON)
    }

    /// `w` planned as a person, asserted accepted, and applied to the sequence (as the engine applies it).
    pub fn apply(&mut self, w: Write) -> Planned {
        let planned = accepted(self.write(w));
        self.seq_mut().vertical = planned.set.clone();
        planned
    }

    /// `w` planned as a person and asserted refused; nothing is applied.
    pub fn refuse(&self, w: Write) -> Refusal {
        refused(self.write(w))
    }

    /// The effective walls' frames.
    pub fn wall_frames(&self) -> Vec<i64> {
        walls(&self.project, self.seq()).iter().map(|w| RATE.frame_at(w.at)).collect()
    }

    /// The frames of plan `plan`'s positions, in stored order.
    pub fn frames(&self, plan: u32) -> Vec<i64> {
        self.plan(plan).positions.iter().map(|p| RATE.frame_at(p.at)).collect()
    }
}

/// The accepted plan of a write, or a failure naming the refusal.
pub fn accepted(r: Result<Planned, Refusal>) -> Planned {
    match r {
        Ok(planned) => planned,
        Err(e) => panic!("refused {}: {} {}", e.code, e.message, e.detail),
    }
}

/// The refusal of a write, or a failure naming the label it was accepted with.
pub fn refused(r: Result<Planned, Refusal>) -> Refusal {
    match r {
        Ok(planned) => panic!("accepted ({}), changed {}", planned.label, planned.changed),
        Err(e) => e,
    }
}

/// A set with one 9:16 plan (id 1) holding `positions`; edit points on, nothing else.
pub fn one_plan(positions: Vec<Position>) -> VerticalSet {
    let mut set = empty_set();
    set.plans = vec![plan_of(positions)];
    set.next_plan_id = 2;
    set
}

/// A position of a person at `frame` (25p), reached by `arrival`.
pub fn at25(id: u32, frame: i64, window: Window, arrival: Arrival) -> Position {
    Position::new(id, tk(frame), window, arrival, Origin::Manual, Writer::Human)
}

/// The keys of a full window.
pub fn keys(cx: f64, cy: f64, h: f64) -> PositionFields {
    PositionFields { cx: Some(cx), cy: Some(cy), h: Some(h), ..Default::default() }
}

/// `vertical.setPosition` on plan 1 at `frame` with `fields`.
pub fn set_at(frame: i64, fields: PositionFields) -> Write {
    Write::SetPosition(SetPosition { plan: 1, frame, fields, ..Default::default() })
}

/// The arrival keys of a move of `frames` frames, with `curve` when given.
pub fn move_keys(frames: i64, curve: Option<Curve>) -> PositionFields {
    PositionFields { arrival: Some(ArrivalKind::Move), move_frames: Some(frames), curve, ..Default::default() }
}

/// A stored wall at `frame`.
pub fn stored(frame: i64, origin: WallOrigin, score: Option<f32>) -> StoredWall {
    StoredWall { at: tk(frame), origin, score }
}

/// The effective wall at `frame`, if any.
pub fn wall_at(fx: &Fx, frame: i64) -> Option<Wall> {
    walls(&fx.project, fx.seq()).into_iter().find(|w| w.at == tk(frame))
}

/// The codes of `issues`.
pub fn codes(issues: &[Issue]) -> Vec<&'static str> {
    issues.iter().map(|i| i.code).collect()
}

/// The issues of `issues` with `code`.
pub fn with_code<'a>(issues: &'a [Issue], code: &str) -> Vec<&'a Issue> {
    issues.iter().filter(|i| i.code == code).collect()
}

/// A part over frames `[from, to)`.
pub fn part25(id: u32, name: &str, from: i64, to: i64) -> Part {
    Part { id, name: name.into(), from: tk(from), to: tk(to), audio: None, meta: PartMeta::default() }
}

/// Adds an enabled caption track (id 9000) holding cues over the frame ranges given, with cue ids 1, 2, ...
pub fn add_cues(fx: &mut Fx, cues: &[(i64, i64)]) {
    let mut track = CaptionTrack::new(TrackId(9_000), "Subtitles".into(), CaptionFormat::Subtitle);
    for (n, &(from, to)) in cues.iter().enumerate() {
        track.captions.push(Caption {
            id: ClipId(n as u64 + 1),
            start: tk(from),
            duration: tk(to - from),
            text: format!("cue {}", n + 1),
            speaker: None,
            cue_id: None,
            settings: String::new(),
        });
    }
    fx.seq_mut().caption_tracks.push(track);
}
