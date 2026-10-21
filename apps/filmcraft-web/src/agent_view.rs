//! `filmcraft.agentView`: the LTS agent hook of the Video editor (LTS slices FF1, R0 and R1, design
//! Appendix A.2).
//!
//! [`install`] puts it on the `window.filmcraft` object of [`crate::api`], for the LTS adapter. Like
//! the members of `window.filmcraft`, every member works in UI-thread closures
//! ([`crate::import::on_ui`]) and answers with a promise:
//!
//! ```js
//! const view = filmcraft.agentView;                 // view.version: "filmcraft.agent-view.v1"
//! await view.renderFrame({tick: "0", maxSide: 1280, deadlineMs: Date.now() + 15000, activeSequence: "7"})
//! //   {rgba: Uint8Array, width, height, tick, frame, revision, activeSequence, complete, attempts}
//! await view.scopes({tick: "0", scopes: ["waveform", "histogram"], maxSide: 960, deadlineMs: Date.now() + 15000})
//! //   {width, height, tick, frame, revision, activeSequence, complete, attempts, samples, unit, matrix, stats, …}
//! await view.executeTyped("12", "7", "timeline.trim", {clip: 41, edge: "out", mode: "regular", delta: -10160640000},
//!                         {deadlineMs: Date.now() + 15000, historyHead: {length: 3, label: "Trim"}, linkedSelection: true,
//!                          targeting: "fnv1a64:5f0c2b9e7d41a836"})
//! //   {resultVersion: "filmcraft.typed-execute.v1", outcome, code?, result?, revisionBefore, revisionAfter, …}
//! await view.watermark()                            // {revision, activeSequence, fontsetDigest, busy, linkedSelection, targeting}
//! await view.uiFallbackFont(bytes)                  // {installed}
//! view.codes                                        // {notApplied, applied, unknown, readFailures}
//! ```
//!
//! Ticks, revisions and sequence ids travel as decimal strings (INV-32).
//!
//! **Frames and scopes** are reads: `renderFrame` renders the active sequence with
//! `Session::try_render_program_at` (the CPU compositor, INV-20) and `scopes` measures
//! `scopes::scope_signal`, both certified pure by the engine's `tests/lts_agent_purity.rs` (T-FI-05 to
//! T-FI-09, T-FI-11). They never move the playhead or change the selection, and they render only the
//! active sequence: none open, or another one than the caller's `activeSequence`, is refused
//! (`sequence_not_active`). The render scale is `min(1, maxSide / long edge)` with `maxSide` at most
//! 2048, so the frame is bounded before anything is allocated (INV-28), and the requested tick snaps
//! down to its frame. While a clip's bytes are still being fetched the render shows the media pool's
//! slate, so each attempt clears the pending-media flag, renders and reads the flag in one UI-thread
//! closure; a pending attempt waits for the host's fetches (`fs::idle`, then 16 ms) and tries again
//! until the deadline (`deadlineMs`, 15 s when not given, at most 30 s ahead), and the answer says
//! whether it is `complete` and after how many `attempts`. A frame with `complete: false` is never the
//! truth. `rgba` is straight RGBA8 over black, row-major, `width × height`. A read that cannot be
//! answered rejects with an `Error` whose `code` is `sequence_not_active`, `budget_exceeded` or
//! `read_failed` (design Appendix C, read failures).
//!
//! `scopes` measures the SDR monitor picture of that frame (R'G'B' code values decimated to at most
//! 480 × 270 samples, BT.709 Y'CbCr) in percent of full scale: always `stats` (minimum, maximum and
//! mean of R, G, B and Y, the mean chroma with its hue angle and saturation), plus, as asked
//! (`scopes`, all four when not given): `waveform` (Y in 8 column bands), `parade` (R, G and B in 8
//! column bands), `histogram` (64 bins of R, G, B and Y) and `vectorscopeYuv` (the mean chroma and the
//! 8 densest cells of a 32 × 32 Cb/Cr plane spanning ±0.6).
//!
//! **`executeTyped`** dispatches one reviewed edit (the policy table of `agent_view_policy`) in a
//! single UI-thread closure that first checks, in this order, the deadline, the project revision, the
//! active sequence, the caller's linked-selection setting when it names one (turning linked
//! selection off or on bumps no revision, yet it decides which linked clips an edit of named clips
//! takes along: a trim, a move, a cut through a clip, a ripple delete, an enable, a speed or gain
//! change, an Essential Sound type), the caller's track targeting when it names one, and always for a
//! command that acts on the targeted tracks (a lift, an extract, and a transcript extract or lift)
//! or selects clips on them (selecting transcript words; changing the targeting bumps no revision
//! either), that the editor is not busy (`editor_busy`: a person's input, an open dialog or window, a
//! draft on the monitor, a recording pass or live trim playback), the caller's history head and the
//! policy table (its keys, the keys a call must give, the values it limits and the effect parameters
//! it reaches), whose native-only entries and values this browser hook refuses as `native_only`.
//! Then come the pre-checks over the engine's serde view of the active sequence: a keyframe added
//! where one already sits is `keyframe_exists` (the engine would take that keyframe away), a keyframe
//! edited or deleted where none sits is `keyframe_not_found`, a template checkbox given anything but
//! `true` or `false` is `control_kind_mismatch` (the engine would read it as `true` where it shows a
//! layer), and a ripple trim that closes time, a ripple delete or a closed gap with a sync-locked
//! track in its way is `sync_locked` (naming the tracks in `syncLockedTracks`), as is, since slice
//! R1, an insert that would cut a clip or a caption in two on a sync-locked track the call does not
//! name (the engine would cut it in silence). It answers a versioned envelope (resultVersion
//! `filmcraft.typed-execute.v1`). Every `not_applied` is decided before the command runs: those
//! refusals, and `command_disabled` when the engine refuses the command itself as not available
//! before running it. Anything else that fails after dispatch is `unknown`
//! (`engine_error_after_dispatch`, with the engine text in `detail`). A trim reports the delta it
//! applied (`actual.deltaTicks`) and whether that is the requested one (`intentSatisfied`; a clamp
//! is `code: "clamped"`). A typed marks edit (`markers.markIn`, `markers.markOut` and
//! `markers.clearInOut`, slice R1) answers the sequence's marks after it, half-open, instead of the
//! engine's null: `result.marks` holds `in`, `outInclusive` and `endExclusive` (one frame after
//! `outInclusive`), each a decimal string of ticks or null. A scene detection that only looks
//! (`lookOnly: true`) is a read: it changes nothing, and its envelope names no history label; this
//! browser hook refuses it as `native_only`, as it does every scene detection. Selecting, extracting
//! or lifting transcript words moves the playhead to the start of the words, as an extract does;
//! with the person's Selection Follows Playhead setting on, those commands, an extract and a seek
//! also replace the person's clip selection with the clips under the new playhead on the targeted
//! tracks, and other edits change
//! the person's selections as the editor's own commands do (`agent_view_policy::POLICY` lists them).
//! The playhead and the selections are the person's, so a caller reads them again before anything
//! that depends on them. `clip.enable` is a toggle: it disables clips that are all enabled. A
//! precondition sent in a form the hook does not read can never hold: a revision that is not a
//! decimal string is `revision_conflict`, an active sequence that is neither a decimal string nor
//! null is `sequence_changed`, a linked selection that is neither a boolean nor null is
//! `linked_selection_changed`, a targeting that is neither a string nor null is
//! `targeting_changed`, a history head that is not `{length, label}` is `history_head_conflict`,
//! and a missing deadline is `deadline_elapsed`. `executeTyped` always resolves with an envelope.
//!
//! **`watermark`** is the cheapest state read: the revision, the active sequence, whether the editor
//! is busy, the linked-selection setting (`linkedSelection`) and the track targeting of the active
//! sequence (`targeting`: `fnv1a64:` and 16 hex digits over its targeted tracks and source patching),
//! which a caller passes back to `executeTyped`, and `fontsetDigest`, `"sha256:"` and the hex SHA-256
//! (WebCrypto) of the compact JSON text of the registered font families as `fonts.list {system:
//! false}` answers them (null when the page has no WebCrypto).
//!
//! **`uiFallbackFont(bytes)`** adds one TrueType or OpenType font (a `Uint8Array` of at most 8 MiB) as
//! the last fallback of the editor's own panels, for scripts its interface fonts lack (Hebrew); it
//! changes no project state. Bytes that are not a single font whose table directory lies within them
//! are refused with `{installed: false, reason}`, because egui cannot load them.
//!
//! **`codes`** (slice R1) lists the codes the hook answers with, by kind: `notApplied` (the refusals
//! of `executeTyped`, among them the codes `executeChecked` rejects with, and in the LTS presentation
//! `filmcraft.request` and `filmcraft.execute`: `agent_view_policy::NOT_APPLIED`), `applied` (an
//! informative code of an applied edit), `unknown` and `readFailures` (those `renderFrame` and
//! `scopes` reject with). Each is registered in the closed code registry, so a caller can check them
//! against the registry before it wires the hook (RD-6). The object and each of its lists are frozen.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use filmcraft_color::{Matrix, rgb_to_ycbcr};
use filmcraft_engine::scopes::scope_signal;
use filmcraft_engine::{EngineError, Session};
use filmcraft_time::Tick;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::panels::{interchange_export, remix};
use serde_json::{Map, Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::agent_view_policy as policy;
use crate::api::to_js;

/// `filmcraft.agentView.version`.
pub const VERSION: &str = "filmcraft.agent-view.v1";

/// The longest side `renderFrame` and `scopes` render.
const MAX_SIDE: u32 = 2048;

/// The pause between two attempts of the pending-media loop, after the host's fetches.
const RETRY_MS: i32 = 16;

/// The deadline of a read whose caller names none, and the furthest one it may name.
const READ_MS: f64 = 15_000.0;
const MAX_READ_MS: f64 = 30_000.0;

/// The longest JSON text `executeTyped` reads as its parameters or options.
const MAX_JSON_BYTES: usize = 1 << 20;

/// The largest font `uiFallbackFont` takes.
const MAX_FONT_BYTES: u32 = 8 << 20;

/// The name of the fallback face in egui's font set.
const UI_FALLBACK_FONT: &str = "lts-ui-fallback";

/// The keys of the `renderFrame` and `scopes` arguments.
const FRAME_ARGS: [&str; 4] = ["tick", "maxSide", "deadlineMs", "activeSequence"];
const SCOPE_ARGS: [&str; 5] = ["tick", "maxSide", "deadlineMs", "activeSequence", "scopes"];

/// Column bands of the waveform and parade numbers.
const BANDS: usize = 8;

/// Bins of the histogram.
const HISTOGRAM_BINS: usize = 64;

/// The vectorscope plane: cells per side, its half-width in Cb/Cr units, and the peaks reported.
const VECTOR_CELLS: usize = 32;
const VECTOR_EXTENT: f32 = 0.6;
const VECTOR_PEAKS: usize = 8;

/// Install `filmcraft.agentView` on `window.filmcraft` (call right after [`crate::api::install`]).
pub fn install() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let filmcraft = js_sys::Reflect::get(&window, &JsValue::from_str("filmcraft"))?;
    if !filmcraft.is_object() {
        return Err(JsValue::from_str("window.filmcraft must be installed before its agent view"));
    }
    let view = js_sys::Object::new();
    js_sys::Reflect::set(&view, &JsValue::from_str("version"), &JsValue::from_str(VERSION))?;
    set_fn(
        &view,
        "renderFrame",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|args: JsValue| {
            let args = arguments(&args, &FRAME_ARGS).and_then(|args| FrameArgs::parse(&args));
            future_to_promise(async move { render_frame(args?).await.map_err(as_error) })
        })
        .into_js_value(),
    )?;
    set_fn(
        &view,
        "scopes",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|args: JsValue| {
            let args = arguments(&args, &SCOPE_ARGS).and_then(|args| ScopeArgs::parse(&args));
            future_to_promise(async move { scopes(args?).await.map_err(as_error) })
        })
        .into_js_value(),
    )?;
    set_fn(
        &view,
        "executeTyped",
        Closure::<dyn Fn(JsValue, JsValue, JsValue, JsValue, JsValue) -> js_sys::Promise>::new(
            |revision: JsValue, active_sequence: JsValue, command: JsValue, params: JsValue, options: JsValue| {
                let call = TypedCall::new(&revision, &active_sequence, &command, &params, &options);
                future_to_promise(async move { Ok(to_js(&execute_typed(call).await)) })
            },
        )
        .into_js_value(),
    )?;
    set_fn(&view, "watermark", Closure::<dyn Fn() -> js_sys::Promise>::new(|| future_to_promise(async { Ok(to_js(&watermark().await)) })).into_js_value())?;
    set_fn(
        &view,
        "uiFallbackFont",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|bytes: JsValue| {
            let font = bytes.dyn_ref::<js_sys::Uint8Array>().filter(|font| font.length() <= MAX_FONT_BYTES).map(|font| font.to_vec());
            future_to_promise(async move { Ok(to_js(&install_ui_font(font).await)) })
        })
        .into_js_value(),
    )?;
    let codes = json!({
        "notApplied": policy::NOT_APPLIED,
        "applied": policy::APPLIED,
        "unknown": policy::UNKNOWN,
        "readFailures": policy::READ_FAILURES,
    });
    let codes = to_js(&codes);
    if let Some(listed) = codes.dyn_ref::<js_sys::Object>() {
        // frozen whole: each list of codes, then the object that holds them
        for list in js_sys::Object::values(listed).iter() {
            if let Some(list) = list.dyn_ref::<js_sys::Object>() {
                js_sys::Object::freeze(list);
            }
        }
        js_sys::Object::freeze(listed);
    }
    js_sys::Reflect::set(&view, &JsValue::from_str("codes"), &codes)?;
    js_sys::Object::freeze(&view);
    js_sys::Reflect::set(&filmcraft, &JsValue::from_str("agentView"), &view)?;
    Ok(())
}

