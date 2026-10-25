//! Vertical Edit: the engine commands and reads the desktop UI uses (spec §7.1–§7.6; plan 63 §3.2, the closed list of
//! M2). Every command reads its parameters strictly: unknown keys (`runId` too), `seconds`, times off a frame start and
//! hostile numbers are refused with the codes of spec §7.6, and the `detail` says how to fix the call.
//!
//! Writes plan with the model's one planner (`filmcraft_project::vertical::plan_write`) on the session's working copy of
//! the sequence, whose id counters are raised to the session's high-water marks (plan 63 C-11), as the session's writer
//! (`Caller {writer, overwrite_allowed: false}`: a person in M2). A dry run returns the real result and records nothing;
//! a write that changes nothing records nothing; a change is one plan-only undo step through [`Session::edit_plan_only`]
//! (no edit hook, no picture-revision change), and a `merge` key folds the sub-steps of a gesture into one step. Every
//! write returns the result envelope of spec §7.2.
//!
//! Reads and editor-state commands are not journaled and change no project data. A plan that breaks an invariant is
//! never evaluated (spec §2.8): its reads answer `invalid_plan {violations}`. The session validates every vertical set
//! when a project is opened or installed and after undo and redo ([`validate_now`]), and keeps the answer for that
//! project snapshot.
//!
//! The session keeps, outside undo ([`VerticalStore`]): what playback showed of each shot (spec §5.10), the positions
//! made while shots were unknown and the ones an applied measurement left off a wall (spec §4.4), and the shot-change
//! measurements (`crate::vertical_measure`). The editor decides nothing here: reads report numbers, findings appear only
//! for the thresholds a caller passes, and walls come only from a write.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use filmcraft_project::vertical::{self as v, Caller, Level, Limits, Raster, VerticalPlan, Violation};
use filmcraft_project::{ItemId, Label, MarkerKind, Project, Sequence};
use filmcraft_time::{FrameRate, Tick, TimeRange, format_timecode_frames, parse_timecode};
use serde_json::{Map, Value, json};

use crate::commands::{CommandSpec, always};
use crate::vertical_measure::{self as measure, Measurements};
use crate::{EngineError, Result, Session, VerticalSelection};

type Run = fn(&mut Session, &Value) -> Result<Value>;

const CREATE: &str =
    r#"{"sequence":id?,"name":str?,"preset":"9:16"|"4:5"|"1:1"?,"width":px?,"height":px?,"filter":"catmullRom"|"bilinear"?,"dryRun":bool=false}"#;
const UPDATE: &str = r#"{"sequence":id?,"plan":id,"name":str?,"width":px?,"height":px?,"filter":"catmullRom"|"bilinear"?,"startingWindow":{"cx","cy","h"}|null?,"captions":{"area":[x,y,w,h]|null,"size":px|null,"minSize":px|null,"maxLines":1..4,"keepLineBreaks":bool,"anchor":"top"|"bottom","track":id|null,"edge":"clip"|"drop","minClippedFrames":n}?,"fitBackground":"blur"|"black"?,"clamp":bool=false,"expect":digest?,"dryRun":bool=false}"#;
const DELETE: &str = r#"{"sequence":id?,"plan":id,"expect":digest?,"dryRun":bool=false}"#;
const SET_POSITION: &str = r#"{"sequence":id?,"plan":id,"frame":n|"time":ticks|"timecode":"01:00:12:03"|"P07 00:00:12:03","cx":px?,"cy":px?,"h":px?|"zoom":pct?,"fromLabel":str?|"fromPosition":id?,"layout":"fill"|"fit"?,"arrival":"cut"|"move"?,"moveFrames":n?,"curve":"linear"|"easeIn"|"easeOut"|"easeInOut"|"speeds"|"bezier"|"ramp"?,"speeds":[a,b]?,"bezier":[x1,y1,x2,y2]?,"ramp":[in,out]?,"captionAnchor":"top"|"bottom"|null?,"label":str?,"color":label|null?,"tags":[str]|null?,"note":str?,"focus":[x,y]|null?,"clamp":bool=false,"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"dryRun":bool=false,"merge":key?}"#;
const RETIME_POSITION: &str = r#"{"sequence":id?,"plan":id,"position":id,"frame":n|"time":ticks|"timecode":str,"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"dryRun":bool=false}"#;
const DELETE_POSITIONS: &str =
    r#"{"sequence":id?,"plan":id,"positions":[id],"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"dryRun":bool=false}"#;
const REPLACE_RANGE: &str = r#"{"sequence":id?,"plan":id,"fromFrame":n,"toFrame":n,"positions":[{"frame":n|"time":ticks,"cx","cy","h"|"zoom","layout"?,"arrival"?,"moveFrames"?,"curve"?,"speeds"?,"bezier"?,"ramp"?,"captionAnchor"?,"label"?,"color"?,"tags"?,"note"?,"focus"?}],"clamp":bool=false,"strict":bool=false,"echo":bool=false,"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"expectWalls":digest?,"dryRun":bool=false}"#;
const SET_ARRIVAL: &str = r#"{"sequence":id?,"plan":id,"positions":[id],"arrival":"cut"|"move","moveFrames":n?,"curve":name?,"speeds":[a,b]?,"bezier":[x1,y1,x2,y2]?,"ramp":[in,out]?,"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"dryRun":bool=false}"#;
const SMOOTH_THROUGH: &str = r#"{"sequence":id?,"plan":id,"positions":[id],"startFromRest":bool=true,"endAtRest":bool=true,"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"dryRun":bool=false}"#;
const FIT_MOVE: &str = r#"{"sequence":id?,"plan":id,"position":id,"maxSpeed":px_per_frame,"maxAccel":px_per_frame2?,"curve":name?,"speeds":[a,b]?,"bezier":[x1,y1,x2,y2]?,"ramp":[in,out]?,"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"dryRun":bool=false}"#;
const SWAP_WINDOWS: &str =
    r#"{"sequence":id?,"plan":id,"positions":[a,b],"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"dryRun":bool=false}"#;
const SET_WALLS: &str = r#"{"sequence":id?,"add":[frame|{"frame","score"?}]?,"remove":[frame]?,"move":[{"from","to"}]?,"promote":[frame]?,"suppress":[frame]?,"unsuppress":[frame]?,"candidates":{"fromFrame","toFrame","set":[{"frame","score"}]}?,"analysed":[[from,to]]?,"useEditPoints":bool?,"fromMarkers":{"name"?,"color"?,"kind"?}?,"onConflict":"refuse"|"shorten"="refuse","expect":wallsDigest?,"expectWalls":effectiveWallsDigest?,"dryRun":bool=false}"#;
const APPLY_MEASUREMENT: &str = r#"{"sequence":id?,"measurement":"m1","fromFrame":n?,"toFrame":n?,"onConflict":"refuse"|"shorten"="refuse","expectWalls":effectiveWallsDigest?,"dryRun":bool=false}"#;
const SEED_SHOTS: &str = r#"{"sequence":id?,"plan":id,"fromFrame":n?,"toFrame":n?,"mode":"carry"|"starting","expect":digest?,"dryRun":bool=false}"#;
const SET_PARTS: &str = r#"{"sequence":id?,"add":[{"name","fromFrame","toFrame","audio":{"fadeInFrames","fadeOutFrames"}?,"meta":{"title","description"}?}]?,"update":[{"part":id,"name"?,"fromFrame"?,"toFrame"?,"audio"?|null,"meta"?}]?,"remove":[id]?,"fromMarkers":{"kind":"segmentation","color"?}?,"moveEdges":[{"frame","to"}]?,"onConflict":"refuse"|"shorten"="refuse","expect":partsDigest?,"dryRun":bool=false}"#;
const SET_REGIONS: &str = r#"{"sequence":id?,"add":[{"label","fromFrame","toFrame","rect":[x,y,w,h],"mode":"keepIn"|"allOrNothing"|"keepOut"}]?,"update":[{"region":id,"label"?,"fromFrame"?,"toFrame"?,"rect"?,"mode"?}]?,"remove":[id]?,"expect":regionsDigest?,"dryRun":bool=false}"#;
const MARK_CHECKED: &str =
    r#"{"sequence":id?,"plan":id,"positions":[id]?,"expect":digest?,"expectRange":{"fromFrame","toFrame","digest"}?,"dryRun":bool=false}"#;
const REPAIR: &str = r#"{"sequence":id?,"plan":id,"dryRun":bool=false}"#;
const SET_ANNOTATIONS: &str = r#"{"sequence":id?,"add":[{"kind","fromFrame","toFrame","label"?,"score"?}]?,"update":[{"id","kind"?,"fromFrame"?,"toFrame"?,"label"?,"score"?|null}]?,"remove":[id]?,"replace":{"kind","fromFrame","toFrame","set":[{"fromFrame","toFrame","label"?,"score"?}]}?,"expect":dataDigest?,"dryRun":bool=false}"#;
const SET_ACTIVE_AREA: &str =
    r#"{"sequence":id?,"set":[{"fromFrame","toFrame","rect":[x,y,w,h]}]?,"clear":[[from,to]]?,"expect":dataDigest?,"dryRun":bool=false}"#;
const APPROVE_PART: &str = r#"{"sequence":id?,"plan":id,"part":id,"note":str?,"dryRun":bool=false}"#;
const SET_REVIEW: &str = r#"{"sequence":id?,"plan":id,"positions":[id]?,"shotFrame":n?,"reviewed":bool,"dryRun":bool=false}"#;
const SET_LOCK: &str = r#"{"sequence":id?,"plan":id,"positions":[id],"locked":bool,"dryRun":bool=false}"#;
const FLAG_ATTENTION: &str = r#"{"sequence":id?,"plan":id,"frame":n|"time":ticks|"timecode":str,"note":str?,"dryRun":bool=false}"#;
const CLEAR_ATTENTION: &str = r#"{"sequence":id?,"plan":id,"frames":[n],"dryRun":bool=false}"#;
const REMAP: &str = r#"{"sequence":id?,"map":[{"fromFrame","toFrame","newFrom"}],"expect":remapDigest,"dryRun":bool=false}"#;
const SELECT: &str = r#"{"sequence":id?,"plan":id?,"positions":[id]?,"walls":[frame]?,"region":id?} (omitted keys select nothing)"#;
const GO_TO: &str = r#"{"sequence":id?,"plan":id?,"to":"prevPosition"|"nextPosition"|"prevWall"|"nextWall"|"prevBreak"|"nextBreak"|"nextAttention","frame":n?|"time":ticks?|"timecode":str?}"#;
const NOTE_WATCHED: &str = r#"{"sequence":id?,"plan":id,"shot":n,"frames":[[from,to]|frame],"pass":n?} (pass: the UI's unbroken playback pass)"#;
const LIST: &str = r#"{"sequence":id?}"#;
const INSPECT: &str = r#"{"sequence":id?,"plan":id,"fromFrame":n?,"toFrame":n?}"#;
const WINDOWS: &str = r#"{"sequence":id?,"plan":id,"fromFrame":n,"toFrame":n,"step":n=1,"fields":["tick"|"x"|"y"|"w"|"h"|"cx"|"cy"|"layout"|"state"|"position"|"u"|"e"|"speed"|"upscale"|"focus"|"active"]?}"#;
const MEASURE: &str = r#"{"sequence":id?,"plan":id,"fromFrame":n?,"toFrame":n?,"thresholds":{"maxUpscale"?,"maxSpeed"?,"maxAccel"?,"minHoldFrames"?,"pingPongFrames"?,"nearWallFrames"?,"maxSpeedStep"?,"jumpSimilar":{"maxShift","maxScale"}?,"minMoveDistance"?,"minHoldAfterWall"?,"maxHoldFrames"?,"straddleFrames"?,"maxEyeTraceJump"?}?,"regions":[{"id","label"?,"space":"sequence"?,"boxes":[{"frame","rect":[x,y,w,h]}],"interpolate"?}|{"source":"protected"|"captions"}]?,"safeZone":"union"|"unionMirrored"|[x,y,w,h]?,"baseline":"centreCrop"?,"series":bool=false}"#;
const FIT: &str = r#"{"sequence":id?,"plan":id,"frame":n,"regions":[{"space":"sequence"?,"rect":[x,y,w,h]}|{"source":"protected"}]?,"h":px?|"zoom":pct?,"margin":px?,"eyeTrace":{"fromFrame","fromPoint":[x,y]?,"toPoint":[x,y]?}?}"#;
const STORYBOARD: &str = r#"{"sequence":id?,"plan":id,"fromFrame":n?,"toFrame":n?,"part":id?}"#;
const MEASURE_CUTS: &str =
    r#"{"sequence":id?,"fromFrame":n?,"toFrame":n?,"sensitivity":0..100=50,"minShotFrames":n=6,"candidateFloor":0..1?,"scores":bool=false,"wait":bool=false}"#;
const JOB_RESULT: &str = r#"{"job":id}"#;

/// A write: journaled, one undo step when it changes something.
fn write(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: true }
}

/// A read or an editor-state command: not journaled, never an undo step.
fn unjournaled(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: false }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        write("vertical.create", "New Vertical Plan", CREATE, create),
        write("vertical.update", "Vertical Plan Settings", UPDATE, update),
        write("vertical.delete", "Delete Vertical Plan", DELETE, delete),
        write("vertical.setPosition", "Set Vertical Position", SET_POSITION, set_position),
        write("vertical.retimePosition", "Retime Vertical Position", RETIME_POSITION, retime_position),
        write("vertical.deletePositions", "Delete Vertical Positions", DELETE_POSITIONS, delete_positions),
        write("vertical.replaceRange", "Replace Vertical Positions in Range", REPLACE_RANGE, replace_range),
        write("vertical.setArrival", "Set Vertical Arrival", SET_ARRIVAL, set_arrival),
        write("vertical.smoothThrough", "Smooth Through", SMOOTH_THROUGH, smooth_through),
        write("vertical.fitMove", "Fit Move to Speed", FIT_MOVE, fit_move),
        write("vertical.swapWindows", "Swap Vertical Windows", SWAP_WINDOWS, swap_windows),
        write("vertical.setWalls", "Edit Shot Walls", SET_WALLS, set_walls),
        write("vertical.applyMeasurement", "Apply Measured Shot Changes", APPLY_MEASUREMENT, apply_measurement),
        write("vertical.seedShots", "Seed Shot Openers", SEED_SHOTS, seed_shots),
        write("vertical.setParts", "Edit Vertical Parts", SET_PARTS, set_parts),
        write("vertical.setRegions", "Edit Protected Regions", SET_REGIONS, set_regions),
        write("vertical.markChecked", "Mark Vertical Positions Checked", MARK_CHECKED, mark_checked),
        write("vertical.repair", "Repair Vertical Plan", REPAIR, repair),
        write("vertical.setAnnotations", "Edit Time Annotations", SET_ANNOTATIONS, set_annotations),
        write("vertical.setActiveArea", "Set Active Picture Area", SET_ACTIVE_AREA, set_active_area),
        write("vertical.approvePart", "Approve Vertical Part", APPROVE_PART, approve_part),
        write("vertical.setReview", "Set Vertical Review", SET_REVIEW, set_review),
        write("vertical.setLock", "Lock Vertical Positions", SET_LOCK, set_lock),
        write("vertical.flagAttention", "Flag Shot for Attention", FLAG_ATTENTION, flag_attention),
        write("vertical.clearAttention", "Clear Attention Flags", CLEAR_ATTENTION, clear_attention),
        write("vertical.remap", "Remap Vertical Framing", REMAP, remap),
        unjournaled("vertical.select", "Select in Vertical Edit", SELECT, select),
        unjournaled("vertical.goTo", "Go To in Vertical Edit", GO_TO, go_to),
        unjournaled("vertical.noteWatched", "Note Watched Frames", NOTE_WATCHED, note_watched),
        unjournaled("vertical.list", "List Vertical Plans", LIST, list),
        unjournaled("vertical.inspect", "Inspect Vertical Plan", INSPECT, inspect),
        unjournaled("vertical.windows", "Vertical Windows per Frame", WINDOWS, windows),
        unjournaled("vertical.measure", "Measure Vertical Plan", MEASURE, measure_plan),
        unjournaled("vertical.fit", "Fit Vertical Window", FIT, fit),
        unjournaled("vertical.storyboard", "Vertical Storyboard", STORYBOARD, storyboard),
        unjournaled("vertical.measureCuts", "Measure Shot Changes", MEASURE_CUTS, measure::measure_cuts),
        unjournaled("vertical.jobResult", "Vertical Job Result", JOB_RESULT, measure::job_result),
    ]
}

// ---------- the keys ----------

