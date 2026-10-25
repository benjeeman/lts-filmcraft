//! The stored data of the Vertical Edit (spec section 2.4) and the value types of its pure functions.
//!
//! Every stored type derives `Clone, Debug, PartialEq, Serialize, Deserialize`, as [`Sequence`](crate::Sequence) does, and
//! has its final JSON shape: camelCase names, optional fields omitted while empty, ticks as integers.

use filmcraft_time::{FrameRate, Tick};
use serde::{Deserialize, Serialize};

use crate::{ItemId, Label};

/// The largest `|cx|`, `|cy|` and `h` a plan holds (I2): the policy's `MAX_COORDINATE`.
pub const MAX_COORDINATE: f64 = 65_536.0;

/// The largest frame count of the Vertical Edit's rules (I8): a split's `total` is at most this.
pub const VE_MAX_FRAMES: u32 = 1_000_000;

/// The Vertical Edit of one 16:9 sequence ([`Sequence::vertical`](crate::Sequence::vertical)).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerticalSet {
    /// The plans (0..=16), in creation order, with unique names.
    pub plans: Vec<VerticalPlan>,
    /// The shot boundaries, shared by every plan.
    pub walls: WallSet,
    /// Named ranges, one per short, sorted by `from` (at most 500).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    /// Protected regions (at most 2 000).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<ProtectedRegion>,
    /// Time annotations, sorted by `(from, id)` (at most 20 000).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Box tracks, sorted by id (at most 256 tracks and 50 000 keys in all).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub box_tracks: Vec<BoxTrack>,
    /// Active picture areas, sorted and non-overlapping (at most 2 000).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_area: Vec<ActiveArea>,
    /// Whether agents may overwrite reviewed framing; `None` (never set) reads as off. People only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_overwrite: Option<OverwriteSetting>,
    /// The edge fades of every part without its own.
    #[serde(default, skip_serializing_if = "PartAudio::is_default")]
    pub part_audio: PartAudio,
    /// The naming fields of the delivery package.
    #[serde(default, skip_serializing_if = "DeliveryInfo::is_empty")]
    pub delivery: DeliveryInfo,
    /// Fingerprints of the media that was framed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<ConformReference>,
    /// The delivery log as last saved (outside undo).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<DeliveryRecord>,
    /// Id counters: never reused, never part of a digest.
    pub next_plan_id: u32,
    pub next_part_id: u32,
    pub next_region_id: u32,
    #[serde(default)]
    pub next_annotation_id: u32,
}

/// The stored walls of a set. The effective walls add the picture edit points and the part boundaries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WallSet {
    /// Picture edit points are walls.
    pub use_edit_points: bool,
    /// Sorted by `at`, unique, inside `(0, end)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stored: Vec<StoredWall>,
    /// Frames that are never walls, whatever their source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressed: Vec<Tick>,
    /// Possible cuts a measurement rejected: never walls until promoted (at most 20 000).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    /// The ranges a finished measurement, or an agent's declaration, covered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysed: Vec<FrameRange>,
}

/// One stored wall: the first frame of a new shot.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredWall {
    pub at: Tick,
    pub origin: WallOrigin,
    /// The measurement's frame difference, for a measured wall.
    pub score: Option<f32>,
}

/// Where a stored wall came from. A moved wall takes its mover's origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WallOrigin {
    Detected,
    Agent,
    Manual,
    Marker,
    /// Stored by the edit hook or `vertical.remap` at a junction it creates.
    Reanchored,
}

/// A possible cut a measurement found and rejected.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub at: Tick,
    /// The measurement's frame difference.
    pub score: f32,
}

/// A frame-aligned, half-open range `[from, to)` with `from < to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRange {
    pub from: Tick,
    pub to: Tick,
}

/// A named range of the sequence that becomes one short. Defined by a person or an agent, never by the editor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub id: u32,
    /// 1..=64 characters, unique.
    pub name: String,
    pub from: Tick,
    pub to: Tick,
    /// Overrides [`VerticalSet::part_audio`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<PartAudio>,
    /// Written into the delivered file's metadata.
    #[serde(default, skip_serializing_if = "PartMeta::is_empty")]
    pub meta: PartMeta,
}

/// The audio fades at a part's edges, in frames (each 0..=250; 2 and 4 by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartAudio {
    pub fade_in_frames: u32,
    pub fade_out_frames: u32,
}

impl Default for PartAudio {
    fn default() -> Self {
        Self { fade_in_frames: 2, fade_out_frames: 4 }
    }
}

impl PartAudio {
    /// The default fades, which a file omits.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The metadata a part's delivered file carries (at most 200 and 5 000 characters).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartMeta {
    pub title: String,
    pub description: String,
}