fn set_fn(object: &js_sys::Object, name: &str, f: JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(object, &JsValue::from_str(name), &f).map(|_| ())
}

/// Every rejection is an `Error`, as with the members of `window.filmcraft`.
fn as_error(error: JsValue) -> JsValue {
    if error.is_instance_of::<js_sys::Error>() {
        return error;
    }
    let message = error.as_string().unwrap_or_else(|| format!("{error:?}"));
    js_sys::Error::new(&message).into()
}

/// A read failure: an `Error` whose `code` is a registered read-failure code.
fn read_error(code: &str, message: &str) -> JsValue {
    let error = js_sys::Error::new(message);
    let _ = js_sys::Reflect::set(&error, &JsValue::from_str("code"), &JsValue::from_str(code));
    error.into()
}

fn bad_arguments(message: &str) -> JsValue {
    read_error("read_failed", message)
}

fn sequence_not_active() -> JsValue {
    read_error("sequence_not_active", "frames and scopes render the active sequence only")
}

/// A JavaScript value as JSON (through `JSON.stringify`), when it has a JSON text of at most
/// [`MAX_JSON_BYTES`].
fn json_of(value: &JsValue) -> Option<Value> {
    let text = js_sys::JSON::stringify(value).ok()?.as_string()?;
    if text.len() > MAX_JSON_BYTES {
        return None;
    }
    serde_json::from_str(&text).ok()
}