/// The keys of every command that names a plan.
const PLAN: &[&str] = &["sequence", "plan"];
/// A frame by number, by time or by timecode.
const TIME: &[&str] = &["frame", "time", "timecode"];
/// A window.
const WINDOW: &[&str] = &["cx", "cy", "h", "zoom"];
/// How the window arrives.
const ARRIVAL: &[&str] = &["arrival", "moveFrames", "curve", "speeds", "bezier", "ramp"];
/// A position's layout and metadata.
const METADATA: &[&str] = &["layout", "captionAnchor", "label", "color", "tags", "note", "focus"];
/// The guards of a write that names positions, and its dry run.
const GUARDED: &[&str] = &["expect", "expectRange", "dryRun"];
/// The guard of a write, and its dry run.
const CHECKED: &[&str] = &["expect", "dryRun"];
/// The keys of one position of `vertical.replaceRange`.
const ITEM: &[&[&str]] = &[&["frame", "time"], WINDOW, ARRIVAL, METADATA];

const LAYOUTS: &[(&str, v::Layout)] = &[("fill", v::Layout::Fill), ("fit", v::Layout::Fit)];
const ARRIVALS: &[(&str, v::ArrivalKind)] = &[("cut", v::ArrivalKind::Cut), ("move", v::ArrivalKind::Move)];
const ANCHORS: &[(&str, v::CaptionAnchor)] = &[("top", v::CaptionAnchor::Top), ("bottom", v::CaptionAnchor::Bottom)];
const FILTERS: &[(&str, v::Filter)] = &[("catmullRom", v::Filter::CatmullRom), ("bilinear", v::Filter::Bilinear)];
const BACKGROUNDS: &[(&str, v::FitBackground)] = &[("blur", v::FitBackground::Blur), ("black", v::FitBackground::Black)];
const PRESETS: &[(&str, v::Preset)] = &[("9:16", v::Preset::NineSixteen), ("4:5", v::Preset::FourFive), ("1:1", v::Preset::OneOne)];
const CONFLICTS: &[(&str, v::OnConflict)] = &[("refuse", v::OnConflict::Refuse), ("shorten", v::OnConflict::Shorten)];
const SEED_MODES: &[(&str, v::SeedMode)] = &[("carry", v::SeedMode::Carry), ("starting", v::SeedMode::Starting)];
const EDGES: &[(&str, v::CueEdge)] = &[("clip", v::CueEdge::Clip), ("drop", v::CueEdge::Drop)];
const REGION_MODES: &[(&str, v::RegionMode)] =
    &[("keepIn", v::RegionMode::KeepIn), ("allOrNothing", v::RegionMode::AllOrNothing), ("keepOut", v::RegionMode::KeepOut)];
const MARKER_KINDS: &[(&str, MarkerKind)] = &[
    ("comment", MarkerKind::Comment),
    ("chapter", MarkerKind::Chapter),
    ("segmentation", MarkerKind::Segmentation),
    ("webLink", MarkerKind::WebLink),
    ("flashCue", MarkerKind::FlashCue),
];
const TARGETS: &[(&str, v::GoTo)] = &[
    ("prevPosition", v::GoTo::PrevPosition),
    ("nextPosition", v::GoTo::NextPosition),
    ("prevWall", v::GoTo::PrevWall),
    ("nextWall", v::GoTo::NextWall),
    ("prevBreak", v::GoTo::PrevBreak),
    ("nextBreak", v::GoTo::NextBreak),
    ("nextAttention", v::GoTo::NextAttention),
];

/// A curve by name (`curve`); the parameters of `speeds`, `bezier` and `ramp` come in their own keys.
#[derive(Clone, Copy)]
enum CurveName {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Speeds,
    Bezier,
    Ramp,
}

const CURVES: &[(&str, CurveName)] = &[
    ("linear", CurveName::Linear),
    ("easeIn", CurveName::EaseIn),
    ("easeOut", CurveName::EaseOut),
    ("easeInOut", CurveName::EaseInOut),
    ("speeds", CurveName::Speeds),
    ("bezier", CurveName::Bezier),
    ("ramp", CurveName::Ramp),
];

/// A column of `vertical.windows`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Tick,
    X,
    Y,
    W,
    H,
    Cx,
    Cy,
    Layout,
    State,
    Position,
    U,
    E,
    Speed,
    Upscale,
    Focus,
    Active,
}

impl Field {
    /// The numbers one row of the column holds.
    fn numbers(self) -> i64 {
        match self {
            Field::Focus => 2,
            Field::Active => 4,
            _ => 1,
        }
    }
}

const FIELDS: &[(&str, Field)] = &[
    ("tick", Field::Tick),
    ("x", Field::X),
    ("y", Field::Y),
    ("w", Field::W),
    ("h", Field::H),
    ("cx", Field::Cx),
    ("cy", Field::Cy),
    ("layout", Field::Layout),
    ("state", Field::State),
    ("position", Field::Position),
    ("u", Field::U),
    ("e", Field::E),
    ("speed", Field::Speed),
    ("upscale", Field::Upscale),
    ("focus", Field::Focus),
    ("active", Field::Active),
];

/// The longest gesture key (`merge`), in characters.
const MAX_GESTURE_KEY: usize = 200;

/// The most numbers one `vertical.windows` call returns, its frame column included (a focus point counts 2, an active
/// rect 4): 133 333 frames of the default fields, 95 238 of every field. The rows are built in memory before the reply,
/// so a long sequence is read with a `step`, fewer fields or in ranges.
const MAX_WINDOW_NUMBERS: i64 = 2_000_000;

/// Picture edit points per minute from which a sequence's shots are known without a measurement (spec §4.4,
/// `editedDensity`; no preference holds another value yet).
const EDITED_DENSITY: f64 = 2.0;

/// The frames a watched shot may miss: playback drops a frame or two (spec §5.10).
const WATCH_SLACK: i64 = 2;

/// The most regions one `vertical.measure` or `vertical.fit` takes inline, and the most boxes they hold.
const MAX_REGIONS: usize = 64;
const MAX_BOXES: usize = 5_000;

/// The union of the platforms' safe zones on a 1080×1920 output (`safeZone: "union"`), output px.
const UNION_SAFE_ZONE: [f64; 4] = [120.0, 288.0, 768.0, 960.0];

// ---------- parameters ----------

/// A refusal of command `cmd` (spec §7.6): nothing changed; `detail` carries the numbers needed to fix the call.
pub(crate) fn refuse(cmd: &str, code: &'static str, message: impl Into<String>, detail: Value) -> EngineError {
    EngineError::Invalid { cmd: cmd.to_string(), code, message: message.into(), detail }
}

/// A JSON object of parameters, checked against its command's closed key lists (spec §7.1, principle 10): every key
/// read refuses a value the key does not take, with the codes of spec §7.6. `index`, an item's place in its list, joins
/// the detail of every refusal of the item.
#[derive(Clone, Copy)]
pub(crate) struct Params<'a> {
    pub(crate) cmd: &'static str,
    map: Option<&'a Map<String, Value>>,
    index: Option<usize>,
}

impl<'a> Params<'a> {
    /// The parameters of a call: a JSON object (null is none) without `seconds` (`use_frame`: time is exact) and
    /// without a key outside `keys` (`out_of_range {key}`; `runId` too: attributing a run is agent integration's).
    pub(crate) fn of(cmd: &'static str, p: &'a Value, keys: &[&[&str]]) -> Result<Self> {
        let map = match p {
            Value::Object(map) => Some(map),
            Value::Null => None,
            _ => return Err(refuse(cmd, "out_of_range", "the parameters are a JSON object", json!({"key": "params"}))),
        };
        let params = Self { cmd, map, index: None };
        params.closed(keys)?;
        Ok(params)
    }

    /// Item `index` of list `key`: a JSON object with no key outside `keys`.
    fn element(&self, key: &str, value: &'a Value, index: usize, keys: &[&[&str]]) -> Result<Params<'a>> {
        let Value::Object(map) = value else {
            return Err(self.refuse("out_of_range", format!("each item of `{key}` is an object"), json!({"key": key, "item": index})));
        };
        let item = Self { cmd: self.cmd, map: Some(map), index: Some(index) };
        item.closed(keys)?;
        Ok(item)
    }

    /// The object under `key` (None when absent or null), with no key outside `keys`.
    fn object(&self, key: &str, keys: &[&[&str]]) -> Result<Option<Params<'a>>> {
        match self.raw(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Object(map)) => {
                let inner = Self { cmd: self.cmd, map: Some(map), index: self.index };
                inner.closed(keys)?;
                Ok(Some(inner))
            }
            Some(_) => Err(self.out_of_range(key, format!("`{key}` is an object"))),
        }
    }

    /// `use_frame` for `seconds`, `out_of_range {key}` for a key outside `keys`.
    fn closed(&self, keys: &[&[&str]]) -> Result<()> {
        let Some(map) = self.map else { return Ok(()) };
        if map.contains_key("seconds") {
            let message = "time is exact: pass a frame, or a time in ticks on a frame start, never seconds";
            return Err(self.refuse("use_frame", message, json!({"key": "seconds"})));
        }
        match map.keys().find(|key| !keys.iter().any(|group| group.contains(&key.as_str()))) {
            Some(key) => Err(self.out_of_range(key, format!("`{}` takes no `{key}`", self.cmd))),
            None => Ok(()),
        }
    }

    /// The value of `key`, null included.
    fn raw(&self, key: &str) -> Option<&'a Value> {
        self.map.and_then(|map| map.get(key))
    }

    /// The value of `key` when it is not null.
    fn value(&self, key: &str) -> Option<&'a Value> {
        self.raw(key).filter(|value| !value.is_null())
    }

    /// Whether `key` is given, null included (an explicit null clears).
    fn present(&self, key: &str) -> bool {
        self.raw(key).is_some()
    }

    /// Whether `key` is given with a value other than null.
    fn given(&self, key: &str) -> bool {
        self.value(key).is_some()
    }

    /// `detail` with the item's `index` when the parameters are an item of a list.
    fn detail(&self, mut detail: Value) -> Value {
        if let (Some(index), Some(map)) = (self.index, detail.as_object_mut()) {
            map.insert("index".into(), json!(index));
        }
        detail
    }

    /// A refusal of the call, with the item's `index` in its detail.
    pub(crate) fn refuse(&self, code: &'static str, message: impl Into<String>, detail: Value) -> EngineError {
        refuse(self.cmd, code, message, self.detail(detail))
    }

    /// The planner's (or the model's) refusal as the call's, with the item's `index` in its detail.
    fn invalid(&self, r: v::Refusal) -> EngineError {
        refuse(self.cmd, r.code, r.message, self.detail(r.detail))
    }

    /// `out_of_range {key}`: a value the key does not take.
    pub(crate) fn out_of_range(&self, key: &str, message: impl Into<String>) -> EngineError {
        self.refuse("out_of_range", message, json!({"key": key}))
    }

    /// `out_of_range {key}` for a key the call needs.
    pub(crate) fn missing(&self, key: &str) -> EngineError {
        self.out_of_range(key, format!("`{}` needs `{key}`", self.cmd))
    }

    /// `bad_number {key}`.
    fn not_finite(&self, key: &str) -> EngineError {
        self.refuse("bad_number", format!("`{key}` is not a finite number"), json!({"key": key}))
    }

    /// A finite number; None when `key` is absent or null. `bad_number` for a boolean, a string (JSON holds no NaN or
    /// infinity, so clients send "NaN", "-inf" or "1e309") or a value that is not finite; `out_of_range` for a list or an
    /// object.
    pub(crate) fn number(&self, key: &str) -> Result<Option<f64>> {
        self.value(key).map(|value| self.number_in(key, value)).transpose()
    }

    fn number_in(&self, key: &str, value: &Value) -> Result<f64> {
        match value {
            Value::Number(n) => n.as_f64().filter(|x| x.is_finite()).ok_or_else(|| self.not_finite(key)),
            Value::Bool(_) | Value::String(_) => Err(self.not_finite(key)),
            _ => Err(self.out_of_range(key, format!("`{key}` is a number"))),
        }
    }

    fn need_number(&self, key: &str) -> Result<f64> {
        self.number(key)?.ok_or_else(|| self.missing(key))
    }

    /// An integer (a float without a fraction counts); None when `key` is absent or null. `bad_number` for a boolean,
    /// `out_of_range` for a fraction, a value past the i64 range or anything that is no number.
    pub(crate) fn int(&self, key: &str) -> Result<Option<i64>> {
        self.value(key).map(|value| self.int_in(key, value)).transpose()
    }

    fn int_in(&self, key: &str, value: &Value) -> Result<i64> {
        if let Some(whole) = value.as_i64() {
            return Ok(whole);
        }
        match value {
            Value::Number(n) => {
                let x = n.as_f64().unwrap_or(f64::NAN);
                if x.is_finite() && x.fract() == 0.0 && x.abs() < 9.0e18 { Ok(x as i64) } else { Err(self.out_of_range(key, format!("`{key}` is an integer"))) }
            }
            Value::Bool(_) => Err(self.not_finite(key)),
            _ => Err(self.out_of_range(key, format!("`{key}` is an integer"))),
        }
    }

    fn need_int(&self, key: &str) -> Result<i64> {
        self.int(key)?.ok_or_else(|| self.missing(key))
    }

    /// A whole number below 2^64 (an integer, or a float without a fraction), as JSON from many clients writes ids.
    fn u64_in(value: &Value) -> Option<u64> {
        value.as_u64().or_else(|| value.as_f64().filter(|x| x.fract() == 0.0 && (0.0..1.8e19).contains(x)).map(|x| x as u64))
    }

    /// An id or a count from 0 to 4 294 967 295; None when `key` is absent or null; `out_of_range` for anything else (a
    /// negative number, a fraction, one past u32, a string, a boolean).
    fn whole(&self, key: &str) -> Result<Option<u32>> {
        self.value(key).map(|value| self.whole_in(key, value)).transpose()
    }

    fn whole_in(&self, key: &str, value: &Value) -> Result<u32> {
        let id = Self::u64_in(value).and_then(|id| u32::try_from(id).ok());
        id.ok_or_else(|| self.out_of_range(key, format!("`{key}` is a whole number from 0 to 4294967295")))
    }

    fn need_id(&self, key: &str) -> Result<u32> {
        self.whole(key)?.ok_or_else(|| self.missing(key))
    }

    /// A project item's id (a sequence); None when `key` is absent or null.
    fn item(&self, key: &str) -> Result<Option<ItemId>> {
        let Some(value) = self.value(key) else { return Ok(None) };
        Self::u64_in(value).map(|id| Some(ItemId(id))).ok_or_else(|| self.out_of_range(key, format!("`{key}` is an item id")))
    }

    /// A job's id; None when `key` is absent or null.
    pub(crate) fn job(&self, key: &str) -> Result<Option<u64>> {
        let Some(value) = self.value(key) else { return Ok(None) };
        Self::u64_in(value).map(Some).ok_or_else(|| self.out_of_range(key, format!("`{key}` is a job id")))
    }

    /// A whole number below 2^64; None when `key` is absent or null; `out_of_range` for anything else.
    fn count(&self, key: &str) -> Result<Option<u64>> {
        let Some(value) = self.value(key) else { return Ok(None) };
        Self::u64_in(value).map(Some).ok_or_else(|| self.out_of_range(key, format!("`{key}` is a whole number")))
    }

    /// True or false; None when `key` is absent or null.
    pub(crate) fn flag(&self, key: &str) -> Result<Option<bool>> {
        match self.raw(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(on)) => Ok(Some(*on)),
            Some(_) => Err(self.out_of_range(key, format!("`{key}` is true or false"))),
        }
    }

    /// A text; None when `key` is absent or null.
    fn text(&self, key: &str) -> Result<Option<&'a str>> {
        match self.raw(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.as_str())),
            Some(_) => Err(self.out_of_range(key, format!("`{key}` is a text"))),
        }
    }

    /// [`Params::text`], owned.
    fn owned(&self, key: &str) -> Result<Option<String>> {
        Ok(self.text(key)?.map(str::to_string))
    }

    fn need_text(&self, key: &str) -> Result<String> {
        self.owned(key)?.ok_or_else(|| self.missing(key))
    }

    /// Metadata text (spec §7.5): None when absent (it keeps its value), empty for null (it clears).
    fn metadata(&self, key: &str) -> Result<Option<String>> {
        match self.raw(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(String::new())),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(self.out_of_range(key, format!("`{key}` is a text or null"))),
        }
    }

    /// One of a closed set of names; None when `key` is absent or null.
    fn name<T: Copy>(&self, key: &str, names: &[(&str, T)]) -> Result<Option<T>> {
        let Some(name) = self.text(key)? else { return Ok(None) };
        match names.iter().find(|(known, _)| *known == name) {
            Some((_, value)) => Ok(Some(*value)),
            None => {
                let known: Vec<&str> = names.iter().map(|(known, _)| *known).collect();
                Err(self.out_of_range(key, format!("`{key}` is one of {}", known.join(", "))))
            }
        }
    }

    /// A name that null clears: None when absent, Some(None) for null.
    fn clearable<T: Copy>(&self, key: &str, names: &[(&str, T)]) -> Result<Option<Option<T>>> {
        match self.raw(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(_) => Ok(Some(self.name(key, names)?)),
        }
    }

    /// The list under `key`; None when absent or null. `too_many {key, count, bound}` past `bound` items.
    fn list(&self, key: &str, bound: usize) -> Result<Option<&'a [Value]>> {
        match self.raw(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) if items.len() > bound => {
                let count = items.len();
                Err(self.refuse("too_many", format!("`{key}` holds {count} items, more than {bound}"), json!({"key": key, "count": count, "bound": bound})))
            }
            Some(Value::Array(items)) => Ok(Some(items.as_slice())),
            Some(_) => Err(self.out_of_range(key, format!("`{key}` is a list"))),
        }
    }

    /// A list of ids (at most [`v::MAX_LIST`]).
    fn ids(&self, key: &str) -> Result<Option<Vec<u32>>> {
        let Some(items) = self.list(key, v::MAX_LIST)? else { return Ok(None) };
        items.iter().map(|item| self.whole_in(key, item)).collect::<Result<Vec<u32>>>().map(Some)
    }

    fn need_ids(&self, key: &str) -> Result<Vec<u32>> {
        self.ids(key)?.ok_or_else(|| self.missing(key))
    }

    /// A list of frames (at most [`v::MAX_LIST`]).
    fn frames(&self, key: &str) -> Result<Option<Vec<i64>>> {
        let Some(items) = self.list(key, v::MAX_LIST)? else { return Ok(None) };
        items.iter().map(|item| self.int_in(key, item)).collect::<Result<Vec<i64>>>().map(Some)
    }

    /// A list of frame pairs `[from, to]` (at most [`v::MAX_LIST`]).
    fn pairs(&self, key: &str) -> Result<Vec<[i64; 2]>> {
        let Some(items) = self.list(key, v::MAX_LIST)? else { return Ok(Vec::new()) };
        items.iter().map(|item| self.pair_in(key, item)).collect()
    }

    fn pair_in(&self, key: &str, value: &Value) -> Result<[i64; 2]> {
        match value.as_array().map(Vec::as_slice) {
            Some([from, to]) => Ok([self.int_in(key, from)?, self.int_in(key, to)?]),
            _ => Err(self.out_of_range(key, format!("each item of `{key}` is a pair [from, to]"))),
        }
    }

    /// `N` numbers; None when `key` is absent or null.
    fn numbers<const N: usize>(&self, key: &str) -> Result<Option<[f64; N]>> {
        self.value(key).map(|value| self.numbers_in::<N>(key, value)).transpose()
    }

    fn numbers_in<const N: usize>(&self, key: &str, value: &Value) -> Result<[f64; N]> {
        let Some(items) = value.as_array().filter(|items| items.len() == N) else {
            return Err(self.out_of_range(key, format!("`{key}` is a list of {N} numbers")));
        };
        let mut out = [0.0; N];
        for (slot, item) in out.iter_mut().zip(items) {
            *slot = self.number_in(key, item)?;
        }
        Ok(out)
    }

    /// A rectangle `[x, y, w, h]` in pixels.
    fn need_rect(&self, key: &str) -> Result<[f64; 4]> {
        self.numbers::<4>(key)?.ok_or_else(|| self.missing(key))
    }

    /// The frame named by `frame`, `time` or `timecode` (at most one of them: `out_of_range {key: "frame"}` otherwise);
    /// None when the call names none. `time` is in ticks, an integer or a string of digits, and refused `bad_time
    /// {floorFrame, nearestFrame}` off a frame start; `timecode` is the sequence's (`01:00:12:03`, counted from its start
    /// timecode; `+N` and `-N` from the playhead) or a part's (`P07 00:00:12:03`, from the part's first frame).
    fn time_frame(&self, s: &Session, t: &Target) -> Result<Option<i64>> {
        let named: Vec<&str> = TIME.iter().copied().filter(|key| self.given(key)).collect();
        match named.as_slice() {
            [] => Ok(None),
            ["frame"] => self.int("frame"),
            ["time"] => self.time(t.q).map(Some),
            ["timecode"] => self.timecode(s, t).map(Some),
            _ => Err(self.out_of_range("frame", "name the frame once: frame, time or timecode")),
        }
    }

    fn time(&self, q: &Sequence) -> Result<i64> {
        let ticks = match self.raw("time") {
            Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
            Some(value) => self.int_in("time", value).ok(),
            None => None,
        };
        let ticks = ticks.ok_or_else(|| self.out_of_range("time", "`time` is a whole number of ticks"))?;
        v::frame_of_time(q.settings.frame_rate, Tick(ticks)).map_err(|r| self.invalid(r))
    }

    fn timecode(&self, s: &Session, t: &Target) -> Result<i64> {
        let text = self.text("timecode")?.unwrap_or_default();
        let settings = &t.q.settings;
        let (rate, drop) = (settings.frame_rate, settings.drop_frame);
        let empty = crate::empty_vertical_set();
        let set = t.q.vertical.as_deref().unwrap_or(&empty);
        if let Some(frame) = v::parse_part_timecode(set, rate, drop, text).map_err(|r| self.invalid(r))? {
            return Ok(frame);
        }
        let start = t.q.start_timecode;
        let current = rate.frame_at(playhead_of(s, t.seq)).saturating_add(start);
        match parse_timecode(text, rate, drop, current) {
            Ok(frames) => Ok(frames.saturating_sub(start)),
            Err(e) => Err(self.refuse("bad_time", format!("{text:?} is not a timecode: {e}"), json!({"timecode": text}))),
        }
    }
}