impl PartMeta {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.description.is_empty()
    }
}

/// A box over a frame range that must stay in the window, be all in or all out, or stay out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectedRegion {
    pub id: u32,
    /// At most 40 characters ("channel logo", "name super").
    pub label: String,
    pub from: Tick,
    pub to: Tick,
    /// `[x, y, w, h]` in sequence pixels, on the grid, inside the frame.
    pub rect: [f64; 4],
    pub mode: RegionMode,
    pub origin: Origin,
    pub modified_by: Writer,
}

/// What a protected region asks of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegionMode {
    KeepIn,
    AllOrNothing,
    KeepOut,
}

/// A time interval with a kind (speech, laughter, music, black, ...), written by an agent or a person.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    pub id: u32,
    /// 1..=32 characters of `[a-z0-9_:-]`, an open set.
    pub kind: String,
    pub from: Tick,
    pub to: Tick,
    /// At most 64 characters: the speaking subject's box-track id for `speech`, a title for `music`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// On the 1/1000 grid; stored and shown, never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub origin: Origin,
    pub modified_by: Writer,
}

/// Boxes over time for one subject, supplied by an agent or drawn by a person.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxTrack {
    /// 1..=64 characters of `[A-Za-z0-9_:.-]`, unique ("char:dana", "face:3").
    pub id: String,
    /// At most 64 characters, shown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// At most 32 characters ("face", "body", "object"): shown, never interpreted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// Sorted by `at`, unique.
    pub keys: Vec<KeyBox>,
    pub origin: Origin,
    pub modified_by: Writer,
}

/// One key of a box track.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyBox {
    /// Frame-aligned.
    pub at: Tick,
    /// `[x, y, w, h]` in sequence pixels, on the grid, inside the frame, with a positive size.
    pub rect: [f64; 4],
    /// Which way the subject faces, for the look-room measure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing: Option<Facing>,
    /// The subject is absent from this key until the next key.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gone: bool,
}

/// The direction a subject faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Facing {
    Left,
    Right,
    Camera,
}

/// The part of the frame that holds picture (inside letterbox or pillarbox bars) over a frame range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveArea {
    pub from: Tick,
    pub to: Tick,
    /// In sequence pixels, on the grid, inside the frame, with a positive size.
    pub rect: [f64; 4],
    pub origin: Origin,
    pub modified_by: Writer,
}

/// "Agents may overwrite reviewed framing", as a person set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverwriteSetting {
    pub allowed: bool,
    /// The person's editor name.
    pub by: String,
    /// Unix seconds, UTC.
    pub when: i64,
    /// The sequence it was set on: a copy whose id differs reads it as off until a person sets it there.
    pub sequence: ItemId,
}

/// The naming fields of the delivery package (each at most 64 characters).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryInfo {
    pub series: String,
    pub season: String,
    pub episode: String,
}

impl DeliveryInfo {
    pub fn is_empty(&self) -> bool {
        self.series.is_empty() && self.season.is_empty() && self.episode.is_empty()
    }
}

/// Fingerprints of program frames taken on the media that was framed (the conform reference).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConformReference {
    /// Unix seconds, UTC.
    pub when: i64,
    /// One per framed media item.
    pub media: Vec<MediaSignature>,
    /// The fingerprinted program frames, sorted, unique, at most 64.
    pub frames: Vec<i64>,
    /// Per frame: 16x9 luma means (u8), base64 of 144 bytes. Never returned by a read.
    pub grids: Vec<String>,
    /// The walls `c` whose pair `(c - 1, c)` is among `frames`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wall_pairs: Vec<i64>,
}

/// The identity of one framed media item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSignature {
    pub item: ItemId,
    pub path: String,
    pub frames: i64,
    pub rate: FrameRate,
    pub start_timecode: i64,
    pub width: u32,
    pub height: u32,
}

/// What was delivered: one file of a vertical export, with its facts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryRecord {
    pub plan: u32,
    /// None for a manifest or a break list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<u32>,
    pub kind: DeliveryKind,
    pub profile: String,
    pub profile_digest: String,
    pub version: u32,
    /// The part's framing digest, when the file has a part; else "".
    pub framing_digest: String,
    /// The part's output digest, when the file has a part; else "".
    pub output_digest: String,
    /// The absolute path as written.
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<LoudnessRecord>,
    pub source: SourceRecord,
    /// The editor name, or "agent".
    pub by: String,
    /// Unix seconds, UTC.
    pub when: i64,
    /// The agent's run id, when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// The kind of a delivered file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryKind {
    Video,
    Sidecar,
    Cover,
    Manifest,
    BreakList,
}

