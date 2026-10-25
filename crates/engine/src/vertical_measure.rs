//! The shot-change measurement (spec §4.2; plan 63 §3.4 and C-9): `vertical.measureCuts` measures the frame differences
//! of the composited 16:9 program (captions off, effects on) at analysis size with FilmCraft's scene detector
//! (`filmcraft_render::scene`: a luma grid and colour histograms per frame, no model), as `clip.sceneEditDetection` does
//! for clips, but on the sequence. It records nothing in the project: the session keeps the last [`KEPT`] measurements
//! per sequence, with the picture digest and the media pool generation they ran at and their per-frame scores, and
//! nothing becomes a wall until `vertical.applyMeasurement` (or `vertical.setWalls`) writes it. On the live path it runs
//! as a job (`vertical.jobResult`, `jobs.list`, `jobs.cancel`); `wait: true` runs it to its end inside the call.
//!
//! **A range equals the full run there**, whatever the walls and the changes around it: every rule is read on a frame's
//! own neighbourhood.
//! - A frame is a *change* when its difference from the frame before reaches the threshold of the sensitivity
//!   (`0.45 − 0.33 · s / 100`), reaches `2.5 · median + 0.05` of the differences of the five frames on each side, and no
//!   neighbour's difference is larger: the detector's rules.
//! - The minimum shot length (spec §4.2, T-M34): a change on an effective wall is a cut (it confirms the wall); a change
//!   fewer than `minShotFrames` frames from an effective wall or from frame 0 is dropped (the shot it would make with the
//!   wall is too short); any other change is dropped when a change fewer than `minShotFrames` frames away, on either
//!   side, outranks it (a larger difference, or an equal one earlier), and is a cut otherwise. Every effective wall
//!   counts, inside the range and around it, in every run. Dropped changes are listed.
//! - The possible cuts are the frames of the range whose difference is a local maximum of at least `candidateFloor` (by
//!   default half the threshold) and that are not cuts.
//!
//! So a range `[from, to)` is analysed over `[from − (minShotFrames + 5), to + minShotFrames + 5)`, clipped to the
//! sequence: the changes up to `minShotFrames − 1` frames on each side of it, and the frames each change reads. Two new
//! cuts are never closer than `minShotFrames`, and each is the strongest change of its neighbourhood, so a wall an apply
//! wrote changes nothing in a later measurement with the same `minShotFrames`, at any sensitivity: re-measuring finds
//! what a measurement without the applied walls finds. The detector's own minimum shot length (`scene::detect_cuts`) is a
//! chain from the cut it kept last, which makes a cut depend on every frame before it and keeps the first of two close
//! changes; the two agree wherever the first of two changes closer than `minShotFrames` is the stronger one. For the same
//! reason `minShotFrames` is at least 2: with 1, the detector also takes the frame after a cut when that frame's
//! difference is the smaller one, and whether it does depends on the cuts before.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use filmcraft_export::{Progress, Report};
use filmcraft_project::vertical::{self as v, VE_MAX_FRAMES};
use filmcraft_project::{ItemId, ItemKind, Project, SequenceSettings, TrackItem, Transition};
use filmcraft_render::scene::{self, SceneOptions, Signature};
use filmcraft_render::{RenderOptions, render_sequence};
use filmcraft_time::FrameRate;
use serde::Serialize;
use serde_json::{Value, json};

use crate::vertical::{Params, sequence_of};
use crate::{EngineError, Job, MediaPool, Result, Services, Session};

/// How many measurements the session keeps per sequence (the oldest is dropped first).
pub const KEPT: usize = 4;

/// The longest side of the frames analysed, as Scene Edit Detection's: the measures need no detail.
const ANALYSIS_WIDTH: f64 = 256.0;

/// The frames the detector reads on each side of a frame: the five of its neighbourhood, and before them the frame the
/// first of their differences compares with.
const MARGIN: i64 = 6;

/// The shortest `minShotFrames` (see the module's notes).
const MIN_SHOT_FRAMES: u32 = 2;

