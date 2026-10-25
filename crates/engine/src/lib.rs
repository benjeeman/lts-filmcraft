//! The FilmCraft engine façade.
//!
//! Every user-visible action is a command with a stable id (`sequence.addEdit`,
//! `timeline.trim`, `markers.markIn`…) and JSON parameters, dispatched through
//! [`Session::execute`]. The egui UI, the CLI, the control channel and the MCP server all use this
//! one entry point; that is what makes the UI swappable and the whole app agent-drivable.
//!
//! The project is an `Arc<Project>` edited copy-on-write; undo keeps whole-project snapshots (cheap
//! thanks to structural sharing of untouched items).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod aaf_omf;
pub mod autosave;
pub mod captions;
pub mod clip_ops;
pub mod color;
pub mod commands;
pub mod demo;
pub mod essential_sound;
pub mod export_tools;
pub mod graphic_templates;
pub mod graphics;
pub mod interchange;
pub mod keyboard;
pub mod masks;
pub mod media_browser;
pub mod media_pool;
pub mod mixer;
pub mod multicam;
pub mod panels;
pub mod perf;
pub mod presets;
pub mod previews;
pub mod project_manager;
pub mod project_panel;
pub mod project_tools;
pub mod proxies;
pub mod relink;
pub mod remix;
pub mod scene_detect;
pub mod scopes;
pub mod sequence_extras;
pub mod sequence_tools;
pub mod settings;
pub mod shortcut_presets;
pub mod shortcuts;
pub mod sync;
pub mod transcript;
pub mod trim;
pub mod vertical;
pub mod vertical_cache;
pub mod vertical_frames;
pub mod vertical_measure;
pub mod voiceover;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Weak};

use filmcraft_edit::EditCtx;
use filmcraft_project::vertical::{
    Caller, DeliveryInfo, FollowReport, PartAudio, PictureDigests, Refusal, SequenceDigests, VerticalPlan, VerticalSet, WallSet, Writer,
};
use filmcraft_project::{ClipId, EffectInstance, ItemId, ItemKind, Project, Sequence, TrackId, TrackItem, TrackKind};
use filmcraft_time::{FrameRate, Tick, format_timecode_frames};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use commands::{CommandSpec, command_specs, find as find_command};
pub use filmcraft_export as export;
pub use filmcraft_project as project;
pub use filmcraft_render as render;
pub use filmcraft_time as time;
pub use media_pool::MediaPool;

/// The OS temp folder; `/tmp` on wasm32, where `std::env::temp_dir()` panics ("no filesystem on
/// this platform"). Use this instead of `std::env::temp_dir()` in any code the web build runs.
pub fn temp_dir() -> std::path::PathBuf {
    if cfg!(target_arch = "wasm32") { std::path::PathBuf::from("/tmp") } else { std::env::temp_dir() }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    /// A refusal with a code from a closed list and, in `detail`, the numbers needed to fix the call (the Vertical
    /// Edit's commands and reads, spec §7.6). Nothing changed.
    #[error("`{cmd}` refused ({code}): {message}")]
    Invalid { cmd: String, code: &'static str, message: String, detail: Value },
    #[error("no active sequence")]
    NoSequence,
    #[error("edit failed: {0}")]
    Edit(#[from] filmcraft_edit::EditError),
    #[error("media: {0}")]
    Media(#[from] filmcraft_media::MediaError),
    #[error("{0}")]
    Other(String),
}

impl EngineError {
    /// Command `cmd`'s refusal `r` (the planner's or a read's) as [`EngineError::Invalid`], code and detail kept.
    pub fn invalid(cmd: &str, r: Refusal) -> Self {
        Self::Invalid { cmd: cmd.to_string(), code: r.code, message: r.message, detail: r.detail }
    }

    /// A refusal raised where the running command is not known (an empty `cmd`: [`Session::edit_plan_only`]) as the
    /// refusal of command `id`, which [`Session::execute`] runs.
    fn for_command(self, id: &str) -> Self {
        match self {
            Self::Invalid { cmd, code, message, detail } if cmd.is_empty() => Self::Invalid { cmd: id.to_string(), code, message, detail },
            other => other,
        }
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Host services (file access, clipboard…) injected by the frontend.
pub trait Services: Send + Sync {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>>;
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()>;
    /// Size of a file in bytes (an error when it is missing). Hosts should override this: the
    /// default reads the whole file.
    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        self.read_file(path).map(|b| b.len() as u64)
    }
    /// Up to `len` bytes of a file starting at `offset` (fewer at the end of the file).
    fn read_range(&self, path: &str, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        let b = self.read_file(path)?;
        let a = (offset as usize).min(b.len());
        Ok(b[a..(a + len).min(b.len())].to_vec())
    }
    /// A random-access reader for a media file, so containers are opened without reading the
    /// whole file (web: `Blob` range reads). `None`: the host has none; media are read whole.
    fn reader(&self, _path: &str) -> Option<std::io::Result<filmcraft_media::SharedReader>> {
        None
    }
    /// File names in a directory (image sequence detection). `None`: the host cannot list
    /// directories; sequences are then found by probing consecutive frame numbers.
    fn list_dir(&self, _dir: &str) -> Option<std::io::Result<Vec<String>>> {
        None
    }
    /// A loader the media pool keeps for reading files later (image sequence frames on demand).
    /// `None`: the frames are read when the sequence is opened.
    fn file_loader(&self) -> Option<filmcraft_media::sequence::FrameLoader> {
        None
    }
    /// Exports are encoded in memory and handed to [`Services::write_file`] (hosts without a
    /// filesystem, e.g. the web: the file is then offered as a download). Otherwise they stream
    /// to the output path.
    fn export_in_memory(&self) -> bool {
        false
    }
    /// Entries of a directory with their kind, size and date (the Media Browser). The default
    /// derives them from [`Services::list_dir`] (every name a file).
    fn list_entries(&self, dir: &str) -> Option<std::io::Result<Vec<media_browser::DirEntry>>> {
        self.list_dir(dir).map(|r| r.map(|names| names.into_iter().map(|name| media_browser::DirEntry { name, ..Default::default() }).collect()))
    }
    /// Drives and network locations (Media Browser ▸ Local Drives / Network).
    fn volumes(&self) -> Vec<media_browser::Volume> {
        Vec::new()
    }
    /// The user's home directory (where the Media Browser starts).
    fn home_dir(&self) -> Option<String> {
        None
    }
}

/// Native filesystem services.
pub struct FsServices;
impl Services for FsServices {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        // Atomic + durable: temp file in the same directory, fsync, rename, fsync the directory.
        filmcraft_format::atomic_write(std::path::Path::new(path), data)
    }
    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        let m = std::fs::metadata(path)?;
        if m.is_file() { Ok(m.len()) } else { Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{path} is not a file"))) }
    }
    fn list_dir(&self, dir: &str) -> Option<std::io::Result<Vec<String>>> {
        let dir = if dir.is_empty() { "." } else { dir };
        Some(std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()))
    }
    fn file_loader(&self) -> Option<filmcraft_media::sequence::FrameLoader> {
        Some(std::sync::Arc::new(|p: &str| std::fs::read(p)))
    }
    /// Media are read in place from the open file: a clip costs its index, not its size.
    #[cfg(any(unix, windows))]
    fn reader(&self, path: &str) -> Option<std::io::Result<filmcraft_media::SharedReader>> {
        Some(filmcraft_media::reader::FileReader::open(std::path::Path::new(path)).map(|r| std::sync::Arc::new(r) as filmcraft_media::SharedReader))
    }
    fn list_entries(&self, dir: &str) -> Option<std::io::Result<Vec<media_browser::DirEntry>>> {
        Some(media_browser::std_list_entries(dir))
    }
    fn volumes(&self) -> Vec<media_browser::Volume> {
        media_browser::std_volumes()
    }
    fn home_dir(&self) -> Option<String> {
        media_browser::std_home_dir()
    }
    fn read_range(&self, path: &str, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path)?;
        f.seek(SeekFrom::Start(offset))?;
        let mut out = Vec::with_capacity(len);
        f.take(len as u64).read_to_end(&mut out)?;
        Ok(out)
    }
}

/// What an undo step records beside its label and its snapshot (spec §8.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepMeta {
    /// Who made the step: [`Session::writer`] when it was recorded.
    pub writer: Writer,
    /// False only for a plan-only step ([`Session::edit_plan_only`]): undoing or redoing it changes no 16:9 frame, so
    /// it keeps [`Session::picture_revision`].
    pub picture: bool,
    /// The caller's run id, with which an agent groups its work; None for a person's steps.
    pub run: Option<String>,
}

/// One undo (or redo) step: its label, the whole project before (or after) it, and what it records.
#[derive(Clone)]
pub struct Step(pub String, pub Arc<Project>, pub StepMeta);

impl Step {
    /// The step's label, as the History panel and `history.list` show it.
    pub fn label(&self) -> &str {
        &self.0
    }
}

/// Undo history of whole-project snapshots.
#[derive(Clone)]
pub struct History {
    pub undo: Vec<Step>,
    pub redo: Vec<Step>,
    /// Labels of all applied states, oldest first (History panel).
    pub limit: usize,
    /// Key of the last [`Session::edit_merged`] step: a continuous gesture (a fader drag) with the
    /// same key folds into that one undo step.
    pub merge_key: Option<String>,
    /// The project life this history belongs to, unique in the process: every new history has its own (a new, opened,
    /// closed or installed project starts one), and a snapshot of the same project put back keeps it (a cancelled dynamic
    /// trim). The Vertical Edit's session store belongs to one life ([`vertical::VerticalStore`]).
    pub life: u64,
}

impl Default for History {
    fn default() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: 0, merge_key: None, life: next_life() }
    }
}

/// A project life no history had before ([`History::life`]).
fn next_life() -> u64 {
    static LIVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    LIVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// The wall clock of the `when` fields the engine writes (approvals, attention marks, delivery records): Unix seconds,
/// UTC. [`Clock::system`] by default; tests install [`Clock::fixed`] in [`Session::clock`].
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> i64 + Send + Sync>);

impl Clock {
    /// The system clock (on the web too).
    pub fn system() -> Self {
        Self(Arc::new(|| web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))))
    }

    /// A clock that always reads `unix`.
    pub fn fixed(unix: i64) -> Self {
        Self(Arc::new(move || unix))
    }

    /// A clock that reads `f`.
    pub fn from_fn(f: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Now, in Unix seconds.
    pub fn now(&self) -> i64 {
        (self.0)()
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::system()
    }
}

/// One id counter of a vertical set, for [`VerticalIdMarks::allocate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdCounter {
    Plan,
    Part,
    Region,
    Annotation,
    /// The positions of the plan with this id.
    Position(u32),
    /// The overwritten-range records of the plan with this id.
    Record(u32),
}

/// The highest values the id counters of one plan reached in this session (spec §2.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanIdMarks {
    pub next_position_id: u32,
    pub next_record_id: u32,
}

/// The highest values the id counters of one vertical set reached in this session (spec §2.6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetIdMarks {
    pub next_plan_id: u32,
    pub next_part_id: u32,
    pub next_region_id: u32,
    pub next_annotation_id: u32,
    /// By plan id.
    pub plans: BTreeMap<u32, PlanIdMarks>,
}

impl PlanIdMarks {
    /// The marks the counters of `plan` give.
    fn of(plan: &VerticalPlan) -> Self {
        Self { next_position_id: plan.next_position_id, next_record_id: plan.next_record_id }
    }

    /// Take the higher of each mark and `other`'s.
    fn absorb(&mut self, other: PlanIdMarks) {
        self.next_position_id = self.next_position_id.max(other.next_position_id);
        self.next_record_id = self.next_record_id.max(other.next_record_id);
    }

    /// Raise the counters of `plan` to the marks.
    fn raise(&self, plan: &mut VerticalPlan) {
        plan.next_position_id = plan.next_position_id.max(self.next_position_id);
        plan.next_record_id = plan.next_record_id.max(self.next_record_id);
    }

    /// Whether a counter of `plan` is below its mark.
    fn is_ahead_of(&self, plan: &VerticalPlan) -> bool {
        self.next_position_id > plan.next_position_id || self.next_record_id > plan.next_record_id
    }
}