/// Loudness as measured for a delivered part: every value as measured, never rounded.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessRecord {
    /// The measuring pass (faded audio), before the gain.
    pub measured_lufs: f64,
    pub gain_db: f64,
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    pub max_short_term_lufs: f64,
    pub lra_lu: f64,
    /// Pearson correlation of the whole part.
    pub correlation: f64,
    /// The minimum over 400 ms windows.
    pub min_correlation: f64,
}

/// The media a delivered file was rendered from, and whether it was conformed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    pub media: Vec<SourceMedia>,
    pub conform: ConformState,
}

/// One source file of a delivery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMedia {
    pub item: ItemId,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// The conform state of a delivery's source: `"match"`, `"unverified"` or `"none"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConformState {
    Match,
    Unverified,
    #[serde(rename = "none")]
    NoReference,
}

/// One output raster (for example 9:16 at 1080x1920) with its positions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerticalPlan {
    pub id: u32,
    /// 1..=120 characters, unique in the set ("9:16").
    pub name: String,
    /// Its ratio is the window's aspect.
    pub output: OutputSize,
    pub filter: Filter,
    /// The window while the plan has no position; None is the centred, largest window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_window: Option<Window>,
    #[serde(default)]
    pub captions: VerticalCaptions,
    /// The background of `fit` positions.
    #[serde(default)]
    pub fit_background: FitBackground,
    /// Sorted by `at`, unique frames, at most 10 000.
    pub positions: Vec<Position>,
    /// A person's approvals, each bound to two digests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub part_approvals: Vec<PartApproval>,
    /// A person's attention marks, sorted by `at`, unique frames, at most 2 000.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attention: Vec<AttentionMark>,
    /// Reviewed ranges an agent changed while the setting allowed it, sorted by `from` (at most 500).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overwritten: Vec<OverwrittenRange>,
    /// The 16:9 frame size `[width, height]` the windows were made on, while the sequence's frame has another aspect
    /// (spec section 4.8, step 11; set by the edit hook): the plan then has the error `frame_changed`, which blocks its
    /// vertical export, until a person repairs it (`vertical.repair`) or the frame returns to that size. None while the
    /// windows belong to the sequence's frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framed_on: Option<[u32; 2]>,
    pub next_position_id: u32,
    /// The id counter of overwritten ranges.
    #[serde(default)]
    pub next_record_id: u32,
}

impl VerticalPlan {
    /// An empty plan with the defaults: Catmull-Rom, the default captions, a blurred fit background, ids from 1.
    pub fn new(id: u32, name: &str, output: OutputSize) -> Self {
        Self {
            id,
            name: name.into(),
            output,
            filter: Filter::CatmullRom,
            starting_window: None,
            captions: VerticalCaptions::default(),
            fit_background: FitBackground::Blur,
            positions: Vec::new(),
            part_approvals: Vec::new(),
            attention: Vec::new(),
            overwritten: Vec::new(),
            framed_on: None,
            next_position_id: 1,
            next_record_id: 1,
        }
    }
}

/// A reviewed range an agent changed while the person allowed it, with the person's positions as reviewed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverwrittenRange {
    pub id: u32,
    /// Frame-aligned, half-open: the reviewed frames whose window or layout changed.
    pub from: Tick,
    pub to: Tick,
    /// Every reviewed position with a frame in `[from, to)`, plus the one governing `from` when it lies before `from`.
    pub positions: Vec<Position>,
    /// The agent step's undo label.
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Unix seconds, UTC.
    pub when: i64,
}

/// An output raster: both sides even and at least 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutputSize {
    pub width: u32,
    pub height: u32,
}

/// A person's approval of a part, bound to the part's two digests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartApproval {
    pub part: u32,
    /// The part's range digest at approval.
    pub framing_digest: String,
    pub output_digest: String,
    /// The person's editor name.
    pub by: String,
    /// Unix seconds, UTC.
    pub when: i64,
    /// At most 1 000 characters.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A person's flag on a frame: this shot needs work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionMark {
    pub at: Tick,
    pub by: String,
    pub when: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A window of the 16:9 frame: its centre and height in sequence pixels. Stored values lie on the 1/1000 px grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub cx: f64,
    pub cy: f64,
    pub h: f64,
}