/// The detector's neighbourhood (`filmcraft_render::scene::detect_cuts`): the frames on each side whose median
/// difference is the local activity.
const NEIGHBOURS: i64 = 5;

/// The detector's activity rule: a cut's difference reaches `ACTIVITY · median + ACTIVITY_FLOOR`.
const ACTIVITY: f32 = 2.5;
const ACTIVITY_FLOOR: f32 = 0.05;

/// How many finished jobs `vertical.jobResult` remembers.
const OUTCOMES: usize = 64;

/// What a job stopped by `jobs.cancel` reports.
const STOPPED: &str = "stopped";

/// A frame a measurement reports, with its frame difference (0 … 1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeasuredCut {
    pub frame: i64,
    pub score: f64,
}

/// A finished shot-change measurement, as the session keeps it (never in the project).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measured {
    /// "m1", "m2", …: unique in the session.
    pub id: String,
    pub sequence: ItemId,
    /// The job it ran as.
    pub job: u64,
    /// The picture digest and the media pool generation it ran at: applying it later is refused `stale_measurement`
    /// when one of them changed (spec §4.2).
    pub picture_digest: String,
    pub media_generation: u64,
    /// The frames whose cuts it keeps, `[from_frame, to_frame)`.
    pub from_frame: i64,
    pub to_frame: i64,
    /// The frames it analysed, its margins included.
    pub frames: i64,
    pub sensitivity: f64,
    /// The detector's absolute threshold at that sensitivity, `0.45 - 0.33 * sensitivity / 100`.
    pub threshold: f64,
    pub min_shot_frames: u32,
    pub candidate_floor: f64,
    pub cuts: Vec<MeasuredCut>,
    pub candidates: Vec<MeasuredCut>,
    /// The changes the minimum shot length dropped: fewer than `min_shot_frames` frames from an effective wall (or frame 0)
    /// but not on one, or fewer than `min_shot_frames` frames from a change that outranks them.
    pub dropped: Vec<MeasuredCut>,
    /// The frame difference of every frame of `[from_frame, to_frame)` (the first frame of the sequence has none: 0).
    pub scores: Vec<f32>,
    /// The caller asked for the per-frame scores in the reply (`scores: true`).
    pub report_scores: bool,
}

/// A measurement job that has not been collected yet.
pub struct PendingMeasure {
    pub job: u64,
    /// The measurement's id, given when the job starts.
    pub id: String,
    pub sequence: ItemId,
    pub from_frame: i64,
    pub to_frame: i64,
    /// Filled by the job when it finishes; empty when it was cancelled or failed.
    pub result: Arc<Mutex<Option<Measured>>>,
}

/// How a measurement job ended, for `vertical.jobResult` once the job was collected.
#[derive(Clone, Debug, PartialEq)]
pub enum JobOutcome {
    /// The measurement's id.
    Done(String),
    /// Why it stopped.
    Failed(String),
    Cancelled,
}

/// The session's measurements (outside undo, never saved).
#[derive(Default)]
pub struct Measurements {
    /// The last [`KEPT`] finished measurements per sequence, oldest first.
    pub by_sequence: BTreeMap<ItemId, VecDeque<Arc<Measured>>>,
    /// The jobs still running or not collected yet.
    pub pending: Vec<PendingMeasure>,
    /// How each collected job ended, by job id (the last ones).
    pub outcomes: BTreeMap<u64, JobOutcome>,
    /// The number of the last measurement id handed out.
    pub last_id: u64,
}

impl Measurements {
    /// The ids of the measurements kept for sequence `sequence`, oldest first.
    pub fn ids(&self, sequence: ItemId) -> Vec<String> {
        self.by_sequence.get(&sequence).map(|kept| kept.iter().map(|m| m.id.clone()).collect()).unwrap_or_default()
    }

    /// The kept measurement `id` of sequence `sequence`.
    pub fn get(&self, sequence: ItemId, id: &str) -> Option<&Arc<Measured>> {
        self.by_sequence.get(&sequence)?.iter().find(|m| m.id == id)
    }