impl SetIdMarks {
    /// Take the higher of each mark and the matching counter of `set`.
    pub fn observe(&mut self, set: &VerticalSet) {
        self.next_plan_id = self.next_plan_id.max(set.next_plan_id);
        self.next_part_id = self.next_part_id.max(set.next_part_id);
        self.next_region_id = self.next_region_id.max(set.next_region_id);
        self.next_annotation_id = self.next_annotation_id.max(set.next_annotation_id);
        for plan in &set.plans {
            self.plans.entry(plan.id).or_default().absorb(PlanIdMarks::of(plan));
        }
    }

    /// Raise every counter of `set` to its mark: no counter goes down, so an id below a mark is never handed out again.
    pub fn raise(&self, set: &mut VerticalSet) {
        set.next_plan_id = set.next_plan_id.max(self.next_plan_id);
        set.next_part_id = set.next_part_id.max(self.next_part_id);
        set.next_region_id = set.next_region_id.max(self.next_region_id);
        set.next_annotation_id = set.next_annotation_id.max(self.next_annotation_id);
        for plan in &mut set.plans {
            if let Some(marks) = self.plans.get(&plan.id) {
                marks.raise(plan);
            }
        }
    }

    /// Whether some counter of `set` is below its mark.
    pub fn is_ahead_of(&self, set: &VerticalSet) -> bool {
        self.next_plan_id > set.next_plan_id
            || self.next_part_id > set.next_part_id
            || self.next_region_id > set.next_region_id
            || self.next_annotation_id > set.next_annotation_id
            || set.plans.iter().any(|plan| self.plans.get(&plan.id).is_some_and(|marks| marks.is_ahead_of(plan)))
    }

    /// Take the higher of each mark and `other`'s.
    pub fn absorb(&mut self, other: &SetIdMarks) {
        self.next_plan_id = self.next_plan_id.max(other.next_plan_id);
        self.next_part_id = self.next_part_id.max(other.next_part_id);
        self.next_region_id = self.next_region_id.max(other.next_region_id);
        self.next_annotation_id = self.next_annotation_id.max(other.next_annotation_id);
        for (plan, theirs) in &other.plans {
            self.plans.entry(*plan).or_default().absorb(*theirs);
        }
    }

    /// The mark of `counter`: 0 for a plan without marks.
    fn mark(&self, counter: IdCounter) -> u32 {
        match counter {
            IdCounter::Plan => self.next_plan_id,
            IdCounter::Part => self.next_part_id,
            IdCounter::Region => self.next_region_id,
            IdCounter::Annotation => self.next_annotation_id,
            IdCounter::Position(plan) => self.plans.get(&plan).map_or(0, |marks| marks.next_position_id),
            IdCounter::Record(plan) => self.plans.get(&plan).map_or(0, |marks| marks.next_record_id),
        }
    }

    /// The first id of `created`, a set made from nothing, that was handed out before (below its mark), with its kind:
    /// every id of such a set is new, so none may be.
    fn reused_by(&self, created: &VerticalSet) -> Option<(&'static str, u32)> {
        fn first_below(kind: &'static str, mut ids: impl Iterator<Item = u32>, mark: u32) -> Option<(&'static str, u32)> {
            ids.find(|id| *id < mark).map(|id| (kind, id))
        }
        first_below("plan", created.plans.iter().map(|p| p.id), self.next_plan_id)
            .or_else(|| first_below("part", created.parts.iter().map(|p| p.id), self.next_part_id))
            .or_else(|| first_below("region", created.regions.iter().map(|r| r.id), self.next_region_id))
            .or_else(|| first_below("annotation", created.annotations.iter().map(|a| a.id), self.next_annotation_id))
            .or_else(|| {
                created.plans.iter().find_map(|plan| {
                    let marks = self.plans.get(&plan.id)?;
                    first_below("position", plan.positions.iter().map(|p| p.id), marks.next_position_id)
                        .or_else(|| first_below("record", plan.overwritten.iter().map(|r| r.id), marks.next_record_id))
                })
            })
    }
}

/// The counter `counter` of `set`; None when `set` has no plan with that id.
fn counter_mut(set: &mut VerticalSet, counter: IdCounter) -> Option<&mut u32> {
    Some(match counter {
        IdCounter::Plan => &mut set.next_plan_id,
        IdCounter::Part => &mut set.next_part_id,
        IdCounter::Region => &mut set.next_region_id,
        IdCounter::Annotation => &mut set.next_annotation_id,
        IdCounter::Position(plan) => &mut set.plans.iter_mut().find(|p| p.id == plan)?.next_position_id,
        IdCounter::Record(plan) => &mut set.plans.iter_mut().find(|p| p.id == plan)?.next_record_id,
    })
}

/// The vertical sets of `project`, by sequence.
fn vertical_sets(project: &Project) -> impl Iterator<Item = (ItemId, &VerticalSet)> {
    project.items.iter().filter_map(|(id, item)| item.as_sequence()?.vertical.as_deref().map(|set| (*id, set)))
}

/// A vertical set with nothing in it, as the planner starts one for a sequence without a set (picture edit points are
/// walls), its id counters at 1.
pub fn empty_vertical_set() -> VerticalSet {
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

/// The id counters of a vertical set and, by index, of its plans.
struct Counters([u32; 4], Vec<[u32; 2]>);

impl Counters {
    fn of(set: &VerticalSet) -> Self {
        let plans = set.plans.iter().map(|p| [p.next_position_id, p.next_record_id]).collect();
        Self([set.next_plan_id, set.next_part_id, set.next_region_id, set.next_annotation_id], plans)
    }

    /// Write them into `set`, plan by plan in order.
    fn put(&self, set: &mut VerticalSet) {
        [set.next_plan_id, set.next_part_id, set.next_region_id, set.next_annotation_id] = self.0;
        for (plan, [position, record]) in set.plans.iter_mut().zip(&self.1) {
            (plan.next_position_id, plan.next_record_id) = (*position, *record);
        }
    }
}

/// Whether vertical data `b` is `a`'s, id counters aside (no counter is in a digest: a write that only moved them changes
/// nothing). No set reads as [`empty_vertical_set`], as it does for the planner.
fn same_data(a: Option<&VerticalSet>, b: Option<&mut VerticalSet>) -> bool {
    let (empty, mut none) = (empty_vertical_set(), empty_vertical_set());
    let (a, b) = (a.unwrap_or(&empty), b.unwrap_or(&mut none));
    if a.plans.len() != b.plans.len() {
        return false;
    }
    // compare with `b`'s counters set to `a`'s, then put `b`'s back
    let own = Counters::of(b);
    Counters::of(a).put(b);
    let same = *a == *b;
    own.put(b);
    same
}

/// Puts back into each vertical set of `after`, which the edit hook wrote from `raised` (`base` with its id counters raised
/// to the session's high-water marks), `base`'s value of every counter the hook handed out no id from: a 16:9 edit moves a
/// counter only by handing out ids, so one that hands out none leaves the vertical data as it found it. The marks, which
/// live outside undo, keep every id handed out; the planner's working copies are raised to them.
fn lower_unused_counters(base: &Project, raised: &Project, after: &mut Project) {
    for (seq, set) in vertical_sets(base) {
        let Some(high) = raised.sequence(seq).and_then(|q| q.vertical.as_deref()) else { continue };
        let Some(out) = after.sequence_mut(seq).and_then(|q| q.vertical.as_deref_mut()) else { continue };
        // the hook keeps every plan in its place; a set that holds other plans keeps its counters
        if !same_plans(out, set) || !same_plans(high, set) {
            continue;
        }
        let (was, up, mut now) = (Counters::of(set), Counters::of(high), Counters::of(out));
        let sets = now.0.iter_mut().zip(was.0.iter().zip(&up.0));
        let plans = now.1.iter_mut().zip(was.1.iter().zip(&up.1)).flat_map(|(n, (w, h))| n.iter_mut().zip(w.iter().zip(h)));
        for (n, (w, h)) in sets.chain(plans) {
            if *n == *h {
                *n = *w;
            }
        }
        now.put(out);
    }
}

/// Whether vertical sets `a` and `b` hold the same plans, by id, in the same order.
fn same_plans(a: &VerticalSet, b: &VerticalSet) -> bool {
    a.plans.iter().map(|p| p.id).eq(b.plans.iter().map(|p| p.id))
}

/// Why committing `set` to a sequence with the id high-water marks `marks` would hand an id out twice (spec §2.6), if it
/// would: a set the write `created` from nothing holds an id below its mark, or the set's counters are behind the marks
/// (it was planned on a copy whose counters were not raised).
fn id_problem(marks: Option<&SetIdMarks>, set: Option<&VerticalSet>, created: bool) -> Option<String> {
    let (marks, set) = (marks?, set?);
    if created && let Some((kind, id)) = marks.reused_by(set) {
        return Some(format!("hand out {kind} id {id} again"));
    }
    if marks.is_ahead_of(set) { Some("store id counters behind the session's high-water marks".into()) } else { None }
}

/// The id high-water marks of the vertical sets of the session's project, by sequence (spec §2.6). They live outside
/// undo, which rolls a set's counters back: the engine raises a working copy's counters to them before it plans a write
/// (plan C-11; [`Session::vertical_working_copy`], which gives a sequence whose set was undone a new set at the marks),
/// [`Session::edit_plan_only`] raises the set it hands out and refuses one that would hand an id out again, so an id is
/// never handed out twice; saving writes them into the file and into the in-memory project. Opening or installing a
/// project starts them from the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerticalIdMarks {
    pub sets: BTreeMap<ItemId, SetIdMarks>,
}

impl VerticalIdMarks {
    /// The marks the counters of `project` give: a project just opened, installed or created.
    pub fn of(project: &Project) -> Self {
        let mut marks = Self::default();
        marks.observe(project);
        marks
    }

    /// Take the higher of each mark and the matching counter of every vertical set of `project`.
    pub fn observe(&mut self, project: &Project) {
        for (seq, set) in vertical_sets(project) {
            self.sets.entry(seq).or_default().observe(set);
        }
    }

    /// Raise the counters of `set`, sequence `seq`'s vertical set or a working copy of it, to the marks.
    pub fn raise_set(&self, seq: ItemId, set: &mut VerticalSet) {
        if let Some(marks) = self.sets.get(&seq) {
            marks.raise(set);
        }
    }

    /// Whether some counter of `project` is below its mark: saving then writes the marks.
    pub fn is_ahead_of(&self, project: &Project) -> bool {
        vertical_sets(project).any(|(seq, set)| self.sets.get(&seq).is_some_and(|marks| marks.is_ahead_of(set)))
    }

    /// Raise every counter of every vertical set of `project` to its mark (what a save writes).
    pub fn raise(&self, project: &mut Project) {
        for (seq, item) in &mut project.items {
            if let Some(set) = item.as_sequence_mut().and_then(|q| q.vertical.as_deref_mut()) {
                self.raise_set(*seq, set);
            }
        }
    }

    /// `project` as a file written from the session holds it (spec §2.6): every counter raised to its mark, in a copy;
    /// `project` itself when no counter is behind.
    pub fn raised<'a>(&self, project: &'a Project) -> Cow<'a, Project> {
        if !self.is_ahead_of(project) {
            return Cow::Borrowed(project);
        }
        let mut raised = project.clone();
        self.raise(&mut raised);
        Cow::Owned(raised)
    }

    /// Take the higher of each mark and `other`'s: a project restored within the same life (a cancelled dynamic trim)
    /// keeps the ids handed out meanwhile.
    pub fn absorb(&mut self, other: &VerticalIdMarks) {
        for (seq, marks) in &other.sets {
            self.sets.entry(*seq).or_default().absorb(marks);
        }
    }

    /// The next id of `counter` in `set`, sequence `seq`'s vertical set or a working copy of it: at least the counter and
    /// the mark, both of which then move past it. None when the counter is spent or `set` has no such plan; nothing
    /// changes then.
    pub fn allocate(&mut self, seq: ItemId, set: &mut VerticalSet, counter: IdCounter) -> Option<u32> {
        let mark = self.sets.get(&seq).map_or(0, |marks| marks.mark(counter));
        let slot = counter_mut(set, counter)?;
        // 0 is never an id, and a counter at the end of its range hands out nothing
        let id = (*slot).max(mark).max(1);
        *slot = id.checked_add(1)?;
        self.sets.entry(seq).or_default().observe(set);
        Some(id)
    }
}