/// The argument object of a read, refused when it is not a plain object or names a key outside `keys`.
fn arguments(value: &JsValue, keys: &[&str]) -> Result<Map<String, Value>, JsValue> {
    let Some(Value::Object(args)) = json_of(value) else {
        return Err(bad_arguments("the argument must be a plain object"));
    };
    if let Some(key) = args.keys().find(|key| !keys.contains(&key.as_str())) {
        return Err(bad_arguments(&format!("unknown argument `{key}`")));
    }
    Ok(args)
}

// ------------------------------------------------------------------------------------ frames and scopes

/// What `renderFrame` (and `scopes`) render.
#[derive(Clone, Copy)]
struct FrameArgs {
    tick: i64,
    max_side: u32,
    deadline_ms: f64,
    /// The sequence the caller expects to be the active one.
    active_sequence: Option<u64>,
}

impl FrameArgs {
    fn parse(args: &Map<String, Value>) -> Result<Self, JsValue> {
        let tick = match args.get("tick") {
            Some(Value::String(tick)) => tick.parse::<i64>().ok(),
            Some(Value::Number(tick)) => tick.as_i64(),
            _ => None,
        };
        let tick = tick.ok_or_else(|| bad_arguments("`tick` must be a whole number of ticks, as a decimal string"))?;
        let max_side = args.get("maxSide").and_then(Value::as_u64).and_then(|side| u32::try_from(side).ok()).filter(|side| (1..=MAX_SIDE).contains(side));
        let max_side = max_side.ok_or_else(|| bad_arguments("`maxSide` must be a whole number from 1 to 2048"))?;
        let now = js_sys::Date::now();
        let deadline_ms = match args.get("deadlineMs") {
            None | Some(Value::Null) => Some(now + READ_MS),
            Some(deadline) => deadline.as_f64().filter(|ms| ms.is_finite()),
        };
        let deadline_ms = deadline_ms.ok_or_else(|| bad_arguments("`deadlineMs` must be a finite time in milliseconds"))?;
        let active_sequence = match args.get("activeSequence") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(id.parse::<u64>().map_err(|_| bad_arguments("`activeSequence` must be a decimal sequence id"))?),
            Some(_) => return Err(bad_arguments("`activeSequence` must be a decimal sequence id or null")),
        };
        Ok(Self { tick, max_side, deadline_ms: deadline_ms.min(now + MAX_READ_MS), active_sequence })
    }
}