    /// The newest kept measurement of sequence `sequence` (the lane's score strip).
    pub fn latest(&self, sequence: ItemId) -> Option<&Arc<Measured>> {
        self.by_sequence.get(&sequence)?.back()
    }

    /// The job of measurement `id` while it runs.
    pub fn running_job(&self, id: &str) -> Option<u64> {
        self.pending.iter().find(|m| m.id == id).map(|m| m.job)
    }

    /// The job measuring sequence `sequence` now.
    pub fn running_on(&self, sequence: ItemId) -> Option<u64> {
        self.pending.iter().find(|m| m.sequence == sequence).map(|m| m.job)
    }

    /// The frames the jobs measuring sequence `sequence` now cover: its shots are not known there (spec §4.4).
    pub fn running_ranges(&self, sequence: ItemId) -> Vec<[i64; 2]> {
        self.pending.iter().filter(|m| m.sequence == sequence).map(|m| [m.from_frame, m.to_frame]).collect()
    }

    /// The kept measurement `id`, of any sequence.
    fn find(&self, id: &str) -> Option<&Arc<Measured>> {
        self.by_sequence.values().flatten().find(|m| m.id == id)
    }

    /// Keep a finished measurement, dropping its sequence's oldest past [`KEPT`].
    fn keep(&mut self, measured: Measured) {
        let kept = self.by_sequence.entry(measured.sequence).or_default();
        kept.push_back(Arc::new(measured));
        while kept.len() > KEPT {
            kept.pop_front();
        }
    }