/// What is selected in the Vertical Edit (spec §8.3): the active plan of the active sequence and, in it, positions,
/// walls and a protected region. Never an undo step; [`Session::fix_state`] drops what no longer exists.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VerticalSelection {
    /// The plan, by id.
    pub plan: Option<u32>,
    /// Positions of that plan, by id.
    pub positions: Vec<u32>,
    /// Walls, by their frame-aligned time.
    pub walls: Vec<Tick>,
    /// A protected region, by id.
    pub region: Option<u32>,
}

/// Keep in `sel` what sequence `seq` (the active one) of `project` still has, each once and in order: the plan and its
/// positions, the walls that are effective walls (spec §4.1: an edit point that is a through edit, a frame without picture
/// or a suppressed frame is none), the region. Nothing stays selected when there is no such sequence or it has no
/// vertical set.
pub(crate) fn retain_vertical(sel: &mut VerticalSelection, project: &Project, seq: Option<&Sequence>) {
    let Some((seq, set)) = seq.and_then(|q| q.vertical.as_deref().map(|set| (q, set))) else {
        *sel = VerticalSelection::default();
        return;
    };
    match sel.plan.and_then(|id| set.plans.iter().find(|p| p.id == id)) {
        Some(plan) => {
            let ids: BTreeSet<u32> = plan.positions.iter().map(|p| p.id).collect();
            let mut seen = BTreeSet::new();
            sel.positions.retain(|id| ids.contains(id) && seen.insert(*id));
        }
        None => {
            sel.plan = None;
            sel.positions.clear();
        }
    }
    // an effective wall stands on a frame start inside the sequence
    let walls: BTreeSet<Tick> =
        if sel.walls.is_empty() { BTreeSet::new() } else { filmcraft_project::vertical::walls(project, seq).into_iter().map(|w| w.at).collect() };
    let mut seen = BTreeSet::new();
    sel.walls.retain(|t| walls.contains(t) && seen.insert(*t));
    if sel.region.is_some_and(|id| !set.regions.iter().any(|r| r.id == id)) {
        sel.region = None;
    }
}

/// The project snapshots whose picture digests the edit hook keeps (spec §4.8 "Cost"): the session's project, which the
/// next edit starts from, and the few that undo and redo bring back.
const HOOK_SNAPSHOTS: usize = 8;

/// How the edit hook's suffix starts in an undo label, after the space that joins it to the label: the start of
/// `FollowReport::label_suffix`, `· vertical: 12 moved, 1 removed, 2 flagged`.
const HOOK_SUFFIX: &str = "\u{b7} vertical: ";

/// The edit hook's state in the session (spec §4.8; plan 63 §3.5): how often it ran, its digest caches (spec §4.8
/// "Cost", §10) with their miss counters, the report of the last step, the gesture its sub-steps follow from, and the
/// dynamic trim it follows.
///
/// The picture digests of a project snapshot are computed once and kept by the snapshot's identity: an edit's `before`
/// is the previous edit's `after`, so an edit digests only its own result. A clip's framing is hashed once per value: its
/// hash is reused while the clip's Motion, time remapping, geometry effects and Scale to Frame Size are the ones hashed.
/// A gesture's base is copied to raise its id counters to the high-water marks only when some counter is behind them,
/// and only once an edit of the gesture changes a picture digest: an edit that changes none hands out no id.
#[derive(Debug, Default)]
pub struct VerticalHook {
    /// Hook runs: one per commit of [`Session::edit`] and per merged sub-step of [`Session::edit_merged`].
    pub runs: u64,
    /// Project snapshots whose picture digests were computed: the snapshot cache's misses.
    pub snapshots_digested: u64,
    /// Picture clips whose framing was hashed: the clip cache's misses.
    pub items_hashed: u64,
    /// Gesture bases copied to raise their id counters to the high-water marks: at most one per gesture.
    pub raises: u64,
    /// The report of the last step the session recorded or folded, when its hook changed vertical data, with the project
    /// that step left: see [`VerticalHook::report`].
    last: Option<(Weak<Project>, FollowReport)>,
    /// The digests of recent snapshots, newest first, by snapshot address: each entry's `Weak` keeps that address from
    /// being reused while the entry lives, without keeping the snapshot alive.
    snapshots: VecDeque<(Weak<Project>, Arc<PictureDigests>)>,
    /// By planned sequence and clip: each picture clip's framing as it was last hashed, with its hash.
    clips: BTreeMap<ItemId, BTreeMap<ClipId, HashedFraming>>,
    /// The gesture the next merged sub-step follows from.
    gesture: Option<Gesture>,
    /// The gesture of the dynamic trim that runs (J/K/L in trim mode, `trim.rs`), from the project before the trim.
    live: Option<Gesture>,
}

impl VerticalHook {
    /// The picture digests of project snapshot `project` (the layout and framing digests of every sequence with a vertical
    /// set): the ones kept when the snapshot was digested before, else computed now and kept.
    pub fn digests(&mut self, project: &Arc<Project>) -> Arc<PictureDigests> {
        let at = Arc::as_ptr(project);
        let known = self.snapshots.iter().position(|(snapshot, _)| std::ptr::eq(snapshot.as_ptr(), at));
        if let Some(hit) = known.and_then(|i| self.snapshots.remove(i)) {
            let digests = Arc::clone(&hit.1);
            self.snapshots.push_front(hit);
            return digests;
        }
        let digests = Arc::new(self.compute(project));
        self.remember(project, Arc::clone(&digests));
        digests
    }

    /// Keep `digests` as the digests of snapshot `project`, newest first: at most [`HOOK_SNAPSHOTS`], none of them gone.
    fn remember(&mut self, project: &Arc<Project>, digests: Arc<PictureDigests>) {
        let at = Arc::as_ptr(project);
        self.snapshots.retain(|(snapshot, _)| snapshot.strong_count() > 0 && !std::ptr::eq(snapshot.as_ptr(), at));
        self.snapshots.push_front((Arc::downgrade(project), digests));
        self.snapshots.truncate(HOOK_SNAPSHOTS);
    }

    /// The picture digests of `project`: for every sequence with a vertical set, the model's layout digest and the
    /// session's framing digest ([`VerticalHook::framing`]).
    fn compute(&mut self, project: &Project) -> PictureDigests {
        self.snapshots_digested += 1;
        let mut out = PictureDigests::default();
        for item in project.items.values() {
            let Some(seq) = item.as_sequence().filter(|q| q.vertical.is_some()) else { continue };
            let layout = filmcraft_project::vertical::layout_digest(project, seq);
            let framing = self.framing(project, item.id, seq);
            out.sequences.insert(item.id, SequenceDigests { layout, framing });
        }
        // the clips of a sequence that is no longer planned are hashed again if it comes back
        self.clips.retain(|seq, _| out.sequences.contains_key(seq));
        out
    }

    /// The framing digest of planned sequence `id` (`seq`): the session's form of the model's `framing_digest` (spec
    /// §4.8), FNV-1a 64 over each video track in order (its id and enabled flag) and each enabled picture clip on it (its
    /// id and the hash of its framing). A clip's hash comes from the cache while its framing is the one hashed (compared
    /// by value, so 0.0 and -0.0 count as one), so an edit hashes only the clips whose framing it changed. It changes when
    /// the model's does, FNV collisions aside.
    fn framing(&mut self, project: &Project, id: ItemId, seq: &Sequence) -> String {
        let mut known = self.clips.remove(&id).unwrap_or_default();
        let mut kept = BTreeMap::new();
        let mut tracks = Vec::with_capacity(seq.video_tracks.len());
        for track in &seq.video_tracks {
            let mut clips = Vec::new();
            for item in track.items.iter().filter(|i| i.enabled && shows_picture(project, i)) {
                let framing = ClipFraming::of(item);
                let hashed = match known.remove(&item.id) {
                    Some(cached) if cached.is_of(&framing) => cached,
                    _ => {
                        self.items_hashed += 1;
                        HashedFraming::of(&framing)
                    }
                };
                clips.push((item.id, hashed.hash));
                kept.insert(item.id, hashed);
            }
            tracks.push((track.id, track.enabled, clips));
        }
        self.clips.insert(id, kept);
        filmcraft_project::vertical::digest::digest_text(fnv(&tracks))
    }

    /// What the hook did to `project`, the session's project: the report of the step that left it, when that step's hook
    /// changed vertical data. None for any other project: after a step whose hook changed nothing, a plan-only step, an
    /// undo or redo and a new project, and once a dynamic trim put back the project from before a live update.
    pub fn report(&self, project: &Arc<Project>) -> Option<&FollowReport> {
        self.last.as_ref().filter(|(of, _)| std::ptr::eq(of.as_ptr(), Arc::as_ptr(project))).map(|(_, report)| report)
    }

    /// Keep `gesture`, the one the last 16:9 edit followed from, while the dynamic trim that runs, if one does, began from
    /// `before`. The trim's gesture from `before` is the trim's record; a later edit point of the trim gives the record
    /// what it reported; any other edit ends the record.
    fn keep(&mut self, gesture: Gesture, before: Option<&Arc<Project>>) {
        match before.filter(|_| gesture.live) {
            None => self.live = None,
            Some(before) if Arc::ptr_eq(&gesture.base, before) => self.live = Some(gesture.clone()),
            Some(before) => {
                if let Some(live) = self.live.as_mut().filter(|live| Arc::ptr_eq(&live.base, before)) {
                    live.reported.clone_from(&gesture.reported);
                }
            }
        }
        self.gesture = Some(gesture);
    }

    /// The gesture ended (a plan-only step, an undo or a redo): the next edit begins its own, a dynamic trim's record is
    /// gone, and the last step's report no longer holds.
    fn end_gesture(&mut self) {
        self.gesture = None;
        self.live = None;
        self.last = None;
    }

    /// The session's project was replaced: nothing of the old one is kept (the counters keep counting).
    fn forget_project(&mut self) {
        self.snapshots.clear();
        self.clips.clear();
        self.end_gesture();
    }
}

/// What the framing digest covers of one picture clip, as the model's `ClipFraming` (spec §4.8): its Motion, enabled or
/// not; its time remapping; its enabled geometry effects in order (the Transform and Distort categories, Crop and Auto
/// Reframe); and Scale to Frame Size.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipFraming<'a> {
    motion: Option<&'a EffectInstance>,
    time_remap: Option<&'a EffectInstance>,
    geometry: Vec<&'a EffectInstance>,
    scale_to_frame: bool,
}

impl<'a> ClipFraming<'a> {
    fn of(item: &'a TrackItem) -> Self {
        let geometry = item.effects.iter().filter(|e| e.enabled && is_geometry(e)).collect();
        Self { motion: item.effect("motion"), time_remap: item.effect("time_remap"), geometry, scale_to_frame: item.scale_to_frame }
    }
}

/// A clip's framing as it was hashed, and its hash.
#[derive(Debug)]
struct HashedFraming {
    motion: Option<EffectInstance>,
    time_remap: Option<EffectInstance>,
    geometry: Vec<EffectInstance>,
    scale_to_frame: bool,
    hash: u64,
}

impl HashedFraming {
    fn of(framing: &ClipFraming) -> Self {
        Self {
            motion: framing.motion.cloned(),
            time_remap: framing.time_remap.cloned(),
            geometry: framing.geometry.iter().copied().cloned().collect(),
            scale_to_frame: framing.scale_to_frame,
            hash: fnv(framing),
        }
    }

    /// Whether this is the hash of `framing`: its framing is equal to it.
    fn is_of(&self, framing: &ClipFraming) -> bool {
        self.scale_to_frame == framing.scale_to_frame
            && self.motion.as_ref() == framing.motion
            && self.time_remap.as_ref() == framing.time_remap
            && self.geometry.len() == framing.geometry.len()
            && self.geometry.iter().zip(&framing.geometry).all(|(a, b)| a == *b)
    }
}

/// Whether clip `item` shows picture that a cut can change, as the model decides it for its digests and walls: a media
/// clip with video, a subclip of one, a nest or a multicam clip (graphics and adjustment layers do not).
fn shows_picture(project: &Project, item: &TrackItem) -> bool {
    match project.item(item.item).map(|i| &i.kind) {
        Some(ItemKind::Media(m)) => m.info.has_video(),
        Some(ItemKind::Sequence(_)) => true,
        Some(ItemKind::Subclip { parent, .. }) => project.item(*parent).is_some_and(|p| p.has_video()),
        _ => false,
    }
}