/// The scopes `scopes` measures, named as `scopes.read` names them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Waveform,
    Parade,
    Histogram,
    VectorscopeYuv,
}

impl Scope {
    const ALL: [Scope; 4] = [Scope::Waveform, Scope::Parade, Scope::Histogram, Scope::VectorscopeYuv];

    fn name(self) -> &'static str {
        match self {
            Scope::Waveform => "waveform",
            Scope::Parade => "parade",
            Scope::Histogram => "histogram",
            Scope::VectorscopeYuv => "vectorscopeYuv",
        }
    }

    fn named(name: &str) -> Option<Scope> {
        Self::ALL.into_iter().find(|scope| scope.name() == name)
    }
}

/// What `scopes` measures.
#[derive(Clone)]
struct ScopeArgs {
    frame: FrameArgs,
    scopes: Vec<Scope>,
}

impl ScopeArgs {
    fn parse(args: &Map<String, Value>) -> Result<Self, JsValue> {
        let frame = FrameArgs::parse(args)?;
        let scopes = match args.get("scopes") {
            None | Some(Value::Null) => Scope::ALL.to_vec(),
            Some(Value::Array(names)) => {
                let mut scopes = Vec::new();
                for name in names {
                    let scope = name.as_str().and_then(Scope::named);
                    let scope = scope.ok_or_else(|| bad_arguments("`scopes` names waveform, parade, histogram or vectorscopeYuv"))?;
                    if !scopes.contains(&scope) {
                        scopes.push(scope);
                    }
                }
                scopes
            }
            Some(_) => return Err(bad_arguments("`scopes` must be an array of scope names")),
        };
        Ok(Self { frame, scopes })
    }
}

/// The render scale, the frame's start tick and the frame size for `args` on the active sequence.
fn target(session: &Session, args: &FrameArgs) -> Result<(f32, Tick, (usize, usize)), JsValue> {
    let active = session.state.active_sequence.map(|id| id.0);
    if args.active_sequence.is_some_and(|wanted| active != Some(wanted)) {
        return Err(sequence_not_active());
    }
    let sequence = session.active_sequence().ok_or_else(sequence_not_active)?;
    let long_edge = sequence.settings.width.max(sequence.settings.height).max(1);
    let scale = (args.max_side as f32 / long_edge as f32).min(1.0);
    let tick = session.sequence_rate().snap(Tick(args.tick).max(Tick::ZERO));
    Ok((scale, tick, filmcraft_engine::render::output_size(sequence, scale)))
}

fn frame_meta(session: &Session, width: usize, height: usize, tick: Tick) -> policy::FrameMeta {
    policy::FrameMeta {
        width,
        height,
        tick: tick.0,
        frame: session.sequence_rate().frame_at(tick),
        revision: session.revision,
        active_sequence: session.state.active_sequence.map(|id| id.0),
    }
}

fn render_error(error: EngineError) -> JsValue {
    match error {
        EngineError::NoSequence => sequence_not_active(),
        EngineError::Other(message) => read_error("budget_exceeded", &message),
        error => read_error("read_failed", &error.to_string()),
    }
}

/// One attempt of `renderFrame`, in a UI-thread closure: the frame and its report when the loop ends
/// here, `None` to try again.
fn frame_attempt(session: &Session, args: FrameArgs, attempts: u32) -> Result<Option<(Vec<u8>, Value)>, JsValue> {
    let (scale, tick, _) = target(session, &args)?;
    let _ = filmcraft_media::pending::take();
    let frame = session.try_render_program_at(scale, tick).map_err(render_error)?;
    let pending = filmcraft_media::pending::take();
    let Some(complete) = policy::attempt(pending, js_sys::Date::now(), args.deadline_ms) else {
        return Ok(None);
    };
    let report = policy::frame_report(&frame_meta(session, frame.w, frame.h, tick), complete, attempts);
    Ok(Some((frame.over_black_rgba8(), report)))
}

async fn render_frame(args: FrameArgs) -> Result<JsValue, JsValue> {
    let mut attempts: u32 = 0;
    loop {
        attempts = attempts.saturating_add(1);
        let done = crate::import::on_ui(move |w, _| frame_attempt(&w.app.session, args, attempts)).await?;
        if let Some((rgba, report)) = done {
            let frame = to_js(&report);
            let pixels: JsValue = js_sys::Uint8Array::from(rgba.as_slice()).into();
            js_sys::Reflect::set(&frame, &JsValue::from_str("rgba"), &pixels)?;
            return Ok(frame);
        }
        crate::fs::idle().await;
        crate::fs::sleep(RETRY_MS).await;
    }
}

/// One attempt of `scopes`, in a UI-thread closure: the measurements when the loop ends here, `None`
/// to try again.
fn scope_attempt(session: &Session, args: &ScopeArgs, attempts: u32) -> Result<Option<Value>, JsValue> {
    let (scale, tick, (width, height)) = target(session, &args.frame)?;
    let _ = filmcraft_media::pending::take();
    let signal = scope_signal(session, tick, scale, false).ok_or_else(sequence_not_active)?;
    let pending = filmcraft_media::pending::take();
    let Some(complete) = policy::attempt(pending, js_sys::Date::now(), args.frame.deadline_ms) else {
        return Ok(None);
    };
    let mut report = policy::frame_report(&frame_meta(session, width, height, tick), complete, attempts);
    if let Some(fields) = report.as_object_mut() {
        fields.insert("samples".into(), json!([signal.w, signal.h]));
        fields.insert("unit".into(), json!("%"));
        fields.insert("matrix".into(), json!("bt709"));
        fields.insert("stats".into(), channel_stats(&signal.rgb));
        for &scope in &args.scopes {
            fields.insert(scope.name().into(), measure(scope, signal.w, &signal.rgb));
        }
    }
    Ok(Some(report))
}