    /// Remember how job `job` ended, forgetting the oldest past [`OUTCOMES`].
    fn remember(&mut self, job: u64, outcome: JobOutcome) {
        self.outcomes.insert(job, outcome);
        while self.outcomes.len() > OUTCOMES {
            self.outcomes.pop_first();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Collect the measurement jobs that finished: keep their measurements and record how they ended. Called once per UI
/// frame ([`Session::poll_persistence`]) and by every `vertical.*` command. A store of another project's life is
/// forgotten first, should a site have replaced the history without [`Session::note_project_replaced`].
pub(crate) fn poll(s: &mut Session) {
    crate::vertical::forget(s);
    let mut index = 0;
    while let Some(job) = s.vertical_store.measurements.pending.get(index).map(|m| m.job) {
        let found = s.jobs.iter().find(|j| j.id == job);
        if found.is_some_and(|j| !j.progress.finished.load(Ordering::Relaxed)) {
            index += 1;
            continue;
        }
        let cancelled = found.is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
        let failure = found.and_then(|j| lock(&j.result).as_ref().and_then(|r| r.as_ref().err().cloned()));
        let pending = s.vertical_store.measurements.pending.remove(index);
        let measured = lock(&pending.result).take();
        let store = &mut s.vertical_store.measurements;
        let outcome = match measured {
            // a job cancelled after it finished is cancelled all the same: nothing is kept
            Some(measured) if !cancelled => {
                store.keep(measured);
                JobOutcome::Done(pending.id)
            }
            _ if cancelled => JobOutcome::Cancelled,
            _ => JobOutcome::Failed(failure.unwrap_or_else(|| "the measurement stopped".to_string())),
        };
        store.remember(job, outcome);
    }
}

/// `vertical.measureCuts` (spec §4.2): measures the shot changes of `[fromFrame, toFrame)` (by default the whole
/// sequence). Refused `measure_running {job}` while a measurement of the sequence runs. Without `wait` it answers
/// `{job, measurement, frames}` at once and runs as a job (on the web it runs to its end, as there are no threads);
/// with `wait` it answers the measurement.
pub(crate) fn measure_cuts(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.measureCuts";
    poll(s);
    let keys: &[&str] = &["sequence", "fromFrame", "toFrame", "sensitivity", "minShotFrames", "candidateFloor", "scores", "wait"];
    let a = Params::of(cmd, p, &[keys])?;
    let seq = sequence_of(s, &a)?;
    let project = Arc::clone(&s.project);
    let Some(q) = project.sequence(seq) else {
        return Err(a.refuse("not_found", format!("there is no sequence {}", seq.0), json!({"sequence": seq.0})));
    };
    let end = v::raster(q).end_frame;
    let from = a.int("fromFrame")?.unwrap_or(0);
    let to = a.int("toFrame")?.unwrap_or(end);
    if from < 0 || from >= to {
        let detail = json!({"key": "fromFrame", "fromFrame": from, "toFrame": to});
        return Err(a.refuse("out_of_range", "the range runs from fromFrame to toFrame, inside the sequence", detail));
    }
    if to > end || to - from > i64::from(VE_MAX_FRAMES) {
        let detail = json!({"key": "toFrame", "toFrame": to, "end": end, "maximum": VE_MAX_FRAMES});
        return Err(a.refuse("out_of_range", "the range lies inside the sequence, at most 1 000 000 frames long", detail));
    }
    let sensitivity = a.number("sensitivity")?.unwrap_or(50.0);
    if !(0.0..=100.0).contains(&sensitivity) {
        return Err(a.out_of_range("sensitivity", "the sensitivity lies in [0, 100]"));
    }
    let shortest = a.int("minShotFrames")?.unwrap_or(6);
    let Some(min_shot_frames) = u32::try_from(shortest).ok().filter(|n| (MIN_SHOT_FRAMES..=VE_MAX_FRAMES).contains(n)) else {
        let detail = json!({"key": "minShotFrames", "minimum": MIN_SHOT_FRAMES, "maximum": VE_MAX_FRAMES});
        return Err(a.refuse("out_of_range", "a shot is at least 2 frames long", detail));
    };
    let options = SceneOptions { sensitivity: sensitivity as f32, min_shot_frames: min_shot_frames as usize };
    let threshold = f64::from(options.threshold());
    let candidate_floor = a.number("candidateFloor")?.unwrap_or(threshold / 2.0);
    if !(0.0..=1.0).contains(&candidate_floor) {
        return Err(a.out_of_range("candidateFloor", "the floor of possible cuts lies in [0, 1]"));
    }
    let (report_scores, wait) = (a.flag("scores")?.unwrap_or(false), a.flag("wait")?.unwrap_or(false));
    if let Some(job) = s.vertical_store.measurements.running_on(seq) {
        return Err(a.refuse("measure_running", "a measurement of this sequence is running: wait for it, or cancel its job", json!({"job": job})));
    }
    let rate = q.settings.frame_rate;
    // every effective wall counts for the minimum shot length, inside the range and around it (sorted, as `walls` gives them)
    let walls: Vec<i64> = v::walls(&project, q).iter().map(|w| rate.frame_at(w.at)).collect();
    // the changes up to `minShotFrames - 1` frames on each side of the range decide its cuts, and each reads `MARGIN` frames
    let margin = MARGIN.saturating_add(i64::from(min_shot_frames).saturating_sub(1));
    let (first, last) = (from.saturating_sub(margin).max(0), to.saturating_add(margin).min(end));
    let (analysed, frames) = ([first, last], last - first);
    let store = &mut s.vertical_store.measurements;
    store.last_id = store.last_id.saturating_add(1);
    let id = format!("m{}", store.last_id);
    let job_id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0).saturating_add(1);
    let measured = Measured {
        id: id.clone(),
        sequence: seq,
        job: job_id,
        picture_digest: picture_digest(&project, seq),
        media_generation: s.media.generation(),
        from_frame: from,
        to_frame: to,
        frames,
        sensitivity,
        threshold,
        min_shot_frames,
        candidate_floor,
        report_scores,
        ..Measured::default()
    };
    let name = project.item(seq).map(|item| item.name.clone()).unwrap_or_default();
    let job = Job { id: job_id, label: format!("Measure Shot Changes ({name})"), progress: Arc::default(), result: Arc::default() };
    job.progress.total.store(u64::try_from(frames).unwrap_or(0), Ordering::Relaxed);
    let scale = (ANALYSIS_WIDTH / f64::from(q.settings.width.max(1))).min(1.0) as f32;
    let services = Arc::clone(&s.services);
    let work = Work { project: Arc::clone(&project), pool: Arc::clone(&s.media), services, rate, scale, options, analysed, end, walls, measured };
    let slot: Arc<Mutex<Option<Measured>>> = Arc::default();
    let (progress, result, out) = (Arc::clone(&job.progress), Arc::clone(&job.result), Arc::clone(&slot));
    let run = move || finish(&work, &progress, &result, &out);
    let run = crate::export_tools::guard_job(Arc::clone(&job.progress), Arc::clone(&job.result), run);
    let (progress, result) = (Arc::clone(&job.progress), Arc::clone(&job.result));
    s.jobs.push(job);
    let pending = PendingMeasure { job: job_id, id: id.clone(), sequence: seq, from_frame: from, to_frame: to, result: slot };
    s.vertical_store.measurements.pending.push(pending);
    if wait || cfg!(target_arch = "wasm32") {
        run();
        poll(s);
        return waited(s, cmd, seq, &id, job_id);
    }
    if let Err(e) = std::thread::Builder::new().name("filmcraft-measure-cuts".into()).spawn(run) {
        // the job never ran: it fails, and nothing is kept
        let why = format!("the measurement could not start: {e}");
        *lock(&progress.error) = Some(why.clone());
        *lock(&result) = Some(Err(why.clone()));
        progress.finished.store(true, Ordering::Relaxed);
        poll(s);
        return Err(EngineError::Other(format!("`{cmd}`: {why}")));
    }
    Ok(json!({"job": job_id, "measurement": id, "frames": frames, "sequence": seq.0, "analysed": [from, to]}))
}

/// The reply of a measurement run to its end inside the call: the measurement, or why it did not finish.
fn waited(s: &Session, cmd: &str, seq: ItemId, id: &str, job: u64) -> Result<Value> {
    let measurements = &s.vertical_store.measurements;
    if let Some(measured) = measurements.get(seq, id) {
        return Ok(measured_json(measured));
    }
    let why = match measurements.outcomes.get(&job) {
        Some(JobOutcome::Failed(why)) => why.clone(),
        Some(JobOutcome::Cancelled) => "the measurement was cancelled".to_string(),
        _ => "the measurement did not finish".to_string(),
    };
    Err(EngineError::Other(format!("`{cmd}`: {why}")))
}

/// A measurement in the reply shape (spec §4.2): its id, job and range, the settings it ran with, its cuts, possible cuts
/// and dropped cuts with their scores, and the per-frame scores when the caller asked for them.
fn measured_json(m: &Measured) -> Value {
    let list = |cuts: &[MeasuredCut]| -> Vec<Value> { cuts.iter().map(|c| json!({"frame": c.frame, "score": c.score})).collect() };
    let mut out = json!({
        "measurement": m.id,
        "job": m.job,
        "sequence": m.sequence.0,
        "frames": m.frames,
        "analysed": [m.from_frame, m.to_frame],
        "sensitivity": m.sensitivity,
        "minShotFrames": m.min_shot_frames,
        "threshold": m.threshold,
        "candidateFloor": m.candidate_floor,
        "cuts": list(&m.cuts),
        "candidates": list(&m.candidates),
        "dropped": list(&m.dropped),
    });
    if m.report_scores {
        out["scores"] = json!(m.scores);
    }
    out
}

/// `vertical.jobResult` (spec §7.4): the state of a measurement job (`running`, `done`, `failed` or `cancelled`), its
/// progress in frames and, once done, its measurement.
pub(crate) fn job_result(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vertical.jobResult";
    poll(s);
    let a = Params::of(cmd, p, &[&["job"]])?;
    let job = a.job("job")?.ok_or_else(|| a.missing("job"))?;
    let progress = s.jobs.iter().find(|j| j.id == job).map(|j| (j.progress.done.load(Ordering::Relaxed), j.progress.total.load(Ordering::Relaxed)));
    let (done, total) = progress.unwrap_or_default();
    let store = &s.vertical_store.measurements;
    let (state, result, error) = if store.pending.iter().any(|m| m.job == job) {
        ("running", Value::Null, None)
    } else {
        match store.outcomes.get(&job) {
            Some(JobOutcome::Done(id)) => ("done", store.find(id).map_or(Value::Null, |m| measured_json(m)), None),
            Some(JobOutcome::Failed(why)) => ("failed", Value::Null, Some(why.clone())),
            Some(JobOutcome::Cancelled) => ("cancelled", Value::Null, None),
            None => return Err(a.refuse("not_found", format!("there is no measurement job {job}"), json!({"job": job}))),
        }
    };
    Ok(json!({"job": job, "state": state, "progress": {"done": done, "total": total}, "result": result, "error": error}))
}

/// What a measurement job needs on its thread: a snapshot of the project, the media and the settings.
struct Work {
    project: Arc<Project>,
    pool: Arc<MediaPool>,
    services: Arc<dyn Services>,
    rate: FrameRate,
    /// The analysis size, as a scale of the sequence's frame.
    scale: f32,
    options: SceneOptions,
    /// The frames analysed, `[first, last)`: the range with its margins.
    analysed: [i64; 2],
    end: i64,
    /// The frames of the effective walls, sorted: every one of them counts for the minimum shot length.
    walls: Vec<i64>,
    /// The measurement as it starts: its ids, digests, range and settings.
    measured: Measured,
}

/// The job's body: measure, then hand the measurement to the session and finish the job.
fn finish(work: &Work, progress: &Progress, result: &Mutex<Option<std::result::Result<Report, String>>>, slot: &Mutex<Option<Measured>>) {
    let t0 = web_time::Instant::now();
    let outcome = analyse(work, progress);
    let seconds = t0.elapsed().as_secs_f64();
    let frames = progress.done.load(Ordering::Relaxed);
    let report = match outcome {
        Ok(measured) => {
            let (cuts, possible) = (measured.cuts.len(), measured.candidates.len());
            *lock(&progress.status) = format!("{cuts} cut(s) and {possible} possible in {frames} frame(s) ({seconds:.1}s)");
            *lock(slot) = Some(measured);
            Ok(Report { path: String::new(), frames, seconds, bytes: 0, render_fps: frames as f64 / seconds.max(1e-6), extra_files: Vec::new() })
        }
        Err(why) => {
            *lock(&progress.status) = if why == STOPPED { "Stopped: nothing was measured".to_string() } else { why.clone() };
            Err(why)
        }
    };
    *lock(result) = Some(report);
    progress.finished.store(true, Ordering::Relaxed);
}

/// The frame differences of the analysed frames: each frame of the composited program (captions off, effects on) at the
/// analysis size, reduced to the detector's signature and compared with the frame before. Stops when the job is
/// cancelled.
fn analyse(work: &Work, progress: &Progress) -> std::result::Result<Measured, String> {
    let provider = work.pool.provider(Arc::clone(&work.project), Arc::clone(&work.services));
    let options = RenderOptions { scale: work.scale, captions: false, effects: true, ..RenderOptions::default() };
    let [first, last] = work.analysed;
    let mut diffs: Vec<f32> = Vec::new();
    diffs.try_reserve_exact(usize::try_from(last - first).unwrap_or(0)).map_err(|e| format!("cannot hold the frame differences: {e}"))?;
    let mut previous: Option<Signature> = None;
    for frame in first..last {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(STOPPED.to_string());
        }
        let image = render_sequence(&work.project, work.measured.sequence, work.rate.tick_of(frame), options, &provider);
        let signature = scene::signature(&image.over_black_rgba8(), image.w, image.h);
        diffs.push(previous.as_ref().map_or(0.0, |before| scene::difference(before, &signature)));
        previous = Some(signature);
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    Ok(detect(work, &diffs))
}

/// The frame differences of an analysed range, by sequence frame.
struct Scores<'a> {
    first: i64,
    diffs: &'a [f32],
    end: i64,
}

impl Scores<'_> {
    /// The difference between frame `frame − 1` and frame `frame` (0 outside what was analysed).
    fn at(&self, frame: i64) -> f32 {
        usize::try_from(frame.saturating_sub(self.first)).ok().and_then(|i| self.diffs.get(i)).copied().unwrap_or(0.0)
    }