/// Whether effect `e` changes the picture's geometry, as the model decides it: the Transform and Distort categories,
/// Crop and Auto Reframe.
fn is_geometry(e: &EffectInstance) -> bool {
    e.effect == "crop" || e.effect == "auto_reframe" || e.def().is_some_and(|d| matches!(d.category, ["Video Effects", "Transform" | "Distort"]))
}

/// FNV-1a 64: its offset basis and its prime (the model's digests, spec §2.6).
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 of `value`'s canonical bytes (`serde_json`, as the model's digests print them), streamed into the hash.
fn fnv<T: Serialize + ?Sized>(value: &T) -> u64 {
    let mut hash = Fnv(FNV_OFFSET);
    // plain data printed into a writer that never fails cannot fail; were it to, the hash would cover the bytes printed
    // before the failure, still a function of the value
    let _ = serde_json::to_writer(&mut hash, value);
    hash.0
}

/// The running FNV-1a 64 state, written into by `serde_json`.
struct Fnv(u64);

impl std::io::Write for Fnv {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = bytes.iter().fold(self.0, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME));
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Where the edit hook follows a gesture's 16:9 sub-steps from (spec §4.8 "Merged gestures"), so that a sub-step's
/// result depends only on where the gesture began and on the sub-step itself: a trim drag that returns to its start
/// leaves the plans bit-identical, and lossy steps never add up. A dynamic trim is one gesture too: each live update puts
/// back the project from before the trim and trims it again.
#[derive(Clone, Debug)]
struct Gesture {
    /// The project before the gesture's first sub-step, or, after a plan-only sub-step folded into the gesture, the
    /// project that sub-step left: its picture is the gesture's start, and its plans are the ones that sub-step wrote.
    base: Arc<Project>,
    /// The session's id high-water marks as they stood when `base` became the base, while some counter of `base` is
    /// behind them: every sub-step hands out its ids from `base` raised to them (spec §2.6), so all hand out the same ids
    /// for the same splits and anchors while the marks move with each sub-step.
    marks: Option<Arc<VerticalIdMarks>>,
    /// `base` raised to `marks`, made by the first sub-step that changes a picture digest (one that changes none hands
    /// out no id).
    raised: Option<Arc<Project>>,
    /// The undo step the gesture's sub-steps fold into, by the project before it: a merged sub-step follows from `base`
    /// while that step is the last one, whatever replaced the session's project between sub-steps (a finished import, a
    /// save, the source graphics' sync).
    step: Weak<Project>,
    /// Whether it follows a dynamic trim's live updates.
    live: bool,
    /// The locked positions the gesture reported removed (true) or changed (false), by sequence, plan and id: each is
    /// reported once.
    reported: BTreeSet<(ItemId, u32, u32, bool)>,
}

impl Gesture {
    /// A gesture from `base`, whose ids come from `base`'s counters raised to `marks`.
    fn new(base: Arc<Project>, marks: Option<Arc<VerticalIdMarks>>, live: bool) -> Self {
        Self { base, marks, raised: None, step: Weak::new(), live, reported: BTreeSet::new() }
    }

    /// `base` raised to its marks: `base` itself when no counter of it was behind them, else a copy made once (counted in
    /// `raises`).
    fn raised(&mut self, raises: &mut u64) -> Arc<Project> {
        if let Some(raised) = &self.raised {
            return Arc::clone(raised);
        }
        let raised = match &self.marks {
            Some(marks) => {
                *raises += 1;
                let mut copy = (*self.base).clone();
                marks.raise(&mut copy);
                Arc::new(copy)
            }
            None => Arc::clone(&self.base),
        };
        self.raised = Some(Arc::clone(&raised));
        raised
    }
}

/// What the edit hook did to a 16:9 edit before it is committed: its report when it changed vertical data, and the
/// picture digests of the edit's result, kept once the result is the session's project (vertical data is in neither
/// digest, so what the hook writes leaves them valid). Both None when no sequence has a vertical set.
#[derive(Default)]
struct Hooked {
    report: Option<FollowReport>,
    digests: Option<Arc<PictureDigests>>,
}

impl Hooked {
    /// The step's label: `label`, and the hook's suffix after a space when positions changed (spec §4.8).
    fn label(&self, label: &str) -> String {
        match self.report.as_ref().and_then(FollowReport::label_suffix) {
            Some(suffix) => format!("{label} {suffix}"),
            None => label.to_string(),
        }
    }
}

/// Whether undo label `stored` is `label`, the edit hook's suffix aside (spec §4.8): the suffix never takes part in the
/// comparison that folds a gesture's sub-steps into one step.
fn same_label(stored: &str, label: &str) -> bool {
    stored.strip_prefix(label).is_some_and(|rest| rest.is_empty() || rest.strip_prefix(' ').is_some_and(|suffix| suffix.starts_with(HOOK_SUFFIX)))
}

/// The Events panel's line for a locked position that the 16:9 edit `label` removed (`removed`) or changed with its
/// picture: position `id` of `plan` (as the edit found it) of sequence `q`, named `name`. While a dynamic trim runs
/// (`live`), the line says so: the trim may still come back to its start or be cancelled.
fn locked_line(label: &str, name: &str, q: &Sequence, plan: Option<&VerticalPlan>, id: u32, removed: bool, live: bool) -> String {
    let rate = q.settings.frame_rate;
    let frame = plan.and_then(|p| p.positions.iter().find(|x| x.id == id)).map(|x| rate.frame_at(x.at).saturating_add(q.start_timecode));
    let at = frame.map(|f| format!(" at {}", format_timecode_frames(f, rate, q.settings.drop_frame))).unwrap_or_default();
    let plan = plan.map_or("", |p| p.name.as_str());
    let what = format!("the locked vertical position #{id} of plan \u{201c}{plan}\u{201d} on \u{201c}{name}\u{201d}{at}");
    let label = if live { format!("Dynamic trimming ({label})") } else { label.to_string() };
    match (removed, live) {
        (true, false) => format!("{label} removed {what} with its picture; Undo brings both back"),
        (true, true) => format!("{label} removed {what} with its picture; Esc, or Undo once the trim is committed, brings both back"),
        (false, _) => format!("{label} changed {what} with its picture; it is flagged for review"),
    }
}

/// Source patching / track targeting (the track header "V1/A1" source buttons + target toggles).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Targeting {
    /// Timeline tracks targeted for navigation / paste / match frame.
    pub targeted: Vec<TrackId>,
    /// Destination track for the source's video (source patch), None = unpatched.
    pub video_dest: Option<TrackId>,
    pub audio_dest: Option<TrackId>,
}

/// Editing state that commands depend on (not project data, but headless-relevant).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EditorState {
    pub active_sequence: Option<ItemId>,
    /// Playhead per sequence.
    pub playheads: std::collections::BTreeMap<ItemId, Tick>,
    /// Item loaded in the Source monitor and its playhead (media time).
    pub source_item: Option<ItemId>,
    pub source_playhead: Tick,
    /// Selected timeline items.
    pub selection: Vec<ClipId>,
    /// Selected project panel items.
    pub project_selection: Vec<ItemId>,
    pub targeting: std::collections::BTreeMap<ItemId, Targeting>,
    pub snapping: bool,
    pub linked_selection: bool,
    /// The Timeline's "Insert and overwrite sequences as nests or individual clips" toggle, turned
    /// off: a sequence edits in as its clips instead of one nested clip.
    #[serde(default)]
    pub sequences_as_clips: bool,
    /// Open sequences (timeline tabs), in tab order.
    pub open_sequences: Vec<ItemId>,
    /// How each sequence is shown in the Timeline panel (zoom, scroll, track heights): every
    /// sequence keeps its own. The frontend writes the active sequence's view here as it changes
    /// and takes a sequence's view from here when it becomes the active one.
    #[serde(default)]
    pub timeline_views: std::collections::BTreeMap<ItemId, filmcraft_project::SequenceView>,
    /// Timeline clipboard (serialized track items with their relative track index).
    #[serde(skip)]
    pub clipboard: Vec<(TrackKind, usize, filmcraft_project::TrackItem)>,
    pub default_video_transition: String,
    pub default_audio_transition: String,
    /// Selected edit points (trim mode).
    #[serde(default)]
    pub edit_points: Vec<trim::EditPoint>,
    /// Trim Monitor Out/In shift counters for the selected edit point.
    #[serde(default)]
    pub trim_shift: trim::TrimShift,
    /// Selected captions (caption tracks / Captions panel).
    #[serde(default)]
    pub caption_selection: Vec<ClipId>,
    /// Selected layers (indices among the graphic layers, 0 = back) of the selected graphic clip.
    #[serde(default)]
    pub graphic_layers: Vec<usize>,
    /// The mask being edited on the Program monitor (Effect Controls selection).
    #[serde(default)]
    pub selected_mask: Option<masks::MaskSel>,
    /// The Vertical Edit's selection on the active sequence: its plan, positions, walls and region.
    #[serde(default)]
    pub vertical: VerticalSelection,
    /// Multi-Camera Audio Follows Video: switching a multi-camera clip's angle switches its linked
    /// audio clips too.
    #[serde(default)]
    pub multicam_audio_follows_video: bool,
    /// Multi-Camera view settings: grid layout and page, selection order, preview monitor,
    /// playback quality, transmit.
    #[serde(default)]
    pub multicam_view: multicam::MulticamView,
    /// Audio Clip Mixer automation mode per audio track (track id → mode; missing = Read):
    /// Latch / Touch / Write record the clip Volume and Panner moves as clip keyframes.
    #[serde(default)]
    pub clip_mixer_modes: std::collections::BTreeMap<u64, filmcraft_project::AutomationMode>,
    /// Sequence ▸ Selection Follows Playhead: moving the playhead selects the clips under it on
    /// targeted tracks.
    #[serde(default)]
    pub selection_follows_playhead: bool,
    /// Sequence ▸ Show Through Edits: the timeline marks cuts between continuous pieces of a clip.
    #[serde(default)]
    pub show_through_edits: bool,
    /// Markers ▸ Ripple Sequence Markers: ripple edits move (and delete) sequence markers too.
    #[serde(default)]
    pub ripple_sequence_markers: bool,
    /// Markers ▸ Copy Paste Includes Sequence Markers.
    #[serde(default)]
    pub copy_paste_sequence_markers: bool,
    /// Marker colours hidden in the Markers panel (Show All Marker Colors clears this).
    #[serde(default)]
    pub hidden_marker_colors: Vec<filmcraft_project::Label>,
    /// Sequence markers copied with the clipboard clips (start relative to the copied range).
    #[serde(skip)]
    pub clipboard_markers: Vec<filmcraft_project::Marker>,
    /// The last Edit ▸ Find… (Find Next continues it).
    #[serde(default)]
    pub find: Option<project_tools::FindState>,
}

/// Events for frontends (drained each frame).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Event {
    ProjectChanged {
        revision: u64,
    },
    Toast {
        message: String,
        error: bool,
    },
    OpenSequence(ItemId),
    OpenSource(ItemId),
    /// Show this item in the Project panel (Reveal in Project).
    RevealInProject(ItemId),
}