async fn scopes(args: ScopeArgs) -> Result<JsValue, JsValue> {
    let mut attempts: u32 = 0;
    loop {
        attempts = attempts.saturating_add(1);
        let request = args.clone();
        if let Some(report) = crate::import::on_ui(move |w, _| scope_attempt(&w.app.session, &request, attempts)).await? {
            return Ok(to_js(&report));
        }
        crate::fs::idle().await;
        crate::fs::sleep(RETRY_MS).await;
    }
}

// ------------------------------------------------------------------------------------ scope numbers

/// Minimum, maximum and mean of a set of levels, in percent of full scale.
#[derive(Clone, Copy, Default)]
struct Levels {
    min: f32,
    max: f32,
    sum: f64,
    count: u64,
}

impl Levels {
    fn push(&mut self, value: f32) {
        let percent = value * 100.0;
        if self.count == 0 {
            self.min = percent;
            self.max = percent;
        } else {
            self.min = self.min.min(percent);
            self.max = self.max.max(percent);
        }
        self.sum += f64::from(percent);
        self.count += 1;
    }

    fn json(&self) -> Value {
        if self.count == 0 {
            return Value::Null;
        }
        json!({"min": round2(f64::from(self.min)), "max": round2(f64::from(self.max)), "mean": round2(self.sum / self.count as f64)})
    }
}

/// Rounded to 0.01.
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn luma(r: f32, g: f32, b: f32) -> f32 {
    let [y, _, _] = rgb_to_ycbcr(r, g, b, Matrix::Bt709);
    y
}

/// The angle of a chroma point, counter-clockwise from +Cb, in degrees.
fn angle_deg(cb: f64, cr: f64) -> f64 {
    cr.atan2(cb).to_degrees().rem_euclid(360.0)
}

/// The distance of a chroma point from the centre, in percent of 0.5.
fn magnitude(cb: f64, cr: f64) -> f64 {
    cb.hypot(cr) / 0.5 * 100.0
}

/// The bin of a level in `0..=1` among `bins` equal bins (levels outside go to the end bins).
fn bin_of(unit: f32, bins: usize) -> usize {
    ((unit.clamp(0.0, 1.0) * bins as f32) as usize).min(bins.saturating_sub(1))
}

/// The levels of R, G, B and Y and the mean chroma of the whole signal.
fn channel_stats(rgb: &[[f32; 3]]) -> Value {
    let mut levels = [Levels::default(); 4];
    let (mut cb_sum, mut cr_sum) = (0.0_f64, 0.0_f64);
    for &[r, g, b] in rgb {
        let [y, cb, cr] = rgb_to_ycbcr(r, g, b, Matrix::Bt709);
        for (level, value) in levels.iter_mut().zip([r, g, b, y]) {
            level.push(value);
        }
        cb_sum += f64::from(cb);
        cr_sum += f64::from(cr);
    }
    let count = rgb.len().max(1) as f64;
    let (cb, cr) = (cb_sum / count, cr_sum / count);
    let [r, g, b, y] = levels;
    json!({
        "r": r.json(),
        "g": g.json(),
        "b": b.json(),
        "y": y.json(),
        "chromaMean": [round2(cb * 100.0), round2(cr * 100.0)],
        "hueDeg": round2(angle_deg(cb, cr)),
        "saturation": round2(magnitude(cb, cr)),
    })
}

/// The levels of one channel in [`BANDS`] column bands of a signal `width` samples wide.
fn bands(width: usize, rgb: &[[f32; 3]], channel: impl Fn([f32; 3]) -> f32) -> Vec<Value> {
    let width = width.max(1);
    let mut bands = [Levels::default(); BANDS];
    for (index, &sample) in rgb.iter().enumerate() {
        if let Some(band) = bands.get_mut((index % width) * BANDS / width) {
            band.push(channel(sample));
        }
    }
    bands.iter().map(Levels::json).collect()
}

fn histogram(rgb: &[[f32; 3]]) -> Value {
    let mut bins = [[0_u32; HISTOGRAM_BINS]; 4];
    for &[r, g, b] in rgb {
        for (channel, value) in bins.iter_mut().zip([r, g, b, luma(r, g, b)]) {
            if let Some(count) = channel.get_mut(bin_of(value, HISTOGRAM_BINS)) {
                *count = count.saturating_add(1);
            }
        }
    }
    let [r, g, b, y] = bins;
    json!({"bins": HISTOGRAM_BINS, "r": r.to_vec(), "g": g.to_vec(), "b": b.to_vec(), "y": y.to_vec()})
}

/// The cell of a Cb or Cr value on one axis of the vectorscope plane.
fn plane_cell(value: f32) -> usize {
    bin_of((value + VECTOR_EXTENT) / (2.0 * VECTOR_EXTENT), VECTOR_CELLS)
}

/// The Cb or Cr value at the centre of a cell on one axis of the vectorscope plane.
fn plane_centre(cell: usize) -> f64 {
    ((cell as f64 + 0.5) / VECTOR_CELLS as f64 * 2.0 - 1.0) * f64::from(VECTOR_EXTENT)
}