    /// Whether no neighbour of `frame` has a larger difference (frame 1 has none before it: frame 0 has no difference).
    fn is_peak(&self, frame: i64) -> bool {
        let d = self.at(frame);
        let next = frame.saturating_add(1);
        let larger_after = next < self.end && self.at(next) > d;
        let larger_before = frame > 1 && self.at(frame - 1) > d;
        !(larger_after || larger_before)
    }

    /// Whether the difference of `frame` stands out of the local activity: at least `2.5 · median + 0.05` of the
    /// differences of the five frames on each side (frame 0 has none; the sequence's end cuts the neighbourhood).
    fn stands_out(&self, frame: i64) -> bool {
        let (lo, hi) = (frame.saturating_sub(NEIGHBOURS).max(1), frame.saturating_add(NEIGHBOURS + 1).min(self.end));
        let mut around: Vec<f32> = (lo..hi).filter(|j| *j != frame).map(|j| self.at(j)).collect();
        around.sort_by(f32::total_cmp);
        let median = around.get(around.len() / 2).copied().unwrap_or(0.0);
        self.at(frame) >= ACTIVITY * median + ACTIVITY_FLOOR
    }

    /// Whether `frame` is a change: its difference reaches `threshold`, stands out of the local activity and is a local
    /// maximum (the detector's rules).
    fn is_change(&self, frame: i64, threshold: f32) -> bool {
        self.at(frame) >= threshold && self.stands_out(frame) && self.is_peak(frame)
    }
}