pub struct Session {
    pub project: Arc<Project>,
    pub history: History,
    pub revision: u64,
    pub saved_revision: u64,
    pub path: Option<String>,
    pub state: EditorState,
    pub media: Arc<MediaPool>,
    pub services: Arc<dyn Services>,
    pub events: Vec<Event>,
    /// Commands executed (for macros/debugging): (id, params).
    pub journal: Vec<(String, Value)>,
    /// Background jobs (exports, render previews…).
    pub jobs: Vec<Job>,
    /// User preferences (`prefs.*` commands) and where they persist (None = not persisted).
    pub prefs: autosave::Preferences,
    pub prefs_path: Option<std::path::PathBuf>,
    /// Auto-save ring + crash-recovery journal (native frontends start it; None = off).
    pub persistence: Option<autosave::Persistence>,
    /// Schema version of the file at `path` as found on disk (older = upgraded on load; the first
    /// save over it keeps a backup of the original).
    pub loaded_schema: u32,
    /// Render preview files + render-bar segments (shared with the frontend's frame workers).
    pub previews: Arc<previews::PreviewStore>,
    /// Audio Track Mixer automation pass in progress.
    pub mixrec: mixer::Recorder,
    /// Multi-camera live switching pass in progress.
    pub mcrec: multicam::Recorder,
    /// Voice-over recording: the input device and the take in progress.
    pub voiceover: voiceover::VoiceOver,
    /// Dynamic (J/K/L) trimming and trim-mode loop playback in progress.
    pub trim_play: trim::TrimPlayback,
    /// Keyboard shortcuts (active bindings, presets; `shortcuts.*` commands).
    pub shortcuts: shortcuts::Shortcuts,
    /// Missing / offline media found by the last scan (Link Media dialog).
    pub offline: relink::OfflineState,
    /// Proxy / ingest / project-manager jobs whose results still have to be applied to the project.
    pub media_jobs: Vec<proxies::PendingJob>,
    /// Mask tracking jobs whose keyframes are still being written.
    pub mask_jobs: Vec<masks::PendingTrack>,
    /// Scene Edit Detection jobs whose results are applied when they finish.
    pub scene_jobs: Vec<scene_detect::PendingScene>,
    /// Imports whose streams are indexed in the background until their jobs finish
    /// ([`commands::import_streamed`]).
    pub background_imports: Vec<commands::BackgroundImport>,
    /// Effect presets (built-in + the user's, persisted in the data directory).
    pub presets: presets::PresetLibrary,
    /// Export presets (built-in + the user's, persisted in the data directory) and favourites.
    pub export_presets: export_tools::ExportPresetLibrary,
    /// The export queue (session state, not saved with the project).
    pub export_queue: export_tools::ExportQueue,
    /// Exports run a batch at a time by [`Session::pump_jobs`] (hosts without threads: web).
    pub stepped: Vec<SteppedJob>,
    /// Speech recogniser for `transcript.generate` (None = the Whisper model named by the command,
    /// feature `whisper`). Hosts and tests install one here.
    pub transcriber: Option<Arc<dyn filmcraft_speech::Transcriber>>,
    /// The Events panel log: failed commands, job results, auto-save errors, messages.
    pub log: panels::EventLog,
    /// Media Browser navigation (directory, back / forward history, selected files).
    pub browser: media_browser::BrowserState,
    /// Who the commands run for (spec §8.2). Plan writes plan with it and every history step records it. The desktop UI
    /// is `Human`; nothing sets `Agent` until agent integration (M5).
    pub writer: Writer,
    /// Advances on every change of [`Session::revision`] except a plan-only step and the undo or redo of one
    /// (spec §8.5): what Program frames and the 16:9 frame cache key by.
    pub picture_revision: u64,
    /// The revision whose picture change was last accounted for: the `execute` safety net advances the picture revision
    /// when the revision moved past it.
    pub picture_seen: u64,
    /// The project epoch: advances whenever the whole project is replaced ([`Session::note_project_replaced`]); part of
    /// the 16:9 frame cache key, since item ids restart in every project.
    pub epoch: u64,
    /// The id high-water marks of the vertical sets, outside undo (spec §2.6).
    pub vertical_ids: VerticalIdMarks,
    /// The clock of the `when` fields this session writes.
    pub clock: Clock,
    /// The full-resolution, captionless 16:9 frames of vertical renders (spec §6.3).
    pub vertical_cache: Arc<vertical_cache::VerticalFrameCache>,
    /// The Vertical Edit's state outside undo: watched shots, positions made while shots were unknown, measurements.
    pub vertical_store: vertical::VerticalStore,
    /// The edit hook's runs, digest caches, last report and gesture (spec §4.8).
    pub vertical_hook: VerticalHook,
    /// Nesting depth of [`Session::execute`] (commands that run other commands).
    exec_depth: u32,
}

/// An export advanced a batch at a time on the host's thread ([`Session::pump_jobs`]).
pub struct SteppedJob {
    pub job: u64,
    pub exporter: filmcraft_export::Exporter,
    pub provider: media_pool::PoolProvider,
    pub progress: Arc<filmcraft_export::Progress>,
    pub result: Arc<std::sync::Mutex<Option<std::result::Result<filmcraft_export::Report, String>>>>,
}

/// A background job with shared progress.
#[derive(Clone)]
pub struct Job {
    pub id: u64,
    pub label: String,
    pub progress: Arc<filmcraft_export::Progress>,
    pub result: Arc<std::sync::Mutex<Option<std::result::Result<filmcraft_export::Report, String>>>>,
}

impl Job {
    pub fn to_json(&self) -> Value {
        use std::sync::atomic::Ordering;
        let res = self.result.lock().unwrap_or_else(|e| e.into_inner()).clone();
        serde_json::json!({
            "id": self.id,
            "label": self.label,
            "progress": self.progress.fraction(),
            // seconds left at the job's recent speed; null until it can tell
            "etaSeconds": self.progress.eta().map(|d| d.as_secs_f64()),
            "done": self.progress.done.load(Ordering::Relaxed),
            "total": self.progress.total.load(Ordering::Relaxed),
            "status": self.progress.status.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            "finished": res.is_some(),
            "result": match res { Some(Ok(r)) => serde_json::to_value(r).unwrap_or_default(), Some(Err(e)) => serde_json::json!({"error": e}), None => Value::Null },
        })
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Arc::new(FsServices))
    }
}

impl Session {
    pub fn new(services: Arc<dyn Services>) -> Self {
        let history = History { limit: 200, ..Default::default() };
        // the store belongs to the first project's life
        let vertical_store = vertical::VerticalStore::of_life(history.life);
        Self {
            project: Arc::new(Project::new("Untitled")),
            history,
            revision: 1,
            saved_revision: 1,
            path: None,
            state: EditorState {
                snapping: true,
                linked_selection: true,
                default_video_transition: "cross_dissolve".into(),
                default_audio_transition: "constant_power".into(),
                ..Default::default()
            },
            media: Arc::new(MediaPool::default()),
            services,
            events: Vec::new(),
            journal: Vec::new(),
            jobs: Vec::new(),
            prefs: Default::default(),
            prefs_path: None,
            persistence: None,
            loaded_schema: filmcraft_format::SCHEMA_VERSION,
            previews: Arc::new(previews::PreviewStore::temp()),
            mixrec: Default::default(),
            mcrec: Default::default(),
            voiceover: Default::default(),
            trim_play: Default::default(),
            shortcuts: shortcuts::Shortcuts::new(),
            offline: Default::default(),
            media_jobs: Vec::new(),
            mask_jobs: Vec::new(),
            scene_jobs: Vec::new(),
            background_imports: Vec::new(),
            presets: Default::default(),
            export_presets: Default::default(),
            export_queue: Default::default(),
            stepped: Vec::new(),
            transcriber: None,
            log: Default::default(),
            browser: Default::default(),
            writer: Writer::Human,
            picture_revision: 1,
            picture_seen: 1,
            epoch: 1,
            vertical_ids: VerticalIdMarks::default(),
            clock: Clock::system(),
            vertical_cache: Arc::new(vertical_cache::VerticalFrameCache::default()),
            vertical_store,
            vertical_hook: VerticalHook::default(),
            exec_depth: 0,
        }
    }

    /// Advance stepped jobs ([`SteppedJob`]) for up to `budget` (at least one step). Returns
    /// whether jobs remain. A job whose media is still loading waits for the next call.
    pub fn pump_jobs(&mut self, budget: std::time::Duration) -> bool {
        export_tools::pump_queue(self, false);
        let t0 = web_time::Instant::now();
        while let Some(j) = self.stepped.first_mut() {
            match j.exporter.step(&j.provider, &j.progress) {
                Ok(filmcraft_export::Step::Progress) => {}
                Ok(filmcraft_export::Step::Pending) => return true,
                Ok(filmcraft_export::Step::Done(r)) => {
                    *j.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(Ok(r));
                    self.stepped.remove(0);
                }
                Err(e) => {
                    let e = e.to_string();
                    *j.progress.error.lock().unwrap_or_else(|x| x.into_inner()) = Some(e.clone());
                    j.progress.finished.store(true, std::sync::atomic::Ordering::Relaxed);
                    *j.result.lock().unwrap_or_else(|x| x.into_inner()) = Some(Err(e));
                    self.stepped.remove(0);
                }
            }
            if t0.elapsed() >= budget {
                break;
            }
        }
        !self.stepped.is_empty() || self.export_queue.is_active()
    }

    /// Start auto-save and the crash-recovery journal (native frontends). Loads preferences from
    /// the data directory and finds unsaved changes left by sessions that died
    /// ([`Session::recovery_candidates`]).
    pub fn start_autosave(&mut self, cfg: autosave::AutosaveConfig) -> std::io::Result<()> {
        let prefs_path = cfg.data_dir.join("preferences.json");
        self.prefs = autosave::Preferences::load(&prefs_path);
        self.prefs.apply_hardware_decoding();
        self.media.set_use_proxies(self.prefs.media.enable_proxies);
        self.shortcuts.set_dir(&cfg.data_dir);
        self.presets.set_dir(&cfg.data_dir);
        self.export_presets.set_dir(&cfg.data_dir);
        self.prefs_path = Some(prefs_path);
        self.apply_media_cache();
        self.persistence = Some(autosave::Persistence::start(&cfg, self.prefs.auto_save.clone())?);
        self.sync_persistence();
        Ok(())
    }

    /// Clean shutdown: flush and stop the worker. Unsaved changes stay in the journal (offered on
    /// the next launch); otherwise the session's journal directory is removed.
    pub fn shutdown(&mut self) {
        self.sync_persistence();
        if let Some(p) = self.persistence.take() {
            p.close();
        }
    }

    pub fn recovery_candidates(&self) -> &[autosave::RecoveryCandidate] {
        self.persistence.as_ref().map(|p| p.candidates.as_slice()).unwrap_or(&[])
    }

    /// Tell the worker about the current project state if it changed (cheap: an `Arc` clone and a
    /// channel send; serialization happens on the worker). Called after every command.
    pub fn sync_persistence(&mut self) {
        let dirty = self.is_dirty();
        let Some(p) = self.persistence.as_mut() else { return };
        let cur = autosave::Sent { revision: self.revision, saved_revision: self.saved_revision, path: self.path.clone() };
        if p.last_sent.as_ref() == Some(&cur) {
            return;
        }
        if dirty {
            p.send_dirty(self.project.clone(), self.revision, self.path.clone(), self.vertical_ids.clone());
        } else {
            p.send_clean();
        }
        p.last_sent = Some(cur);
    }

    /// Apply worker notifications (call regularly, e.g. once per UI frame). Also applies the
    /// results of finished proxy / ingest jobs.
    pub fn poll_persistence(&mut self) {
        proxies::poll(self);
        masks::poll(self);
        scene_detect::poll(self);
        vertical_measure::poll(self);
        commands::poll_background_imports(self);
        export_tools::pump_queue(self, false);
        panels::log_jobs(self);
        let Some(p) = self.persistence.as_mut() else { return };
        for ev in p.drain_events() {
            match ev {
                autosave::WorkerEvent::SavedProject { path, revision } => {
                    if self.path.as_deref() == Some(path.as_str()) && revision > self.saved_revision && revision <= self.revision {
                        self.saved_revision = revision;
                    }
                }
                autosave::WorkerEvent::Error(m) => {
                    self.log.push(panels::Level::Error, "autosave", m.clone());
                    self.events.push(Event::Toast { message: m, error: true });
                }
                _ => {}
            }
        }
        self.sync_persistence();
    }

    /// Replace preferences (persisting them and updating the worker).
    pub fn set_prefs(&mut self, p: autosave::Preferences) -> std::io::Result<()> {
        let cache_changed = self.prefs.media_cache != p.media_cache;
        self.prefs = p;
        self.prefs.apply_hardware_decoding();
        if cache_changed {
            self.apply_media_cache();
        }
        if self.media.use_proxies() != self.prefs.media.enable_proxies {
            self.media.set_use_proxies(self.prefs.media.enable_proxies);
            self.bump_view();
        }
        if let Some(w) = &self.persistence {
            w.set_prefs(self.prefs.auto_save.clone());
        }
        match &self.prefs_path {
            Some(path) => self.prefs.save(path),
            None => Ok(()),
        }
    }

    /// Keep render previews next to the saved project (moves an unsaved project's previews).
    pub fn previews_follow_path(&mut self) {
        if let Some(p) = &self.path
            && !cfg!(target_arch = "wasm32")
        {
            self.previews.move_to(project_tools::previews_dir(self).unwrap_or_else(|| previews::dir_for_project(p)));
        }
    }