fn vectorscope(rgb: &[[f32; 3]]) -> Value {
    // Cb to the right, Cr up: row 0 holds the highest Cr
    let mut plane = [[0_u32; VECTOR_CELLS]; VECTOR_CELLS];
    let (mut cb_sum, mut cr_sum) = (0.0_f64, 0.0_f64);
    for &[r, g, b] in rgb {
        let [_, cb, cr] = rgb_to_ycbcr(r, g, b, Matrix::Bt709);
        cb_sum += f64::from(cb);
        cr_sum += f64::from(cr);
        if let Some(count) = plane.get_mut(plane_cell(-cr)).and_then(|row| row.get_mut(plane_cell(cb))) {
            *count = count.saturating_add(1);
        }
    }
    let count = rgb.len().max(1) as f64;
    let (cb, cr) = (cb_sum / count, cr_sum / count);
    let mut cells = Vec::new();
    for (row, counts) in plane.iter().enumerate() {
        for (column, &hits) in counts.iter().enumerate() {
            if hits > 0 {
                cells.push((hits, row, column));
            }
        }
    }
    cells.sort_unstable_by_key(|&(hits, row, column)| (std::cmp::Reverse(hits), row, column));
    let peaks: Vec<Value> = cells
        .iter()
        .take(VECTOR_PEAKS)
        .map(|&(hits, row, column)| {
            let (cb, cr) = (plane_centre(column), -plane_centre(row));
            json!({
                "cb": round2(cb * 100.0),
                "cr": round2(cr * 100.0),
                "angleDeg": round2(angle_deg(cb, cr)),
                "magnitude": round2(magnitude(cb, cr)),
                "share": (f64::from(hits) / count * 10_000.0).round() / 10_000.0,
            })
        })
        .collect();
    json!({
        "samples": rgb.len(),
        "cells": VECTOR_CELLS,
        "extent": round2(f64::from(VECTOR_EXTENT)),
        "mean": [round2(cb * 100.0), round2(cr * 100.0)],
        "meanAngleDeg": round2(angle_deg(cb, cr)),
        "meanMagnitude": round2(magnitude(cb, cr)),
        "peaks": peaks,
    })
}

fn measure(scope: Scope, width: usize, rgb: &[[f32; 3]]) -> Value {
    match scope {
        Scope::Waveform => json!({"columns": BANDS, "y": bands(width, rgb, |[r, g, b]| luma(r, g, b))}),
        Scope::Parade => json!({
            "columns": BANDS,
            "r": bands(width, rgb, |[r, _, _]| r),
            "g": bands(width, rgb, |[_, g, _]| g),
            "b": bands(width, rgb, |[_, _, b]| b),
        }),
        Scope::Histogram => histogram(rgb),
        Scope::VectorscopeYuv => vectorscope(rgb),
    }
}

// ------------------------------------------------------------------------------------ typed execute

/// What `executeTyped(revision, activeSequence, command, params, {deadlineMs, historyHead?,
/// linkedSelection?, targeting?})` was called with.
struct TypedCall {
    /// `None`: not a decimal revision string.
    revision: Option<u64>,
    /// `None`: neither a decimal sequence id string nor null.
    active_sequence: Option<Option<u64>>,
    /// Empty when not a string (no command has that id).
    command: String,
    /// Null when not JSON (the policy table refuses it).
    params: Value,
    /// NaN when missing or not a number (never in time).
    deadline_ms: f64,
    /// `None`: given, but not `{length, label}`.
    history_head: Option<Option<policy::HistoryHead>>,
    /// `None`: given, but neither a boolean nor null.
    linked_selection: Option<Option<bool>>,
    /// `None`: given, but neither a string nor null.
    targeting: Option<Option<String>>,
}

impl TypedCall {
    fn new(revision: &JsValue, active_sequence: &JsValue, command: &JsValue, params: &JsValue, options: &JsValue) -> Self {
        let options = json_of(options).unwrap_or_default();
        let history_head = match options.get("historyHead") {
            None | Some(Value::Null) => Some(None),
            Some(head) => history_head_of(head).map(Some),
        };
        let linked_selection = match options.get("linkedSelection") {
            None | Some(Value::Null) => Some(None),
            Some(Value::Bool(linked)) => Some(Some(*linked)),
            Some(_) => None,
        };
        let targeting = match options.get("targeting") {
            None | Some(Value::Null) => Some(None),
            Some(Value::String(digest)) => Some(Some(digest.clone())),
            Some(_) => None,
        };
        Self {
            revision: revision.as_string().and_then(|revision| revision.parse().ok()),
            active_sequence: if active_sequence.is_null() { Some(None) } else { active_sequence.as_string().and_then(|id| id.parse().ok()).map(Some) },
            command: command.as_string().unwrap_or_default(),
            params: json_of(params).unwrap_or_default(),
            deadline_ms: options.get("deadlineMs").and_then(Value::as_f64).unwrap_or(f64::NAN),
            history_head,
            linked_selection,
            targeting,
        }
    }

    /// The caller's side of the preconditions. A precondition sent in a form the hook does not read
    /// can never hold: it stands as a value the session cannot have, so [`policy::admit`] refuses it
    /// with its own code, in its place in the order.
    fn expect(&self, observed: &policy::Observed) -> policy::Expect {
        let other_revision = observed.revision.wrapping_add(1);
        let other_sequence = if observed.active_sequence.is_some() { None } else { Some(0) };
        // no targeting digest is empty
        let other_targeting = String::new();
        let other_head = policy::HistoryHead { length: observed.history_head.length.wrapping_add(1), label: String::new() };
        policy::Expect {
            revision: self.revision.unwrap_or(other_revision),
            active_sequence: self.active_sequence.unwrap_or(other_sequence),
            linked_selection: self.linked_selection.unwrap_or(Some(!observed.linked_selection)),
            targeting: self.targeting.clone().unwrap_or(Some(other_targeting)),
            deadline_ms: self.deadline_ms,
            history_head: self.history_head.clone().unwrap_or(Some(other_head)),
        }
    }
}

fn history_head_of(value: &Value) -> Option<policy::HistoryHead> {
    let head = value.as_object().filter(|head| head.len() == 2)?;
    Some(policy::HistoryHead { length: head.get("length")?.as_u64()?, label: head.get("label")?.as_str()?.to_string() })
}

/// The undo stack's length and newest label, as `history.list` reports them.
fn history_head(session: &Session) -> policy::HistoryHead {
    let undo = &session.history.undo;
    policy::HistoryHead { length: undo.len() as u64, label: undo.last().map(|(label, _)| label.clone()).unwrap_or_default() }
}