/// The sequence a call names (`sequence`, by default the active one): `not_found {sequence}` for an id that names no
/// sequence (a media item included), `not_found` when none is open.
pub(crate) fn sequence_of(s: &Session, a: &Params) -> Result<ItemId> {
    let seq = match a.item("sequence")? {
        Some(seq) => seq,
        None => s.state.active_sequence.ok_or_else(|| a.refuse("not_found", "no sequence is open: name one", json!({"sequence": null})))?,
    };
    if s.project.sequence(seq).is_none() {
        return Err(not_found_sequence(a.cmd, seq));
    }
    Ok(seq)
}

fn not_found_sequence(cmd: &str, seq: ItemId) -> EngineError {
    refuse(cmd, "not_found", format!("there is no sequence {}", seq.0), json!({"sequence": seq.0}))
}

/// Sequence `seq` of `project`.
fn sequence_in<'p>(cmd: &str, project: &'p Project, seq: ItemId) -> Result<&'p Sequence> {
    project.sequence(seq).ok_or_else(|| not_found_sequence(cmd, seq))
}

/// Plan `id` of sequence `q`.
fn plan_in(q: &Sequence, id: u32) -> Option<&VerticalPlan> {
    q.vertical.as_deref()?.plans.iter().find(|plan| plan.id == id)
}

/// Plan `id` of sequence `q`: `not_found {plan}`.
fn plan_of<'q>(cmd: &str, q: &'q Sequence, id: u32) -> Result<&'q VerticalPlan> {
    plan_in(q, id).ok_or_else(|| refuse(cmd, "not_found", format!("the sequence has no plan {id}"), json!({"plan": id})))
}

/// The playhead of sequence `seq`.
fn playhead_of(s: &Session, seq: ItemId) -> Tick {
    s.state.playheads.get(&seq).copied().unwrap_or_default()
}

/// Half-open frame ranges sorted and merged (touching ranges join).
fn merged(mut ranges: Vec<[i64; 2]>) -> Vec<[i64; 2]> {
    ranges.retain(|[from, to]| from < to);
    ranges.sort_unstable();
    let mut out: Vec<[i64; 2]> = Vec::with_capacity(ranges.len());
    for [from, to] in ranges {
        match out.last_mut() {
            Some([_, end]) if from <= *end => *end = (*end).max(to),
            _ => out.push([from, to]),
        }
    }
    out
}

/// Whether `frame` lies in one of the half-open `ranges`.
fn within(ranges: &[[i64; 2]], frame: i64) -> bool {
    ranges.iter().any(|&[from, to]| (from..to).contains(&frame))
}

/// The frames where the shots of sequence `q` are not known (spec §4.4): outside the analysed ranges of a sequence below
/// the edited density, and wherever a measurement job of the sequence is running.
fn unknown_frames(s: &Session, project: &Project, seq: ItemId, q: &Sequence) -> Vec<[i64; 2]> {
    let mut ranges = v::unknown_ranges(project, q, EDITED_DENSITY);
    ranges.extend(s.vertical_store.measurements.running_ranges(seq));
    merged(ranges)
}

// ---------- validity ----------

/// The violations of sequence `seq`'s vertical set in the session's project: the store's when it validated this very
/// project snapshot, else computed now.
fn violations(s: &Session, seq: ItemId) -> Cow<'_, [Violation]> {
    let current = Arc::as_ptr(&s.project);
    match s.vertical_store.validity.get(&seq) {
        Some(known) if std::ptr::eq(known.project.as_ptr(), current) => Cow::Borrowed(known.violations.as_slice()),
        _ => Cow::Owned(s.project.sequence(seq).map(|q| v::validate(&s.project, q)).unwrap_or_default()),
    }
}

/// The violations that keep plan `plan` of sequence `seq` from being evaluated: the set's own and the plan's (spec
/// §2.8). Empty for a valid plan.
pub(crate) fn plan_violations(s: &Session, seq: ItemId, plan: u32) -> Vec<Violation> {
    violations(s, seq).iter().filter(|violation| violation.plan.is_none_or(|id| id == plan)).cloned().collect()
}

/// Validate sequence `seq` unless the store knows the answer for the session's project, and keep it.
fn refresh(s: &mut Session, seq: ItemId) {
    let current = Arc::as_ptr(&s.project);
    if s.vertical_store.validity.get(&seq).is_some_and(|known| std::ptr::eq(known.project.as_ptr(), current)) {
        return;
    }
    let violations = s.project.sequence(seq).map(|q| v::validate(&s.project, q)).unwrap_or_default();
    s.vertical_store.validity.insert(seq, Validity { project: Arc::downgrade(&s.project), violations });
}

/// `invalid_plan {violations}` for a plan that breaks an invariant: it is never evaluated until it is repaired.
fn ensure_valid(s: &mut Session, cmd: &str, seq: ItemId, plan: u32) -> Result<()> {
    refresh(s, seq);
    let violations = plan_violations(s, seq, plan);
    if violations.is_empty() {
        return Ok(());
    }
    Err(refuse(cmd, "invalid_plan", "the plan breaks an invariant; vertical.repair fixes it", json!({"violations": violations})))
}

/// Validate every vertical set of the session's project now (spec §2.8: on open, on install, after undo and redo), so
/// the reads and writes that follow answer `invalid_plan` for a plan that breaks an invariant.
pub(crate) fn validate_now(s: &mut Session) {
    let project = Arc::clone(&s.project);
    let weak = Arc::downgrade(&project);
    let mut validity = BTreeMap::new();
    for (id, item) in &project.items {
        if let Some(q) = item.as_sequence().filter(|q| q.vertical.is_some()) {
            validity.insert(*id, Validity { project: Weak::clone(&weak), violations: v::validate(&project, q) });
        }
    }
    s.vertical_store.validity = validity;
}

/// The session's project was replaced ([`Session::note_project_replaced`]). Another project (a new history: new, open,
/// close, new from template, install) is another life ([`crate::History::life`]): the store forgets what it knew of the
/// old one and stops its measurement jobs, whose results would belong to it; measurement ids keep counting, so an id is
/// never handed out twice in the session. A snapshot of the same project put back (a cancelled dynamic trim keeps the
/// history) keeps the store and its jobs: a measurement goes stale only by its picture digest and media generation, and
/// a watched shot only by its range digest.
pub(crate) fn forget(s: &mut Session) {
    if s.vertical_store.life == s.history.life {
        return;
    }
    for pending in &s.vertical_store.measurements.pending {
        if let Some(job) = s.jobs.iter().find(|job| job.id == pending.job) {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
    }
    let last_id = s.vertical_store.measurements.last_id;
    s.vertical_store = VerticalStore::of_life(s.history.life);
    s.vertical_store.measurements.last_id = last_id;
}

/// A recorded write changed sequence `seq`'s vertical set and nothing else (a plan-only step): the validity the store
/// knew for the snapshot the write started from (`before`) holds in the new one for every other sequence, and `seq`'s
/// is found once now, so the exact frames and reads that follow find it instead of validating again.
fn keep_validity(s: &mut Session, before: &Arc<Project>, seq: ItemId) {
    let now = Arc::downgrade(&s.project);
    for (id, known) in &mut s.vertical_store.validity {
        if *id != seq && std::ptr::eq(known.project.as_ptr(), Arc::as_ptr(before)) {
            known.project = Weak::clone(&now);
        }
    }
    refresh(s, seq);
}

// ---------- writes ----------

/// How a write command reads its parameters into the planner's write.
type Parse = fn(&Session, &Params, &Target) -> Result<Parsed>;

/// The sequence a write names: its id and its state before the write.
struct Target<'q> {
    seq: ItemId,
    q: &'q Sequence,
}

/// A write as the planner takes it, with the keys the engine handles itself.
struct Parsed {
    write: v::Write,
    /// The gesture key (`merge`, setPosition): sub-steps with the same key and label fold into one undo step.
    merge: Option<String>,
    /// `echo` (replaceRange): the result lists the added and changed positions in the read shape.
    echo: bool,
    /// The range `expectRange` guards: the result carries its new digest (`rangeDigest`), the next sub-step's guard.
    range: Option<[i64; 2]>,
}

impl Parsed {
    fn of(write: v::Write) -> Self {
        Self { write, merge: None, echo: false, range: None }
    }
}

/// Runs write `cmd` (spec §7.1, principles 3 and 4): parse, plan on the working copy as the session's writer, answer the
/// envelope; a dry run or a write that changes nothing records nothing; a change is one plan-only undo step.
fn run_write(s: &mut Session, p: &Value, cmd: &'static str, keys: &[&[&str]], parse: Parse) -> Result<Value> {
    measure::poll(s);
    let a = Params::of(cmd, p, keys)?;
    let seq = sequence_of(s, &a)?;
    let dry = a.flag("dryRun")?.unwrap_or(false);
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let parsed = parse(s, &a, &Target { seq, q })?;
    let copy = s.vertical_working_copy(seq).ok_or_else(|| not_found_sequence(cmd, seq))?;
    let caller = Caller { writer: s.writer, overwrite_allowed: false };
    let mut planned = v::plan_write(&project, &copy, &parsed.write, caller).map_err(|r| EngineError::invalid(cmd, r))?;
    let mut after = copy;
    after.vertical = planned.set.take();
    let mut out = envelope(&project, seq, q, &after, &planned, &parsed, dry);
    if dry || !planned.changed {
        return Ok(out);
    }
    let unknown = unknown_frames(s, &project, seq, q);
    let label = planned.label.clone();
    let set = after.vertical.take();
    let revision = s.revision;
    s.edit_plan_only(seq, &label, parsed.merge.as_deref(), move |_, slot, _| {
        *slot = set;
        Ok(())
    })?;
    if s.revision == revision {
        // the session found nothing to record but id counters: the write changed nothing after all
        out["changed"] = json!(false);
        return Ok(out);
    }
    out["applied"] = json!(true);
    out["label"] = json!(label);
    keep_validity(s, &project, seq);
    note_made(s, seq, &planned, &unknown);
    if matches!(parsed.write, v::Write::ApplyMeasurement(_)) {
        note_applied(s, seq);
    }
    keep_selection(s);
    Ok(out)
}

/// The keys of the result envelope of spec §7.2 that every write answers: what it does (`applied` and `label` are set
/// when it is recorded) and what it changed.
fn base_envelope(seq: ItemId, planned: &v::Planned, dry: bool) -> Value {
    json!({
        "applied": false,
        "changed": planned.changed,
        "dryRun": dry,
        "sequence": seq.0,
        "label": null,
        "added": planned.added,
        "updated": planned.updated,
        "removed": planned.removed,
        "changedFrames": planned.changed_frames,
        "clamped": planned.clamped,
        "issues": planned.issues,
        "changes": planned.changes,
        "plans": planned.plans,
        "ids": planned.ids,
    })
}

/// The result envelope of spec §7.2 for a planned write: [`base_envelope`], the named plan's digests before and after
/// (null on a merged sub-step, which keeps a drag cheap), the range's new digest when `expectRange` was given, and the
/// write's own keys.
fn envelope(project: &Project, seq: ItemId, before: &Sequence, after: &Sequence, planned: &v::Planned, parsed: &Parsed, dry: bool) -> Value {
    let mut out = base_envelope(seq, planned, dry);
    if let Some(id) = planned.plan {
        let digests = parsed.merge.is_none();
        let digest = |q: &Sequence| plan_in(q, id).filter(|_| digests).map(|plan| v::plan_digest(q, plan));
        out["plan"] = json!(id);
        out["digestBefore"] = json!(digest(before));
        out["digest"] = json!(digest(after));
        if let Some([from, to]) = parsed.range {
            out["rangeDigest"] = json!(plan_in(after, id).map(|plan| v::range_digest(project, after, plan, from, to)));
        }
        if parsed.echo {
            out["positions"] = echo(project, after, id, planned);
        }
    }
    if matches!(parsed.write, v::Write::DeletePositions(_)) {
        out["deleted"] = json!(planned.removed);
        out["missing"] = json!(planned.missing);
    }
    out
}