    /// A change of what the project *looks like* that isn't an edit (proxies toggled): bumps the
    /// revision so frame caches refresh, without marking a clean project as modified.
    pub fn bump_view(&mut self) {
        let clean = !self.is_dirty();
        self.bump();
        if clean {
            self.saved_revision = self.revision;
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Run a command by id.
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value> {
        let spec = commands::find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        self.execute_spec(spec, params)
    }

    /// [`Session::execute`] for a command already found (tests also run commands that are not registered).
    pub(crate) fn execute_spec(&mut self, spec: &CommandSpec, params: Value) -> Result<Value> {
        let id = spec.id;
        if self.exec_depth == 0 && spec.journal && self.trim_play.active() {
            self.settle_trim_playback(id);
        }
        if let Err(why) = self.enabled_for(spec, &params) {
            let e = EngineError::Disabled(id.to_string(), why);
            if self.exec_depth == 0 {
                self.log.push(panels::Level::Warning, id, e.to_string());
            }
            return Err(e);
        }
        self.exec_depth += 1;
        // Last-resort guard: a panic in a command (a bug that escaped the never-crash rules)
        // becomes an error for the caller (UI, CLI, control channel, MCP) instead of taking the
        // session and its unsaved project down. The panic hook has logged where it happened.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, &params)))
            .unwrap_or_else(|_| Err(EngineError::Other(format!("internal error in `{id}` (see the crash log); the command did not complete"))))
            .map_err(|e| e.for_command(id));
        self.exec_depth -= 1;
        // source graphics: an edited instance updates the shared layers and the other instances
        if r.is_ok() && spec.journal && id.starts_with("graphics.") && !self.project.source_graphics.is_empty() {
            graphic_templates::sync_source_graphics(self);
        }
        if self.exec_depth == 0
            && let Err(e) = &r
        {
            self.log.push(panels::Level::Error, id, e.to_string());
        }
        self.settle_picture();
        self.sync_persistence();
        // playback reads the newest snapshot (mixer moves, mutes… are heard while playing)
        self.previews.live.publish_project(self.project.clone());
        if r.is_ok() && spec.journal {
            self.journal.push((id.to_string(), params));
            if self.journal.len() > 10_000 {
                self.journal.drain(..1000);
            }
        }
        r
    }

    /// Another command arrives while trim-mode playback runs: a dynamic trim is committed first
    /// (so e.g. Undo undoes it as one step); loop playback stops unless it is a trim command.
    fn settle_trim_playback(&mut self, id: &str) {
        const LIVE: [&str; 5] = ["trim.shuttle", "trim.tick", "trim.shuttleStop", "trim.cancelDynamic", "trim.playAround"];
        if LIVE.contains(&id) {
            return;
        }
        if self.trim_play.dynamic.is_some() {
            trim::commit(self);
        }
        if !id.starts_with("trim.") {
            self.trim_play.around = None;
        }
    }

    /// Enablement of `spec` for one call. `enabled()` reads the selection, which is right for the
    /// menus and for a call without targets. A caller that names its targets in `params`, under a
    /// key the command documents (`clips` / `clip`: timeline clips, `items` / `item`: project
    /// items), is asked the same question about those targets instead: the check runs again with
    /// them standing in for the selection, so every other condition (an open sequence, a filled
    /// clipboard, the kind of item…) still applies. The selections are put back before returning.
    fn enabled_for(&mut self, spec: &CommandSpec, params: &Value) -> std::result::Result<(), String> {
        let by_selection = (spec.enabled)(self);
        if by_selection.is_ok() {
            return by_selection;
        }
        let clips = commands::named_clips(self, spec, params);
        let items = commands::named_items(self, spec, params);
        if clips.is_none() && items.is_none() {
            return by_selection;
        }
        let mut clips = clips.unwrap_or_else(|| self.state.selection.clone());
        let mut items = items.unwrap_or_else(|| self.state.project_selection.clone());
        std::mem::swap(&mut self.state.selection, &mut clips);
        std::mem::swap(&mut self.state.project_selection, &mut items);
        let by_params = (spec.enabled)(self);
        self.state.selection = clips;
        self.state.project_selection = items;
        by_params
    }

    /// Whether the command can run on the current selection (what the menus show).
    pub fn is_enabled(&self, id: &str) -> bool {
        commands::find(id).is_some_and(|c| (c.enabled)(self).is_ok())
    }

    /// What is open, to store beside the project when it is saved.
    pub fn project_view(&self) -> filmcraft_project::ProjectView {
        let is_seq = |id: &ItemId| self.project.sequence(*id).is_some();
        filmcraft_project::ProjectView {
            open_sequences: self.state.open_sequences.iter().copied().filter(is_seq).collect(),
            active_sequence: self.state.active_sequence.filter(is_seq),
            sequences: self.state.timeline_views.iter().filter(|(id, _)| is_seq(id)).map(|(id, v)| (*id, *v)).collect(),
        }
    }

    /// Open what was open when the project was saved. Nothing in `view` is trusted: ids that are
    /// not sequences of this project are dropped (also a second mention of the same sequence),
    /// and numbers are brought into range. A view without any open sequence leaves the project
    /// on its first sequence: a project saved by a session that never showed one (a script, the
    /// CLI) should not open on an empty Timeline.
    pub fn restore_project_view(&mut self, view: filmcraft_project::ProjectView) {
        let mut open: Vec<ItemId> = Vec::new();
        for id in view.open_sequences {
            if self.project.sequence(id).is_some() && !open.contains(&id) {
                open.push(id);
            }
        }
        if !open.is_empty() {
            self.state.active_sequence = view.active_sequence.filter(|id| open.contains(id)).or(open.first().copied());
            self.state.open_sequences = open;
        }
        self.state.timeline_views =
            view.sequences.into_iter().filter(|(id, _)| self.project.sequence(*id).is_some()).filter_map(|(id, v)| Some((id, v.checked()?))).collect();
    }

    /// Every edit passes through here: one that would put a sequence inside itself (directly or
    /// through another nested sequence) is refused, whichever command asked for it. A project that
    /// was opened with such a sequence already in it can still be edited (and repaired).
    fn refuse_self_nesting(&self, edited: &Project) -> Result<()> {
        match edited.nest_cycle() {
            Some(id) if self.project.nest_cycle().is_none() => {
                let name = edited.item(id).map(|i| i.name.as_str()).unwrap_or_default();
                Err(EngineError::Other(format!("a sequence cannot be nested inside itself (\u{201c}{name}\u{201d})")))
            }
            _ => Ok(()),
        }
    }

    /// Apply an undoable project edit. The closure gets a mutable copy; on error nothing changes. Before the step is
    /// recorded, the edit hook makes every vertical set follow the edit (spec §4.8), and the step's label carries the
    /// hook's suffix (`Ripple Delete · vertical: 12 moved, 1 removed, 2 flagged`).
    pub fn edit<R>(&mut self, label: &str, f: impl FnOnce(&mut Project, &mut EditorState) -> Result<R>) -> Result<R> {
        let mut p = (*self.project).clone();
        let mut st = self.state.clone();
        let r = f(&mut p, &mut st)?;
        self.refuse_self_nesting(&p)?;
        let mut gesture = self.hook_base(false);
        let hooked = self.run_edit_hook(&mut gesture, &mut p)?;
        let old = std::mem::replace(&mut self.project, Arc::new(p));
        let meta = self.step_meta(true);
        self.history.undo.push(Step(hooked.label(label), old, meta));
        if self.history.undo.len() > self.history.limit {
            self.history.undo.remove(0);
        }
        self.history.redo.clear();
        self.history.merge_key = None;
        let active = self.state.active_sequence;
        self.state = st;
        self.bump();
        self.settle_hook(label, gesture, hooked, active);
        Ok(r)
    }

    /// Like [`Session::edit`], but consecutive calls with the same `key` (and no other edit in
    /// between) share one undo step: dragging a control is one undoable change. The edit hook follows a sub-step that
    /// folds in from its gesture's base (spec §4.8), the step's label follows the last sub-step, and the hook's suffix
    /// takes no part in deciding whether a call folds in.
    pub fn edit_merged<R>(&mut self, label: &str, key: &str, f: impl FnOnce(&mut Project, &mut EditorState) -> Result<R>) -> Result<R> {
        if !self.folds_into_last(Some(key), label) {
            let r = self.edit(label, f)?;
            self.history.merge_key = Some(key.to_string());
            return Ok(r);
        }
        let mut p = (*self.project).clone();
        let mut st = self.state.clone();
        let r = f(&mut p, &mut st)?;
        self.refuse_self_nesting(&p)?;
        let mut gesture = self.hook_base(true);
        let hooked = self.run_edit_hook(&mut gesture, &mut p)?;
        self.project = Arc::new(p);
        // a picture change folded into a plan-only step makes it a picture step; the label follows the last sub-step,
        // whose hook ran from the gesture's base
        if let Some(last) = self.history.undo.last_mut() {
            last.2.picture = true;
            last.0 = hooked.label(label);
        }
        let active = self.state.active_sequence;
        self.state = st;
        self.bump();
        self.settle_hook(label, gesture, hooked, active);
        Ok(r)
    }

    /// What the edit hook did in the step that left the session's project, when that step's hook changed vertical data
    /// (spec §4.8): what a toast after a 16:9 command shows, while the History panel shows its suffix. None after a step
    /// whose hook changed nothing, a plan-only step, an undo or redo, a new project, and once a dynamic trim put back the
    /// project from before a live update.
    pub fn hook_report(&self) -> Option<&FollowReport> {
        self.vertical_hook.report(&self.project)
    }

    /// Where the edit hook follows the next 16:9 edit from (spec §4.8 "Merged gestures"). An edit of a dynamic trim's
    /// live update follows from the trim's gesture (`live_base`). A merged sub-step (`merged`) follows from its gesture's
    /// base while the undo step its sub-steps fold into is the last one, whatever replaced the session's project in
    /// between. Any other edit, and a merged sub-step after a plan-only sub-step folded into its gesture (which ends the
    /// gesture), follows from the session's project, which becomes the base.
    fn hook_base(&self, merged: bool) -> Gesture {
        if let Some(d) = &self.trim_play.dynamic {
            return self.live_base(d);
        }
        let step = self.history.undo.last().map(|step| Arc::as_ptr(&step.1));
        if merged && let Some(g) = self.vertical_hook.gesture.as_ref().filter(|g| !g.live && step == Some(g.step.as_ptr())) {
            return g.clone();
        }
        self.new_gesture(false)
    }

    /// A gesture from the session's project (`live` while a dynamic trim runs): the ids the hook hands out come from the
    /// project's counters raised to the session's high-water marks as they stand now (spec §2.6).
    fn new_gesture(&self, live: bool) -> Gesture {
        let base = Arc::clone(&self.project);
        let marks = self.vertical_ids.is_ahead_of(&base).then(|| Arc::new(self.vertical_ids.clone()));
        Gesture::new(base, marks, live)
    }

    /// Where the hook follows an edit of a dynamic trim's live update from (J/K/L in trim mode). Each update puts back the
    /// project from before the trim (`d.before()`) and trims it again, one edit per edit point. Its first edit point
    /// follows from that project with the trim's gesture, raised once for the whole trim; a later point follows from the
    /// point before it, raised to the same marks and sharing what the trim reported. So every update hands out the ids
    /// that the same edits made one after another would, and each locked position is reported once per trim. The trim's
    /// first update (at offset 0) begins its gesture, and so does an update after the trim came back to its start, which
    /// may be another trim from the same project: it reuses none of an earlier trim's ids or reports.
    fn live_base(&self, d: &trim::DynamicTrim) -> Gesture {
        let before = d.before();
        if let Some(live) = self.vertical_hook.live.as_ref().filter(|live| Arc::ptr_eq(&live.base, before)) {
            if !Arc::ptr_eq(&self.project, before) {
                let base = Arc::clone(&self.project);
                let marks = live.marks.clone().filter(|marks| marks.is_ahead_of(&base));
                return Gesture { reported: live.reported.clone(), ..Gesture::new(base, marks, true) };
            }
            if d.offset != Tick::ZERO {
                return live.clone();
            }
        }
        self.new_gesture(true)
    }

    /// The edit hook (spec §4.8): before a 16:9 edit is committed, every sequence that has a vertical set in `gesture`'s
    /// base follows the edit from there to `after`, whose vertical data the model writes (`follow_picture_with`, from
    /// the base raised to the high-water marks, so no id it hands out was handed out before). Both picture digests come
    /// from the session's caches. An edit that changes no picture digest hands out no id: it follows from the base itself,
    /// and only an edit that changes one raises the base, once per gesture. Only [`Session::edit`] and the merging branch
    /// of [`Session::edit_merged`] run it; a plan-only commit never does. It plans as the session's writer (a person in
    /// M2, who is never refused); a refusal changes nothing. A counter it handed out no id from keeps the base's value,
    /// so an edit that hands out no id leaves the vertical data as it was.
    fn run_edit_hook(&mut self, gesture: &mut Gesture, after: &mut Project) -> Result<Hooked> {
        self.vertical_hook.runs += 1;
        if vertical_sets(&gesture.base).next().is_none() {
            return Ok(Hooked::default());
        }
        let before = self.vertical_hook.digests(&gesture.base);
        let digests = Arc::new(self.vertical_hook.compute(after));
        let unchanged = before.sequences.iter().all(|(seq, was)| digests.sequences.get(seq) == Some(was));
        let raised = if unchanged { Arc::clone(&gesture.base) } else { gesture.raised(&mut self.vertical_hook.raises) };
        let caller = Caller { writer: self.writer, overwrite_allowed: false };
        let followed = filmcraft_project::vertical::follow_picture_with(&raised, &before, after, &digests, caller);
        // the command that runs is named by `execute`, which knows it
        let report = followed.map_err(|r| EngineError::invalid("", r))?;
        if !Arc::ptr_eq(&raised, &gesture.base) {
            lower_unused_counters(&gesture.base, &raised, after);
        }
        Ok(Hooked { report: (!report.is_empty()).then_some(report), digests: Some(digests) })
    }

    /// After the 16:9 edit `label` was committed with the hook's result `hooked`, followed from `gesture`'s base while
    /// sequence `active` was active: the result's digests are kept for the next edit, the person is told of the locked
    /// positions it removed or changed, the hook keeps its report with the project the edit left and the gesture with the
    /// undo step it folds into (and, in a dynamic trim, the trim's gesture), the Vertical Edit's selection keeps what
    /// still exists, and every vertical set is validated (spec §2.8), so that the reads, writes and exact frames that
    /// follow find the answer.
    fn settle_hook(&mut self, label: &str, mut gesture: Gesture, hooked: Hooked, active: Option<ItemId>) {
        if let Some(digests) = hooked.digests {
            self.vertical_hook.remember(&self.project, digests);
        }
        if let Some(report) = &hooked.report {
            self.report_locked(label, report, &mut gesture);
        }
        self.vertical_hook.last = hooked.report.map(|report| (Arc::downgrade(&self.project), report));
        gesture.step = self.history.undo.last().map(|step| Arc::downgrade(&step.1)).unwrap_or_default();
        self.vertical_hook.keep(gesture, self.trim_play.dynamic.as_ref().map(trim::DynamicTrim::before));
        // the selection names things of the active sequence (spec §8.3): what the hook removed goes, and another active
        // sequence starts with nothing selected
        if self.state.active_sequence != active {
            self.state.vertical = VerticalSelection::default();
        }
        let project = Arc::clone(&self.project);
        retain_vertical(&mut self.state.vertical, &project, self.state.active_sequence.and_then(|s| project.sequence(s)));
        vertical::validate_now(self);
    }

    /// Tell the person, in the Events panel, of each locked position the 16:9 edit `label` removed or changed with its
    /// picture (I16: a person's edit applies and reports it), as `gesture`'s base, the project the hook followed from,
    /// held it: each once per gesture, so once per dynamic trim, whose lines say that the trim runs.
    fn report_locked(&mut self, label: &str, report: &FollowReport, gesture: &mut Gesture) {
        let base = Arc::clone(&gesture.base);
        for entry in &report.sequences {
            let Some(q) = base.sequence(entry.sequence) else { continue };
            let name = base.item(entry.sequence).map_or("", |i| i.name.as_str());
            for follow in &entry.plans {
                let plan = q.vertical.as_deref().and_then(|set| set.plans.iter().find(|p| p.id == follow.plan));
                for &id in &follow.locked {
                    let removed = follow.removed.contains(&id);
                    if gesture.reported.insert((entry.sequence, follow.plan, id, removed)) {
                        self.log.push(panels::Level::Warning, "vertical", locked_line(label, name, q, plan, id, removed, gesture.live));
                    }
                }
            }
        }
    }

    /// Whether a commit with gesture key `key` and `label` folds into the last undo step (a gesture is one step): the key
    /// is the last step's and the label is too, the edit hook's suffix aside (spec §4.8), with no other step in between.
    fn folds_into_last(&self, key: Option<&str>, label: &str) -> bool {
        key.is_some_and(|key| self.history.merge_key.as_deref() == Some(key)) && self.history.undo.last().is_some_and(|u| same_label(&u.0, label))
    }

    /// The one commit path of every `vertical.*` write (spec §8.5). `f` gets sequence `seq` read-only, its vertical set
    /// taken out for the call, the set itself (its id counters raised to [`Session::vertical_ids`]) and the editor
    /// state, so by its type it can change nothing but the set and the editor state. With a `merge` key, a call with the
    /// same key and label as the last step (the edit hook's suffix aside), and no other step in between, folds into that
    /// step as [`Session::edit_merged`] does: a gesture is one undo step. It never runs the edit hook, records
    /// `StepMeta {picture: false}` and keeps [`Session::picture_revision`]: no 16:9 frame changes.
    ///
    /// A call that changes no data (id counters aside) records nothing and keeps the revision (spec §7.5); only the
    /// editor state is taken. A set `f` leaves with id counters behind the marks, or a set it creates holding an id
    /// handed out before, is refused as an internal error: ids are never handed out twice (§2.6), so a write plans on
    /// [`Session::vertical_working_copy`]. A `seq` that is no sequence is refused `not_found {sequence}`. On error
    /// nothing changes.
    pub fn edit_plan_only<R>(
        &mut self,
        seq: ItemId,
        label: &str,
        merge: Option<&str>,
        f: impl FnOnce(&Sequence, &mut Option<Box<VerticalSet>>, &mut EditorState) -> Result<R>,
    ) -> Result<R> {
        let mut p = (*self.project).clone();
        let mut st = self.state.clone();
        let Some(sequence) = p.sequence_mut(seq) else {
            // the command that runs is named by `execute`, which knows it
            let message = format!("there is no sequence {} in the project", seq.0);
            return Err(EngineError::Invalid { cmd: String::new(), code: "not_found", message, detail: json!({"sequence": seq.0}) });
        };
        let mut set = sequence.vertical.take();
        if let Some(set) = set.as_deref_mut() {
            self.vertical_ids.raise_set(seq, set);
        }
        let r = f(sequence, &mut set, &mut st)?;
        let stored = self.project.sequence(seq).and_then(|q| q.vertical.as_deref());
        if same_data(stored, set.as_deref_mut()) {
            self.state = st;
            return Ok(r);
        }
        if let Some(problem) = id_problem(self.vertical_ids.sets.get(&seq), set.as_deref(), stored.is_none()) {
            return Err(EngineError::Other(format!("internal error: `{label}` would {problem}; nothing changed")));
        }
        sequence.vertical = set;
        let merging = self.folds_into_last(merge, label);
        let old = std::mem::replace(&mut self.project, Arc::new(p));
        if !merging {
            let meta = self.step_meta(false);
            self.history.undo.push(Step(label.to_string(), old, meta));
            if self.history.undo.len() > self.history.limit {
                self.history.undo.remove(0);
            }
            self.history.redo.clear();
            self.history.merge_key = merge.map(str::to_string);
        }
        // the step ran no hook: a gesture's next 16:9 sub-step follows from what it wrote, and the hook's report of an
        // earlier step describes it no longer
        self.vertical_hook.end_gesture();
        self.state = st;
        self.bump_as(false);
        Ok(r)
    }

    /// Edit the active sequence with the edit-algebra context.
    pub fn edit_sequence<R>(&mut self, label: &str, f: impl FnOnce(&mut Sequence, &mut EditCtx, &mut EditorState) -> Result<R>) -> Result<R> {
        self.edit_sequence_as(label, None, f)
    }

    /// [`Session::edit_sequence`] that shares one undo step with the previous edit of the same
    /// `merge` key, like [`Session::edit_merged`] (a drag is one undoable change).
    pub fn edit_sequence_as<R>(
        &mut self,
        label: &str,
        merge: Option<&str>,
        f: impl FnOnce(&mut Sequence, &mut EditCtx, &mut EditorState) -> Result<R>,
    ) -> Result<R> {
        let seq_id = self.state.active_sequence.ok_or(EngineError::NoSequence)?;
        let media = self.media.clone();
        let body = move |p: &mut Project, st: &mut EditorState| {
            let project_snapshot = std::sync::Arc::new(p.clone());
            let snap = project_snapshot.clone();
            let durations = move |id: ItemId| -> Option<Tick> { media_duration(&project_snapshot, &media, id) };
            let starts = move |id: ItemId| media_start(&snap, id);
            let min = p.sequence(seq_id).map(|s| s.settings.frame_rate.frame_duration()).unwrap_or(Tick(1));
            let mut next = p.next_id;
            let r = {
                let seq = p.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
                let mut ctx = EditCtx { next_id: &mut next, media_duration: &durations, media_start: &starts, min_duration: min };
                f(seq, &mut ctx, st)?
            };
            p.next_id = next;
            if let Some(s) = p.sequence(seq_id) {
                s.check().map_err(EngineError::Other)?;
            }
            Ok(r)
        };
        match merge {
            Some(key) => self.edit_merged(label, key, body),
            None => self.edit(label, body),
        }
    }

    pub fn undo(&mut self) -> Option<String> {
        let Step(label, prev, meta) = self.history.undo.pop()?;
        self.history.merge_key = None;
        let cur = std::mem::replace(&mut self.project, prev);
        let picture = meta.picture;
        self.history.redo.push(Step(label.clone(), cur, meta));
        self.fix_state();
        self.bump_as(picture);
        vertical::validate_now(self);
        self.vertical_hook.end_gesture();
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let Step(label, next, meta) = self.history.redo.pop()?;
        self.history.merge_key = None;
        let cur = std::mem::replace(&mut self.project, next);
        let picture = meta.picture;
        self.history.undo.push(Step(label.clone(), cur, meta));
        self.fix_state();
        self.bump_as(picture);
        vertical::validate_now(self);
        self.vertical_hook.end_gesture();
        Some(label)
    }

    /// Drop dangling references after undo/redo/delete.
    pub fn fix_state(&mut self) {
        let p = self.project.clone();
        let active = self.state.active_sequence;
        if let Some(m) = self.state.selected_mask {
            let ok =
                self.active_sequence().and_then(|q| q.find_item(m.clip)).and_then(|(_, it)| it.effects.get(m.effect)).is_some_and(|e| m.mask < e.masks.len());
            if !ok {
                self.state.selected_mask = None;
            }
        }
        if let Some(s) = self.state.active_sequence
            && p.sequence(s).is_none()
        {
            self.state.active_sequence = p.sequences().next().map(|i| i.id);
        }
        self.state.open_sequences.retain(|s| p.sequence(*s).is_some());
        self.state.timeline_views.retain(|s, _| p.sequence(*s).is_some());
        if let Some(s) = self.state.source_item
            && p.item(s).is_none()
        {
            self.state.source_item = None;
        }
        if let Some(seq) = self.state.active_sequence.and_then(|s| p.sequence(s)) {
            self.state.selection.retain(|c| seq.find_item(*c).is_some());
            self.state.caption_selection.retain(|c| seq.find_caption(*c).is_some());
        } else {
            self.state.selection.clear();
            self.state.caption_selection.clear();
        }
        self.state.project_selection.retain(|i| p.item(*i).is_some());
        // the Vertical Edit's selection names things of the active sequence: another one starts with nothing selected
        if self.state.active_sequence != active {
            self.state.vertical = VerticalSelection::default();
        }
        retain_vertical(&mut self.state.vertical, &p, self.state.active_sequence.and_then(|s| p.sequence(s)));
    }

    fn bump(&mut self) {
        self.bump_as(true);
    }

    /// Advance the revision, and the picture revision unless the change is plan-only (spec §8.5): `picture` is false for
    /// [`Session::edit_plan_only`]'s own commit and for the undo or redo of a step whose `StepMeta.picture` is false. A
    /// revision change nothing accounted for before this one (a direct `revision += 1`) advances the picture revision
    /// anyway: it may have changed a frame. The id high-water marks follow the project's counters.
    fn bump_as(&mut self, picture: bool) {
        let unaccounted = self.picture_seen != self.revision;
        self.revision += 1;
        if picture || unaccounted {
            self.picture_revision += 1;
        }
        self.picture_seen = self.revision;
        self.vertical_ids.observe(&self.project);
        self.events.push(Event::ProjectChanged { revision: self.revision });
    }

    /// The whole project was replaced (new, demo, open, close, new from template, install, a cancelled dynamic trim):
    /// advances the revision, the picture revision and the project epoch, clears the 16:9 frame cache and the edit hook's
    /// caches and gesture, starts the id high-water marks from the new project and pushes `ProjectChanged` (spec §8.5);
    /// the Vertical Edit's session store forgets the old project when the history is another project's
    /// ([`History::life`]). Every site that replaces the project calls it instead of advancing the revision itself.
    pub fn note_project_replaced(&mut self) {
        self.revision += 1;
        self.picture_revision += 1;
        self.picture_seen = self.revision;
        self.epoch += 1;
        self.vertical_cache.clear();
        self.vertical_hook.forget_project();
        self.vertical_ids = VerticalIdMarks::of(&self.project);
        vertical::forget(self);
        self.events.push(Event::ProjectChanged { revision: self.revision });
    }

    /// The `execute` safety net (spec §8.5): a revision change no helper accounted for (a direct `revision += 1`, in this
    /// command or since the last one) advances the picture revision too, so it costs a cache refresh, never a stale frame.
    fn settle_picture(&mut self) {
        if self.picture_seen != self.revision {
            self.picture_revision += 1;
            self.picture_seen = self.revision;
            self.vertical_ids.observe(&self.project);
        }
    }

    /// The meta of a step recorded now: this session's writer, whether undoing it can change a 16:9 frame, and no run id
    /// (agents' run ids arrive with agent integration, M5).
    pub(crate) fn step_meta(&self, picture: bool) -> StepMeta {
        StepMeta { writer: self.writer, picture, run: None }
    }

    /// Sequence `seq` as stored, its vertical set's id counters raised to [`Session::vertical_ids`]: the working copy
    /// the planner allocates ids from (plan C-11). A sequence without a set that has marks (its set was created and
    /// undone) gets [`Session::new_vertical_set`], which the planner reads as no set but allocates from above the marks.
    /// None when `seq` is not a sequence of the project.
    pub fn vertical_working_copy(&self, seq: ItemId) -> Option<Sequence> {
        let mut copy = self.project.sequence(seq)?.clone();
        match copy.vertical.as_deref_mut() {
            Some(set) => self.vertical_ids.raise_set(seq, set),
            None => {
                let set = self.new_vertical_set(seq);
                if set != empty_vertical_set() {
                    copy.vertical = Some(Box::new(set));
                }
            }
        }
        Some(copy)
    }

    /// [`empty_vertical_set`] with its counters raised to the marks of sequence `seq`: what a write that creates the
    /// sequence's set starts from, so that no id is handed out twice in the project's life (spec §2.6).
    pub fn new_vertical_set(&self, seq: ItemId) -> VerticalSet {
        let mut set = empty_vertical_set();
        self.vertical_ids.raise_set(seq, &mut set);
        set
    }

    /// Now, by [`Session::clock`]: the `when` of what this session writes.
    pub fn now_unix(&self) -> i64 {
        self.clock.now()
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        let message = msg.into();
        self.log.push(panels::Level::Info, "app", message.clone());
        self.events.push(Event::Toast { message, error: false });
    }

    /// An error message for the user (a toast, and an Events panel entry from `source`).
    pub fn error_toast(&mut self, source: &str, msg: impl Into<String>) {
        let message = msg.into();
        self.log.push(panels::Level::Error, source, message.clone());
        self.events.push(Event::Toast { message, error: true });
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    // ---- accessors ----

    pub fn active_sequence(&self) -> Option<&Sequence> {
        self.state.active_sequence.and_then(|s| self.project.sequence(s))
    }

    pub fn playhead(&self) -> Tick {
        self.state.active_sequence.and_then(|s| self.state.playheads.get(&s).copied()).unwrap_or_default()
    }

    pub fn set_playhead(&mut self, t: Tick) {
        if let Some(s) = self.state.active_sequence {
            let rate = self.active_sequence().map(|s| s.settings.frame_rate).unwrap_or_default();
            self.state.playheads.insert(s, rate.snap(t.max(Tick::ZERO)));
            if self.state.selection_follows_playhead {
                sequence_tools::select_under_playhead(self);
            }
        }
    }

    pub fn sequence_rate(&self) -> FrameRate {
        self.active_sequence().map(|s| s.settings.frame_rate).unwrap_or_default()
    }

    pub fn targeting(&self) -> Targeting {
        let Some(sid) = self.state.active_sequence else { return Targeting::default() };
        if let Some(t) = self.state.targeting.get(&sid) {
            return t.clone();
        }
        // Default: V1/A1 patched and all tracks targeted.
        let seq = self.active_sequence();
        Targeting {
            targeted: seq.map(|s| s.all_tracks().map(|t| t.id).collect()).unwrap_or_default(),
            video_dest: seq.and_then(|s| s.video_tracks.first().map(|t| t.id)),
            audio_dest: seq.and_then(|s| s.audio_tracks.first().map(|t| t.id)),
        }
    }

    /// The media source for an item (creating it on first use).
    pub fn source(&self, item: ItemId) -> Option<filmcraft_media::SharedSource> {
        self.media.source_for(&self.project, item, &*self.services)
    }

    /// Render the active sequence at the playhead in its working colour space (HDR values kept;
    /// for scopes and analysis).
    pub fn render_program_working(&self, scale: f32) -> Option<filmcraft_render::Image> {
        let seq = self.renderable_sequence(scale).ok()?;
        let provider = self.media.provider(self.project.clone(), self.services.clone());
        let opts = filmcraft_render::RenderOptions { scale, working_output: true, ..Default::default() };
        Some(filmcraft_render::render_sequence(&self.project, seq, self.playhead(), opts, &provider))
    }

    /// Render the active sequence at the playhead (CPU reference path).
    pub fn render_program(&self, scale: f32) -> Option<filmcraft_render::Image> {
        self.render_program_at(scale, self.playhead())
    }

    /// Render the active sequence at `t` (snapped to its frame, as the playhead would be) without
    /// moving the playhead (CPU reference path). `None` when there is no sequence or the frame
    /// cannot be rendered at `scale`; [`Session::try_render_program_at`] says why.
    pub fn render_program_at(&self, scale: f32, t: Tick) -> Option<filmcraft_render::Image> {
        self.try_render_program_at(scale, t).ok()
    }

    /// [`Session::render_program_at`], with the reason when nothing can be rendered.
    pub fn try_render_program_at(&self, scale: f32, t: Tick) -> Result<filmcraft_render::Image> {
        let seq = self.renderable_sequence(scale)?;
        let t = self.sequence_rate().snap(t.max(Tick::ZERO));
        let provider = self.media.provider(self.project.clone(), self.services.clone());
        let opts = filmcraft_render::RenderOptions { scale, captions: true, ..Default::default() };
        Ok(filmcraft_render::render_sequence(&self.project, seq, t, opts, &provider))
    }

    /// The active sequence, if a frame of it at `scale` is within the image size limits.
    fn renderable_sequence(&self, scale: f32) -> Result<ItemId> {
        let id = self.state.active_sequence.ok_or(EngineError::NoSequence)?;
        let seq = self.project.sequence(id).ok_or(EngineError::NoSequence)?;
        if !scale.is_finite() || scale <= 0.0 {
            return Err(EngineError::Other(format!("render scale {scale} must be finite and positive")));
        }
        let (w, h) = filmcraft_render::output_size(seq, scale);
        let size = u32::try_from(w).ok().zip(u32::try_from(h).ok());
        let Some((w, h)) = size else { return Err(EngineError::Other("the requested render frame exceeds the image size limits".into())) };
        filmcraft_project::validate_frame_size(w, h).map_err(|e| EngineError::Other(format!("cannot render a {w}x{h} frame: {e}")))?;
        Ok(id)
    }
}

/// The earliest media time clips of an item may show: a subclip that restricts trims starts at its
/// In point, everything else at 0.
pub fn media_start(p: &Project, id: ItemId) -> Tick {
    match p.item(id).map(|i| &i.kind) {
        Some(filmcraft_project::ItemKind::Subclip { range, restrict_trims: true, .. }) => range.start,
        _ => Tick::ZERO,
    }
}

/// Media duration of an item (None for stills/adjustment layers = unlimited handles).
pub fn media_duration(p: &Project, _pool: &MediaPool, id: ItemId) -> Option<Tick> {
    // a subclip's limit is its parent's: follow the chain, bounded (a damaged project file can make
    // it cyclic or arbitrarily deep)
    let mut id = id;
    for _ in 0..=MAX_SUBCLIP_CHAIN {
        let it = p.item(id)?;
        return match &it.kind {
            filmcraft_project::ItemKind::Media(m) => match m.info.kind {
                filmcraft_media::MediaKind::Still | filmcraft_media::MediaKind::Synthetic => None,
                _ => Some(m.info.duration),
            },
            filmcraft_project::ItemKind::Sequence(s) => Some(s.duration()),
            // a subclip that restricts trims ends at its Out point; otherwise its parent's media is the limit
            filmcraft_project::ItemKind::Subclip { range, restrict_trims: true, .. } => Some(range.end()),
            filmcraft_project::ItemKind::Subclip { parent, .. } => {
                id = *parent;
                continue;
            }
            filmcraft_project::ItemKind::AdjustmentLayer { .. } | filmcraft_project::ItemKind::Graphic { .. } => None,
        };
    }
    None
}

/// Subclips of subclips followed before giving up (real projects nest one or two deep).
const MAX_SUBCLIP_CHAIN: usize = 16;

#[cfg(test)]
mod aaf_omf_tests;
#[cfg(test)]
mod audio_effects_tests;
#[cfg(test)]
mod autosave_tests;
#[cfg(test)]
mod clip_ops_tests;
#[cfg(test)]
mod color_tests;
#[cfg(test)]
mod essential_sound_tests;
#[cfg(test)]
mod explicit_targets_tests;
#[cfg(test)]
mod export_tests;
#[cfg(test)]
mod file_tests;
#[cfg(test)]
mod image_sequence_tests;
#[cfg(test)]
mod interchange_auto_points_tests;
#[cfg(test)]
mod keyboard_tests;
#[cfg(test)]
mod linked_speed_tests;
#[cfg(test)]
mod masks_tests;
#[cfg(test)]
mod media_browser_tests;
#[cfg(test)]
mod media_test_util;
#[cfg(test)]
mod mixer_tests;
#[cfg(test)]
mod multicam_tests;
#[cfg(test)]
mod nest_editing_tests;
#[cfg(test)]
mod nest_fidelity_tests;
#[cfg(test)]
mod nesting_tests;
#[cfg(test)]
mod panels_tests;
#[cfg(test)]
mod presets_tests;
#[cfg(test)]
mod previews_tests;
#[cfg(test)]
mod project_manager_tests;
#[cfg(test)]
mod project_panel_tests;
#[cfg(test)]
mod proxies_tests;
#[cfg(test)]
mod relink_tests;
#[cfg(test)]
mod remix_tests;
#[cfg(test)]
mod ripple_delete_tests;
#[cfg(test)]
mod scopes_tests;
#[cfg(test)]
mod sequence_inspect_tests;
#[cfg(test)]
mod sequence_tabs_tests;
#[cfg(test)]
mod sequence_tools_tests;
#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod shortcuts_tests;
#[cfg(test)]
mod split_edit_ripple_trim_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod timeline_move_tests;
#[cfg(test)]
mod track_params_tests;
#[cfg(test)]
mod transcript_tests;
#[cfg(test)]
mod trim_tests;
#[cfg(test)]
mod vertical_hook_tests;
#[cfg(test)]
mod vertical_session_tests;
#[cfg(test)]
mod vertical_tests;
#[cfg(test)]
mod vfx_tests;
#[cfg(test)]
mod voiceover_tests;