/// Whether the editor is busy with a person or with itself. It reads these, and nothing else:
/// - the person's input: a pointer held down (`ctx.input`), keyboard focus in a text field
///   (`ctx.text_edit_focused`) and egui's top modal layer of the last frame (`ctx.memory`);
/// - the app's own dialogs (`app.dialog`): About, Keyboard Shortcuts, New Sequence, Preferences,
///   Recovery, Revert, Audio Gain, Add Tracks and Delete Tracks;
/// - the dialogs the interface state keeps open (`app.ui`): `settings`, `color_dialog`,
///   `clip_dialog`, `extras.dialog`, `sync_dialog`, `edit_cameras`, `guide_dialog`,
///   `text_props_dialog`, `workspace_dialog`, `link_media`, `create_proxies`, `project_manager`,
///   `make_offline`, `save_preset`, `project_panel.dialog`, `media_browser.edit_columns`,
///   `gfx_templates.export`, `gfx_templates.replace`, `export.manager` and `export.quick_open`;
/// - the windows a panel keeps open in egui's memory, through that panel's `is_open`: AAF and OMF
///   Export Settings (`panels::interchange_export`) and Remix Properties (`panels::remix`); and the
///   error window of a caught UI panic (`app.ui_error`);
/// - the drafts on the monitor: a graphic text edit, a pen mask and a pen path (`ui.gfx_edit`,
///   `ui.mask_pen`, `ui.pen_points`);
/// - the person's recording passes: a voice-over take (`session.voiceover.recording()`); an
///   automation pass of the Audio Track Mixer that writes automation
///   (`!session.mixrec.gestures.is_empty()`: a strip or Clip Mixer track in Write mode, or a control
///   the person has touched during the pass in Latch or Touch mode), but not the pass the editor runs
///   through every forward playback at normal speed while nothing writes; and a multi-camera
///   switching pass (`session.mcrec.active()`: playback at normal speed in the Multi-Camera view,
///   recording armed, over a multi-camera clip);
/// - trim playback, a dynamic trim or a trim-mode loop (`session.trim_play.active()`), which
///   `Session::execute` would settle before anything else.
///
/// Blind spots, which leave the editor not busy: an open menu or popup; an inline field once it no
/// longer holds keyboard focus (a bin rename, the Media Browser's path), left out so that a field
/// someone forgot cannot hold the editor busy; the tool windows that stay open beside the panels
/// (the Clip and Track Fx editors, floating bins); two windows whose panel offers no `is_open`:
/// Voice-Over Record Settings (`panels::voiceover`), whose draft egui keeps under a type private to
/// the panel, so nothing outside it can read it, and the Audio Track Mixer's Show/Hide Tracks
/// (`panels::mixer`), whose open flag is a `bool` in egui's temporary memory under the panel's own
/// id string `mixer-show-hide-open`, left unread so that the hook depends on no private string of a
/// panel; an automation pass with strips in Latch or Touch mode until the person first touches a
/// control; and the browser's own dialogs outside the canvas (file pickers).
fn editor_busy(app: &FilmcraftApp, ctx: &egui::Context) -> bool {
    let ui = &app.ui;
    let dialogs = [
        ui.settings.is_some(),
        ui.color_dialog.is_some(),
        ui.clip_dialog.is_some(),
        ui.extras.dialog.is_some(),
        ui.sync_dialog.is_some(),
        ui.edit_cameras.is_some(),
        ui.guide_dialog.is_some(),
        ui.text_props_dialog.is_some(),
        ui.workspace_dialog.is_some(),
        ui.link_media.is_some(),
        ui.create_proxies.is_some(),
        ui.project_manager.is_some(),
        ui.make_offline.is_some(),
        ui.save_preset.is_some(),
        ui.project_panel.dialog.is_some(),
        ui.media_browser.edit_columns.is_some(),
        ui.gfx_templates.export.is_some(),
        ui.gfx_templates.replace.is_some(),
        ui.export.manager.is_some(),
        ui.export.quick_open,
    ];
    let windows = interchange_export::is_open(ctx) || remix::is_open(ctx) || app.ui_error.is_some();
    let drafting = ui.gfx_edit.is_some() || ui.mask_pen.is_some() || !ui.pen_points.is_empty();
    let session = &app.session;
    let recording = session.voiceover.recording() || !session.mixrec.gestures.is_empty() || session.mcrec.active();
    ctx.input(|input| input.pointer.any_down())
        || ctx.text_edit_focused()
        || ctx.memory(|memory| memory.top_modal_layer().is_some())
        || app.dialog.is_some()
        || dialogs.contains(&true)
        || windows
        || drafting
        || recording
        || session.trim_play.active()
}

/// The digest of the active sequence's track targeting (`policy::targeting_digest`): its targeted
/// tracks and its source patching, which `executeTyped` compares with the caller's.
fn targeting_of(session: &Session) -> String {
    let targeting = session.targeting();
    let targeted: Vec<u64> = targeting.targeted.iter().map(|track| track.0).collect();
    policy::targeting_digest(&targeted, targeting.video_dest.map(|track| track.0), targeting.audio_dest.map(|track| track.0))
}

/// The active sequence as the engine serializes it: the view the keyframe, template, ripple and
/// insert pre-checks read (null when none is open).
fn sequence_view(session: &Session) -> Value {
    session.active_sequence().and_then(|sequence| serde_json::to_value(sequence).ok()).unwrap_or_default()
}

/// The marks a typed marks edit reports instead of the engine's null result (slice R1): the active
/// sequence's In and Out marks after the edit, half-open (`policy::marks_result`); `None` for every
/// other command, and with no active sequence.
fn marks_of(session: &Session, command: &str) -> Option<Value> {
    let sequence = session.active_sequence()?;
    policy::marks_result(command, sequence.mark_in.map(|tick| tick.0), sequence.mark_out.map(|tick| tick.0), session.sequence_rate().frame_duration().0)
}