/// The positions a write added or changed in plan `id` of `after`, in the read shape (`echo`).
fn echo(project: &Project, after: &Sequence, id: u32, planned: &v::Planned) -> Value {
    let (Some(plan), Some(derived)) = (plan_in(after, id), v::derive_plan(project, after, id)) else { return json!([]) };
    let shown = Shown::new(after, plan);
    let named = |p: &v::Position| planned.added.contains(&p.id) || planned.updated.contains(&p.id);
    let echoed = plan.positions.iter().zip(&derived.positions).filter(|(p, _)| named(p));
    Value::Array(echoed.map(|(p, info)| position_json(&shown, p, Some(info))).collect())
}

/// Remember the positions a write made at frames where shots were not known (spec §4.4): applying a measurement later
/// lists the ones it leaves off a wall.
fn note_made(s: &mut Session, seq: ItemId, planned: &v::Planned, unknown: &[[i64; 2]]) {
    if unknown.is_empty() {
        return;
    }
    let project = Arc::clone(&s.project);
    let Some(q) = project.sequence(seq) else { return };
    let rate = q.settings.frame_rate;
    for delta in planned.plans.iter().filter(|delta| !delta.added.is_empty()) {
        let Some(plan) = plan_in(q, delta.plan) else { continue };
        let made: Vec<u32> = plan.positions.iter().filter(|p| delta.added.contains(&p.id) && within(unknown, rate.frame_at(p.at))).map(|p| p.id).collect();
        if made.is_empty() {
            continue;
        }
        let known = s.vertical_store.made_unknown.entry((seq, plan.id)).or_default();
        known.extend(made);
        known.retain(|id| plan.positions.iter().any(|p| p.id == *id));
    }
}

/// A measurement was applied to sequence `seq`: `unanchoredSinceMeasurement` now lists the positions made while shots
/// were unknown, those of them that are not on a wall (spec §4.4).
fn note_applied(s: &mut Session, seq: ItemId) {
    let project = Arc::clone(&s.project);
    let Some(set) = project.sequence(seq).and_then(|q| q.vertical.as_deref()) else { return };
    for plan in &set.plans {
        let made = s.vertical_store.made_unknown.get(&(seq, plan.id));
        let ids: Vec<u32> = plan.positions.iter().map(|p| p.id).filter(|id| made.is_some_and(|made| made.contains(id))).collect();
        s.vertical_store.unanchored.insert((seq, plan.id), ids);
    }
}

/// Keep the Vertical Edit's selection to what the active sequence still has.
fn keep_selection(s: &mut Session) {
    let project = Arc::clone(&s.project);
    let active = s.state.active_sequence.and_then(|id| project.sequence(id));
    crate::retain_vertical(&mut s.state.vertical, &project, active);
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.create", &[&["sequence", "name", "preset", "width", "height", "filter", "dryRun"]], parse_create)
}

fn parse_create(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let create = v::CreatePlan {
        name: a.owned("name")?,
        preset: a.name("preset", PRESETS)?,
        width: a.whole("width")?,
        height: a.whole("height")?,
        filter: a.name("filter", FILTERS)?,
    };
    Ok(Parsed::of(v::Write::Create(create)))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let keys: &[&[&str]] = &[PLAN, CHECKED, &["name", "width", "height", "filter", "startingWindow", "captions", "fitBackground", "clamp"]];
    run_write(s, p, "vertical.update", keys, parse_update)
}

fn parse_update(_s: &Session, a: &Params, t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let update = v::UpdatePlan {
        plan,
        name: a.owned("name")?,
        width: a.whole("width")?,
        height: a.whole("height")?,
        filter: a.name("filter", FILTERS)?,
        starting_window: starting_window(a)?,
        captions: captions(a, t.q, plan)?,
        fit_background: a.name("fitBackground", BACKGROUNDS)?,
        clamp: a.flag("clamp")?.unwrap_or(false),
        expect: a.owned("expect")?,
    };
    Ok(Parsed::of(v::Write::Update(update)))
}

/// `startingWindow`: None when absent, Some(None) for null (the centred, largest window), else `{cx, cy, h}`.
fn starting_window(a: &Params) -> Result<Option<Option<v::Window>>> {
    if !a.present("startingWindow") {
        return Ok(None);
    }
    let Some(w) = a.object("startingWindow", &[&["cx", "cy", "h"]])? else { return Ok(Some(None)) };
    Ok(Some(Some(v::Window { cx: w.need_number("cx")?, cy: w.need_number("cy")?, h: w.need_number("h")? })))
}

/// `captions`: the plan's subtitle layout with the keys given (an omitted key keeps its value, null clears one that has
/// no default); None when absent.
fn captions(a: &Params, q: &Sequence, plan: u32) -> Result<Option<v::VerticalCaptions>> {
    let keys: &[&[&str]] = &[&["area", "size", "minSize", "maxLines", "keepLineBreaks", "anchor", "track", "edge", "minClippedFrames"]];
    let Some(c) = a.object("captions", keys)? else { return Ok(None) };
    let mut out = plan_in(q, plan).map(|p| p.captions.clone()).unwrap_or_default();
    if c.present("area") {
        out.area = match c.numbers::<4>("area")? {
            None => None,
            Some([x, y, w, h]) => Some([pixels(&c, x)?, pixels(&c, y)?, pixels(&c, w)?, pixels(&c, h)?]),
        };
    }
    if c.present("size") {
        out.size = c.number("size")?.map(|size| size as f32);
    }
    if c.present("minSize") {
        out.min_size = c.number("minSize")?.map(|size| size as f32);
    }
    if let Some(lines) = c.int("maxLines")? {
        out.max_lines = u8::try_from(lines).ok().filter(|n| (1..=4).contains(n)).ok_or_else(|| c.out_of_range("maxLines", "subtitles take 1 to 4 lines"))?;
    }
    if let Some(keep) = c.flag("keepLineBreaks")? {
        out.keep_line_breaks = keep;
    }
    if let Some(anchor) = c.name("anchor", ANCHORS)? {
        out.anchor = anchor;
    }
    if c.present("track") {
        out.track = c.whole("track")?;
    }
    if let Some(edge) = c.name("edge", EDGES)? {
        out.edge = edge;
    }
    if let Some(frames) = c.whole("minClippedFrames")? {
        out.min_clipped_frames = frames;
    }
    Ok(Some(out))
}

/// A subtitle area's side, a whole number of output pixels.
fn pixels(c: &Params, x: f64) -> Result<u32> {
    if x.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&x) {
        return Ok(x as u32);
    }
    Err(c.out_of_range("area", "the area is [x, y, w, h] in whole output pixels"))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.delete", &[PLAN, CHECKED], parse_delete)
}

fn parse_delete(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    Ok(Parsed::of(v::Write::Delete(v::DeletePlan { plan: a.need_id("plan")?, expect: a.owned("expect")? })))
}

fn set_position(s: &mut Session, p: &Value) -> Result<Value> {
    let keys: &[&[&str]] = &[PLAN, TIME, WINDOW, ARRIVAL, METADATA, GUARDED, &["fromLabel", "fromPosition", "clamp", "merge"]];
    run_write(s, p, "vertical.setPosition", keys, parse_set_position)
}

fn parse_set_position(s: &Session, a: &Params, t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let frame = a.time_frame(s, t)?.ok_or_else(|| a.missing("frame"))?;
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::SetPosition {
        plan,
        frame,
        fields: position_fields(a)?,
        from_label: a.owned("fromLabel")?,
        from_position: a.whole("fromPosition")?,
        clamp: a.flag("clamp")?.unwrap_or(false),
        expect: a.owned("expect")?,
        expect_range,
    };
    Ok(Parsed { write: v::Write::SetPosition(write), merge: gesture_key(a)?, echo: false, range })
}

/// The window, arrival and metadata keys of a position (spec §7.2).
fn position_fields(a: &Params) -> Result<v::PositionFields> {
    Ok(v::PositionFields {
        cx: a.number("cx")?,
        cy: a.number("cy")?,
        h: a.number("h")?,
        zoom: a.number("zoom")?,
        layout: a.name("layout", LAYOUTS)?,
        arrival: a.name("arrival", ARRIVALS)?,
        move_frames: a.int("moveFrames")?,
        curve: curve(a)?,
        caption_anchor: a.clearable("captionAnchor", ANCHORS)?,
        label: a.metadata("label")?,
        color: color(a)?,
        tags: tags(a)?,
        note: a.metadata("note")?,
        subjects: None,
        focus: focus(a)?,
        confidence: None,
        risk: None,
    })
}

/// The curve the keys name: `curve` by name, the parameters of `speeds`, `bezier` and `ramp` in their own keys; one of
/// those three alone names its curve, and `ramp` without its shares is Ramp 20/20. A curve without its parameters, or
/// parameters of another curve, are `out_of_range`.
fn curve(a: &Params) -> Result<Option<v::Curve>> {
    let speeds = a.numbers::<2>("speeds")?.map(|[start, end]| v::Curve::Speeds { start, end });
    let bezier = a.numbers::<4>("bezier")?.map(|[x1, y1, x2, y2]| v::Curve::Bezier { x1, y1, x2, y2 });
    let ramp = a.numbers::<2>("ramp")?.map(|[r_in, r_out]| v::Curve::Ramp { r_in, r_out });
    let given: Vec<(&str, v::Curve)> = [("speeds", speeds), ("bezier", bezier), ("ramp", ramp)].into_iter().filter_map(|(key, c)| Some((key, c?))).collect();
    if let Some((key, _)) = given.get(1) {
        return Err(a.out_of_range(key, "a move has one curve: pass one of speeds, bezier and ramp"));
    }
    let parameters = given.first().copied();
    let Some(name) = a.name("curve", CURVES)? else { return Ok(parameters.map(|(_, c)| c)) };
    let curve = match (name, parameters) {
        (CurveName::Linear, None) => v::Curve::Linear,
        (CurveName::EaseIn, None) => v::Curve::EaseIn,
        (CurveName::EaseOut, None) => v::Curve::EaseOut,
        (CurveName::EaseInOut, None) => v::Curve::EaseInOut,
        (CurveName::Ramp, None) => v::DEFAULT_CURVE,
        (CurveName::Speeds, Some(("speeds", c))) | (CurveName::Bezier, Some(("bezier", c))) | (CurveName::Ramp, Some(("ramp", c))) => c,
        (CurveName::Speeds, None) => return Err(a.missing("speeds")),
        (CurveName::Bezier, None) => return Err(a.missing("bezier")),
        (_, Some((key, _))) => return Err(a.out_of_range(key, "these parameters belong to another curve")),
    };
    Ok(Some(curve))
}

/// `color`: a label colour by name (any case), null clears.
fn color(a: &Params) -> Result<Option<Option<Label>>> {
    let refused = || a.out_of_range("color", "`color` is a label colour, such as \"Rose\" or \"Mango\", or null");
    match a.raw("color") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(name)) => Label::from_name(name).map(|label| Some(Some(label))).ok_or_else(refused),
        Some(_) => Err(refused()),
    }
}

/// `tags`: a list of texts, null clears.
fn tags(a: &Params) -> Result<Option<Vec<String>>> {
    if a.raw("tags").is_some_and(Value::is_null) {
        return Ok(Some(Vec::new()));
    }
    let Some(items) = a.list("tags", v::MAX_LIST)? else { return Ok(None) };
    let mut out = Vec::with_capacity(items.len());
    for (item, value) in items.iter().enumerate() {
        let tag = value.as_str().ok_or_else(|| a.refuse("out_of_range", "each tag is a text", json!({"key": "tags", "item": item})))?;
        out.push(tag.to_string());
    }
    Ok(Some(out))
}

/// `focus`: a point `[x, y]` in sequence pixels, null clears.
fn focus(a: &Params) -> Result<Option<Option<[f64; 2]>>> {
    match a.raw("focus") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => a.numbers_in::<2>("focus", value).map(|point| Some(Some(point))),
    }
}

/// `merge`: a gesture key of 1 to 200 characters.
fn gesture_key(a: &Params) -> Result<Option<String>> {
    let Some(key) = a.text("merge")? else { return Ok(None) };
    if key.is_empty() || key.chars().count() > MAX_GESTURE_KEY {
        return Err(a.out_of_range("merge", "a gesture key has 1 to 200 characters"));
    }
    Ok(Some(key.to_string()))
}

/// `expectRange {fromFrame, toFrame, digest}`.
fn range_guard(a: &Params) -> Result<Option<v::RangeGuard>> {
    let Some(g) = a.object("expectRange", &[&["fromFrame", "toFrame", "digest"]])? else { return Ok(None) };
    Ok(Some(v::RangeGuard { from_frame: g.need_int("fromFrame")?, to_frame: g.need_int("toFrame")?, digest: g.need_text("digest")? }))
}

fn retime_position(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.retimePosition", &[PLAN, TIME, GUARDED, &["position"]], parse_retime)
}

fn parse_retime(s: &Session, a: &Params, t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let position = a.need_id("position")?;
    let frame = a.time_frame(s, t)?.ok_or_else(|| a.missing("frame"))?;
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::RetimePosition { plan, position, frame, expect: a.owned("expect")?, expect_range };
    Ok(Parsed { range, ..Parsed::of(v::Write::RetimePosition(write)) })
}

fn delete_positions(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.deletePositions", &[PLAN, GUARDED, &["positions"]], parse_delete_positions)
}

fn parse_delete_positions(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let positions = a.need_ids("positions")?;
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::DeletePositions { plan, positions, expect: a.owned("expect")?, expect_range };
    Ok(Parsed { range, ..Parsed::of(v::Write::DeletePositions(write)) })
}

fn replace_range(s: &mut Session, p: &Value) -> Result<Value> {
    let keys: &[&[&str]] = &[PLAN, GUARDED, &["fromFrame", "toFrame", "positions", "clamp", "strict", "echo", "expectWalls"]];
    run_write(s, p, "vertical.replaceRange", keys, parse_replace_range)
}

fn parse_replace_range(s: &Session, a: &Params, t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let (from_frame, to_frame) = (a.need_int("fromFrame")?, a.need_int("toFrame")?);
    let list = a.list("positions", v::MAX_REPLACE_ITEMS)?.ok_or_else(|| a.missing("positions"))?;
    let mut items = Vec::with_capacity(list.len());
    for (index, value) in list.iter().enumerate() {
        let item = a.element("positions", value, index, ITEM)?;
        let frame = item.time_frame(s, t)?.ok_or_else(|| item.missing("frame"))?;
        items.push(v::ReplaceItem { frame, fields: position_fields(&item)? });
    }
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::ReplaceRange {
        plan,
        from_frame,
        to_frame,
        items,
        clamp: a.flag("clamp")?.unwrap_or(false),
        strict: a.flag("strict")?.unwrap_or(false),
        expect: a.owned("expect")?,
        expect_range,
        expect_walls: a.owned("expectWalls")?,
    };
    Ok(Parsed { write: v::Write::ReplaceRange(write), merge: None, echo: a.flag("echo")?.unwrap_or(false), range })
}

fn set_arrival(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.setArrival", &[PLAN, ARRIVAL, GUARDED, &["positions"]], parse_set_arrival)
}

fn parse_set_arrival(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let positions = a.need_ids("positions")?;
    let arrival = a.name("arrival", ARRIVALS)?.ok_or_else(|| a.missing("arrival"))?;
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::SetArrival { plan, positions, arrival, move_frames: a.int("moveFrames")?, curve: curve(a)?, expect: a.owned("expect")?, expect_range };
    Ok(Parsed { range, ..Parsed::of(v::Write::SetArrival(write)) })
}

fn smooth_through(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.smoothThrough", &[PLAN, GUARDED, &["positions", "startFromRest", "endAtRest"]], parse_smooth_through)
}

fn parse_smooth_through(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let positions = a.need_ids("positions")?;
    let rest = v::SmoothOpts::default();
    let opts = v::SmoothOpts {
        start_from_rest: a.flag("startFromRest")?.unwrap_or(rest.start_from_rest),
        end_at_rest: a.flag("endAtRest")?.unwrap_or(rest.end_at_rest),
    };
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::SmoothThrough { plan, positions, opts, expect: a.owned("expect")?, expect_range };
    Ok(Parsed { range, ..Parsed::of(v::Write::SmoothThrough(write)) })
}

fn fit_move(s: &mut Session, p: &Value) -> Result<Value> {
    let keys: &[&[&str]] = &[PLAN, GUARDED, &["position", "maxSpeed", "maxAccel", "curve", "speeds", "bezier", "ramp"]];
    run_write(s, p, "vertical.fitMove", keys, parse_fit_move)
}