/// Where a change stands for the minimum shot length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Near {
    /// On an effective wall: a cut, which confirms the wall.
    OnWall,
    /// Fewer than `minShotFrames` frames from an effective wall or from frame 0: the shot it would make is too short.
    Wall,
    /// Neither: it is a cut unless a change fewer than `minShotFrames` frames away outranks it.
    Free,
}

/// Where `frame` stands among the effective walls `walls` (sorted) and frame 0, for a minimum shot length of `shortest`.
fn near(walls: &[i64], frame: i64, shortest: i64) -> Near {
    let i = walls.partition_point(|wall| *wall < frame);
    if walls.get(i) == Some(&frame) {
        return Near::OnWall;
    }
    let before = i.checked_sub(1).and_then(|k| walls.get(k)).copied().unwrap_or(0);
    let close = frame.saturating_sub(before) < shortest || walls.get(i).is_some_and(|after| after.saturating_sub(frame) < shortest);
    if close { Near::Wall } else { Near::Free }
}

/// Whether change `(p, dp)` (its frame and difference) outranks change `(frame, d)`: a larger difference, or an equal one
/// earlier.
fn outranks((p, dp): (i64, f32), (frame, d): (i64, f32)) -> bool {
    dp.total_cmp(&d).then(frame.cmp(&p)).is_gt()
}