impl Window {
    /// The window's rectangle in sequence pixels: `w = h * OW / OH`, `x = cx - w * 0.5`, `y = cy - h * 0.5`, in that order.
    pub fn rect(&self, out: OutputSize) -> Rect {
        let w = self.h * f64::from(out.width) / f64::from(out.height);
        Rect { x: self.cx - w * 0.5, y: self.cy - self.h * 0.5, w, h: self.h }
    }
    /// The upscale `OH / h`: output pixels per sequence pixel.
    pub fn upscale(&self, out: OutputSize) -> f64 {
        f64::from(out.height) / self.h
    }
}

/// A stored window at an exact frame, and how the window reaches it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub id: u32,
    /// Frame-aligned.
    pub at: Tick,
    pub window: Window,
    #[serde(default)]
    pub layout: Layout,
    pub arrival: Arrival,
    /// Where the subtitles go for cues that start while this position governs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_anchor: Option<CaptionAnchor>,
    /// At most 40 characters: the framing library's key ("Dana CU").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Label>,
    /// At most 16 tags of 1..=64 characters, never interpreted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// At most 1 000 characters: the reason.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// At most 8 box-track ids this framing is for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    /// The point to look at, in sequence pixels, on the grid, inside the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<[f64; 2]>,
    /// 0..=1 on the 1/1000 grid: the agent's, shown and sorted, never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// At most 200 characters: the agent's reason a person should look.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub risk: String,
    /// Who created it.
    pub origin: Origin,
    /// Who last changed it.
    pub modified_by: Writer,
    /// Only a person sets it.
    #[serde(default)]
    pub reviewed: bool,
    /// Only a person sets it; no command changes a locked position.
    #[serde(default)]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<Flag>,
    /// The agent's version, kept on a person's first change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed: Option<Proposal>,
}

impl Position {
    /// A `fill` position with no metadata, as a write creates it before setting the optional fields.
    pub fn new(id: u32, at: Tick, window: Window, arrival: Arrival, origin: Origin, modified_by: Writer) -> Self {
        Self {
            id,
            at,
            window,
            layout: Layout::Fill,
            arrival,
            caption_anchor: None,
            label: String::new(),
            color: None,
            tags: Vec::new(),
            note: String::new(),
            subjects: Vec::new(),
            focus: None,
            confidence: None,
            risk: String::new(),
            origin,
            modified_by,
            reviewed: false,
            locked: false,
            flags: Vec::new(),
            proposed: None,
        }
    }
}

/// The agent's version of a position, kept beside a person's change.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub at: Tick,
    pub window: Window,
    pub layout: Layout,
    pub arrival: Arrival,
}

/// How the window reaches a position: `{"kind":"cut"}` or `{"kind":"move","moveFrames":40,"curve":{...}}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Arrival {
    /// The window changes on the position's frame.
    Cut,
    /// The window moves over `move_frames` frames (at least 2) along `curve`; an exact split sets `split_of`.
    Move {
        move_frames: u32,
        curve: Curve,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        split_of: Option<SplitOf>,
    },
}

/// This move shows frames `offset ..= offset + move_frames` of an original `total`-frame move with the same curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SplitOf {
    pub offset: u32,
    pub total: u32,
}

/// A move's curve `E(u)`: progress over time, with `E(0) = 0` and `E(1) = 1`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Curve {
    /// Constant speed.
    Linear,
    /// Slow start: FilmCraft's keyframe curve (1/3, 0, 2/3, 2/3).
    EaseIn,
    /// Slow end: (1/3, 1/3, 2/3, 1).
    EaseOut,
    /// Slow start and end: (1/3, 0, 2/3, 1).
    EaseInOut,
    /// The cubic with these end speeds, in units of the move's mean speed (Smooth through).
    Speeds { start: f64, end: f64 },
    /// CSS `cubic-bezier(x1, y1, x2, y2)`.
    Bezier { x1: f64, y1: f64, x2: f64, y2: f64 },
    /// Accelerate over the share `in` of the move, cruise, decelerate over `out`: `{"type":"ramp","in":0.2,"out":0.2}`.
    Ramp {
        #[serde(rename = "in")]
        r_in: f64,
        #[serde(rename = "out")]
        r_out: f64,
    },
}

/// The resampling kernel of a plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Filter {
    #[default]
    CatmullRom,
    Bilinear,
}

/// `fill`: the window fills the output; `fit`: the whole 16:9 frame at the output width over a background.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    #[default]
    Fill,
    Fit,
}

/// Who created a position or a datum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Manual,
    Agent,
    Seeded,
    Reanchored,
}

/// Who wrote a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Writer {
    Human,
    Agent,
}

/// What the edit hook flags on a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Flag {
    PictureChanged,
    MoveShortened,
    TimeApproximate,
}

/// The background of `fit` positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FitBackground {
    #[default]
    Blur,
    Black,
}