fn parse_fit_move(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let position = a.need_id("position")?;
    let max_speed = a.need_number("maxSpeed")?;
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::FitMove { plan, position, max_speed, max_accel: a.number("maxAccel")?, curve: curve(a)?, expect: a.owned("expect")?, expect_range };
    Ok(Parsed { range, ..Parsed::of(v::Write::FitMove(write)) })
}

fn swap_windows(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.swapWindows", &[PLAN, GUARDED, &["positions"]], parse_swap_windows)
}

fn parse_swap_windows(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let positions = a.need_ids("positions")?;
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::SwapWindows { plan, positions, expect: a.owned("expect")?, expect_range };
    Ok(Parsed { range, ..Parsed::of(v::Write::SwapWindows(write)) })
}

fn set_walls(s: &mut Session, p: &Value) -> Result<Value> {
    let lists: &[&str] = &["sequence", "add", "remove", "move", "promote", "suppress", "unsuppress", "candidates", "analysed"];
    run_write(s, p, "vertical.setWalls", &[lists, &["useEditPoints", "fromMarkers", "onConflict", "expectWalls"], CHECKED], parse_set_walls)
}

fn parse_set_walls(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let mut add = Vec::new();
    for (index, value) in a.list("add", v::MAX_LIST)?.unwrap_or_default().iter().enumerate() {
        add.push(match value {
            Value::Object(_) => {
                let wall = a.element("add", value, index, &[&["frame", "score"]])?;
                v::WallAdd { frame: wall.need_int("frame")?, score: wall.number("score")? }
            }
            _ => v::WallAdd { frame: a.int_in("add", value)?, score: None },
        });
    }
    let mut moves = Vec::new();
    for (index, value) in a.list("move", v::MAX_LIST)?.unwrap_or_default().iter().enumerate() {
        let wall = a.element("move", value, index, &[&["from", "to"]])?;
        moves.push(v::WallMove { from: wall.need_int("from")?, to: wall.need_int("to")? });
    }
    let candidates = match a.object("candidates", &[&["fromFrame", "toFrame", "set"]])? {
        None => None,
        Some(c) => {
            let mut set = Vec::new();
            for (index, value) in c.list("set", v::walls::MAX_CANDIDATES)?.unwrap_or_default().iter().enumerate() {
                let item = c.element("set", value, index, &[&["frame", "score"]])?;
                set.push(v::CandidateAt { frame: item.need_int("frame")?, score: item.need_number("score")? });
            }
            Some(v::CandidateSet { from_frame: c.need_int("fromFrame")?, to_frame: c.need_int("toFrame")?, set })
        }
    };
    let from_markers = match a.object("fromMarkers", &[&["name", "color", "kind"]])? {
        None => None,
        Some(m) => Some(v::MarkerFilter { name: m.owned("name")?, color: color(&m)?.flatten(), kind: m.name("kind", MARKER_KINDS)? }),
    };
    let write = v::SetWalls {
        add,
        remove: a.frames("remove")?.unwrap_or_default(),
        moves,
        promote: a.frames("promote")?.unwrap_or_default(),
        suppress: a.frames("suppress")?.unwrap_or_default(),
        unsuppress: a.frames("unsuppress")?.unwrap_or_default(),
        candidates,
        analysed: a.pairs("analysed")?,
        use_edit_points: a.flag("useEditPoints")?,
        from_markers,
        on_conflict: a.name("onConflict", CONFLICTS)?.unwrap_or_default(),
        expect: a.owned("expect")?,
        expect_walls: a.owned("expectWalls")?,
    };
    Ok(Parsed::of(v::Write::SetWalls(write)))
}

fn apply_measurement(s: &mut Session, p: &Value) -> Result<Value> {
    let keys: &[&[&str]] = &[&["sequence", "measurement", "fromFrame", "toFrame", "onConflict", "expectWalls", "dryRun"]];
    run_write(s, p, "vertical.applyMeasurement", keys, parse_apply_measurement)
}

/// `vertical.applyMeasurement` (spec §4.2): the session's measurement `measurement` of the sequence, refused
/// `measure_running {job}` while its job runs and `stale_measurement` when the picture (its digest) or the media pool (its
/// generation) changed since it was taken; the detail names what changed, with the measured and the current values.
fn parse_apply_measurement(s: &Session, a: &Params, t: &Target) -> Result<Parsed> {
    let id = a.text("measurement")?.ok_or_else(|| a.missing("measurement"))?;
    let store = &s.vertical_store.measurements;
    if let Some(job) = store.running_job(id) {
        return Err(a.refuse("measure_running", "the measurement is still running: wait for its job, or cancel it", json!({"job": job})));
    }
    let Some(m) = store.get(t.seq, id) else {
        let message = format!("the session keeps no measurement {id:?} of this sequence (it keeps the last 4)");
        return Err(a.refuse("not_found", message, json!({"measurement": id})));
    };
    let (picture, media) = (measure::picture_digest(&s.project, t.seq), s.media.generation());
    let moved = [("picture", m.picture_digest != picture), ("media", m.media_generation != media)];
    let changed: Vec<&str> = moved.iter().filter(|(_, x)| *x).map(|(what, _)| *what).collect();
    if !changed.is_empty() {
        let message = "the picture or the media changed since the measurement: measure again";
        let detail = json!({
            "measurement": id,
            "changed": changed,
            "pictureDigest": {"measured": m.picture_digest, "current": picture},
            "mediaGeneration": {"measured": m.media_generation, "current": media},
        });
        return Err(a.refuse("stale_measurement", message, detail));
    }
    let measurement = v::Measurement { from_frame: m.from_frame, to_frame: m.to_frame, cuts: measured(&m.cuts), candidates: measured(&m.candidates) };
    let write = v::ApplyMeasurement {
        measurement,
        from_frame: a.int("fromFrame")?,
        to_frame: a.int("toFrame")?,
        on_conflict: a.name("onConflict", CONFLICTS)?.unwrap_or_default(),
        expect_walls: a.owned("expectWalls")?,
    };
    Ok(Parsed::of(v::Write::ApplyMeasurement(write)))
}

/// A measurement's cuts (or possible cuts) as the planner takes them.
fn measured(cuts: &[measure::MeasuredCut]) -> Vec<v::MeasuredFrame> {
    cuts.iter().map(|c| v::MeasuredFrame { frame: c.frame, score: c.score }).collect()
}

fn seed_shots(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.seedShots", &[PLAN, CHECKED, &["fromFrame", "toFrame", "mode"]], parse_seed_shots)
}

fn parse_seed_shots(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let mode = a.name("mode", SEED_MODES)?.ok_or_else(|| a.missing("mode"))?;
    let write = v::SeedShots { plan, from_frame: a.int("fromFrame")?, to_frame: a.int("toFrame")?, mode, expect: a.owned("expect")? };
    Ok(Parsed::of(v::Write::SeedShots(write)))
}

fn set_parts(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.setParts", &[&["sequence", "add", "update", "remove", "fromMarkers", "moveEdges", "onConflict"], CHECKED], parse_set_parts)
}

fn parse_set_parts(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let mut add = Vec::new();
    for (index, value) in a.list("add", v::MAX_LIST)?.unwrap_or_default().iter().enumerate() {
        let part = a.element("add", value, index, &[&["name", "fromFrame", "toFrame", "audio", "meta"]])?;
        let (name, from_frame, to_frame) = (part.need_text("name")?, part.need_int("fromFrame")?, part.need_int("toFrame")?);
        add.push(v::PartAdd { name, from_frame, to_frame, audio: part_audio(&part)?, meta: part_meta(&part)? });
    }
    let mut update = Vec::new();
    for (index, value) in a.list("update", v::MAX_LIST)?.unwrap_or_default().iter().enumerate() {
        let part = a.element("update", value, index, &[&["part", "name", "fromFrame", "toFrame", "audio", "meta"]])?;
        let audio = if part.present("audio") { Some(part_audio(&part)?) } else { None };
        let (from_frame, to_frame) = (part.int("fromFrame")?, part.int("toFrame")?);
        update.push(v::PartUpdate { part: part.need_id("part")?, name: part.owned("name")?, from_frame, to_frame, audio, meta: part_meta(&part)? });
    }
    let from_markers = match a.object("fromMarkers", &[&["kind", "color"]])? {
        None => None,
        Some(m) => {
            if m.text("kind")?.is_some_and(|kind| kind != "segmentation") {
                return Err(m.out_of_range("kind", "parts come from segmentation markers"));
            }
            Some(v::PartMarkers { color: color(&m)?.flatten() })
        }
    };
    let mut move_edges = Vec::new();
    for (index, value) in a.list("moveEdges", v::MAX_LIST)?.unwrap_or_default().iter().enumerate() {
        let edge = a.element("moveEdges", value, index, &[&["frame", "to"]])?;
        move_edges.push(v::EdgeMove { frame: edge.need_int("frame")?, to: edge.need_int("to")? });
    }
    let write = v::SetParts {
        add,
        update,
        remove: a.ids("remove")?.unwrap_or_default(),
        from_markers,
        move_edges,
        on_conflict: a.name("onConflict", CONFLICTS)?.unwrap_or_default(),
        expect: a.owned("expect")?,
    };
    Ok(Parsed::of(v::Write::SetParts(write)))
}

/// A part's own edge fades `{fadeInFrames, fadeOutFrames}`; None when absent or null.
fn part_audio(part: &Params) -> Result<Option<v::PartAudio>> {
    let Some(audio) = part.object("audio", &[&["fadeInFrames", "fadeOutFrames"]])? else { return Ok(None) };
    let fade = |key: &str| audio.whole(key)?.ok_or_else(|| audio.missing(key));
    Ok(Some(v::PartAudio { fade_in_frames: fade("fadeInFrames")?, fade_out_frames: fade("fadeOutFrames")? }))
}

/// A part's file metadata `{title, description}`; None when absent or null.
fn part_meta(part: &Params) -> Result<Option<v::PartMeta>> {
    let Some(meta) = part.object("meta", &[&["title", "description"]])? else { return Ok(None) };
    Ok(Some(v::PartMeta { title: meta.owned("title")?.unwrap_or_default(), description: meta.owned("description")?.unwrap_or_default() }))
}

fn set_regions(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.setRegions", &[&["sequence", "add", "update", "remove"], CHECKED], parse_set_regions)
}

fn parse_set_regions(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let mut add = Vec::new();
    for (index, value) in a.list("add", v::MAX_LIST)?.unwrap_or_default().iter().enumerate() {
        let region = a.element("add", value, index, &[&["label", "fromFrame", "toFrame", "rect", "mode"]])?;
        let (label, from_frame, to_frame) = (region.need_text("label")?, region.need_int("fromFrame")?, region.need_int("toFrame")?);
        let rect = region.need_rect("rect")?;
        let mode = region.name("mode", REGION_MODES)?.ok_or_else(|| region.missing("mode"))?;
        add.push(v::RegionAdd { label, from_frame, to_frame, rect, mode });
    }
    let mut update = Vec::new();
    for (index, value) in a.list("update", v::MAX_LIST)?.unwrap_or_default().iter().enumerate() {
        let region = a.element("update", value, index, &[&["region", "label", "fromFrame", "toFrame", "rect", "mode"]])?;
        update.push(v::RegionUpdate {
            region: region.need_id("region")?,
            label: region.owned("label")?,
            from_frame: region.int("fromFrame")?,
            to_frame: region.int("toFrame")?,
            rect: region.numbers::<4>("rect")?,
            mode: region.name("mode", REGION_MODES)?,
        });
    }
    let write = v::SetRegions { add, update, remove: a.ids("remove")?.unwrap_or_default(), expect: a.owned("expect")? };
    Ok(Parsed::of(v::Write::SetRegions(write)))
}

fn mark_checked(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.markChecked", &[PLAN, GUARDED, &["positions"]], parse_mark_checked)
}

fn parse_mark_checked(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let expect_range = range_guard(a)?;
    let range = expect_range.as_ref().map(|g| [g.from_frame, g.to_frame]);
    let write = v::MarkChecked { plan, positions: a.ids("positions")?, expect: a.owned("expect")?, expect_range };
    Ok(Parsed { range, ..Parsed::of(v::Write::MarkChecked(write)) })
}

fn repair(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.repair", &[PLAN, &["dryRun"]], parse_repair)
}

fn parse_repair(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    Ok(Parsed::of(v::Write::Repair(v::Repair { plan: a.need_id("plan")? })))
}

fn set_annotations(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.setAnnotations", &[&["sequence", "add", "update", "remove", "replace"], CHECKED], parse_set_annotations)
}

fn parse_set_annotations(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let mut add = Vec::new();
    for (index, value) in a.list("add", v::MAX_ANNOTATIONS)?.unwrap_or_default().iter().enumerate() {
        let item = a.element("add", value, index, &[&["kind", "fromFrame", "toFrame", "label", "score"]])?;
        let (kind, from_frame, to_frame) = (item.need_text("kind")?, item.need_int("fromFrame")?, item.need_int("toFrame")?);
        add.push(v::AnnotationAdd { kind, from_frame, to_frame, label: item.owned("label")?.unwrap_or_default(), score: item.number("score")? });
    }
    let mut update = Vec::new();
    for (index, value) in a.list("update", v::MAX_LIST)?.unwrap_or_default().iter().enumerate() {
        let item = a.element("update", value, index, &[&["id", "kind", "fromFrame", "toFrame", "label", "score"]])?;
        let score = if item.present("score") { Some(item.number("score")?) } else { None };
        let (from_frame, to_frame) = (item.int("fromFrame")?, item.int("toFrame")?);
        update.push(v::AnnotationUpdate { id: item.need_id("id")?, kind: item.owned("kind")?, from_frame, to_frame, label: item.owned("label")?, score });
    }
    let replace = match a.object("replace", &[&["kind", "fromFrame", "toFrame", "set"]])? {
        None => None,
        Some(r) => {
            let mut set = Vec::new();
            for (index, value) in r.list("set", v::MAX_ANNOTATIONS)?.unwrap_or_default().iter().enumerate() {
                let item = r.element("set", value, index, &[&["fromFrame", "toFrame", "label", "score"]])?;
                let (from_frame, to_frame) = (item.need_int("fromFrame")?, item.need_int("toFrame")?);
                set.push(v::AnnotationItem { from_frame, to_frame, label: item.owned("label")?.unwrap_or_default(), score: item.number("score")? });
            }
            Some(v::AnnotationReplace { kind: r.need_text("kind")?, from_frame: r.need_int("fromFrame")?, to_frame: r.need_int("toFrame")?, set })
        }
    };
    let write = v::SetAnnotations { add, update, remove: a.ids("remove")?.unwrap_or_default(), replace, expect: a.owned("expect")? };
    Ok(Parsed::of(v::Write::SetAnnotations(write)))
}

fn set_active_area(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.setActiveArea", &[&["sequence", "set", "clear"], CHECKED], parse_set_active_area)
}

fn parse_set_active_area(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let mut set = Vec::new();
    for (index, value) in a.list("set", v::MAX_AREAS)?.unwrap_or_default().iter().enumerate() {
        let area = a.element("set", value, index, &[&["fromFrame", "toFrame", "rect"]])?;
        set.push(v::AreaSet { from_frame: area.need_int("fromFrame")?, to_frame: area.need_int("toFrame")?, rect: area.need_rect("rect")? });
    }
    let write = v::SetActiveArea { set, clear: a.pairs("clear")?, expect: a.owned("expect")? };
    Ok(Parsed::of(v::Write::SetActiveArea(write)))
}

fn approve_part(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.approvePart", &[PLAN, &["part", "note", "dryRun"]], parse_approve_part)
}

/// `vertical.approvePart`: `by` is the editor-name preference and `when` the session's clock (spec §4.6).
fn parse_approve_part(s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let (plan, part) = (a.need_id("plan")?, a.need_id("part")?);
    let note = a.owned("note")?.unwrap_or_default();
    let write = v::ApprovePart { plan, part, note, by: s.prefs.vertical.editor_name(), when: s.now_unix() };
    Ok(Parsed::of(v::Write::ApprovePart(write)))
}

fn set_review(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.setReview", &[PLAN, &["positions", "shotFrame", "reviewed", "dryRun"]], parse_set_review)
}

fn parse_set_review(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let positions = a.ids("positions")?.unwrap_or_default();
    let shot_frame = a.int("shotFrame")?;
    let reviewed = a.flag("reviewed")?.ok_or_else(|| a.missing("reviewed"))?;
    Ok(Parsed::of(v::Write::SetReview(v::SetReview { plan, positions, shot_frame, reviewed })))
}

fn set_lock(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.setLock", &[PLAN, &["positions", "locked", "dryRun"]], parse_set_lock)
}

fn parse_set_lock(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let positions = a.need_ids("positions")?;
    let locked = a.flag("locked")?.ok_or_else(|| a.missing("locked"))?;
    Ok(Parsed::of(v::Write::SetLock(v::SetLock { plan, positions, locked })))
}

fn flag_attention(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.flagAttention", &[PLAN, TIME, &["note", "dryRun"]], parse_flag_attention)
}