/// The cuts, dropped changes, possible cuts and per-frame scores of the measurement's range.
fn detect(work: &Work, diffs: &[f32]) -> Measured {
    let m = &work.measured;
    let [first, _] = work.analysed;
    let scores = Scores { first, diffs, end: work.end };
    let threshold = work.options.threshold();
    let shortest = i64::from(m.min_shot_frames);
    let span = shortest.unsigned_abs();
    // the changes of the range and of the `shortest - 1` frames on each side of it, which decide its cuts
    let reach = shortest.saturating_sub(1);
    let (lo, hi) = (m.from_frame.saturating_sub(reach).max(1), m.to_frame.saturating_add(reach).min(work.end));
    let changes: Vec<(i64, f32)> = (lo..hi).filter(|frame| scores.is_change(*frame, threshold)).map(|frame| (frame, scores.at(frame))).collect();
    let (mut cuts, mut dropped) = (Vec::new(), Vec::new());
    for (k, &(frame, d)) in changes.iter().enumerate() {
        if !(m.from_frame..m.to_frame).contains(&frame) {
            continue;
        }
        let cut = MeasuredCut { frame, score: f64::from(d) };
        let kept = match near(&work.walls, frame, shortest) {
            Near::OnWall => true,
            Near::Wall => false,
            // off the walls: dropped when a change fewer than `shortest` frames away outranks it
            Near::Free => {
                let close = |c: &&(i64, f32)| c.0.abs_diff(frame) < span;
                let before = changes.get(..k).unwrap_or_default().iter().rev().take_while(close);
                let after = changes.get(k + 1..).unwrap_or_default().iter().take_while(close);
                !before.chain(after).any(|c| outranks(*c, (frame, d)))
            }
        };
        if kept {
            cuts.push(cut);
        } else {
            dropped.push(cut);
        }
    }
    let kept: BTreeSet<i64> = cuts.iter().map(|c| c.frame).collect();
    let possible = |frame: &i64| {
        let d = scores.at(*frame);
        d > 0.0 && f64::from(d) >= m.candidate_floor && scores.is_peak(*frame) && !kept.contains(frame)
    };
    let candidates = (m.from_frame.max(1)..m.to_frame).filter(possible).map(|frame| MeasuredCut { frame, score: f64::from(scores.at(frame)) }).collect();
    let per_frame = (m.from_frame..m.to_frame).map(|frame| scores.at(frame)).collect();
    Measured { cuts, candidates, dropped, scores: per_frame, ..m.clone() }
}