/// `executeTyped`: the preconditions, the policy table, the keyframe, template, ripple and insert
/// pre-checks and the dispatch in one UI-thread closure.
async fn execute_typed(call: TypedCall) -> Value {
    crate::import::on_ui(move |w, ctx| {
        let busy = editor_busy(&w.app, ctx);
        let session = &mut w.app.session;
        let observed = policy::Observed {
            now_ms: js_sys::Date::now(),
            revision: session.revision,
            active_sequence: session.state.active_sequence.map(|id| id.0),
            linked_selection: session.state.linked_selection,
            targeting: targeting_of(session),
            busy,
            history_head: history_head(session),
        };
        if let Err(code) = policy::admit(&call.command, &call.params, &call.expect(&observed), &observed, policy::Runtime::HERE) {
            return policy::refused(code, observed.revision);
        }
        if let Some(code) = policy::keyframe_refusal(&call.command, &call.params, || sequence_view(session)) {
            return policy::refused(code, observed.revision);
        }
        if let Some(code) = policy::template_refusal(&call.command, &call.params, || sequence_view(session)) {
            return policy::refused(code, observed.revision);
        }
        let in_the_way = policy::sync_locked_tracks(&call.command, &call.params, observed.linked_selection, || sequence_view(session));
        if !in_the_way.is_empty() {
            return policy::sync_locked(observed.revision, in_the_way);
        }
        let answer = match session.execute(&call.command, call.params.clone()) {
            Ok(result) => policy::Answer::Ok(marks_of(session, &call.command).unwrap_or(result)),
            Err(EngineError::Disabled(named, _)) => policy::Answer::Disabled(named),
            Err(error) => policy::Answer::Failed(error.to_string()),
        };
        let label = session.history.undo.last().map(|(label, _)| label.as_str());
        policy::settle(&call.command, &call.params, observed.revision, session.revision, label, answer)
    })
    .await
}

// ------------------------------------------------------------------------------------ watermark, UI font

/// The registered font families and their styles, as `fonts.list {system: false}` answers them, in
/// compact JSON.
fn fontset() -> String {
    Value::Array(filmcraft_text::families().into_iter().map(|(family, styles)| json!({"family": family, "styles": styles})).collect()).to_string()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        for nibble in [byte >> 4, byte & 0x0f] {
            if let Some(&digit) = DIGITS.get(usize::from(nibble)) {
                text.push(char::from(digit));
            }
        }
    }
    text
}

/// The hex SHA-256 of `data`, from WebCrypto.
async fn sha256_hex(data: &[u8]) -> Result<String, JsValue> {
    let crypto = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("crypto"))?;
    let subtle = js_sys::Reflect::get(&crypto, &JsValue::from_str("subtle"))?;
    let digest = js_sys::Reflect::get(&subtle, &JsValue::from_str("digest"))?.dyn_into::<js_sys::Function>()?;
    let bytes: JsValue = js_sys::Uint8Array::from(data).into();
    let hashing = digest.call2(&subtle, &JsValue::from_str("SHA-256"), &bytes)?.dyn_into::<js_sys::Promise>()?;
    let hash = JsFuture::from(hashing).await?;
    Ok(hex(&js_sys::Uint8Array::new(&hash).to_vec()))
}

async fn watermark() -> Value {
    let (revision, active_sequence, busy, linked_selection, targeting, fonts) = crate::import::on_ui(|w, ctx| {
        let session = &w.app.session;
        let active_sequence = session.state.active_sequence.map(|id| id.0.to_string());
        (session.revision.to_string(), active_sequence, editor_busy(&w.app, ctx), session.state.linked_selection, targeting_of(session), fontset())
    })
    .await;
    let digest = sha256_hex(fonts.as_bytes()).await.ok().map(|hash| format!("sha256:{hash}"));
    json!({
        "revision": revision,
        "activeSequence": active_sequence,
        "fontsetDigest": digest,
        "busy": busy,
        "linkedSelection": linked_selection,
        "targeting": targeting,
    })
}

/// Whether `data` is one TrueType or OpenType font whose table directory lies within it and that has
/// a character map: what egui needs to load the face when it rebuilds its fonts (it cannot recover
/// from a font it fails to parse there).
fn plausible_font(data: &[u8]) -> bool {
    const VERSIONS: [[u8; 4]; 3] = [[0, 1, 0, 0], *b"OTTO", *b"true"];
    let bytes = |at: usize, len: usize| data.get(at..at.checked_add(len)?);
    let word = |at: usize| bytes(at, 4).and_then(|word| <[u8; 4]>::try_from(word).ok());
    let (Some(version), Some(tables)) = (word(0), bytes(4, 2).and_then(|half| <[u8; 2]>::try_from(half).ok()).map(u16::from_be_bytes)) else {
        return false;
    };
    if !VERSIONS.contains(&version) || tables == 0 {
        return false;
    }
    let mut has_cmap = false;
    for table in 0..usize::from(tables) {
        let record = 12 + 16 * table;
        let (Some(tag), Some(offset), Some(length)) = (word(record), word(record + 8), word(record + 12)) else {
            return false;
        };
        let end = u64::from(u32::from_be_bytes(offset)) + u64::from(u32::from_be_bytes(length));
        if end > data.len() as u64 {
            return false;
        }
        has_cmap |= tag == *b"cmap";
    }
    has_cmap
}

async fn install_ui_font(font: Option<Vec<u8>>) -> Value {
    let Some(font) = font.filter(|font| plausible_font(font)) else {
        return json!({"installed": false, "reason": "not one TrueType or OpenType font of at most 8 MiB"});
    };
    crate::import::on_ui(move |_, ctx| {
        let families = [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
            .into_iter()
            .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest })
            .collect();
        ctx.add_font(FontInsert::new(UI_FALLBACK_FONT, egui::FontData::from_owned(font), families));
        ctx.request_repaint();
    })
    .await;
    json!({"installed": true})
}