/// `vertical.flagAttention`: `by` is the editor-name preference and `when` the session's clock (spec §5.10).
fn parse_flag_attention(s: &Session, a: &Params, t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let frame = a.time_frame(s, t)?.ok_or_else(|| a.missing("frame"))?;
    let note = a.owned("note")?.unwrap_or_default();
    let write = v::FlagAttention { plan, frame, note, by: s.prefs.vertical.editor_name(), when: s.now_unix() };
    Ok(Parsed::of(v::Write::FlagAttention(write)))
}

fn clear_attention(s: &mut Session, p: &Value) -> Result<Value> {
    run_write(s, p, "vertical.clearAttention", &[PLAN, &["frames", "dryRun"]], parse_clear_attention)
}

fn parse_clear_attention(_s: &Session, a: &Params, _t: &Target) -> Result<Parsed> {
    let plan = a.need_id("plan")?;
    let frames = a.frames("frames")?.ok_or_else(|| a.missing("frames"))?;
    Ok(Parsed::of(v::Write::ClearAttention(v::ClearAttention { plan, frames })))
}

/// `vertical.remap {sequence?, map, expect, dryRun?}` (spec §4.12): the framing and every frame-based datum of a sequence
/// follow a re-delivered master through the caller's map. Each entry `{fromFrame, toFrame, newFrom}` says that the old
/// frames `[fromFrame, toFrame)` now start at `newFrom`; old frames in no entry were removed, new frames in no entry are
/// new content, and every junction is a wall (the editor cannot see the new master's cuts). The model's `plan_remap`
/// runs the edit hook's machinery (spec §4.8, steps 3-12) with this map: exact splits and openers at the junctions,
/// positions remapped exactly keep `reviewed`, and removed content takes its data with it (reported; for a person a
/// locked position too, I16).
///
/// `expect` is required: the sequence's `remapDigest` (every plan's digest, the set's data digest, and the walls, parts,
/// regions and conform reference a remap moves, which a set can hold before it has a plan), so a retried remap is
/// refused `stale_plan {digest}`. The map is read strictly: a list of at most `MAX_MAP` entries (`too_many {key:
/// "map"}`, before any is read), each an object of whole frames (`out_of_range {key, index}`); the model refuses `bad_map
/// {entry, reason}` and `invalid_plan`. As every write, it plans on the working copy (plan 63, C-11) as the session's
/// writer; a dry run returns the real result and records nothing, a map that changes nothing (the identity) records
/// nothing, and a change is one plan-only undo step (no edit hook, no picture change).
fn remap(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "vertical.remap";
    measure::poll(s);
    let a = Params::of(CMD, p, &[&["sequence", "map", "expect", "dryRun"]])?;
    let seq = sequence_of(s, &a)?;
    let dry = a.flag("dryRun")?.unwrap_or(false);
    let map = remap_entries(&a)?;
    let expect = a.text("expect")?.ok_or_else(|| a.missing("expect"))?;
    let project = Arc::clone(&s.project);
    let q = sequence_in(CMD, &project, seq)?;
    let digest = v::remap_digest(q);
    if expect != digest {
        return Err(a.refuse("stale_plan", "the sequence's vertical data changed since it was read", json!({"digest": digest})));
    }
    // a sequence without vertical data has nothing to follow; the model checks the map all the same
    let copy = if q.vertical.is_some() { s.vertical_working_copy(seq).ok_or_else(|| not_found_sequence(CMD, seq))? } else { q.clone() };
    let caller = Caller { writer: s.writer, overwrite_allowed: false };
    let mut planned = v::plan_remap(&project, &copy, &map, caller).map_err(|r| EngineError::invalid(CMD, r))?;
    let mut after = copy;
    after.vertical = planned.set.take();
    let mut out = base_envelope(seq, &planned, dry);
    out["digestBefore"] = json!(digest);
    out["digest"] = json!(v::remap_digest(&after));
    if dry || !planned.changed {
        return Ok(out);
    }
    let label = planned.label.clone();
    let set = after.vertical.take();
    let revision = s.revision;
    s.edit_plan_only(seq, &label, None, move |_, slot, _| {
        *slot = set;
        Ok(())
    })?;
    if s.revision == revision {
        // the session found nothing to record but id counters: the remap changed nothing after all
        out["changed"] = json!(false);
        return Ok(out);
    }
    out["applied"] = json!(true);
    out["label"] = json!(label);
    keep_validity(s, &project, seq);
    keep_selection(s);
    Ok(out)
}

/// The map of `vertical.remap`: a list of at most [`v::MAX_MAP`] entries (`too_many {key: "map"}`, before any is read),
/// each an object of whole frames `{fromFrame, toFrame, newFrom}` (`out_of_range` with the entry's `index` otherwise).
/// Their order and ranges are the model's to check (`bad_map`).
fn remap_entries(a: &Params) -> Result<Vec<v::RemapEntry>> {
    let items = a.list("map", v::MAX_MAP)?.ok_or_else(|| a.missing("map"))?;
    let mut map = Vec::with_capacity(items.len());
    for (index, value) in items.iter().enumerate() {
        let e = a.element("map", value, index, &[&["fromFrame", "toFrame", "newFrom"]])?;
        map.push(v::RemapEntry { from_frame: e.need_int("fromFrame")?, to_frame: e.need_int("toFrame")?, new_from: e.need_int("newFrom")? });
    }
    Ok(map)
}

// ---------- reads ----------

/// A plan as the reads show it: its sequence, its raster and the heights it allows.
struct Shown<'a> {
    q: &'a Sequence,
    plan: &'a VerticalPlan,
    r: Raster,
    limits: Limits,
}

impl<'a> Shown<'a> {
    fn new(q: &'a Sequence, plan: &'a VerticalPlan) -> Self {
        let r = v::raster(q);
        Self { q, plan, r, limits: v::limits(&r, plan.output) }
    }
}

/// The timecode of sequence frame `frame` (the sequence's start timecode counted in).
fn timecode(q: &Sequence, frame: i64) -> String {
    format_timecode_frames(frame.saturating_add(q.start_timecode), q.settings.frame_rate, q.settings.drop_frame)
}

/// The reduced ratio of an output: "9:16".
fn aspect(out: v::OutputSize) -> String {
    let (mut a, mut b) = (out.width.max(1), out.height.max(1));
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let gcd = a.max(1);
    format!("{}:{}", out.width / gcd, out.height / gcd)
}

/// A curve's name in the write shape, and its parameters under their own key.
fn curve_into(out: &mut Map<String, Value>, curve: &v::Curve) {
    let (name, parameters) = match *curve {
        v::Curve::Linear => ("linear", None),
        v::Curve::EaseIn => ("easeIn", None),
        v::Curve::EaseOut => ("easeOut", None),
        v::Curve::EaseInOut => ("easeInOut", None),
        v::Curve::Speeds { start, end } => ("speeds", Some(json!([start, end]))),
        v::Curve::Bezier { x1, y1, x2, y2 } => ("bezier", Some(json!([x1, y1, x2, y2]))),
        v::Curve::Ramp { r_in, r_out } => ("ramp", Some(json!([r_in, r_out]))),
    };
    out.insert("curve".into(), json!(name));
    if let Some(parameters) = parameters {
        out.insert(name.into(), parameters);
    }
}

/// A position in the read shape (spec §7.4): every stored field, its frame, tick and timecode, its window's rectangle,
/// zoom and upscale, its arrival in the write shape, and what is derived from it.
fn position_json(shown: &Shown, p: &v::Position, info: Option<&v::PositionInfo>) -> Value {
    let rate = shown.r.rate;
    let frame = rate.frame_at(p.at);
    let out_size = shown.plan.output;
    let rect = p.window.rect(out_size);
    let upscale = match p.layout {
        v::Layout::Fill => p.window.upscale(out_size),
        v::Layout::Fit => f64::from(out_size.width) / f64::from(shown.r.width.max(1)),
    };
    let mut out = json!({
        "id": p.id,
        "frame": frame,
        "tick": p.at.0.to_string(),
        "timecode": timecode(shown.q, frame),
        "cx": p.window.cx,
        "cy": p.window.cy,
        "h": p.window.h,
        "zoom": v::zoom_percent(shown.limits.max_height, p.window.h),
        "rect": {"x": rect.x, "y": rect.y, "w": rect.w, "h": rect.h},
        "layout": p.layout,
        "upscale": upscale,
        "arrival": "cut",
        "moveFrames": null,
        "curve": null,
        "splitOf": null,
        "captionAnchor": p.caption_anchor,
        "label": p.label,
        "color": p.color,
        "tags": p.tags,
        "note": p.note,
        "subjects": p.subjects,
        "focus": p.focus,
        "confidence": p.confidence,
        "risk": p.risk,
        "origin": p.origin,
        "modifiedBy": p.modified_by,
        "reviewed": p.reviewed,
        "locked": p.locked,
        "flags": p.flags,
        "proposed": p.proposed,
    });
    if let Some(map) = out.as_object_mut() {
        if let v::Arrival::Move { move_frames, curve, split_of } = p.arrival {
            map.insert("arrival".into(), json!("move"));
            map.insert("moveFrames".into(), json!(move_frames));
            map.insert("splitOf".into(), json!(split_of));
            curve_into(map, &curve);
        }
        if let Some(info) = info {
            map.insert("tag".into(), json!(info.tag));
            map.insert("fit".into(), json!(info.fit));
            map.insert("shot".into(), json!(info.shot.saturating_add(1)));
            map.insert("moving".into(), json!(info.moving));
            map.insert("static".into(), json!(info.still));
            map.insert("span".into(), json!(info.span));
            map.insert("holdFrames".into(), json!(info.hold_frames));
            map.insert("peakSpeed".into(), json!(info.peak_speed));
            map.insert("curveLabel".into(), json!(info.curve_label));
        }
    }
    out
}

/// The store's record of what playback showed of shot `[from, to)` of plan `plan` of sequence `seq`.
fn watch_record(s: &Session, seq: ItemId, plan: u32, from: i64, to: i64) -> Option<&WatchRecord> {
    s.vertical_store.watched.get(&(seq, plan))?.iter().find(|r| r.from == from && r.to == to)
}

/// Whether playback showed shot `[from, to)` of a plan (spec §5.10): the store's record for the shot holds for the
/// shot's current range digest and covers all of its frames but at most two.
fn is_watched(s: &Session, project: &Project, q: &Sequence, seq: ItemId, plan: &VerticalPlan, from: i64, to: i64) -> bool {
    let Some(record) = watch_record(s, seq, plan.id, from, to) else { return false };
    covered(&record.frames).saturating_add(WATCH_SLACK) >= to.saturating_sub(from) && record.digest == v::range_digest(project, q, plan, from, to)
}

/// How many frames merged half-open ranges cover.
fn covered(ranges: &[[i64; 2]]) -> i64 {
    ranges.iter().map(|[from, to]| to.saturating_sub(*from)).fold(0, i64::saturating_add)
}

/// A shot of a plan in the read shape: `watched`, and in `watch` the store's record behind it (spec §7.4: the range
/// digest it holds for, the frames playback showed, the pass that last noted them), null without one.
fn shot_json(shot: &v::Shot, watched: bool, record: Option<&WatchRecord>) -> Value {
    let watch = record.map(|r| json!({"digest": r.digest, "frames": covered(&r.frames), "pass": r.pass}));
    json!({
        "number": shot.number,
        "fromFrame": shot.from,
        "toFrame": shot.to,
        "sources": shot.sources,
        "status": shot.status,
        "opener": shot.opener,
        "carriedFrom": shot.carried_from,
        "positions": shot.positions,
        "reviewed": shot.reviewed,
        "flagged": shot.flagged,
        "attention": shot.attention,
        "watched": watched,
        "watch": watch,
    })
}

/// `fromFrame` and `toFrame` of a read inside the sequence (by default all of it): `out_of_range` otherwise.
fn read_range(a: &Params, end: i64) -> Result<[i64; 2]> {
    let from = a.int("fromFrame")?.unwrap_or(0);
    let to = a.int("toFrame")?.unwrap_or(end);
    if from < 0 || from > to {
        let detail = json!({"key": "fromFrame", "fromFrame": from, "toFrame": to});
        return Err(a.refuse("out_of_range", "the range runs from fromFrame to toFrame, inside the sequence", detail));
    }
    if to > end {
        return Err(a.refuse("out_of_range", format!("the sequence ends at frame {end}"), json!({"key": "toFrame", "toFrame": to, "end": end})));
    }
    Ok([from, to])
}

/// `vertical.list` (spec §7.4): every plan with its output, its position count, its digest, whether it is valid, its
/// issue counts and its parts' statuses, the parts, and the guards' digests.
fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.list";
    measure::poll(s);
    let a = Params::of(cmd, p, &[&["sequence"]])?;
    let seq = sequence_of(s, &a)?;
    refresh(s, seq);
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let empty = crate::empty_vertical_set();
    let set = q.vertical.as_deref().unwrap_or(&empty);
    let r = v::raster(q);
    let mut plans = Vec::with_capacity(set.plans.len());
    for plan in &set.plans {
        let valid = plan_violations(s, seq, plan.id).is_empty();
        // a plan that breaks an invariant is not evaluated: its one issue is `invalid_plan`
        let issues = if valid { v::issues(&project, q, plan.id, 0, r.end_frame) } else { Vec::new() };
        let count = |level: Level| issues.iter().filter(|issue| issue.level == level).count();
        let counts = json!({"error": count(Level::Error) + usize::from(!valid), "warning": count(Level::Warning), "info": count(Level::Info)});
        let status = |part: &v::Part| v::part_status(&project, q, plan.id, part.id).map(|status| json!({"part": part.id, "status": status}));
        let parts: Vec<Value> = if valid { set.parts.iter().filter_map(status).collect() } else { Vec::new() };
        plans.push(json!({
            "id": plan.id,
            "name": plan.name,
            "output": plan.output,
            "aspect": aspect(plan.output),
            "positions": plan.positions.len(),
            "valid": valid,
            "digest": v::plan_digest(q, plan),
            "issues": counts,
            "parts": parts,
        }));
    }
    let rate = r.rate;
    let edges = |part: &v::Part| json!({"id": part.id, "name": part.name, "fromFrame": rate.frame_at(part.from), "toFrame": rate.frame_at(part.to)});
    let parts: Vec<Value> = set.parts.iter().map(edges).collect();
    Ok(json!({
        "sequence": seq.0,
        "plans": plans,
        "parts": parts,
        "wallsDigest": v::walls_digest(set),
        "effectiveWallsDigest": v::effective_walls_digest(&project, q),
        "dataDigest": v::data_digest(set),
        "partsDigest": v::parts_digest(set),
        "regionsDigest": v::regions_digest(set),
    }))
}