/// The subtitle layout of a plan's output (spec section 6.4). Missing fields read as their defaults. The container-level
/// `serde(default)` is deliberate: the stored plan of spec section 2.9 writes `"captions": {"maxLines": 2}`, which must
/// read (T-M24 reads it), and the per-field defaults that section 6.4 lists (`track`, `edge`, `min_clipped_frames`) would
/// refuse it. It changes no printed field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VerticalCaptions {
    /// `[x, y, w, h]` in output pixels; None is the default area.
    pub area: Option<[u32; 4]>,
    /// The font size in output pixels; None is the track style's size scaled to the output.
    pub size: Option<f32>,
    /// The auto-shrink floor; None is 40 px scaled to the output.
    pub min_size: Option<f32>,
    /// 1..=4, 2 by default.
    pub max_lines: u8,
    /// The subtitler's line breaks carry meaning (true by default).
    pub keep_line_breaks: bool,
    pub anchor: CaptionAnchor,
    /// One caption track (its id); None is every enabled track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<u32>,
    /// Cues that straddle a part edge.
    #[serde(default)]
    pub edge: CueEdge,
    /// A clipped remainder shorter than this is dropped; 0 keeps all.
    #[serde(default)]
    pub min_clipped_frames: u32,
}

impl Default for VerticalCaptions {
    fn default() -> Self {
        Self {
            area: None,
            size: None,
            min_size: None,
            max_lines: 2,
            keep_line_breaks: true,
            anchor: CaptionAnchor::Bottom,
            track: None,
            edge: CueEdge::Clip,
            min_clipped_frames: 0,
        }
    }
}

/// Where the subtitles sit inside their area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptionAnchor {
    Top,
    #[default]
    Bottom,
}

/// What a part export does with a cue that straddles the part's edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CueEdge {
    #[default]
    Clip,
    Drop,
}

/// The 16:9 frame a plan is evaluated on: the sequence's frame size and rate, and its end frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    pub end_frame: i64,
}

/// A rectangle in sequence pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The window heights a plan allows on a raster, rounded inward onto the grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub min_height: f64,
    pub max_height: f64,
}

/// What the evaluator returns: the window shown at a tick and how it was reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub window: Window,
    pub layout: Layout,
    pub phase: Phase,
    /// The governing position; inside a move, the one it departs from. None while the plan has no position.
    pub position: Option<u32>,
    /// Inside a move, the position it arrives at.
    pub next: Option<u32>,
    /// Inside a move, its progress: the share of its frames done (0 elsewhere).
    pub u: f64,
    /// Inside a move, the eased progress given to the path (0 elsewhere).
    pub e: f64,
}

/// Where a tick lies in a plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// The plan has no position: the starting window.
    #[default]
    Starting,
    /// Before the first position: its window.
    Before,
    /// A position's window holds.
    Hold,
    /// Inside a move.
    Move,
}

/// The planner's view of who writes. `overwrite_allowed` is true only when the setting is on for this sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    pub writer: Writer,
    pub overwrite_allowed: bool,
}

/// A refused call: nothing changed. `detail` carries the numbers needed to fix the call.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
    pub detail: serde_json::Value,
}

impl Refusal {
    pub fn new(code: &'static str, message: impl Into<String>, detail: serde_json::Value) -> Self {
        Self { code, message: message.into(), detail }
    }
}

/// The options of Smooth through: whether the run starts and ends at rest (both true by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmoothOpts {
    pub start_from_rest: bool,
    pub end_at_rest: bool,
}

impl Default for SmoothOpts {
    fn default() -> Self {
        Self { start_from_rest: true, end_at_rest: true }
    }
}

/// How serious an issue is (spec section 7.6): an error blocks vertical export; a warning or an info is reported only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Error,
    Warning,
    Info,
}

/// A fact a read or a write result reports about a plan or the set (spec section 7.6), with the frames it concerns.
/// Threshold findings are issues too; they exist only for the thresholds a caller passes.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    /// One of the closed list of spec section 7.6.
    pub code: &'static str,
    pub level: Level,
    /// The plan it concerns; None for the set's own data (walls, parts, regions, annotations, areas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<u32>,
    /// The frames it concerns, half-open `[from, to)`; None for an issue of the whole plan or set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<[i64; 2]>,
    /// The numbers behind it (ids, frames, runs, scores, the threshold compared).
    pub detail: serde_json::Value,
}

/// One change a write made besides the ones it names: a move shortened at a new wall (W5), positions carried with a moved
/// wall (W8), a repair step. A write result lists them in order; nothing in it is ever a choice of the editor's.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Change {
    pub code: &'static str,
    pub detail: serde_json::Value,
}