/// FNV-1a 64, streamed.
struct Fnv(u64);

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

impl std::io::Write for Fnv {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What the picture digest hashes of one sequence: its settings and its video tracks.
#[derive(Serialize)]
struct PictureView<'a> {
    settings: &'a SequenceSettings,
    tracks: Vec<TrackView<'a>>,
}

/// A video track as the picture digest sees it: whether it shows, its clips and its transitions.
#[derive(Serialize)]
struct TrackView<'a> {
    enabled: bool,
    items: &'a [TrackItem],
    transitions: &'a [Transition],
}

/// The digest of what the composited 16:9 program of sequence `seq` shows, captions aside (spec §4.2): its settings, its
/// video tracks and every item they show, through nests and subclips to the renderer's depth. A marker, a caption, an
/// audio change or a vertical plan's change keeps it; a change of a video clip, a transition or an effect changes it, an
/// Add Edit included.
pub(crate) fn picture_digest(project: &Project, seq: ItemId) -> String {
    let mut hash = Fnv(FNV_OFFSET);
    let mut seen = BTreeSet::new();
    feed_sequence(&mut hash, project, seq, 0, &mut seen);
    v::digest::digest_text(hash.0)
}

fn feed_sequence(hash: &mut Fnv, project: &Project, seq: ItemId, depth: u32, seen: &mut BTreeSet<ItemId>) {
    if depth > filmcraft_render::MAX_NEST_DEPTH || !seen.insert(seq) {
        return;
    }
    let Some(q) = project.sequence(seq) else { return };
    let tracks = q.video_tracks.iter().map(|t| TrackView { enabled: t.enabled, items: &t.items, transitions: &t.transitions }).collect();
    serde_json::to_writer(&mut *hash, &PictureView { settings: &q.settings, tracks }).ok();
    for item in q.video_tracks.iter().flat_map(|t| &t.items) {
        feed_item(hash, project, item.item, depth, seen);
    }
}

fn feed_item(hash: &mut Fnv, project: &Project, item: ItemId, depth: u32, seen: &mut BTreeSet<ItemId>) {
    match project.item(item).map(|it| &it.kind) {
        Some(ItemKind::Sequence(_)) => feed_sequence(hash, project, item, depth.saturating_add(1), seen),
        Some(kind) => {
            if depth > filmcraft_render::MAX_NEST_DEPTH || !seen.insert(item) {
                return;
            }
            serde_json::to_writer(&mut *hash, &(item, kind)).ok();
            if let ItemKind::Subclip { parent, .. } = kind {
                feed_item(hash, project, *parent, depth.saturating_add(1), seen);
            }
        }
        None => {
            serde_json::to_writer(&mut *hash, &item).ok();
        }
    }
}