/// `vertical.inspect` (spec §7.4): the plan's settings and limits, the walls with every source, the stored wall data,
/// every position in the read shape, the shots with their status, review counts, `watched` and attention marks, the parts
/// with their status and approval, the set's regions, annotations and active areas, the framing library, the positions an
/// applied measurement left off a wall, the frames where shots are not known, the issues and the digests. `fromFrame` and
/// `toFrame` narrow the positions, the shots and the issues. Unpaged.
fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.inspect";
    measure::poll(s);
    let a = Params::of(cmd, p, &[PLAN, &["fromFrame", "toFrame"]])?;
    let seq = sequence_of(s, &a)?;
    let id = a.need_id("plan")?;
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let plan = plan_of(cmd, q, id)?;
    let shown = Shown::new(q, plan);
    let [from, to] = read_range(&a, shown.r.end_frame)?;
    ensure_valid(s, cmd, seq, id)?;
    let derived = v::derive_plan(&project, q, id).ok_or_else(|| refuse(cmd, "not_found", format!("the sequence has no plan {id}"), json!({"plan": id})))?;
    let empty = crate::empty_vertical_set();
    let set = q.vertical.as_deref().unwrap_or(&empty);
    let rate = shown.r.rate;
    let frame_of = |t: Tick| rate.frame_at(t);
    let narrowed = a.given("fromFrame") || a.given("toFrame");
    let mut positions = Vec::new();
    for (p, info) in plan.positions.iter().zip(&derived.positions) {
        if (from..to).contains(&info.frame) {
            positions.push(position_json(&shown, p, Some(info)));
        }
    }
    let shown_shot = |shot: &v::Shot| shot_json(shot, is_watched(s, &project, q, seq, plan, shot.from, shot.to), watch_record(s, seq, id, shot.from, shot.to));
    let shots: Vec<Value> = derived.shots.iter().filter(|shot| shot.from < to && from < shot.to).map(shown_shot).collect();
    let walls: Vec<Value> = derived.walls.iter().map(|w| json!({"frame": frame_of(w.at), "sources": w.sources, "score": w.score})).collect();
    let wall_frames: Vec<i64> = derived.walls.iter().map(|w| frame_of(w.at)).collect();
    let stored = &set.walls;
    let suppressed: Vec<i64> = stored.suppressed.iter().map(|t| frame_of(*t)).collect();
    let candidates: Vec<Value> = stored.candidates.iter().map(|c| json!({"frame": frame_of(c.at), "score": c.score})).collect();
    let analysed: Vec<[i64; 2]> = stored.analysed.iter().map(|r| [frame_of(r.from), frame_of(r.to)]).collect();
    let status = |part: u32| derived.parts.iter().find(|state| state.part == part).map(|state| state.status);
    let approval = |part: u32| plan.part_approvals.iter().find(|x| x.part == part);
    let parts: Vec<Value> = set.parts.iter().map(|part| part_json(part, rate, status(part.id), approval(part.id))).collect();
    let regions: Vec<Value> = set.regions.iter().map(|g| region_json(g, rate)).collect();
    let annotations: Vec<Value> = set.annotations.iter().map(|n| annotation_json(n, rate)).collect();
    let areas: Vec<Value> = set.active_area.iter().map(|area| area_json(area, rate)).collect();
    // box tracks as stored, without their keys (their detail is agent integration's, M5): empty in manual use
    let tracks: Vec<Value> = set.box_tracks.iter().map(track_json).collect();
    let attention: Vec<Value> = plan.attention.iter().map(|m| json!({"frame": frame_of(m.at), "by": m.by, "when": m.when, "note": m.note})).collect();
    let unknown = unknown_frames(s, &project, seq, q);
    let unanchored = unanchored(s, seq, plan, rate, &wall_frames, &unknown);
    let issues = if narrowed { v::issues(&project, q, id, from, to) } else { derived.issues.clone() };
    let mut out = json!({
        "sequence": seq.0,
        "plan": id,
        "name": plan.name,
        "output": plan.output,
        "aspect": aspect(plan.output),
        "filter": plan.filter,
        "fitBackground": plan.fit_background,
        "startingWindow": plan.starting_window,
        "captions": plan.captions,
        "limits": derived.limits,
        "digest": v::plan_digest(q, plan),
        "wallsDigest": v::walls_digest(set),
        "effectiveWallsDigest": v::effective_walls_digest(&project, q),
        "useEditPoints": stored.use_edit_points,
        "walls": walls,
        "suppressed": suppressed,
        "candidates": candidates,
        "analysed": analysed,
        "positions": positions,
        "shots": shots,
        "parts": parts,
        "regions": regions,
        "annotations": annotations,
        "activeAreas": areas,
        "boxTracks": tracks,
        "attention": attention,
        "library": derived.library,
        "overwritten": plan.overwritten,
        "agentOverwrite": set.agent_overwrite,
        "partAudio": set.part_audio,
        "unanchoredSinceMeasurement": unanchored,
        "shotsUnknown": unknown,
        "issues": issues,
    });
    if narrowed {
        out["fromFrame"] = json!(from);
        out["toFrame"] = json!(to);
    }
    Ok(out)
}

/// The positions an applied measurement left off a wall (spec §4.4): those made while shots were unknown that still
/// stand off every wall (frame 0 opens a shot), where shots are known now.
fn unanchored(s: &Session, seq: ItemId, plan: &VerticalPlan, rate: FrameRate, walls: &[i64], unknown: &[[i64; 2]]) -> Vec<u32> {
    let Some(ids) = s.vertical_store.unanchored.get(&(seq, plan.id)) else { return Vec::new() };
    let off = |p: &&v::Position| {
        let frame = rate.frame_at(p.at);
        ids.contains(&p.id) && frame != 0 && walls.binary_search(&frame).is_err() && !within(unknown, frame)
    };
    plan.positions.iter().filter(off).map(|p| p.id).collect()
}

/// A part in the read shape, with its status in the plan and the plan's approval of it.
fn part_json(part: &v::Part, rate: FrameRate, status: Option<v::PartStatus>, approval: Option<&v::PartApproval>) -> Value {
    let approval = approval.map(|a| json!({"by": a.by, "when": a.when, "note": a.note, "framingDigest": a.framing_digest, "outputDigest": a.output_digest}));
    json!({
        "id": part.id,
        "name": part.name,
        "fromFrame": rate.frame_at(part.from),
        "toFrame": rate.frame_at(part.to),
        "status": status,
        "approval": approval,
        "audio": part.audio,
        "meta": part.meta,
    })
}

/// A protected region in the read shape.
fn region_json(g: &v::ProtectedRegion, rate: FrameRate) -> Value {
    json!({
        "id": g.id,
        "label": g.label,
        "fromFrame": rate.frame_at(g.from),
        "toFrame": rate.frame_at(g.to),
        "rect": g.rect,
        "mode": g.mode,
        "origin": g.origin,
        "modifiedBy": g.modified_by,
    })
}

/// An active picture area in the read shape.
fn area_json(area: &v::ActiveArea, rate: FrameRate) -> Value {
    json!({
        "fromFrame": rate.frame_at(area.from),
        "toFrame": rate.frame_at(area.to),
        "rect": area.rect,
        "origin": area.origin,
        "modifiedBy": area.modified_by,
    })
}

/// A box track in the read shape (spec §7.4: ids, labels, kinds and key counts).
fn track_json(t: &v::BoxTrack) -> Value {
    json!({"id": t.id, "label": t.label, "kind": t.kind, "keys": t.keys.len(), "origin": t.origin, "modifiedBy": t.modified_by})
}

/// A time annotation in the read shape.
fn annotation_json(n: &v::Annotation, rate: FrameRate) -> Value {
    json!({
        "id": n.id,
        "kind": n.kind,
        "fromFrame": rate.frame_at(n.from),
        "toFrame": rate.frame_at(n.to),
        "label": n.label,
        "score": n.score,
        "origin": n.origin,
        "modifiedBy": n.modified_by,
    })
}

/// `vertical.windows` (spec §7.4): the exact window of every `step`th frame of `[fromFrame, toFrame)`, columnar, one
/// column per field asked (by default every field but `focus` and `active`); `layout` as a string of `f` (fill) and `F`
/// (fit), `state` as a string of the model's state characters. Unpaged, so one call holds at most
/// [`MAX_WINDOW_NUMBERS`] numbers: `too_many {key: "frames", count, bound}` names how many rows the fields asked allow.
fn windows(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.windows";
    measure::poll(s);
    let a = Params::of(cmd, p, &[PLAN, &["fromFrame", "toFrame", "step", "fields"]])?;
    let seq = sequence_of(s, &a)?;
    let id = a.need_id("plan")?;
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let plan = plan_of(cmd, q, id)?;
    if !a.given("fromFrame") {
        return Err(a.missing("fromFrame"));
    }
    if !a.given("toFrame") {
        return Err(a.missing("toFrame"));
    }
    let [from, to] = read_range(&a, v::raster(q).end_frame)?;
    let step = a.int("step")?.unwrap_or(1);
    if step < 1 {
        return Err(a.out_of_range("step", "`step` is at least 1"));
    }
    let fields: Vec<(&str, Field)> = match a.list("fields", FIELDS.len())? {
        None => FIELDS.iter().copied().filter(|(_, field)| !matches!(field, Field::Focus | Field::Active)).collect(),
        Some(names) => {
            let field = |name: &Value| FIELDS.iter().copied().find(|(known, _)| name.as_str() == Some(*known));
            names.iter().map(|name| field(name).ok_or_else(|| a.out_of_range("fields", "unknown field"))).collect::<Result<Vec<_>>>()?
        }
    };
    let rows = to.saturating_sub(from).saturating_add(step - 1) / step;
    // the frame column and the fields asked
    let per_row = fields.iter().map(|(_, field)| field.numbers()).fold(1, i64::saturating_add);
    let bound = (MAX_WINDOW_NUMBERS / per_row).min(i64::from(v::VE_MAX_FRAMES));
    if rows > bound {
        let message = format!("one call reads at most {bound} frames with these fields: pass a step, fewer fields or a shorter range");
        return Err(a.refuse("too_many", message, json!({"key": "frames", "count": rows, "bound": bound})));
    }
    ensure_valid(s, cmd, seq, id)?;
    let rows = v::frame_rows(&project, q, id, from, to, step);
    let column = |f: &dyn Fn(&v::FrameRow) -> Value| Value::Array(rows.iter().map(f).collect());
    let mut out = Map::new();
    out.insert("sequence".into(), json!(seq.0));
    out.insert("plan".into(), json!(id));
    out.insert("digest".into(), json!(v::plan_digest(q, plan)));
    out.insert("fromFrame".into(), json!(from));
    out.insert("toFrame".into(), json!(to));
    out.insert("step".into(), json!(step));
    out.insert("frame".into(), column(&|r| json!(r.frame)));
    for (name, field) in fields {
        let values = match field {
            Field::Tick => column(&|r| json!(r.tick.0.to_string())),
            Field::X => column(&|r| json!(r.rect.x)),
            Field::Y => column(&|r| json!(r.rect.y)),
            Field::W => column(&|r| json!(r.rect.w)),
            Field::H => column(&|r| json!(r.rect.h)),
            Field::Cx => column(&|r| json!(r.window.cx)),
            Field::Cy => column(&|r| json!(r.window.cy)),
            Field::Layout => json!(rows.iter().map(|r| if r.layout == v::Layout::Fit { 'F' } else { 'f' }).collect::<String>()),
            Field::State => json!(rows.iter().map(|r| r.state).collect::<String>()),
            Field::Position => column(&|r| json!(r.position)),
            Field::U => column(&|r| json!(r.u)),
            Field::E => column(&|r| json!(r.e)),
            Field::Speed => column(&|r| json!(r.speed)),
            Field::Upscale => column(&|r| json!(r.upscale)),
            Field::Focus => column(&|r| json!(r.focus)),
            Field::Active => column(&|r| json!(r.active)),
        };
        out.insert(name.into(), values);
    }
    Ok(Value::Object(out))
}

/// `vertical.measure` (spec §3.9, §7.4): the model's measures of the plan over `[fromFrame, toFrame)`; findings only for
/// the thresholds given. `regions` are inline boxes in sequence pixels, `{source: "protected"}` (the set's protected
/// regions) or `{source: "captions"}` (the plan's laid-out subtitle boxes); `safeZone` is `"union"`, `"unionMirrored"`
/// or a rectangle in output pixels ([`safe_zone`]).
fn measure_plan(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.measure";
    measure::poll(s);
    let a = Params::of(cmd, p, &[PLAN, &["fromFrame", "toFrame", "thresholds", "regions", "safeZone", "baseline", "series"]])?;
    let seq = sequence_of(s, &a)?;
    let id = a.need_id("plan")?;
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let plan = plan_of(cmd, q, id)?;
    let range = read_range(&a, v::raster(q).end_frame)?;
    let baseline = match a.text("baseline")? {
        None => false,
        Some("centreCrop") => true,
        Some(_) => return Err(a.out_of_range("baseline", "the baseline is \"centreCrop\"")),
    };
    let mut opts = v::MeasureOpts {
        from_frame: a.int("fromFrame")?,
        to_frame: a.int("toFrame")?,
        thresholds: thresholds(&a)?,
        safe_zone: safe_zone(&a, plan.output)?,
        series: a.flag("series")?.unwrap_or(false),
        baseline,
        ..v::MeasureOpts::default()
    };
    let captions = measure_regions(&a, &mut opts)?;
    ensure_valid(s, cmd, seq, id)?;
    if captions {
        // laid out only now: the layout reads the plan's windows, and a plan that breaks an invariant is never evaluated
        opts.captions = caption_boxes(q, plan, range);
    }
    let measures = v::measure(&project, q, id, &opts).map_err(|r| EngineError::invalid(cmd, r))?;
    let mut out = serde_json::to_value(measures).map_err(|e| EngineError::Other(format!("`{cmd}`: {e}")))?;
    if let Some(map) = out.as_object_mut() {
        map.insert("sequence".into(), json!(seq.0));
        map.insert("digest".into(), json!(v::plan_digest(q, plan)));
    }
    Ok(out)
}

/// `thresholds`: the caller's thresholds; a finding exists only for a threshold given.
fn thresholds(a: &Params) -> Result<v::Thresholds> {
    let motion: &[&str] = &["maxUpscale", "maxSpeed", "maxAccel", "minHoldFrames", "pingPongFrames", "nearWallFrames", "maxSpeedStep"];
    let shots: &[&str] = &["jumpSimilar", "minMoveDistance", "minHoldAfterWall", "maxHoldFrames", "straddleFrames", "maxEyeTraceJump"];
    let Some(t) = a.object("thresholds", &[motion, shots])? else { return Ok(v::Thresholds::default()) };
    let jump_similar = match t.object("jumpSimilar", &[&["maxShift", "maxScale"]])? {
        None => None,
        Some(j) => Some(v::JumpSimilar { max_shift: j.need_number("maxShift")?, max_scale: j.need_number("maxScale")? }),
    };
    Ok(v::Thresholds {
        max_upscale: t.number("maxUpscale")?,
        max_speed: t.number("maxSpeed")?,
        max_accel: t.number("maxAccel")?,
        min_hold_frames: t.int("minHoldFrames")?,
        ping_pong_frames: t.int("pingPongFrames")?,
        near_wall_frames: t.int("nearWallFrames")?,
        max_speed_step: t.number("maxSpeedStep")?,
        jump_similar,
        min_move_distance: t.number("minMoveDistance")?,
        min_hold_after_wall: t.int("minHoldAfterWall")?,
        max_hold_frames: t.int("maxHoldFrames")?,
        straddle_frames: t.int("straddleFrames")?,
        max_eye_trace_jump: t.number("maxEyeTraceJump")?,
    })
}

/// `safeZone` (spec §5.14), in output pixels: `"union"`, the union of the platforms' overlays (safe x 120–888 and y
/// 288–1248 on 1080×1920), scaled to the output along each axis; `"unionMirrored"`, the union and its mirror about the
/// output's vertical centre line (safe whichever side a platform puts its button rail); or `[x, y, w, h]`.
/// `not_found {safeZone}` for another name: the spec gives no single platform's numbers.
fn safe_zone(a: &Params, out: v::OutputSize) -> Result<Option<[f64; 4]>> {
    let (sx, sy) = (f64::from(out.width) / 1080.0, f64::from(out.height) / 1920.0);
    let [x, y, w, h] = UNION_SAFE_ZONE;
    let union = [x * sx, y * sy, w * sx, h * sy];
    match a.value("safeZone") {
        None => Ok(None),
        Some(Value::String(name)) if name == "union" => Ok(Some(union)),
        Some(Value::String(name)) if name == "unionMirrored" => Ok(Some(mirrored(union, f64::from(out.width)))),
        Some(Value::String(name)) => {
            let message = format!("no safe zone is named {name:?}: pass \"union\", \"unionMirrored\" or [x, y, w, h] in output pixels");
            Err(a.refuse("not_found", message, json!({"safeZone": name})))
        }
        Some(value) => a.numbers_in::<4>("safeZone", value).map(Some),
    }
}

/// The part of the safe rectangle `[x, y, w, h]` that its mirror about the vertical centre line of an output `width`
/// pixels wide holds too: what is outside both overlays.
pub(crate) fn mirrored([x, y, w, h]: [f64; 4], width: f64) -> [f64; 4] {
    let (left, right) = (x.max(width - (x + w)), (x + w).min(width - x));
    [left, y, (right - left).max(0.0), h]
}

/// `regions` of `vertical.measure` into `opts`; whether they ask for the plan's caption boxes (`{source: "captions"}`),
/// which the caller lays out once it knows the plan holds every invariant.
fn measure_regions(a: &Params, opts: &mut v::MeasureOpts) -> Result<bool> {
    let Some(items) = a.list("regions", MAX_REGIONS)? else { return Ok(false) };
    let (mut boxes, mut captions) = (0_usize, false);
    for (index, value) in items.iter().enumerate() {
        let region = a.element("regions", value, index, &[&["id", "label", "space", "boxes", "interpolate", "source"]])?;
        if region.given("source") {
            match region.text("source")? {
                Some("protected") => opts.protected = true,
                Some("captions") => captions = true,
                _ => return Err(region.out_of_range("source", "a region's source is \"protected\" or \"captions\"")),
            }
            continue;
        }
        if region.text("space")?.is_some_and(|space| space != "sequence") {
            return Err(region.out_of_range("space", "regions are in sequence pixels (\"space\": \"sequence\")"));
        }
        let id = region.need_text("id")?;
        let label = region.owned("label")?.unwrap_or_default();
        let interpolate = region.flag("interpolate")?.unwrap_or(false);
        let list = region.list("boxes", MAX_BOXES)?.unwrap_or_default();
        boxes = boxes.saturating_add(list.len());
        if boxes > MAX_BOXES {
            return Err(a.refuse("too_many", "the regions hold more than 5 000 boxes", json!({"key": "boxes", "count": boxes, "bound": MAX_BOXES})));
        }
        let mut parsed = Vec::with_capacity(list.len());
        for (k, value) in list.iter().enumerate() {
            let b = region.element("boxes", value, k, &[&["frame", "rect"]])?;
            parsed.push(v::RegionBox { frame: b.need_int("frame")?, rect: b.need_rect("rect")? });
        }
        opts.regions.push(v::RegionInput { id, label, boxes: parsed, interpolate });
    }
    Ok(captions)
}

/// The plan's laid-out subtitle boxes over `[from, to)`, in output pixels.
fn caption_boxes(q: &Sequence, plan: &VerticalPlan, [from, to]: [i64; 2]) -> Vec<v::CaptionBox> {
    let rate = q.settings.frame_rate;
    let range = TimeRange::from_bounds(rate.tick_of(from), rate.tick_of(to));
    let laid = filmcraft_render::vertical::caption_boxes(q, plan, range, None);
    let shown = |b: &filmcraft_captions::burn::CaptionBox| v::CaptionBox {
        cue: b.cue.0,
        from_frame: v::shown_frame(rate, b.from),
        to_frame: v::shown_frame(rate, b.to),
        rect: b.rect.map(f64::from),
    };
    laid.iter().map(shown).collect()
}

/// `vertical.fit` (spec §7.4): the window that contains the regions given (sequence-space rectangles, or `{source:
/// "protected"}`: the protected regions at the frame that ask to be in the window) at the size given or the smallest
/// legal one, or the window of an eye trace (Match eye-trace); pure geometry.
fn fit(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.fit";
    measure::poll(s);
    let a = Params::of(cmd, p, &[PLAN, &["frame", "regions", "h", "zoom", "margin", "eyeTrace"]])?;
    let seq = sequence_of(s, &a)?;
    let id = a.need_id("plan")?;
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let plan = plan_of(cmd, q, id)?;
    let frame = a.need_int("frame")?;
    let eye_trace = match a.object("eyeTrace", &[&["fromFrame", "fromPoint", "toPoint"]])? {
        None => None,
        Some(e) => Some(v::EyeTrace { from_frame: e.need_int("fromFrame")?, from_point: e.numbers::<2>("fromPoint")?, to_point: e.numbers::<2>("toPoint")? }),
    };
    let regions = fit_regions(&a, q, frame)?;
    if regions.is_none() && eye_trace.is_none() {
        return Err(a.out_of_range("regions", "fit takes regions, an eye trace, or both"));
    }
    ensure_valid(s, cmd, seq, id)?;
    let (h, zoom, margin) = (a.number("h")?, a.number("zoom")?, a.number("margin")?);
    let request = v::FitRequest { frame, regions: regions.unwrap_or_default(), h, zoom, margin, eye_trace };
    let fitted = v::fit::fit(&project, q, id, &request).map_err(|r| EngineError::invalid(cmd, r))?;
    let limits = v::limits(&v::raster(q), plan.output);
    let mut out = serde_json::to_value(&fitted).map_err(|e| EngineError::Other(format!("`{cmd}`: {e}")))?;
    if let Some(map) = out.as_object_mut() {
        map.insert("sequence".into(), json!(seq.0));
        map.insert("plan".into(), json!(id));
        map.insert("frame".into(), json!(frame));
        map.insert("zoom".into(), json!(v::zoom_percent(limits.max_height, fitted.window.h)));
    }
    Ok(out)
}

/// The rectangles `regions` names for a fit at `frame`; None when the key is absent. A keepOut region asks to stay out
/// of the window, so `{source: "protected"}` takes the keepIn and allOrNothing ones.
fn fit_regions(a: &Params, q: &Sequence, frame: i64) -> Result<Option<Vec<[f64; 4]>>> {
    let Some(items) = a.list("regions", MAX_REGIONS)? else { return Ok(None) };
    let rate = q.settings.frame_rate;
    let mut rects = Vec::new();
    for (index, value) in items.iter().enumerate() {
        let region = a.element("regions", value, index, &[&["space", "rect", "source"]])?;
        if region.given("source") {
            if region.text("source")? != Some("protected") {
                return Err(region.out_of_range("source", "a fit region's source is \"protected\""));
            }
            let held = |g: &&v::ProtectedRegion| g.mode != v::RegionMode::KeepOut && (rate.frame_at(g.from)..rate.frame_at(g.to)).contains(&frame);
            rects.extend(q.vertical.iter().flat_map(|set| &set.regions).filter(held).map(|g| g.rect));
            continue;
        }
        if region.text("space")?.is_some_and(|space| space != "sequence") {
            return Err(region.out_of_range("space", "regions are in sequence pixels (\"space\": \"sequence\")"));
        }
        rects.push(region.need_rect("rect")?);
    }
    Ok(Some(rects))
}

/// `vertical.storyboard` (spec §7.4): per shot of the range (or of part `part`), its number, frames, wall sources,
/// status, positions, review counts, `watched`, attention marks, opener and its label, and the first and last windows.
fn storyboard(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.storyboard";
    measure::poll(s);
    let a = Params::of(cmd, p, &[PLAN, &["fromFrame", "toFrame", "part"]])?;
    let seq = sequence_of(s, &a)?;
    let id = a.need_id("plan")?;
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let plan = plan_of(cmd, q, id)?;
    let r = v::raster(q);
    let [mut from, mut to] = read_range(&a, r.end_frame)?;
    if let Some(part) = a.whole("part")? {
        let Some(found) = q.vertical.iter().flat_map(|set| &set.parts).find(|x| x.id == part) else {
            return Err(a.refuse("not_found", format!("there is no part {part}"), json!({"part": part})));
        };
        from = from.max(r.rate.frame_at(found.from));
        to = to.min(r.rate.frame_at(found.to));
    }
    ensure_valid(s, cmd, seq, id)?;
    let window = |frame: i64| v::window_at(plan, &r, r.rate.tick_of(frame)).window;
    let label = |opener: Option<u32>| opener.and_then(|id| plan.positions.iter().find(|p| p.id == id)).map(|p| p.label.clone()).filter(|l| !l.is_empty());
    let mut shots = Vec::new();
    for shot in v::shots(&project, q, id).iter().filter(|shot| shot.from < to && from < shot.to) {
        let last = shot.to.saturating_sub(1);
        shots.push(json!({
            "number": shot.number,
            "fromFrame": shot.from,
            "toFrame": shot.to,
            "lastFrame": last,
            "sources": shot.sources,
            "status": shot.status,
            "positions": shot.positions,
            "reviewed": shot.reviewed,
            "flagged": shot.flagged,
            "watched": is_watched(s, &project, q, seq, plan, shot.from, shot.to),
            "attention": shot.attention,
            "opener": shot.opener,
            "openerLabel": label(shot.opener),
            "firstWindow": window(shot.from),
            "lastWindow": window(last),
        }));
    }
    Ok(json!({"sequence": seq.0, "plan": id, "fromFrame": from, "toFrame": to, "shots": shots}))
}

// ---------- editor state ----------

/// `vertical.select` (spec §7.3): sets the Vertical Edit's selection on the active sequence; an omitted key selects
/// nothing. Every id must exist (`not_found`) and every wall must be an effective wall (`bad_wall {frame}`).
fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.select";
    measure::poll(s);
    let a = Params::of(cmd, p, &[PLAN, &["positions", "walls", "region"]])?;
    let seq = sequence_of(s, &a)?;
    if s.state.active_sequence != Some(seq) {
        return Err(a.out_of_range("sequence", "the Vertical Edit's selection is the active sequence's"));
    }
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let plan = a.whole("plan")?.map(|id| plan_of(cmd, q, id)).transpose()?;
    let mut positions: Vec<u32> = Vec::new();
    for id in a.ids("positions")?.unwrap_or_default() {
        let Some(plan) = plan else { return Err(a.out_of_range("plan", "positions are a plan's: name the plan")) };
        if !plan.positions.iter().any(|p| p.id == id) {
            return Err(a.refuse("not_found", format!("plan {} has no position {id}", plan.id), json!({"position": id})));
        }
        if !positions.contains(&id) {
            positions.push(id);
        }
    }
    let rate = q.settings.frame_rate;
    let frames = a.frames("walls")?.unwrap_or_default();
    let walls: Vec<i64> = if frames.is_empty() { Vec::new() } else { v::walls(&project, q).iter().map(|w| rate.frame_at(w.at)).collect() };
    let mut selected: Vec<i64> = Vec::new();
    for frame in frames {
        if walls.binary_search(&frame).is_err() {
            return Err(a.refuse("bad_wall", format!("frame {frame} is not a wall"), json!({"frame": frame})));
        }
        if !selected.contains(&frame) {
            selected.push(frame);
        }
    }
    let region = a.whole("region")?;
    if let Some(id) = region
        && !q.vertical.iter().flat_map(|set| &set.regions).any(|g| g.id == id)
    {
        return Err(a.refuse("not_found", format!("there is no region {id}"), json!({"region": id})));
    }
    let plan = plan.map(|plan| plan.id);
    let ticks: Vec<Tick> = selected.iter().map(|frame| rate.tick_of(*frame)).collect();
    s.state.vertical = VerticalSelection { plan, positions: positions.clone(), walls: ticks, region };
    Ok(json!({"sequence": seq.0, "plan": plan, "positions": positions, "walls": selected, "region": region}))
}

/// `vertical.goTo` (spec §7.3): moves the playhead, as `playhead.set` does, to the previous or next position, wall,
/// window break (a cut or a jump) or attention mark from the playhead (or from the frame given); `{frame: null}`, and the
/// playhead stays, when there is none. Walls are the sequence's; the other targets are a plan's.
fn go_to(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.goTo";
    measure::poll(s);
    let a = Params::of(cmd, p, &[PLAN, TIME, &["to"]])?;
    let seq = sequence_of(s, &a)?;
    let to = a.name("to", TARGETS)?.ok_or_else(|| a.missing("to"))?;
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let rate = q.settings.frame_rate;
    let start = match a.time_frame(s, &Target { seq, q })? {
        Some(frame) => frame,
        None => rate.frame_at(playhead_of(s, seq)),
    };
    let walls_only = matches!(to, v::GoTo::PrevWall | v::GoTo::NextWall);
    let found = match a.whole("plan")? {
        Some(id) => {
            plan_of(cmd, q, id)?;
            if walls_only {
                wall_step(&project, q, start, to)
            } else {
                ensure_valid(s, cmd, seq, id)?;
                v::go_to(&project, q, id, start, to)
            }
        }
        None if walls_only => wall_step(&project, q, start, to),
        None => return Err(a.out_of_range("plan", "positions, breaks and attention marks are a plan's: name the plan")),
    };
    if let Some(frame) = found {
        let tick = rate.tick_of(frame);
        if s.state.active_sequence == Some(seq) {
            s.set_playhead(tick);
        } else {
            s.state.playheads.insert(seq, tick);
        }
    }
    Ok(json!({"sequence": seq.0, "frame": found, "tick": found.map(|frame| rate.tick_of(frame).0.to_string())}))
}

/// The wall before or after `start`.
fn wall_step(project: &Project, q: &Sequence, start: i64, to: v::GoTo) -> Option<i64> {
    let rate = q.settings.frame_rate;
    let frames = v::walls(project, q).into_iter().map(|w| rate.frame_at(w.at));
    match to {
        v::GoTo::PrevWall => frames.filter(|frame| *frame < start).max(),
        _ => frames.filter(|frame| *frame > start).min(),
    }
}

/// `vertical.noteWatched` (spec §5.10): the UI reports the frames playback showed of shot `shot` (its number), as frame
/// ranges `[from, to)` or single frames, all inside the shot, and with `pass` the unbroken playback pass that showed them
/// (a number the UI gives each pass: a new one whenever playback starts again). The session keeps them per shot with the
/// shot's range digest: they merge while the digest holds, and a change of the shot's framing starts the record again,
/// except for a person's live edits during the pass: when the record was last noted in the same pass and only plan-only
/// writes came since (the picture revision held), the frames stay and the record takes the new digest. A note without a
/// pass, or from another pass, after a change of the shot (an edit while paused) starts again.
fn note_watched(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.noteWatched";
    measure::poll(s);
    let a = Params::of(cmd, p, &[PLAN, &["shot", "frames", "pass"]])?;
    let seq = sequence_of(s, &a)?;
    let id = a.need_id("plan")?;
    let number = a.need_id("shot")?;
    let list = a.list("frames", v::MAX_LIST)?.ok_or_else(|| a.missing("frames"))?;
    let pass = a.count("pass")?;
    let project = Arc::clone(&s.project);
    let q = sequence_in(cmd, &project, seq)?;
    let plan = plan_of(cmd, q, id)?;
    ensure_valid(s, cmd, seq, id)?;
    let shots = v::shots(&project, q, id);
    let Some(shot) = shots.iter().find(|shot| shot.number == number) else {
        return Err(a.refuse("not_found", format!("plan {id} has no shot {number}"), json!({"shot": number})));
    };
    let (from, to) = (shot.from, shot.to);
    let mut seen = Vec::with_capacity(list.len());
    for value in list {
        let [a0, b0] = match value {
            Value::Array(_) => a.pair_in("frames", value)?,
            _ => {
                let frame = a.int_in("frames", value)?;
                [frame, frame.saturating_add(1)]
            }
        };
        if a0 < from || b0 > to || a0 >= b0 {
            let detail = json!({"key": "frames", "fromFrame": from, "toFrame": to});
            return Err(a.refuse("out_of_range", format!("shot {number} holds the frames {from} to {to}"), detail));
        }
        seen.push([a0, b0]);
    }
    let digest = v::range_digest(&project, q, plan, from, to);
    let picture = s.picture_revision;
    let records = s.vertical_store.watched.entry((seq, id)).or_default();
    // a record of a shot that no longer stands (its walls moved) is dropped when another shot overlaps it
    records.retain(|r| (r.from == from && r.to == to) || r.to <= from || to <= r.from);
    // the person's live edits during this pass: the record was last noted in it, and no picture changed since
    let live = |r: &WatchRecord| pass.is_some() && r.pass == pass && r.picture_revision == picture;
    let frames = match records.iter_mut().find(|r| r.from == from && r.to == to) {
        Some(record) if record.digest == digest || live(record) => {
            seen.append(&mut record.frames);
            *record = WatchRecord { from, to, digest, frames: merged(seen), pass, picture_revision: picture };
            covered(&record.frames)
        }
        Some(record) => {
            *record = WatchRecord { from, to, digest, frames: merged(seen), pass, picture_revision: picture };
            covered(&record.frames)
        }
        None => {
            let frames = merged(seen);
            let count = covered(&frames);
            records.push(WatchRecord { from, to, digest, frames, pass, picture_revision: picture });
            count
        }
    };
    let watched = frames.saturating_add(WATCH_SLACK) >= to.saturating_sub(from);
    Ok(json!({"sequence": seq.0, "plan": id, "shot": number, "fromFrame": from, "toFrame": to, "frames": frames, "watched": watched, "pass": pass}))
}

// ---------- the session's store ----------

/// What playback showed of one shot (spec §5.10), as the UI reports it with `vertical.noteWatched`: the shot's frames
/// `[from, to)`, the range digest of the shot the record holds for, and the frames shown, merged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WatchRecord {
    pub from: i64,
    pub to: i64,
    pub digest: String,
    pub frames: Vec<[i64; 2]>,
    /// The playback pass that noted the frames last (none: the UI named none).
    pub pass: Option<u64>,
    /// The session's picture revision when they were noted: a live edit is a plan-only write, which keeps it.
    pub picture_revision: u64,
}

/// The violations of one sequence's vertical set, for the project snapshot they were found in.
pub struct Validity {
    /// The snapshot: the violations hold while the session's project is this very `Arc` (the `Weak` keeps its allocation,
    /// so its address is never another project's).
    pub project: Weak<Project>,
    pub violations: Vec<Violation>,
}

/// The Vertical Edit's session state outside undo, never saved (spec §4.2, §4.4, §5.10). It belongs to one project life
/// ([`crate::History::life`]): replacing the project with another ([`Session::note_project_replaced`]) empties it.
#[derive(Default)]
pub struct VerticalStore {
    /// The project life the store belongs to.
    pub life: u64,
    /// By (sequence, plan): what playback showed of each shot.
    pub watched: BTreeMap<(ItemId, u32), Vec<WatchRecord>>,
    /// By (sequence, plan): the positions made at a frame where shots were not known (spec §4.4).
    pub made_unknown: BTreeMap<(ItemId, u32), BTreeSet<u32>>,
    /// By (sequence, plan): the positions made while shots were unknown when a measurement was last applied
    /// (`vertical.inspect`'s `unanchoredSinceMeasurement` lists those that stand off a wall).
    pub unanchored: BTreeMap<(ItemId, u32), Vec<u32>>,
    /// The shot-change measurements (spec §4.2).
    pub measurements: Measurements,
    /// By sequence: the violations of its vertical set, for the project snapshot validated last.
    pub validity: BTreeMap<ItemId, Validity>,
}

impl VerticalStore {
    /// An empty store of project life `life`.
    pub fn of_life(life: u64) -> Self {
        Self { life, ..Self::default() }
    }
}
