//! The pure half of `filmcraft.agentView` (LTS slices FF1, R0 and R1, design Appendix A.2): the
//! reviewed policy table of `executeTyped` with the exact parameter keys of each command, the keys a
//! call must give, the values some of them are limited to, the effect parameters and transition
//! settings a command may reach and where each command may run; the tables of `executeChecked`; the
//! order in which `executeTyped` checks the caller's preconditions before it dispatches; the
//! pre-checks of a keyframe command, a template value, a ripple and an insert; the versioned envelopes
//! it answers with and the codes they carry; and the verdict of the pending-media loop behind
//! `renderFrame` and `scopes`.
//!
//! It depends only on `serde_json`. The web crate compiles it for `agent_view.rs` beside it, and the
//! engine's `tests/lts_agent_purity.rs` compiles this same file natively through `#[path]` (T-FI-10a
//! to T-FI-15), so the native tests certify the code the hook runs. A native worker compiles it the
//! same way, so the browser hook and the worker share one table: an entry that only a native worker
//! may run is refused in the browser as `native_only`, and the one entry that only the browser runs,
//! a media export into the browser's file table, is refused in a native worker as
//! `use_worker_export` ([`Runtimes`]).
//!
//! The typed path never forwards a raw command (INV-12, INV-15): a command reaches the engine only
//! when it is in [`POLICY`], every parameter key it carries is one its entry allows, it gives every
//! key its entry requires ([`Entry::required`], [`Need`]), every value its entry limits is one the
//! entry's [`Rule`] takes, and the effect parameter it names is one its entry reaches ([`Target`]).
//! A call must give every key without which the engine would fail inside the command, and, in the
//! entries slice R0 adds, every key without which the engine would take the person's own state
//! instead (the playhead, the clip, caption, mask or layer selection, the Source monitor, the
//! default transition), which no precondition binds; a key the engine fills in with a fixed default
//! of its own (a mask's ellipse, alignment to the frame, the caption rules) may be left out. A key a
//! call may leave out has a rule too wherever the engine would read a value of another type as the
//! key left out: every key that names a time, a number of frames, a target (a clip, item, sequence,
//! track or layer, a character range), a font, a switch (every key its command documents as
//! `bool`), a speed, a count or a number of seconds. Every time lies within [`MAX_TICKS`] of zero,
//! and every number of frames, speed, count and duration within a bound, because the engine adds,
//! scales and multiplies them without a check. Values the table does not type (a gain, a size, a
//! position, an opacity, a frame rate, a colour, a text, a list of fillers) the engine reads as left
//! out too when they come in another type, taking its default or keeping the value it had, so the
//! caller sends them typed. A transcript, the properties of a graphic layer and the values of a
//! template are checked inside ([`Rule::Transcript`], [`Rule::Props`], [`Rule::Values`]), and a
//! template's checkbox takes `true` or `false` only ([`template_refusal`]). A command that acts on
//! the person's track targeting needs the targeting the caller read ([`Entry::targeting`]), and a
//! ripple and a keyframe edit are checked against the active sequence first ([`sync_locked_tracks`],
//! [`keyframe_refusal`]). Tier-3 commands (ambient file paths, project replacement, preferences,
//! permanent deletes, model downloads; INV-13) are absent from the table, and so is every query:
//! queries are read through `executeChecked`. So are `file.exportFrame`, which moves the playhead
//! (frames are read with `renderFrame`), `effects.toggleAnimation`, which acts at the playhead, and
//! `sequence.makeSubsequence`, which takes the person's clip selection, track targeting and In/Out.
//!
//! Slice R1 (master plan v3 W3) closes the fork's entry points and widens the table (widen-1).
//! `executeChecked` runs only what its tables hold ([`checked`]): a certified read of [`READS`], with
//! the keys it was certified with, or one of the legacy edits the LTS lab's mirror adapter sends there
//! until FF3 routes its writes through `executeTyped` ([`CHECKED_EDITS`], a place only as an
//! overwrite), under the table's own keys and values; every other engine id, and every other edit of
//! the table, is `command_not_allowed` before anything runs (D13, backlog item 16). So no edit that
//! needs the preconditions or the pre-checks of `executeTyped` runs there. Every rejection it decides
//! before dispatch carries a registered code ([`CheckedExpect`], [`checked_guard`],
//! [`checked_failure`]). In the LTS presentation the members of `window.filmcraft` that dispatch by
//! name send reads only ([`raw_request`]). Widen-1 types a video transition's settings over the
//! engine's catalogue of the ten Video Transitions folders (`VIDEO_TRANSITIONS`, [`Entry::settings`]),
//! a layer's place by index ([`Rule::IndexOr`]), an insert, which names its time and is refused
//! `sync_locked` when it would cut a clip or a caption in two on a sync-locked track it does not name
//! ([`sync_locked_tracks`]), the sequence marks, which a typed marks edit reports half-open
//! ([`marks_result`]), and a track's lock, output and mute beside its sync lock. A scene detection
//! that only looks is the table's one read ([`Entry::reads_with`], [`reads_only`]): it changes
//! nothing, and its envelope names no history label. Every code the hook's entry points answer or
//! reject with is in [`NOT_APPLIED`], [`APPLIED`], [`UNKNOWN`] or [`READ_FAILURES`], each registered
//! in the closed code registry (RD-6).

use serde_json::{Map, Value, json};

/// The version every `executeTyped` envelope names.
pub const RESULT_VERSION: &str = "filmcraft.typed-execute.v1";

/// The one folder a typed media or interchange export writes in ([`Rule::ExportPath`]). In the
/// browser it is a folder of the virtual file table that holds neither the media the editor reads
/// (`/files/`) nor the auto-save (`/opfs/`). A native worker never runs `file.exportMedia` through
/// this table, because the engine writes a media export with `std::fs` outside its file services
/// (unless the services export in memory): the worker renders through its own export, with
/// host-built paths under `outputs/<requestId>/` (MC §10). The interchange files (EDL, OTIO,
/// FCPXML) go through the engine's file services, which in a native worker are its contained ones:
/// they map this folder into the worker's workspace, or refuse it (NW-1a).
pub const EXPORT_ROOT: &str = "/lts-exports/";

/// The longest export path the table takes, in bytes.
const MAX_EXPORT_PATH: usize = 512;

/// The longest side of a frame the engine takes, in pixels (`filmcraft_project::MAX_FRAME_SIDE`).
pub const MAX_FRAME_SIDE: i64 = 32_768;

/// The furthest a point of mask or shape geometry, or a mask's move, may lie from zero on either
/// axis, in pixels: twice the longest frame side, beyond any picture the engine renders.
pub const MAX_COORDINATE: f64 = 2.0 * MAX_FRAME_SIDE as f64;

/// The longest duration or pause a call may give, in seconds (a day): in ticks it stays far from
/// what would overflow the engine's sums of times.
pub const MAX_SECONDS: f64 = 86_400.0;

/// The furthest a time, or a difference of times, may lie from zero, in ticks: the engine's own
/// `Tick::MAX` (`filmcraft_time`), the end of its representable timeline. The engine adds and
/// subtracts times without a check: four times within this bound still sum within `i64`, while a
/// time past it overflows the engine's own arithmetic (a frame snap, a trim's clamp).
pub const MAX_TICKS: i64 = i64::MAX / 4;

/// Ticks in a second (`filmcraft_time::TICKS_PER_SECOND`).
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// The longest word a transcript may give, in ticks (ten minutes): the engine maps every word
/// through each clip that shows it, scaled by the clip's speed down to [`MIN_SPEED`], without a
/// check.
pub const MAX_WORD_TICKS: i64 = 600 * TICKS_PER_SECOND;

/// The highest speaker index a word of a transcript may give: the engine adds a speaker label for
/// every index up to the highest one a word names.
pub const MAX_SPEAKER: i64 = 999;

/// The most frames a transition or a mask tracking pass may take: the most the engine's own Video
/// and Audio Transition Default Duration settings take in frames.
pub const MAX_FRAMES: i64 = 100_000;

/// The slowest and the fastest speed of a clip, in percent: the range of the editor's own
/// Speed/Duration dialog. The engine maps media time through a clip's speed without a check (the
/// words of a transcript, for one), so a slower clip could overflow it.
pub const MIN_SPEED: f64 = 0.01;
pub const MAX_SPEED: f64 = 100_000.0;

/// The fields of the engine's transcript record (`filmcraft_project::Transcript`).
const TRANSCRIPT_FIELDS: &[&str] = &["language", "source", "speakers", "words"];

/// The refusal for a command, a parameter key or a value outside the table, or a required key left out.
const COMMAND_NOT_ALLOWED: &str = "command_not_allowed";

/// The refusal in the browser of a command, or a value, that only a native worker runs.
const NATIVE_ONLY: &str = "native_only";

/// The refusal in a native worker of the media export only the browser runs through this table.
const USE_WORKER_EXPORT: &str = "use_worker_export";

/// The refusal of a ripple, or of an insert, with a sync-locked track in its way.
const SYNC_LOCKED: &str = "sync_locked";

/// The refusal of a keyframe added where one already sits (the engine would take it away).
const KEYFRAME_EXISTS: &str = "keyframe_exists";

/// The refusal of a keyframe edited or deleted where none sits.
const KEYFRAME_NOT_FOUND: &str = "keyframe_not_found";

/// The refusal of a template control's value that is not of the control's kind (a checkbox takes
/// `true` or `false`).
const CONTROL_KIND_MISMATCH: &str = "control_kind_mismatch";

/// The `code` of an `unknown` envelope: the engine failed after the command was dispatched.
const ENGINE_ERROR_AFTER_DISPATCH: &str = "engine_error_after_dispatch";

/// The informative `code` of an applied trim that moved less than requested (design Appendix C).
const CLAMPED: &str = "clamped";

/// The longest engine text an `unknown` envelope carries in `detail` (never used to classify).
const DETAIL_CHARS: usize = 300;

/// The `not_applied` codes of the hook (MC §9.9.1), every one decided before dispatch (`command_disabled` by the engine,
/// before the command runs), in the order `executeTyped` answers them: the caller's preconditions ([`admit`]), the
/// table ([`allowed`]), the pre-checks over the active sequence and the engine's own refusal. `executeChecked` rejects
/// with five of them, each the `code` of its `Error` (slice R1): `command_not_allowed` ([`checked`]),
/// `deadline_elapsed`, `revision_conflict` and `sequence_changed` ([`CheckedExpect::from_call`], [`checked_guard`]),
/// and `command_disabled` ([`checked_failure`]); a rejection of it without a code failed after dispatch. In the LTS
/// presentation the members that dispatch by name reject with `command_not_allowed` ([`raw_request`]). Each code of
/// this list and of the three below is registered in the closed code registry (`codes.registry.v1.json`), so none
/// reaches the broker as `unknown` (RD-6); the hook lists them for its caller (`agentView.codes`), which can check them
/// against the registry before it wires the hook.
pub const NOT_APPLIED: &[&str] = &[
    "deadline_elapsed",
    "revision_conflict",
    "sequence_changed",
    "linked_selection_changed",
    "targeting_changed",
    "editor_busy",
    "history_head_conflict",
    COMMAND_NOT_ALLOWED,
    NATIVE_ONLY,
    USE_WORKER_EXPORT,
    KEYFRAME_EXISTS,
    KEYFRAME_NOT_FOUND,
    CONTROL_KIND_MISMATCH,
    SYNC_LOCKED,
    "command_disabled",
];

/// The informative code of an applied outcome whose intent was not fully satisfied (a clamped trim).
pub const APPLIED: &[&str] = &[CLAMPED];

/// The code of an `unknown` envelope: the engine failed after dispatch.
pub const UNKNOWN: &[&str] = &[ENGINE_ERROR_AFTER_DISPATCH];

/// The codes `renderFrame` and `scopes` reject a read with (`agent_view.rs`).
pub const READ_FAILURES: &[&str] = &["read_failed", "sequence_not_active", "budget_exceeded"];

/// FNV-1a (64 bits), the hash of [`targeting_digest`]: its offset basis and its prime.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Where an entry may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtimes {
    /// In the browser hook and in a native worker.
    Both,
    /// In a native worker only: work that holds the editor's thread for as long as it runs (scene
    /// detection, mask tracking, the ducking analysis), which in the browser is the page's only
    /// thread, or that lands wherever the person's playhead happens to be (an audio transition).
    NativeOnly,
    /// In the browser hook only: the media export into the browser's file table ([`EXPORT_ROOT`]).
    BrowserOnly,
}

/// Where this `executeTyped` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    /// `filmcraft.agentView` in the browser (wasm32).
    Browser,
    /// A native worker (and the engine's own tests).
    Native,
}

impl Runtime {
    /// The runtime this build of the file serves: the browser hook on wasm32, a native worker on
    /// every other target.
    pub const HERE: Runtime = if cfg!(target_arch = "wasm32") { Runtime::Browser } else { Runtime::Native };
}

/// What the value of a limited parameter must be, when the caller gives that parameter.
#[derive(Debug, PartialEq)]
pub enum Rule {
    /// One of these strings (case-sensitive): one of `both` in either runtime, or one of `native`
    /// in a native worker only (`native_only` in the browser).
    OneOf { both: &'static [&'static str], native: &'static [&'static str] },
    /// A file under [`EXPORT_ROOT`], named by the caller that builds the call (the LTS adapter, never
    /// the agent): plain segments, never a folder, never a Windows device name.
    ExportPath,
    /// A time, or a difference of times, in ticks: a whole number within [`MAX_TICKS`] of zero.
    Ticks,
    /// A whole number from the first bound to the second (a frame side in pixels, a number of
    /// frames, a count).
    Within(i64, i64),
    /// A number, whole or not, from the first bound to the second (a speed in percent).
    Between(f64, f64),
    /// A number of seconds from 0 to [`MAX_SECONDS`]: a duration, or a pause length to find or keep
    /// (a negative one would make a pause removal cut into the words around the pause, and a larger
    /// one overflow the engine's sums of times).
    Seconds,
    /// An id or an index (a JSON integer from 0, within `u64`).
    Id,
    /// A non-empty array of ids.
    Ids,
    /// An id or an index, or a non-empty name (a track such as `V1`, a layer, an effect, a speaker).
    IdOrName,
    /// `true` or `false`.
    Bool,
    /// `true`: a switch that only means something when it is on (clearing a style).
    True,
    /// A string, the empty one included (a caption's text, a speaker label to set or to clear).
    Text,
    /// A string with more than white space in it (a new speaker name, a template control).
    Name,
    /// A non-empty object; when names are listed, each of its keys is one of them.
    Object(&'static [&'static str]),
    /// The properties of a graphic layer to set: a non-empty object, none of its values null, whose
    /// `enabled` (the layer's visibility) is `true` or `false` when it is given. The engine reads any
    /// other `enabled` as `true`, and fails on a null property.
    Props,
    /// The values of a template's controls, by control: a non-empty object, none of its values null
    /// (the engine would write a null to the control, which shows a layer the control hides).
    Values,
    /// A transcript as the engine's record has it ([`TRANSCRIPT_FIELDS`]), with at least one word:
    /// each a text with more than white space in it (the engine drops a blank word), a start and an
    /// end in ticks, the end no earlier than the start and at most [`MAX_WORD_TICKS`] after it, and
    /// a speaker index up to [`MAX_SPEAKER`] when it names one. Without a word the engine would
    /// clear the item's transcript.
    Transcript,
    /// Any value but null (the new value of a template control, whose kind the control decides).
    Given,
    /// A point `[x, y]` in pixels: two numbers, each within [`MAX_COORDINATE`] of zero.
    Point,
    /// At least three such points (the path of a shape).
    Points,
    /// A non-empty array of moves, each exactly `{clip, track, time}`: a clip id, a track (an id or a
    /// name such as `V2`) and a time in ticks.
    Moves,
    /// A position: one of these names, or an index from 0 within `u32` (a layer's place in the paint
    /// order: the engine reads a number of another type as 0, and the browser's `usize` has 32 bits).
    IndexOr(&'static [&'static str]),
}

/// What a call must give beyond the keys its entry always requires ([`Entry::required`]).
#[derive(Debug)]
pub enum Need {
    /// At least one of these groups of keys, each group in full (`style`, or `clear`; `control` with
    /// `value`, or `values`): without any of them the engine fails inside the command.
    AnyOf(&'static [&'static [&'static str]]),
    /// All of these keys, or none of them (a width needs its height, or the engine fails; a template
    /// control needs its value, or the engine writes null to it).
    Together(&'static [&'static str]),
    /// The key `then`, whenever the key `key` holds a value of the rule `holds` (a speaker given by
    /// its index needs the item whose transcript names it; a shape drawn as a path needs its points).
    When { key: &'static str, holds: Rule, then: &'static str },
    /// None of the keys `others`, whenever the key `key` holds a value of the rule `holds` (a scene
    /// detection that only looks names no apply choice: it applies nothing).
    Without { key: &'static str, holds: Rule, others: &'static [&'static str] },
}

impl Need {
    /// Whether `params` give what this asks for.
    fn met(&self, params: &Map<String, Value>) -> bool {
        match self {
            Need::AnyOf(groups) => groups.iter().any(|group| group.iter().all(|key| params.contains_key(*key))),
            Need::Together(keys) => keys.iter().all(|key| params.contains_key(*key)) || !keys.iter().any(|key| params.contains_key(*key)),
            Need::When { key, holds, then } => !holding(params, key, holds) || params.contains_key(*then),
            Need::Without { key, holds, others } => !holding(params, key, holds) || !others.iter().any(|other| params.contains_key(*other)),
        }
    }
}

/// Whether `params` give `key` a value `rule` takes, in either runtime.
fn holding(params: &Map<String, Value>, key: &str, rule: &Rule) -> bool {
    params.get(key).is_some_and(|value| !matches!(fit(rule, value), Fit::Nowhere))
}

/// What the value of an effect parameter, or of a transition's setting, is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A number (a slider or an angle).
    Number,
    /// A point, `[x, y]`.
    Point,
    /// A colour: `#rrggbb` or `#rrggbbaa`, or `[r, g, b]` or `[r, g, b, a]`, each from 0 to 1.
    Color,
    /// A switch: `true` or `false`.
    Switch,
    /// One of these option names, or its index.
    Choice(&'static [&'static str]),
}

/// An effect, by its id, and the parameters of it a command may reach, each with the kind of its value; or a transition,
/// by its id, and the settings a call may give it, each with the kind of its value.
pub type Target = (&'static str, &'static [(&'static str, Kind)]);

/// One reviewed command of the typed path.
#[derive(Debug)]
pub struct Entry {
    /// The engine command id.
    pub command: &'static str,
    /// The parameter keys it accepts, exactly (case-sensitive), each one its command documents. The
    /// keys decide which parameters may come; values are the adapter's and the engine's business,
    /// except those `rules` limits and the effect parameters `targets` names.
    pub keys: &'static [&'static str],
    /// The keys every call must give, each with a rule or naming an effect parameter (`effect`,
    /// `param`, `value`): left out, the engine would fail inside the command, or fill them from the
    /// person's own state or an editor default.
    pub required: &'static [&'static str],
    /// What a call must give beyond them: one of several keys, keys that come together, or a key
    /// that one value of another key needs; every key they name has a rule.
    pub needs: &'static [Need],
    /// Whether the caller must name the history head it expects (undo and redo, INV-34).
    pub history_head: bool,
    /// Whether the caller must name the track targeting it read (`watermark().targeting`): the
    /// command acts on the targeted tracks, which the person sets and no revision counts.
    pub targeting: bool,
    /// Where it may run.
    pub runtimes: Runtimes,
    /// The keys whose values are limited, and to what.
    pub rules: &'static [(&'static str, Rule)],
    /// The effect parameters a call may name with `effect` and `param` (and set with `value`), as
    /// pairs; empty when the command names no effect parameter.
    pub targets: &'static [Target],
    /// The transitions a call may name with `effect`, each with the settings its `params` may give,
    /// by kind; empty when the command takes no transition settings.
    pub settings: &'static [Target],
    /// The switch with which a call only reads: given `true`, the command reports what it finds and
    /// changes nothing (no revision, no history step), so a caller admits it as a read (tier 0);
    /// `None` when every call is an edit.
    pub reads_with: Option<&'static str>,
}

/// A reviewed edit that takes `keys`, none of them required or limited, in both runtimes.
const fn edit(command: &'static str, keys: &'static [&'static str]) -> Entry {
    Entry {
        command,
        keys,
        required: &[],
        needs: &[],
        history_head: false,
        targeting: false,
        runtimes: Runtimes::Both,
        rules: &[],
        targets: &[],
        settings: &[],
        reads_with: None,
    }
}

/// A certified read of `executeChecked` that takes `keys`, none of them required or limited (slice R1).
const fn read(command: &'static str, keys: &'static [&'static str]) -> Entry {
    edit(command, keys)
}

/// Undo or redo: no parameter, and only against the caller's own history head.
const fn history_step(command: &'static str) -> Entry {
    Entry {
        command,
        keys: &[],
        required: &[],
        needs: &[],
        history_head: true,
        targeting: false,
        runtimes: Runtimes::Both,
        rules: &[],
        targets: &[],
        settings: &[],
        reads_with: None,
    }
}

impl Entry {
    /// The entry, with the keys a call must give.
    const fn requires(mut self, required: &'static [&'static str]) -> Entry {
        self.required = required;
        self
    }

    /// The entry, with what a call must give beyond its required keys.
    const fn needs(mut self, needs: &'static [Need]) -> Entry {
        self.needs = needs;
        self
    }

    /// The entry, with the values some of its keys are limited to.
    const fn limits(mut self, rules: &'static [(&'static str, Rule)]) -> Entry {
        self.rules = rules;
        self
    }

    /// The entry, with the effect parameters it reaches.
    const fn reaches(mut self, targets: &'static [Target]) -> Entry {
        self.targets = targets;
        self
    }

    /// The entry, with the transitions a call may name and the settings each takes.
    const fn with_settings(mut self, settings: &'static [Target]) -> Entry {
        self.settings = settings;
        self
    }

    /// The entry, with the switch with which a call only reads.
    const fn reads_with(mut self, switch: &'static str) -> Entry {
        self.reads_with = Some(switch);
        self
    }

    /// The entry, for a command that acts on the tracks the person targets.
    const fn targeted(mut self) -> Entry {
        self.targeting = true;
        self
    }

    /// The entry, for a native worker only.
    const fn native(mut self) -> Entry {
        self.runtimes = Runtimes::NativeOnly;
        self
    }

    /// The entry, for the browser hook only.
    const fn browser(mut self) -> Entry {
        self.runtimes = Runtimes::BrowserOnly;
        self
    }
}

/// One of these names, in either runtime.
const fn one_of(names: &'static [&'static str]) -> Rule {
    Rule::OneOf { both: names, native: &[] }
}

/// The Motion intrinsic, which reframing sets (master plan v2 §3.1 tier A, CC-26).
const MOTION: Target = ("motion", &[("position", Kind::Point), ("scale", Kind::Number), ("rotation", Kind::Number), ("anchor", Kind::Point)]);

/// What `effects.setParam` may set: the Motion intrinsic only. None of its parameters is text, so no
/// file is reachable (Lumetri names its LUT files in text parameters, and its commands for them are
/// tier 3).
const REFRAME: &[Target] = &[MOTION];

/// What the keyframe commands animate: Motion (reframing), Opacity (picture fades) and Volume (sound
/// fades), none with a text parameter.
const ANIMATE: &[Target] = &[MOTION, ("opacity", &[("opacity", Kind::Number)]), ("volume", &[("level", Kind::Number)])];

/// The edges of a clip.
const EDGE: Rule = one_of(&["in", "out"]);

/// The audio layout of a new sequence's mix and of its audio tracks (`Stereo` and `Standard` are the
/// same): the engine refuses any other name from inside the command.
const NEW_SEQUENCE: &[(&str, Rule)] =
    &[("mix", one_of(&["Stereo", "Mono", "5.1", "Adaptive"])), ("trackType", one_of(&["Standard", "Mono", "5.1", "Adaptive"]))];

/// The properties of a graphic layer to set ([`Rule::Props`]), on a graphic clip by id and a layer
/// of it by index or name, at a time (ticks): the engine would read a clip, a layer or a time of
/// another type as left out, and take the person's clip or layer selection or the playhead instead.
const PROPS: &[(&str, Rule)] = &[("clip", Rule::Id), ("layer", Rule::IdOrName), ("props", Rule::Props), ("time", Rule::Ticks)];

/// The item to place, the track of its picture and the track of its sound, the time, whether it is
/// inserted, and its source range (ticks): the engine would read a value of another type as left
/// out, and take the playhead, an overwrite or the Source monitor's In and Out points instead (a
/// track of another type fails inside the command). An insert names its time ([`INSERT_TIME`]).
const PLACE: &[(&str, Rule)] = &[
    ("item", Rule::Id),
    ("track", Rule::IdOrName),
    ("audioTrack", Rule::IdOrName),
    ("time", Rule::Ticks),
    ("insert", Rule::Bool),
    ("sourceIn", Rule::Ticks),
    ("duration", Rule::Ticks),
];

/// An insert names its time (slice R1): the insert pre-check ([`sync_locked_tracks`]) looks there for the clips and
/// captions it would cut in two, which it could not do at the playhead.
const INSERT_TIME: &[Need] = &[Need::When { key: "insert", holds: Rule::True, then: "time" }];

/// A new title: its time (ticks), its duration in seconds, the video track it goes on by index from
/// V1 (the engine reads no track name there), whether its text stays as written, whether it starts
/// a graphic clip of its own, and its font and font style. The engine would read a value of another
/// type as left out, and take the playhead, the free track above the clips there, or the person's
/// smart-quote and default-font settings instead.
const TITLE: &[(&str, Rule)] = &[
    ("time", Rule::Ticks),
    ("seconds", Rule::Seconds),
    ("track", Rule::Id),
    ("literal", Rule::Bool),
    ("newClip", Rule::Bool),
    ("font", Rule::Name),
    ("fontStyle", Rule::Name),
];

/// A cut at a time, on a track or through a clip and its linked clips: the engine would read a time
/// or a clip of another type as left out, and cut at the playhead, or across every track (a track
/// of another type fails inside the command).
const RAZOR: &[(&str, Rule)] = &[("time", Rule::Ticks), ("track", Rule::IdOrName), ("clip", Rule::Id)];

/// A trim: a clip by id, an edge, a mode (the engine reads any other edge as `out` and any other
/// mode as `regular`) and the delta in ticks.
const TRIM: &[(&str, Rule)] = &[("clip", Rule::Id), ("edge", EDGE), ("mode", one_of(&["regular", "ripple"])), ("delta", Rule::Ticks)];

/// The options of a transition's direction, the quality of its anti-aliasing, its axis, the corner it starts from, the
/// shapes it draws, the way it spins, the order of its shapes and the picture a luma fade follows, each list exactly as
/// the engine's catalogue has it (`filmcraft_project::vtransition`): a call names an option by its exact name or its
/// index (the engine would also take another case of a name).
const DIRECTIONS: &[&str] = &["From North", "From East", "From South", "From West"];
const ANTIALIASING: &[&str] = &["Off", "Low", "Medium", "High"];
const AXES: &[&str] = &["Horizontal", "Vertical"];
const CORNERS: &[&str] = &["Top Left", "Top Right", "Bottom Right", "Bottom Left"];
const TRANSITION_SHAPES: &[&str] = &["Circle", "Square", "Diamond", "Star", "Heart", "Triangle", "Hexagon"];
const SPINS: &[&str] = &["Clockwise", "Counter-clockwise"];
const SHAPE_ORDERS: &[&str] = &["Random", "Left to Right", "Center Out"];
const LUMA_SOURCES: &[&str] = &["Outgoing", "Incoming"];

/// The settings several transitions share, as the engine's catalogue defines them: a direction, a motion blur, a
/// random seed, an amount, a centre, a feather, a softness, the shape drawn, and the border of a wipe or an iris (its
/// width, its colour and the quality of its anti-aliasing).
const DIRECTION: (&str, Kind) = ("direction", Kind::Choice(DIRECTIONS));
const MOTION_BLUR: (&str, Kind) = ("motion_blur", Kind::Number);
const SEED: (&str, Kind) = ("seed", Kind::Number);
const AMOUNT: (&str, Kind) = ("amount", Kind::Number);
const CENTER: (&str, Kind) = ("center", Kind::Point);
const FEATHER: (&str, Kind) = ("feather", Kind::Number);
const SOFTNESS: (&str, Kind) = ("softness", Kind::Number);
const TRANSITION_SHAPE: (&str, Kind) = ("shape", Kind::Choice(TRANSITION_SHAPES));
const BORDER_WIDTH: (&str, Kind) = ("border_width", Kind::Number);
const BORDER_COLOR: (&str, Kind) = ("border_color", Kind::Color);
const ANTIALIAS: (&str, Kind) = ("antialias", Kind::Choice(ANTIALIASING));

/// The video transitions a typed call may apply (slice R1, widen-1): the engine's catalogue of the ten Video
/// Transitions folders, in its order (`filmcraft_project::vtransition`; the Legacy and Obsolete transitions are left
/// out), each with its settings: its parameters in the engine's order, by kind (a number for a slider or an angle; none
/// is a text). A call may give any of them in `params` and leave the others at the transition's defaults; the engine
/// fails inside the command on a setting the transition does not have, and reads a value of another kind its own way,
/// so neither reaches it. The engine places a transition as its own command places every one: a two-sided one centred
/// on the cut, a one-sided one ending at a clip's Out point or starting at an In point with nothing before it. It
/// documents no key for another alignment (an engine change, widen-2).
const VIDEO_TRANSITIONS: [Target; 84] = [
    // Animation
    ("block_motion", &[DIRECTION, ("blocks", Kind::Number), MOTION_BLUR]),
    ("flip_motion", &[DIRECTION, ("perspective", Kind::Number), MOTION_BLUR]),
    ("fold_motion", &[DIRECTION, ("perspective", Kind::Number)]),
    ("pop_motion", &[("overshoot", Kind::Number), MOTION_BLUR]),
    ("pull_motion", &[DIRECTION, MOTION_BLUR]),
    ("spin_motion", &[("rotation", Kind::Number), ("spin", Kind::Choice(SPINS)), MOTION_BLUR]),
    ("spring_motion", &[DIRECTION, ("bounce", Kind::Number), MOTION_BLUR]),
    ("travel_motion", &[DIRECTION, ("zoom", Kind::Number), MOTION_BLUR]),
    // Dissolve
    ("additive_dissolve", &[]),
    ("blur_dissolve", &[("blur", Kind::Number)]),
    ("burn_alpha", &[("burn_color", Kind::Color), SOFTNESS, SEED]),
    ("cross_dissolve", &[]),
    ("dip_to_black", &[]),
    ("dip_to_color", &[("color", Kind::Color), ("hold", Kind::Number)]),
    ("dip_to_white", &[]),
    ("film_dissolve", &[]),
    ("luma_fade", &[("source", Kind::Choice(LUMA_SOURCES)), SOFTNESS, ("invert", Kind::Switch)]),
    ("morph_cut", &[]),
    ("mosaic_transition", &[("block_size", Kind::Number), ("dissolve", Kind::Switch)]),
    // Grunge & Distort
    ("chaos", &[AMOUNT, SEED]),
    ("earthquake", &[("shake", Kind::Number), SEED, MOTION_BLUR]),
    ("flicker", &[("flickers", Kind::Number), SEED]),
    ("glass", &[("cell_size", Kind::Number), ("refraction", Kind::Number), SEED]),
    ("glitch", &[AMOUNT, SEED]),
    ("grunge", &[("scale", Kind::Number), ("edge_color", Kind::Color), ("edge_width", Kind::Number), SEED]),
    ("kaleidoscope", &[("segments", Kind::Number), ("zoom", Kind::Number)]),
    ("liquid_distortion", &[AMOUNT, ("scale", Kind::Number), SEED]),
    ("tv_power", &[("glow_color", Kind::Color), ("line", Kind::Number)]),
    ("vhs_damage", &[AMOUNT, SEED]),
    // Immersive Video
    ("vr_chroma_leaks", &[AMOUNT, SEED]),
    ("vr_gradient_wipe", &[SOFTNESS, ("invert", Kind::Switch)]),
    ("vr_iris_wipe", &[CENTER, FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS]),
    ("vr_light_leaks", &[AMOUNT, SEED]),
    ("vr_light_rays", &[CENTER, AMOUNT]),
    ("vr_mobius_zoom", &[("zoom", Kind::Number), ("twist", Kind::Number)]),
    ("vr_random_blocks", &[("block_size", Kind::Number), SOFTNESS, SEED]),
    ("vr_spherical_blur", &[("blur", Kind::Number)]),
    // Lights & Blurs
    ("burn_chroma", &[AMOUNT]),
    ("chroma_leak", &[AMOUNT, SEED]),
    ("cross_zoom", &[CENTER, ("strength", Kind::Number)]),
    ("directional_blur_transition", &[("angle", Kind::Number), ("blur", Kind::Number)]),
    ("flare", &[("color", Kind::Color), AMOUNT, ("angle", Kind::Number)]),
    ("flash", &[("color", Kind::Color), ("intensity", Kind::Number)]),
    ("glow", &[("radius", Kind::Number), ("intensity", Kind::Number)]),
    ("lens_blur", &[("radius", Kind::Number)]),
    ("light_leak", &[("color", Kind::Color), AMOUNT, SEED]),
    ("light_sweep", &[("angle", Kind::Number), ("width", Kind::Number), ("color", Kind::Color), AMOUNT]),
    ("phosphor", &[("color", Kind::Color), ("decay", Kind::Number)]),
    ("radial_blur", &[CENTER, ("angle", Kind::Number)]),
    ("ray", &[CENTER, AMOUNT, ("length", Kind::Number)]),
    ("solarize", &[AMOUNT]),
    ("stripe", &[("stripes", Kind::Number), ("angle", Kind::Number), ("color", Kind::Color)]),
    ("zoom_blur", &[CENTER, ("strength", Kind::Number)]),
    // Slide
    ("roll_3d", &[DIRECTION, ("perspective", Kind::Number)]),
    ("film_roll", &[DIRECTION, ("gap", Kind::Number), ("gap_color", Kind::Color), MOTION_BLUR]),
    ("push", &[DIRECTION, MOTION_BLUR]),
    ("roll", &[DIRECTION, MOTION_BLUR]),
    ("slide", &[DIRECTION, MOTION_BLUR]),
    ("split", &[("orientation", Kind::Choice(AXES)), MOTION_BLUR]),
    ("stretch", &[DIRECTION]),
    ("whip", &[DIRECTION, ("blur", Kind::Number)]),
    // Smart Tools
    ("motion_camera", &[("zoom", Kind::Number), ("rotation", Kind::Number), MOTION_BLUR]),
    ("motion_tween", &[("scale", Kind::Number)]),
    ("shape_dissolve", &[TRANSITION_SHAPE, ("size", Kind::Number), SOFTNESS, ("rotation", Kind::Number), ("order", Kind::Choice(SHAPE_ORDERS)), SEED]),
    ("shape_flow", &[TRANSITION_SHAPE, DIRECTION, SOFTNESS]),
    // Text
    ("text_animator", &[("columns", Kind::Number), ("rows", Kind::Number)]),
    ("typewriter", &[("columns", Kind::Number), ("rows", Kind::Number), ("cursor", Kind::Switch)]),
    // Transformers
    ("spin_3d", &[DIRECTION, ("perspective", Kind::Number)]),
    ("spinback_3d", &[DIRECTION, ("depth", Kind::Number)]),
    ("frame", &[("frame_width", Kind::Number), ("frame_color", Kind::Color), ("scale", Kind::Number)]),
    ("louver", &[("slats", Kind::Number), ("orientation", Kind::Choice(AXES))]),
    ("mirror_transition", &[DIRECTION]),
    ("page_peel", &[("corner", Kind::Choice(CORNERS)), ("radius", Kind::Number), ("shadow", Kind::Switch)]),
    ("slice", &[("slices", Kind::Number), ("angle", Kind::Number), MOTION_BLUR]),
    ("wave", &[DIRECTION, ("amplitude", Kind::Number), ("wavelength", Kind::Number)]),
    // Wipe
    ("clock_wipe", &[CENTER, ("start", Kind::Number), FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS]),
    ("linear_wipe", &[("angle", Kind::Number), FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS]),
    ("neon_wipe", &[("angle", Kind::Number), ("color", Kind::Color), ("glow", Kind::Number), AMOUNT]),
    ("panel_wipe", &[DIRECTION, ("panels", Kind::Number), FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS]),
    ("plateau_wipe", &[("angle", Kind::Number), ("plateaus", Kind::Number), FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS]),
    ("radial_wipe", &[("corner", Kind::Choice(CORNERS)), FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS]),
    ("soft_wipe", &[("angle", Kind::Number), FEATHER]),
    ("star_wipe", &[CENTER, ("points", Kind::Number), ("inner", Kind::Number), ("rotation", Kind::Number), FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS]),
    ("stretch_wipe", &[DIRECTION, ("stretch", Kind::Number)]),
];

/// The ids of `transitions`, in order: the names a call may give its `effect`.
const fn transition_ids<const N: usize>(transitions: &[Target; N]) -> [&'static str; N] {
    let mut ids = [""; N];
    let mut index = 0;
    while index < N {
        ids[index] = transitions[index].0;
        index += 1;
    }
    ids
}

/// The ids of [`VIDEO_TRANSITIONS`].
const VIDEO_TRANSITION_IDS: [&str; 84] = transition_ids(&VIDEO_TRANSITIONS);

/// A video transition on a clip's edge: one of the engine's catalogue ([`VIDEO_TRANSITIONS`]), for a number of frames,
/// and whether it plays in reverse (the engine would read a switch of another type as left out: forward).
const VIDEO_TRANSITION: &[(&str, Rule)] =
    &[("clip", Rule::Id), ("effect", one_of(&VIDEO_TRANSITION_IDS)), ("frames", Rule::Within(1, MAX_FRAMES)), ("edge", EDGE), ("reverse", Rule::Bool)];

/// The shapes of a new shape layer, the points of one drawn as a path, and the graphic clip it joins
/// (the engine would read a clip of another type as left out, and make a new clip at the playhead).
const SHAPE: &[(&str, Rule)] = &[("shape", one_of(&["rectangle", "ellipse", "polygon", "path"])), ("points", Rule::Points), ("clip", Rule::Id)];

/// A shape drawn as a path needs its points (the engine fails without three of them).
const PATH_POINTS: &[Need] = &[Need::When { key: "shape", holds: one_of(&["path"]), then: "points" }];

/// A section of Lumetri Color, by the name the engine documents, on a clip by id, and whether it is
/// on: the engine would read a clip of another type as left out and take the person's selection,
/// and a switch of another type as a toggle.
const SECTION: &[(&str, Rule)] = &[("clip", Rule::Id), ("section", one_of(&["basic", "creative", "curves", "wheels", "hsl", "vignette"])), ("on", Rule::Bool)];

/// Clips moved to tracks and times, whether the move inserts, and whether linked clips follow: the
/// engine would read a switch of another type as left out, and overwrite, or follow the person's
/// linked-selection setting.
const MOVES: &[(&str, Rule)] = &[("moves", Rule::Moves), ("insert", Rule::Bool), ("linked", Rule::Bool)];

/// The sixteen label colours of the engine (`Label`), which a marker takes (the engine reads another
/// name as Green, or leaves the colour as it was).
const LABELS: &[&str] =
    &["Violet", "Iris", "Caribbean", "Lavender", "Cerulean", "Forest", "Rose", "Mango", "Purple", "Blue", "Teal", "Magenta", "Tan", "Green", "Brown", "Yellow"];

/// A new marker's time (ticks; the engine would read one of another type as left out, and take the
/// playhead) and its colour.
const MARKER: &[(&str, Rule)] = &[("time", Rule::Ticks), ("color", one_of(LABELS))];

/// A marker by id, and its colour.
const MARKER_EDIT: &[(&str, Rule)] = &[("marker", Rule::Id), ("color", one_of(LABELS))];

/// Clips by id (the engine would read clips of another type as left out, and take the person's
/// selection), how an audio gain applies, and, without a mode, whether it adjusts the gain (the
/// engine would read a switch of another type as left out, and adjust a gain the caller set).
const GAIN: &[(&str, Rule)] = &[("clips", Rule::Ids), ("mode", one_of(&["set", "adjust", "normalizeMax", "normalizeAll"])), ("relative", Rule::Bool)];

/// A clip, at a sequence time (ticks).
const CLIP_AT: &[(&str, Rule)] = &[("clip", Rule::Id), ("time", Rule::Ticks)];

/// A clip, and a keyframe by its media time (ticks).
const CLIP_KEYFRAME: &[(&str, Rule)] = &[("clip", Rule::Id), ("mediaTime", Rule::Ticks)];

/// A keyframe again, with the interpolations the engine documents (it reads any other name as
/// linear, which would change a keyframe in silence).
const INTERPOLATION: &[(&str, Rule)] = &[
    ("clip", Rule::Id),
    ("mediaTime", Rule::Ticks),
    ("interpolation", one_of(&["linear", "bezier", "autoBezier", "continuousBezier", "hold", "easeIn", "easeOut"])),
];

/// Clips by id.
const CLIPS: &[(&str, Rule)] = &[("clips", Rule::Ids)];

/// Clips by id, at a sequence time.
const CLIPS_AT: &[(&str, Rule)] = &[("clips", Rule::Ids), ("time", Rule::Ticks)];

/// A track (an id, or a name such as `V2`) at a sequence time.
const TRACK_AT: &[(&str, Rule)] = &[("track", Rule::IdOrName), ("time", Rule::Ticks)];

/// A track (an id, or a name such as `V2`) and the switches a call sets on it (slice R1): its sync lock, its lock, its
/// output (the eye of a video track) and its mute; the engine would read a switch of another type as left out, and
/// keep it as it was. Its solo, name, volume and pan are not keys (widen-2).
const TRACK_STATE: &[(&str, Rule)] =
    &[("track", Rule::IdOrName), ("syncLock", Rule::Bool), ("locked", Rule::Bool), ("enabled", Rule::Bool), ("muted", Rule::Bool)];

/// A track call sets at least one switch: without one the engine makes an undo step that changes nothing.
const TRACK_SWITCHES: &[Need] = &[Need::AnyOf(&[&["syncLock"], &["locked"], &["enabled"], &["muted"]])];

/// An audio transition on a clip: one of the engine's audio transitions, for a number of frames.
const AUDIO_TRANSITION: &[(&str, Rule)] =
    &[("clip", Rule::Id), ("effect", one_of(&["constant_power", "constant_gain", "exponential_fade"])), ("frames", Rule::Within(1, MAX_FRAMES))];

/// The keys of `file.exportMedia`.
const EXPORT_MEDIA_KEYS: &[&str] = &[
    "path",
    "format",
    "width",
    "height",
    "fps",
    "bitrateKbps",
    "bitrateMode",
    "scale",
    "audio",
    "quality",
    "burnCaptions",
    "captionSidecar",
    "proresProfile",
    "dnxProfile",
    "apvProfile",
    "mxfVideoCodec",
    "sequence",
    "range",
];

/// `file.exportMedia`: into the export root, the sequence's own ranges (a custom range takes float
/// seconds, INV-32), and the formats written as a stepped job with a built-in encoder, which the
/// browser advances between UI frames. GIF, WAV and image sequences are written in one synchronous
/// run, which would hold the browser's thread for the whole export, so only a native worker writes
/// them (here: `native_only`, then the worker's own export). HEVC has no built-in encoder: it needs
/// an encoder the system registers, which no runtime of the typed path has, so it is not here. A
/// frame size is whole pixels the engine takes; the profile, codec and sidecar names are the ones it
/// documents (it reads an unknown ProRes, DNxHR or APV profile as its default one in silence); the
/// sequence is an id (it would read one of another type as left out, and export the active one);
/// whether the sound goes along and the captions are burned in are switches (it would read one of
/// another type as left out: with the sound, without burned-in captions).
const EXPORT_MEDIA: &[(&str, Rule)] = &[
    ("path", Rule::ExportPath),
    ("sequence", Rule::Id),
    ("format", Rule::OneOf { both: &["h264", "prores", "dnxhr", "apv", "mjpeg", "mxf-op1a", "mxf-opatom"], native: &["gif", "wav", "png", "tiff", "bmp"] }),
    ("range", one_of(&["entire", "inOut", "workArea"])),
    ("width", Rule::Within(1, MAX_FRAME_SIDE)),
    ("height", Rule::Within(1, MAX_FRAME_SIDE)),
    ("bitrateMode", one_of(&["cbr", "vbr1Pass", "vbr2Pass"])),
    ("audio", Rule::Bool),
    ("burnCaptions", Rule::Bool),
    ("proresProfile", one_of(&["proxy", "lt", "standard", "hq"])),
    ("dnxProfile", one_of(&["lb", "sq", "hq", "hqx"])),
    ("apvProfile", one_of(&["422-10", "422-12", "444-10", "444-12"])),
    ("mxfVideoCodec", one_of(&["dnxhr", "proRes", "h264"])),
    ("captionSidecar", one_of(&["srt", "vtt"])),
];

/// A frame size comes whole: the engine fails on a width without a height, or a height without a
/// width.
const FRAME_SIZE: &[Need] = &[Need::Together(&["width", "height"])];

/// An interchange file (EDL, OTIO, FCPXML), into the export root.
const EXPORT_FILE: &[(&str, Rule)] = &[("path", Rule::ExportPath)];

/// A background job.
const JOB: &[(&str, Rule)] = &[("job", Rule::Id)];

/// A transcript for a media item ([`Rule::Transcript`]): the engine would drop any other field in
/// silence, and replace the item's transcript with one without words, clearing it.
const TRANSCRIPT: &[(&str, Rule)] = &[("item", Rule::Id), ("transcript", Rule::Transcript)];

/// A stretch of the transcript, by word index.
const WORDS: &[(&str, Rule)] = &[("from", Rule::Id), ("to", Rule::Id)];

/// Pauses to remove: the shortest pause that counts and the time kept at each side of it, in
/// seconds (a negative time kept would cut into the words around the pause).
const PAUSES: &[(&str, Rule)] = &[("minSeconds", Rule::Seconds), ("keepSeconds", Rule::Seconds)];

/// A transcript speaker, by name or index, its new name, and the media item whose transcript a
/// speaker index counts in. The engine reads an `item` that names no media item as no `item`, so a
/// rename by name then renames that speaker in every transcript: the adapter checks `item` over the
/// certified project read first.
const RENAME: &[(&str, Rule)] = &[("speaker", Rule::IdOrName), ("name", Rule::Name), ("item", Rule::Id)];

/// A speaker given by its index needs the item (the engine fails without it).
const SPEAKER_INDEX: &[Need] = &[Need::When { key: "speaker", holds: Rule::Id, then: "item" }];

/// The caption formats of the engine (`CaptionFormat`), which it would otherwise replace with its
/// default in silence.
const CAPTION_FORMATS: &[&str] = &["Subtitle", "CEA-608", "CEA-708", "Teletext"];

/// A caption track's format.
const CAPTION_FORMAT: &[(&str, Rule)] = &[("format", one_of(CAPTION_FORMATS))];

/// Captions made from the transcript: their format, at most 200 characters a line, one or two lines
/// a caption (as the engine documents), a gap of up to 1000 frames between captions, and their
/// shortest and longest durations in seconds. The engine multiplies the characters by the lines,
/// and the gap by the frame duration, without a check: larger numbers would overflow.
const TRANSCRIPT_CAPTIONS: &[(&str, Rule)] = &[
    ("format", one_of(CAPTION_FORMATS)),
    ("maxChars", Rule::Within(1, 200)),
    ("lines", Rule::Within(1, 2)),
    ("gapFrames", Rule::Within(0, 1000)),
    ("minSeconds", Rule::Seconds),
    ("maxSeconds", Rule::Seconds),
];

/// A sequence time.
const AT: &[(&str, Rule)] = &[("time", Rule::Ticks)];

/// A new caption's track (an id, or `C1` and on), its time (ticks) and its duration in seconds: the
/// engine would read a track of another type as left out, and take the first caption track.
const NEW_CAPTION: &[(&str, Rule)] = &[("track", Rule::IdOrName), ("time", Rule::Ticks), ("durationSeconds", Rule::Seconds)];

/// A caption's text and speaker label (an empty label clears it).
const CAPTION_TEXT: &[(&str, Rule)] = &[("caption", Rule::Id), ("text", Rule::Text), ("speaker", Rule::Text)];

/// A caption edit names its text, its speaker, or both (the engine fails with neither).
const TEXT_OR_SPEAKER: &[Need] = &[Need::AnyOf(&[&["text"], &["speaker"]])];

/// A caption, at a sequence time.
const CAPTION_AT: &[(&str, Rule)] = &[("caption", Rule::Id), ("time", Rule::Ticks)];

/// A caption and its new bounds (ticks).
const CAPTION_TIMES: &[(&str, Rule)] = &[("caption", Rule::Id), ("startTime", Rule::Ticks), ("endTime", Rule::Ticks)];

/// Captions by id.
const CAPTIONS: &[(&str, Rule)] = &[("captions", Rule::Ids)];

/// The keys of a caption track's style.
const CAPTION_STYLE_KEYS: &[&str] =
    &["track", "font", "size", "color", "background", "backgroundColor", "align", "anchor", "margin", "lineSpacing", "outline", "outlineColor", "reset"];

/// The caption track a style is for (an id, or `C1` and on: the engine would read one of another
/// type as left out, and style the first caption track), the alignments the style takes (the
/// engine fails on another name), its font, and the switches for a background and for starting
/// from the default style (the engine would read a font or a switch of another type as left out).
const CAPTION_STYLE: &[(&str, Rule)] = &[
    ("track", Rule::IdOrName),
    ("align", one_of(&["left", "center", "right"])),
    ("anchor", one_of(&["top", "middle", "bottom"])),
    ("font", Rule::Name),
    ("background", Rule::Bool),
    ("reset", Rule::Bool),
];

/// The redaction effects `effects.apply` may add (tier B), on clips by id.
const REDACTION: &[(&str, Rule)] = &[("clips", Rule::Ids), ("effect", one_of(&["gaussian_blur", "mosaic"]))];

/// The effect of a clip that takes a new mask, and the mask's shape (a bezier mask needs a path,
/// which is never a key).
const MASKED_EFFECT: &[(&str, Rule)] = &[("clip", Rule::Id), ("effect", Rule::IdOrName), ("shape", one_of(&["ellipse", "polygon"]))];

/// A mask, by clip, effect and index.
const MASK: &[(&str, Rule)] = &[("clip", Rule::Id), ("effect", Rule::IdOrName), ("mask", Rule::Id)];

/// A mask, at a sequence time, the modes the engine documents (it fails on another name), and
/// whether the mask is inverted (the engine would read a switch of another type as left out).
const MASK_AT: &[(&str, Rule)] = &[
    ("clip", Rule::Id),
    ("effect", Rule::IdOrName),
    ("mask", Rule::Id),
    ("time", Rule::Ticks),
    ("mode", one_of(&["none", "add", "subtract", "intersect", "lighten", "darken", "difference"])),
    ("inverted", Rule::Bool),
];

/// A mask, and the move of its path in pixels.
const MASK_MOVE: &[(&str, Rule)] = &[("clip", Rule::Id), ("effect", Rule::IdOrName), ("mask", Rule::Id), ("delta", Rule::Point)];

/// A mask to track, the direction and the tracking method (the engine fails on another name), the
/// number of frames, and whether the run waits for the job (the engine would read a number or a
/// switch of another type as left out: to the clip's end, without waiting).
const MASK_TRACK: &[(&str, Rule)] = &[
    ("clip", Rule::Id),
    ("effect", Rule::IdOrName),
    ("mask", Rule::Id),
    ("direction", one_of(&["forward", "backward"])),
    ("frames", Rule::Within(1, MAX_FRAMES)),
    ("method", one_of(&["position", "positionRotation", "positionScaleRotation"])),
    ("wait", Rule::Bool),
];

/// A transition on the timeline, and the switches to play it in reverse and to reset its parameters
/// (the engine would read a switch of another type as left out).
const TRANSITION: &[(&str, Rule)] = &[("transition", Rule::Id), ("reverse", Rule::Bool), ("reset", Rule::Bool)];

/// Clips by id, their new speed in percent within the editor's own range, whether they play in
/// reverse (the engine would read a speed or a switch of another type as left out: 100 %, forward),
/// and how their new speed samples the source (the engine fails on another name).
const SPEED: &[(&str, Rule)] = &[
    ("clips", Rule::Ids),
    ("speed", Rule::Between(MIN_SPEED, MAX_SPEED)),
    ("reverse", Rule::Bool),
    ("interpolation", one_of(&["frameSampling", "frameBlending", "opticalFlow"])),
];

/// A subclip of a media item, with its bounds in the item's own time (ticks), and whether trims stay
/// within them (the engine would read a switch of another type as left out: they do).
const SUBCLIP: &[(&str, Rule)] = &[("item", Rule::Id), ("start", Rule::Ticks), ("end", Rule::Ticks), ("restrictTrims", Rule::Bool)];

/// An Essential Sound type, on clips by id.
const AUDIO_TYPE: &[(&str, Rule)] = &[("clips", Rule::Ids), ("type", one_of(&["dialogue", "music", "sfx", "ambience"]))];

/// A text layer and the kind of text it becomes.
const TEXT_TYPE: &[(&str, Rule)] = &[("clip", Rule::Id), ("layer", Rule::IdOrName), ("type", one_of(&["point", "paragraph"]))];

/// A text layer's character style, by the properties the engine documents (it fails on another
/// one), or the switch that clears the styles of a range, from the character `start` to the
/// character `end` (the engine would read either of another type as left out: the whole text).
const CHAR_STYLE: &[(&str, Rule)] = &[
    ("clip", Rule::Id),
    ("layer", Rule::IdOrName),
    ("start", Rule::Id),
    ("end", Rule::Id),
    ("style", Rule::Object(&["font", "fontStyle", "size", "color", "bold", "italic", "underline", "tracking", "baselineShift", "caps"])),
    ("clear", Rule::True),
];

/// A character style edit sets a style or clears one (the engine fails with neither).
const STYLE_OR_CLEAR: &[Need] = &[Need::AnyOf(&[&["style"], &["clear"]])];

/// Layers of a graphic clip, the alignment and what they align to (the engine fails on another
/// alignment, and reads another target as the frame).
const ALIGN: &[(&str, Rule)] = &[
    ("clip", Rule::Id),
    ("layers", Rule::Ids),
    ("align", one_of(&["left", "hcenter", "right", "top", "vcenter", "bottom"])),
    ("to", one_of(&["frame", "group", "selection"])),
];

/// Layers of a graphic clip, the axis they spread along (the engine reads another axis as
/// horizontal), and whether the gaps between them come out equal instead of their centres (the
/// engine would read a switch of another type as left out).
const DISTRIBUTE: &[(&str, Rule)] = &[("clip", Rule::Id), ("layers", Rule::Ids), ("axis", one_of(&["horizontal", "vertical"])), ("space", Rule::Bool)];

/// A layer of a graphic clip and where in the paint order it goes: a place by name (the engine fails on another name)
/// or, since slice R1, an index from the back within `u32` (the engine clamps an index past the front to the front,
/// and reads a number of another type as 0, the back).
const ARRANGE: &[(&str, Rule)] = &[("clip", Rule::Id), ("layer", Rule::IdOrName), ("to", Rule::IndexOr(&["front", "back", "forward", "backward"]))];

/// A graphic clip made from a template: one control and its value, or several values by control
/// ([`Rule::Values`]); a checkbox's value is checked against the template ([`template_refusal`]).
const TEMPLATE: &[(&str, Rule)] = &[("clip", Rule::Id), ("control", Rule::Name), ("value", Rule::Given), ("values", Rule::Values)];

/// A template edit names a control with its value, or values (the engine fails with neither), and
/// never a control without its value (the engine would write null to the control) or a value
/// without its control.
const CONTROL_OR_VALUES: &[Need] = &[Need::AnyOf(&[&["control", "value"], &["values"]]), Need::Together(&["control", "value"])];

/// Clips to search for cuts, the shortest shot in frames (the engine's own range), what the run
/// applies, whether it waits for the job, and whether it only looks (slice R1, OC-33 E3: it waits for the
/// job, reports each clip's cuts and its shots as half-open timeline ranges and changes nothing): the
/// engine would read a number or a switch of another type as left out (six frames; cuts, no subclips,
/// no markers, no waiting, an edit).
const SCENES: &[(&str, Rule)] = &[
    ("clips", Rule::Ids),
    ("minShotFrames", Rule::Within(1, 10_000)),
    ("applyCuts", Rule::Bool),
    ("createSubclips", Rule::Bool),
    ("generateMarkers", Rule::Bool),
    ("wait", Rule::Bool),
    ("lookOnly", Rule::Bool),
];

/// A scene detection that only looks names no apply choice: it applies nothing (with one it would read as an edit).
const LOOK_ONLY: &[Need] = &[Need::Without { key: "lookOnly", holds: Rule::True, others: &["applyCuts", "createSubclips", "generateMarkers"] }];

/// The reviewed table (plan FF1, design §4.4; master plan v2 §3.1): every command of the legacy
/// video operations, of the newsroom set (C1) and of the newsroom families (N0) that slice R0
/// pre-declares, so neither C1 nor the later newsroom adapters need a second WASM build. Every key is
/// one its command documents (`commands()`). Every time is in ticks (INV-32): the `frame`,
/// `seconds` and `timecode` aliases of a time are not allowed, and a trim's delta is `delta` in
/// ticks, never `deltaFrames`. Durations keep their command's unit: `frames` for a transition,
/// `seconds` for a new title, whose caller also sends `time` (the engine reads `seconds` as the start
/// when `time` is missing), `durationSeconds` for a new caption (exact bounds come from
/// `captions.setTimes`), and the pause and caption limits of the transcript commands. A new shape
/// takes no `seconds` for that reason: it documents no `time`, so it starts at the playhead, or joins
/// the graphic clip named by `clip`. Undo-merge keys (`merge`, `begin`), presets and raw export
/// settings, the system encoders (`hardwareEncoding`), a loudness pass that would measure the whole
/// export in one call (`loudnessLufs`), ripple switches the ripple pre-check does not cover, mask
/// geometry given as `path`, and a subclip's `clip` (its `item` and bounds say the same without the
/// Source monitor) are left out. So is `wait`, a run that holds the editor's thread until a job
/// ends, except in the two native-only entries that document it (scene detection and mask
/// tracking), which keep it: there the worker may wait for its own job, and the browser refuses
/// those entries as `native_only` whatever their keys.
///
/// Selecting, extracting or lifting transcript words moves the playhead to the start of the words,
/// as `sequence.extract` does; selecting them also sets the sequence's In and Out points, and
/// extracting or lifting them clears those. With the person's Selection Follows Playhead setting
/// on, each of these, and `playhead.set`, also replaces the person's clip selection with the clips
/// under the new playhead on the targeted tracks (with their linked clips while linked selection is
/// on); so selecting words needs the caller's targeting too. Other entries change the person's
/// selections as the editor's own commands do: placed clips, a frame hold's clips, and a new or
/// rearranged graphic layer with its clip become the clip (and layer) selection; a ripple delete
/// clears the clip selection; a new caption becomes the caption selection and clears the clip
/// selection; split or merged captions become the caption selection, and captions made from the
/// transcript clear it; a new mask is selected, and removing a mask of the selected mask's effect
/// clears that; a subclip or a duplicate becomes the Project selection. The playhead and the
/// selections are the person's: the agent reads them again before anything that depends on them.
///
/// `clip.enable` is a toggle, as Clip ▸ Enable is: when every clip it names (with their linked clips
/// while linked selection is on) is enabled, it disables them all, and otherwise it enables them
/// all. No key says which way, so the caller reads the clips' `enabled` at the revision it binds.
///
/// Slice R1 (master plan v3 W3, widen-1) adds the sequence marks, set at a time the call names on the
/// program only (a mark set past the other one clears it, and setting a mark clears the split marks of
/// its side, as the editor's own Mark In and Mark Out do), and widens five entries: a video transition
/// is any of the engine's catalogue, with its own settings ([`Entry::settings`]); an insert names its
/// time, and the insert pre-check refuses one that would cut a clip or a caption in two on a
/// sync-locked track the call does not name; a layer goes to an index as well as to a named place; a
/// track's lock, output and mute are set beside its sync lock, at least one of them; and a scene
/// detection may only look (`lookOnly`), the table's one read ([`Entry::reads_with`]), which names no
/// apply choice and runs in a native worker only, as every scene detection does.
pub const POLICY: &[Entry] = &[
    // the legacy operations: seek, new sequence, place, title (text, then its colour), trim,
    // dissolve; since R0 each requires the keys the engine fails without, limits its names, and
    // types the keys it may leave out where the engine would read another type as left out
    edit("playhead.set", &["time"]).requires(&["time"]).limits(AT),
    edit("file.newSequence", &["name", "width", "height", "fps", "sampleRate", "video", "audio", "mix", "trackType"]).limits(NEW_SEQUENCE),
    edit("timeline.place", &["item", "track", "audioTrack", "time", "insert", "sourceIn", "duration"]).requires(&["item"]).needs(INSERT_TIME).limits(PLACE),
    edit("graphics.newText", &["text", "literal", "position", "box", "newClip", "size", "font", "fontStyle", "seconds", "track", "time"]).limits(TITLE),
    edit("graphics.set", &["clip", "layer", "props", "time"]).requires(&["props"]).limits(PROPS),
    edit("timeline.trim", &["clip", "edge", "mode", "delta"]).requires(&["clip"]).limits(TRIM),
    edit("sequence.applyVideoTransition", &["clip", "effect", "frames", "edge", "reverse", "params"])
        .requires(&["clip", "effect", "frames", "edge"])
        .limits(VIDEO_TRANSITION)
        .with_settings(&VIDEO_TRANSITIONS),
    // the newsroom set (C1); since R0 a closed gap names its track and time, and a ripple delete its
    // clips, so the ripple pre-check sees what the engine will close, and a lift or an extract, which
    // acts on the targeted tracks, needs the caller's targeting
    edit("graphics.newShape", &["shape", "position", "size", "points", "clip"]).needs(PATH_POINTS).limits(SHAPE),
    edit("lumetri.setSection", &["clip", "section", "on"]).requires(&["section"]).limits(SECTION),
    edit("timeline.razor", &["time", "track", "clip"]).limits(RAZOR),
    edit("timeline.move", &["moves", "insert", "linked"]).requires(&["moves"]).limits(MOVES),
    edit("sequence.lift", &[]).targeted(),
    edit("sequence.extract", &[]).targeted(),
    edit("sequence.closeGap", &["track", "time"]).requires(&["track", "time"]).limits(TRACK_AT),
    edit("edit.rippleDelete", &["clips"]).requires(&["clips"]).limits(CLIPS),
    edit("clip.enable", &["clips"]).limits(CLIPS),
    edit("edit.duplicate", &[]),
    edit("markers.add", &["time", "name", "comment", "color"]).limits(MARKER),
    edit("markers.edit", &["marker", "name", "comment", "color"]).requires(&["marker"]).limits(MARKER_EDIT),
    edit("clip.audioGain", &["clips", "mode", "db", "relative"]).limits(GAIN),
    history_step("edit.undo"),
    history_step("edit.redo"),
    // R0 tier A: Motion and fit/fill, sync lock, audio transitions, keyframes, export and its jobs
    edit("effects.setParam", &["clip", "effect", "param", "value", "time"])
        .requires(&["clip", "effect", "param", "value", "time"])
        .limits(CLIP_AT)
        .reaches(REFRAME),
    edit("clip.fillFrame", &["clips"]).requires(&["clips"]).limits(CLIPS),
    edit("clip.fitToFrame", &["clips"]).requires(&["clips"]).limits(CLIPS),
    edit("timeline.setTrack", &["track", "syncLock", "locked", "enabled", "muted"]).requires(&["track"]).needs(TRACK_SWITCHES).limits(TRACK_STATE),
    edit("sequence.applyAudioTransition", &["clip", "effect", "frames"]).requires(&["clip", "effect", "frames"]).limits(AUDIO_TRANSITION).native(),
    edit("effects.addKeyframe", &["clip", "effect", "param", "time"]).requires(&["clip", "effect", "param", "time"]).limits(CLIP_AT).reaches(ANIMATE),
    edit("effects.setKeyframe", &["clip", "effect", "param", "mediaTime", "value", "inInfluence", "outInfluence"])
        .requires(&["clip", "effect", "param", "mediaTime"])
        .limits(CLIP_KEYFRAME)
        .reaches(ANIMATE),
    edit("effects.setInterpolation", &["clip", "effect", "param", "mediaTime", "interpolation"])
        .requires(&["clip", "effect", "param", "mediaTime", "interpolation"])
        .limits(INTERPOLATION)
        .reaches(ANIMATE),
    edit("effects.deleteKeyframe", &["clip", "effect", "param", "mediaTime"])
        .requires(&["clip", "effect", "param", "mediaTime"])
        .limits(CLIP_KEYFRAME)
        .reaches(ANIMATE),
    edit("file.exportMedia", EXPORT_MEDIA_KEYS).requires(&["path", "format"]).needs(FRAME_SIZE).limits(EXPORT_MEDIA).browser(),
    edit("jobs.cancel", &["job"]).requires(&["job"]).limits(JOB),
    // R0 tier A: the transcript (selecting, extracting or lifting words moves the playhead, which
    // may select the clips under it on the targeted tracks; extracting or lifting them acts on the
    // targeted tracks)
    edit("transcript.set", &["item", "transcript"]).requires(&["item", "transcript"]).limits(TRANSCRIPT),
    edit("transcript.select", &["from", "to"]).requires(&["from"]).limits(WORDS).targeted(),
    edit("transcript.extract", &["from", "to"]).requires(&["from"]).limits(WORDS).targeted(),
    edit("transcript.lift", &["from", "to"]).requires(&["from"]).limits(WORDS).targeted(),
    edit("transcript.removePauses", &["minSeconds", "keepSeconds"]).limits(PAUSES),
    edit("transcript.removeFillers", &["fillers"]),
    edit("transcript.createCaptions", &["maxChars", "lines", "minSeconds", "maxSeconds", "gapFrames", "format", "name"]).limits(TRANSCRIPT_CAPTIONS),
    edit("transcript.renameSpeaker", &["speaker", "name", "item"]).requires(&["speaker", "name"]).needs(SPEAKER_INDEX).limits(RENAME),
    // R0 tier A: captions
    edit("captions.newTrack", &["format", "name", "language"]).limits(CAPTION_FORMAT),
    edit("captions.add", &["track", "text", "time", "durationSeconds"]).requires(&["time"]).limits(NEW_CAPTION),
    edit("captions.setText", &["caption", "text", "speaker"]).requires(&["caption"]).needs(TEXT_OR_SPEAKER).limits(CAPTION_TEXT),
    edit("captions.setTimes", &["caption", "startTime", "endTime"]).requires(&["caption"]).limits(CAPTION_TIMES),
    edit("captions.setStyle", CAPTION_STYLE_KEYS).limits(CAPTION_STYLE),
    edit("captions.split", &["caption", "time"]).requires(&["caption", "time"]).limits(CAPTION_AT),
    edit("captions.merge", &["captions"]).requires(&["captions"]).limits(CAPTIONS),
    edit("captions.delete", &["captions"]).requires(&["captions"]).limits(CAPTIONS),
    // R0 tier B: redaction effects and masks without tracking
    edit("effects.apply", &["clips", "effect"]).requires(&["clips", "effect"]).limits(REDACTION),
    edit("masks.add", &["clip", "effect", "shape", "center", "size"]).requires(&["clip", "effect"]).limits(MASKED_EFFECT),
    edit("masks.set", &["clip", "effect", "mask", "name", "inverted", "mode", "feather", "opacity", "expansion", "time"])
        .requires(&["clip", "effect", "mask", "time"])
        .limits(MASK_AT),
    edit("masks.translate", &["clip", "effect", "mask", "delta"]).requires(&["clip", "effect", "mask", "delta"]).limits(MASK_MOVE),
    edit("masks.remove", &["clip", "effect", "mask"]).requires(&["clip", "effect", "mask"]).limits(MASK),
    // R0 tier B: transitions, speed, holds and subclips
    edit("sequence.setTransition", &["transition", "params", "reverse", "reset"]).requires(&["transition"]).limits(TRANSITION),
    edit("clip.speedDuration", &["clips", "speed", "reverse", "interpolation"]).requires(&["clips"]).limits(SPEED),
    edit("clip.frameHold", &["clips", "time"]).requires(&["clips", "time"]).limits(CLIPS_AT),
    edit("clip.makeSubclip", &["item", "name", "start", "end", "restrictTrims"]).requires(&["item"]).limits(SUBCLIP),
    // R0 tier B: Essential Sound types, and ducking, whose analysis decodes the trigger sound in one
    // call (native only)
    edit("essentialSound.setType", &["clips", "type"]).requires(&["clips", "type"]).limits(AUDIO_TYPE),
    edit("essentialSound.clearType", &["clips"]).requires(&["clips"]).limits(CLIPS),
    edit("essentialSound.generateDucking", &["clips"]).requires(&["clips"]).limits(CLIPS).native(),
    // R0 tier B: graphics style and template values
    edit("graphics.setTextType", &["clip", "layer", "type"]).requires(&["clip", "layer", "type"]).limits(TEXT_TYPE),
    edit("graphics.setCharStyle", &["clip", "layer", "start", "end", "style", "clear"]).requires(&["clip", "layer"]).needs(STYLE_OR_CLEAR).limits(CHAR_STYLE),
    edit("graphics.align", &["clip", "layers", "align", "to"]).requires(&["clip", "layers", "align"]).limits(ALIGN),
    edit("graphics.distribute", &["clip", "layers", "axis", "space"]).requires(&["clip", "layers"]).limits(DISTRIBUTE),
    edit("graphics.arrangeLayer", &["clip", "layer", "to"]).requires(&["clip", "layer"]).limits(ARRANGE),
    edit("graphics.template.set", &["clip", "control", "value", "values"]).requires(&["clip"]).needs(CONTROL_OR_VALUES).limits(TEMPLATE),
    // R0 tier B: interchange for human finishing tools (J35), never for conform
    edit("file.exportEdl", &["path"]).requires(&["path"]).limits(EXPORT_FILE),
    edit("file.exportOtio", &["path"]).requires(&["path"]).limits(EXPORT_FILE),
    edit("file.exportFcpxml", &["path"]).requires(&["path"]).limits(EXPORT_FILE),
    // R0 native only: scene detection and mask tracking, which keep `wait`; since R1 a scene detection may only look
    edit("clip.sceneEditDetection", &["clips", "sensitivity", "minShotFrames", "applyCuts", "createSubclips", "generateMarkers", "wait", "lookOnly"])
        .requires(&["clips"])
        .needs(LOOK_ONLY)
        .limits(SCENES)
        .reads_with("lookOnly")
        .native(),
    edit("masks.track", &["clip", "effect", "mask", "direction", "frames", "method", "wait"]).requires(&["clip", "effect", "mask"]).limits(MASK_TRACK).native(),
    // R1 (widen-1, m22): the sequence marks, at a time the call names (never the playhead), on the program only (a
    // source mark's `target` is not a key); a typed marks edit reports the marks it leaves, half-open (`marks_result`)
    edit("markers.markIn", &["time"]).requires(&["time"]).limits(AT),
    edit("markers.markOut", &["time"]).requires(&["time"]).limits(AT),
    edit("markers.clearInOut", &[]),
];

/// An item by id (a media item, or a sequence): the engine would read an item of another type as left out, and take
/// the active sequence or the person's selection instead.
const ITEM: &[(&str, Rule)] = &[("item", Rule::Id)];

/// A graphic clip by id: the engine would read a clip of another type as left out, and take the person's selection.
const CLIP: &[(&str, Rule)] = &[("clip", Rule::Id)];

/// Whether the font list scans the system font folders (a switch; the browser has none to scan).
const SYSTEM_FONTS: &[(&str, Rule)] = &[("system", Rule::Bool)];

/// Whether the effect list gives the details of each parameter (a switch).
const EFFECT_DETAIL: &[(&str, Rule)] = &[("detail", Rule::Bool)];

/// The certified reads `executeChecked` serves besides its edits ([`CHECKED_EDITS`], slice R1): queries the
/// engine never journals, certified pure by the engine's `tests/lts_agent_purity.rs` (T-FI-01b,
/// T-FI-01d, T-FI-01e), each with the exact keys it was certified with, typed. A read that would
/// otherwise take the person's selection names its target (an item, a graphic clip); the sequence
/// read takes the active sequence when it names none.
pub const READS: &[Entry] = &[
    // F0 (T-FI-01b): the project, a sequence, the registered fonts, the history, an item's metadata and colour
    read("project.inspect", &[]),
    read("sequence.inspect", &["item"]).limits(ITEM),
    read("fonts.list", &["system"]).limits(SYSTEM_FONTS),
    read("history.list", &[]),
    read("metadata.get", &["item"]).requires(&["item"]).limits(ITEM),
    read("media.colorInfo", &["item"]).requires(&["item"]).limits(ITEM),
    // R0 (T-FI-01d): the transcript, the captions, the background jobs and the effect catalogue
    read("transcript.inspect", &[]),
    read("captions.list", &[]),
    read("jobs.list", &[]),
    read("effects.list", &["detail"]).limits(EFFECT_DETAIL),
    // R1 (T-FI-01e): the editor state (the targeting, the patching, linked selection, the marks) and a graphic's layers
    read("state.inspect", &[]),
    read("graphics.list", &["clip"]).requires(&["clip"]).limits(CLIP),
];

/// Where a limited value may be used.
enum Fit {
    Anywhere,
    NativeOnly,
    Nowhere,
}

/// Where `value` may be used under `rule`.
fn fit(rule: &Rule, value: &Value) -> Fit {
    match (rule, value.as_str()) {
        (Rule::OneOf { both, .. }, Some(value)) if both.contains(&value) => Fit::Anywhere,
        (Rule::OneOf { native, .. }, Some(value)) if native.contains(&value) => Fit::NativeOnly,
        _ if holds(rule, value) => Fit::Anywhere,
        _ => Fit::Nowhere,
    }
}

/// Whether `value` is one `rule` takes in both runtimes, for every rule but `OneOf` (which [`fit`]
/// decides).
fn holds(rule: &Rule, value: &Value) -> bool {
    match rule {
        Rule::OneOf { .. } => false,
        Rule::ExportPath => value.as_str().is_some_and(export_path),
        Rule::Ticks => value.as_i64().is_some_and(|ticks| (-MAX_TICKS..=MAX_TICKS).contains(&ticks)),
        Rule::Within(min, max) => value.as_i64().is_some_and(|number| (*min..=*max).contains(&number)),
        Rule::Between(min, max) => value.as_f64().is_some_and(|number| (*min..=*max).contains(&number)),
        Rule::Seconds => value.as_f64().is_some_and(|seconds| (0.0..=MAX_SECONDS).contains(&seconds)),
        Rule::Id => value.as_u64().is_some(),
        Rule::Ids => value.as_array().is_some_and(|ids| !ids.is_empty() && ids.iter().all(|id| id.as_u64().is_some())),
        Rule::IdOrName => value.as_u64().is_some() || value.as_str().is_some_and(|name| !name.is_empty()),
        Rule::Bool => value.is_boolean(),
        Rule::True => value.as_bool() == Some(true),
        Rule::Text => value.is_string(),
        Rule::Name => value.as_str().is_some_and(|name| !name.trim().is_empty()),
        Rule::Object(fields) => value.as_object().is_some_and(|object| fields_of(fields, object)),
        Rule::Props => value.as_object().is_some_and(|props| given_fields(props) && props.get("enabled").is_none_or(Value::is_boolean)),
        Rule::Values => value.as_object().is_some_and(given_fields),
        Rule::Transcript => transcript(value),
        Rule::Given => !value.is_null(),
        Rule::Point => point(value),
        Rule::Points => value.as_array().is_some_and(|points| points.len() >= 3 && points.iter().all(point)),
        Rule::Moves => value.as_array().is_some_and(|moves| !moves.is_empty() && moves.iter().all(timeline_move)),
        Rule::IndexOr(names) => value.as_str().is_some_and(|name| names.contains(&name)) || value.as_u64().is_some_and(|index| u32::try_from(index).is_ok()),
    }
}

/// Whether `object` has a key, and, when `fields` lists names, only keys among them.
fn fields_of(fields: &[&str], object: &Map<String, Value>) -> bool {
    !object.is_empty() && object.keys().all(|key| fields.is_empty() || fields.contains(&key.as_str()))
}

/// Whether `object` has a key, and no null value.
fn given_fields(object: &Map<String, Value>) -> bool {
    !object.is_empty() && object.values().all(|value| !value.is_null())
}

/// Whether `value` is a transcript `transcript.set` takes ([`Rule::Transcript`]).
fn transcript(value: &Value) -> bool {
    let Some(record) = value.as_object() else {
        return false;
    };
    let words = record.get("words").and_then(Value::as_array);
    record.keys().all(|key| TRANSCRIPT_FIELDS.contains(&key.as_str())) && words.is_some_and(|words| !words.is_empty() && words.iter().all(word))
}

/// Whether `value` is a word of a transcript: a text with more than white space in it, its start
/// and end in ticks, the end from the start to [`MAX_WORD_TICKS`] after it, and no speaker, or a
/// speaker index up to [`MAX_SPEAKER`].
fn word(value: &Value) -> bool {
    let time = |key: &str| value.get(key).filter(|ticks| holds(&Rule::Ticks, ticks)).and_then(Value::as_i64);
    let timed = time("start").zip(time("end")).is_some_and(|(start, end)| (start..=start.saturating_add(MAX_WORD_TICKS)).contains(&end));
    let text = value.get("text").is_some_and(|text| holds(&Rule::Name, text));
    let speaker = value.get("speaker").is_none_or(|speaker| speaker.is_null() || holds(&Rule::Within(0, MAX_SPEAKER), speaker));
    timed && text && speaker
}

/// Whether `value` is a point `[x, y]` within [`MAX_COORDINATE`] of zero on both axes.
fn point(value: &Value) -> bool {
    value.as_array().is_some_and(|xy| xy.len() == 2 && xy.iter().all(|axis| axis.as_f64().is_some_and(|axis| axis.abs() <= MAX_COORDINATE)))
}

/// Whether `value` is one move of `timeline.move`: exactly a clip id, a track id or name, and a time.
fn timeline_move(value: &Value) -> bool {
    let Some(fields) = value.as_object() else {
        return false;
    };
    let field = |key: &str, rule: Rule| fields.get(key).is_some_and(|value| holds(&rule, value));
    fields.len() == 3 && field("clip", Rule::Id) && field("track", Rule::IdOrName) && field("time", Rule::Ticks)
}

/// Whether `value` is a value of `kind`.
fn holds_kind(kind: Kind, value: &Value) -> bool {
    match kind {
        Kind::Number => value.is_number(),
        Kind::Point => value.as_array().is_some_and(|point| point.len() == 2 && point.iter().all(Value::is_number)),
        Kind::Color => color(value),
        Kind::Switch => value.is_boolean(),
        Kind::Choice(options) => names_option(options, value),
    }
}

/// Whether `value` is a colour: `#rrggbb` or `#rrggbbaa` in hex digits, or `[r, g, b]` or `[r, g, b, a]`, each
/// component from 0 to 1.
fn color(value: &Value) -> bool {
    let hex = |digits: &str| matches!(digits.len(), 6 | 8) && digits.bytes().all(|digit| digit.is_ascii_hexdigit());
    let unit = |component: &Value| component.as_f64().is_some_and(|component| (0.0..=1.0).contains(&component));
    let digits = value.as_str().and_then(|text| text.strip_prefix('#'));
    digits.is_some_and(hex) || value.as_array().is_some_and(|rgba| matches!(rgba.len(), 3 | 4) && rgba.iter().all(unit))
}

/// Whether `value` names one of `options`: by its exact name, or by its index.
fn names_option(options: &[&str], value: &Value) -> bool {
    let index = value.as_u64().and_then(|index| usize::try_from(index).ok());
    value.as_str().is_some_and(|name| options.contains(&name)) || index.is_some_and(|index| index < options.len())
}

/// Whether `params` name, with `effect` and `param`, an effect parameter `targets` reaches, and give
/// a `value` of its kind when they give one. Any `params` pass when `targets` is empty.
fn reaches(targets: &[Target], params: &Map<String, Value>) -> bool {
    if targets.is_empty() {
        return true;
    }
    let (Some(effect), Some(param)) = (params.get("effect").and_then(Value::as_str), params.get("param").and_then(Value::as_str)) else {
        return false;
    };
    let kind = targets.iter().filter(|(id, _)| *id == effect).flat_map(|(_, reached)| reached.iter()).find(|(id, _)| *id == param).map(|(_, kind)| *kind);
    kind.is_some_and(|kind| params.get("value").is_none_or(|value| holds_kind(kind, value)))
}

/// Whether the transition settings `params` give (its `params`, an object) are settings of the transition its `effect`
/// names among `settings`, each with a value of its kind; a call may give none of them, or leave `params` out. Any
/// `params` pass when `settings` is empty.
fn settings_fit(settings: &[Target], params: &Map<String, Value>) -> bool {
    if settings.is_empty() {
        return true;
    }
    let Some(given) = params.get("params") else {
        return true;
    };
    let effect = params.get("effect").and_then(Value::as_str);
    let Some((_, kinds)) = settings.iter().find(|(id, _)| Some(*id) == effect) else {
        return false;
    };
    given.as_object().is_some_and(|given| given.iter().all(|(name, value)| kinds.iter().any(|(setting, kind)| *setting == name && holds_kind(*kind, value))))
}

/// Whether `path` names a file under [`EXPORT_ROOT`]: at most [`MAX_EXPORT_PATH`] bytes, and every
/// segment after the root a plain name of ASCII letters, digits, `.`, `_` and `-` that neither
/// starts nor ends with `.` and is no Windows device ([`device_name`]). So no `..`, no drive, no
/// backslash, no empty segment, and no folder.
fn export_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix(EXPORT_ROOT) else {
        return false;
    };
    path.len() <= MAX_EXPORT_PATH
        && rest.split('/').all(|segment| {
            !segment.is_empty()
                && !segment.starts_with('.')
                && !segment.ends_with('.')
                && !device_name(segment)
                && segment.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}

/// Whether `segment` names a Windows device (`CON`, `PRN`, `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to
/// `LPT9`), in any case, with or without an extension (`NUL.mp4`, `aux.tar.gz`): a file of that name
/// opens the device instead.
fn device_name(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or_default().to_ascii_uppercase();
    match stem.as_bytes() {
        b"CON" | b"PRN" | b"AUX" | b"NUL" => true,
        [b'C', b'O', b'M', digit] | [b'L', b'P', b'T', digit] => digit.is_ascii_digit(),
        _ => false,
    }
}

/// The entry of `command` when the table holds it, `params` is an object whose every key the entry
/// allows, that gives every key the entry requires and what its needs ask for, whose every limited
/// value its rule takes, whose effect parameter the entry reaches and whose transition settings the
/// transition it names takes, and it may run in `runtime`. Otherwise `command_not_allowed` (an
/// unknown or near-miss id, a query, a tier-3 command, an unknown key or a key in another case, a
/// required key left out, a need not met, a value outside its rule, an effect parameter the entry
/// does not reach or a value of another kind, a transition setting the transition does not have or
/// a value of another kind, parameters that are not an object), or, for a call the table otherwise
/// allows: `native_only` in the browser when the entry or one of its values only runs in a native
/// worker, and `use_worker_export` in a native worker for the entry only the browser runs.
pub fn allowed(command: &str, params: &Value, runtime: Runtime) -> Result<&'static Entry, &'static str> {
    let entry = POLICY.iter().find(|entry| entry.command == command).ok_or(COMMAND_NOT_ALLOWED)?;
    admissible(entry, params, runtime)
}

/// `entry`, when `params` and `runtime` pass the checks [`allowed`] makes once it has found the entry, which
/// [`checked`] makes on a certified read as well; otherwise the refusal [`allowed`] describes.
fn admissible(entry: &'static Entry, params: &Value, runtime: Runtime) -> Result<&'static Entry, &'static str> {
    let params = params.as_object().ok_or(COMMAND_NOT_ALLOWED)?;
    let known = params.keys().all(|key| entry.keys.contains(&key.as_str()));
    let given = entry.required.iter().all(|key| params.contains_key(*key)) && entry.needs.iter().all(|need| need.met(params));
    let fits = known && given && reaches(entry.targets, params) && settings_fit(entry.settings, params);
    if !fits {
        return Err(COMMAND_NOT_ALLOWED);
    }
    let mut native_only = entry.runtimes == Runtimes::NativeOnly;
    for (key, rule) in entry.rules {
        match params.get(*key).map(|value| fit(rule, value)) {
            None | Some(Fit::Anywhere) => {}
            Some(Fit::NativeOnly) => native_only = true,
            Some(Fit::Nowhere) => return Err(COMMAND_NOT_ALLOWED),
        }
    }
    match runtime {
        Runtime::Browser if native_only => Err(NATIVE_ONLY),
        Runtime::Native if entry.runtimes == Runtimes::BrowserOnly => Err(USE_WORKER_EXPORT),
        _ => Ok(entry),
    }
}

/// The edits `executeChecked` runs besides its reads (slice R1): the legacy operations the LTS lab's mirror adapter
/// sends there (`creative-filmcraft-adapter.mjs`: a seek, a new sequence, a place, a title and its colour, a dissolve
/// and a trim) until FF3 routes its writes through `executeTyped` (W5), each under its entry of [`POLICY`] and a place
/// only as an overwrite ([`checked`]). None needs a precondition or a pre-check `executeChecked` does not run: none is
/// a history step or acts on the targeted tracks, none is a keyframe or template edit, and a ripple trim with a
/// sync-locked track in its way is one the engine refuses itself, from inside the command (nothing changes but an
/// Events line). Every other edit of the table runs through `executeTyped` only.
pub const CHECKED_EDITS: &[&str] =
    &["playhead.set", "file.newSequence", "timeline.place", "graphics.newText", "graphics.set", "sequence.applyVideoTransition", "timeline.trim"];

/// What `executeChecked` admits (slice R1; D13, backlog item 16): a certified read of [`READS`] with the keys it was
/// certified with and values of their rules, or an edit of [`CHECKED_EDITS`] that [`allowed`] lets run in `runtime`,
/// a place only as an overwrite (an insert cuts in two, in silence, a clip or a caption that runs across it on a
/// sync-locked track the call does not name, which only the insert pre-check of `executeTyped` refuses,
/// [`sync_locked_tracks`]). Every other id or edit, a read or an edit outside its keys or values, parameters that are
/// not an object and an insert are `command_not_allowed`. Nothing has run when this refuses.
pub fn checked(command: &str, params: &Value, runtime: Runtime) -> Result<(), &'static str> {
    if let Some(read) = READS.iter().find(|read| read.command == command) {
        return admissible(read, params, runtime).map(|_| ());
    }
    if !CHECKED_EDITS.contains(&command) || params.get("insert").and_then(Value::as_bool) == Some(true) {
        return Err(COMMAND_NOT_ALLOWED);
    }
    allowed(command, params, runtime).map(|_| ())
}

/// A refusal of `executeChecked` decided before dispatch: its registered `not_applied` code, and the message of the
/// `Error` it rejects with.
pub type Refusal = (&'static str, String);

/// What `executeChecked(revision, command, params, {activeSequence, deadlineMs})` binds its dispatch to (slice R1): the
/// project revision and the active sequence the caller read (`context()`), and the latest dispatch time.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckedExpect {
    pub revision: u64,
    /// `None` when no sequence is open.
    pub active_sequence: Option<u64>,
    /// In milliseconds since the epoch (`Date.now()`).
    pub deadline_ms: f64,
}

impl CheckedExpect {
    /// The preconditions of an `executeChecked` call, from its `revision` and its `context`, or the refusal of one sent
    /// in a form it does not read, which can never hold (as `executeTyped` refuses one), in the order [`admit`] checks
    /// them: a deadline that is not a finite number is `deadline_elapsed`, a revision that is not a decimal string
    /// `revision_conflict`, and an active sequence that is neither a decimal string nor null `sequence_changed` (a
    /// context that is not an object names none of them). Nothing has run when this refuses.
    pub fn from_call(revision: &str, context: &Value) -> Result<CheckedExpect, Refusal> {
        let deadline_ms = context.get("deadlineMs").and_then(Value::as_f64).filter(|ms| ms.is_finite());
        let deadline_ms = deadline_ms.ok_or_else(|| ("deadline_elapsed", "finite deadlineMs is required".to_string()))?;
        let revision = revision.parse::<u64>().map_err(|_| ("revision_conflict", "invalid decimal project revision".to_string()))?;
        let active_sequence = match context.get("activeSequence") {
            Some(Value::Null) => None,
            Some(Value::String(id)) => Some(id.parse::<u64>().map_err(|_| ("sequence_changed", "invalid decimal sequence id".to_string()))?),
            _ => return Err(("sequence_changed", "explicit activeSequence string or null is required".to_string())),
        };
        Ok(CheckedExpect { revision, active_sequence, deadline_ms })
    }
}

/// The refusal of an `executeChecked` call in its UI-thread closure, just before it dispatches (slice R1), in the
/// order [`admit`] checks the same preconditions: a dispatch at `now_ms` past the caller's deadline is
/// `deadline_elapsed`, a project `revision` other than the caller's `revision_conflict`, and an `active_sequence` other
/// than the caller's `sequence_changed`, each with the message `executeChecked` rejects with. Nothing has run when
/// this refuses.
pub fn checked_guard(expect: &CheckedExpect, now_ms: f64, revision: u64, active_sequence: Option<u64>) -> Result<(), Refusal> {
    if !in_time(now_ms, expect.deadline_ms) {
        return Err(("deadline_elapsed", "command deadline elapsed before dispatch".to_string()));
    }
    if expect.revision != revision {
        return Err(("revision_conflict", format!("project revision conflict: expected {}, current {revision}", expect.revision)));
    }
    if expect.active_sequence != active_sequence {
        return Err(("sequence_changed", "active sequence changed before dispatch".to_string()));
    }
    Ok(())
}

/// The code of an `executeChecked` rejection once `Session::execute` failed (slice R1): `command_disabled` when the
/// engine refused the dispatched command itself before running it (`disabled` names the command of its `Disabled`
/// error) and the revision did not move, as [`settle`] decides it for `executeTyped`; `None` for any other failure,
/// after dispatch (an edit's outcome is then unknown; a failing certified read changed nothing but an Events line).
pub fn checked_failure(command: &str, disabled: Option<&str>, revision_before: u64, revision_after: u64) -> Option<&'static str> {
    (disabled == Some(command) && revision_after == revision_before).then_some("command_disabled")
}

/// The control requests `window.filmcraft.request(method, params)` sends in the LTS presentation besides a command
/// (slice R1, [`raw_request`]): reads of the editor's own state, which change nothing: the command list and the menu
/// tree, the interface state and its elements, the performance counters, a screenshot (the browser returns it in its
/// reply) and the timeline's hit tests.
pub const CONTROL_READS: &[&str] =
    &["engine.commands", "ui.menu.list", "ui.inspect", "ui.elements", "perf.stats", "ui.screenshot", "ui.timeline.hit", "ui.timeline.locate"];

/// What the members of `window.filmcraft` that dispatch by name send in the LTS presentation (`?embedded`; slice R1,
/// D13, backlog item 16, INV-12): `filmcraft.request(method, params)` a read of [`CONTROL_READS`], or a command
/// (`engine.execute` and `ui.menu.invoke` name it `command`, else `id`, with its `params`, `{}` when left out, as the
/// control channel reads them) that is a certified read of [`READS`] with the keys it was certified with, which is all
/// `filmcraft.execute(command, params)` sends. Every other method and every other command is `command_not_allowed`,
/// and nothing is sent: the control channel's synthetic input (`ui.click`, `ui.key`, `ui.type` and the rest) reaches
/// whatever a person can reach, and an edit names the revision it was decided on (`executeChecked`) or goes through
/// `executeTyped`. The standalone presentation sends every method, as upstream does.
pub fn raw_request(method: &str, params: &Value) -> Result<(), &'static str> {
    if CONTROL_READS.contains(&method) {
        return Ok(());
    }
    if !matches!(method, "engine.execute" | "ui.menu.invoke") {
        return Err(COMMAND_NOT_ALLOWED);
    }
    let named = |key: &str| params.get(key).and_then(Value::as_str);
    let command = named("command").or_else(|| named("id")).ok_or(COMMAND_NOT_ALLOWED)?;
    let read = READS.iter().find(|read| read.command == command).ok_or(COMMAND_NOT_ALLOWED)?;
    let empty = Value::Object(Map::new());
    admissible(read, params.get("params").unwrap_or(&empty), Runtime::Browser).map(|_| ())
}

/// Whether a call only reads: its entry has a [`Entry::reads_with`] switch and the call sets it
/// `true` (a scene detection that only looks). Such a call changes nothing (no revision, no undo
/// step), so a caller admits it as a read (tier 0), and its envelope names no history label
/// ([`settle`]).
pub fn reads_only(command: &str, params: &Value) -> bool {
    POLICY.iter().any(|entry| entry.command == command && entry.reads_with.is_some_and(|key| params.get(key).and_then(Value::as_bool) == Some(true)))
}

/// The digest of a sequence's track targeting: the tracks it targets (in any order) and the tracks a
/// source's picture and sound are patched to, as `fnv1a64:` and the 16 hex digits of the FNV-1a hash
/// of a canonical text of them. Equal targeting has an equal digest; `watermark()` reports it and
/// `executeTyped` compares it ([`admit`]).
pub fn targeting_digest(targeted: &[u64], video: Option<u64>, audio: Option<u64>) -> String {
    let mut tracks = targeted.to_vec();
    tracks.sort_unstable();
    tracks.dedup();
    let listed: Vec<String> = tracks.iter().map(u64::to_string).collect();
    let patch = |track: Option<u64>| track.map_or_else(|| "-".to_string(), |track| track.to_string());
    let text = format!("targeted={};video={};audio={}", listed.join(","), patch(video), patch(audio));
    let hash = text.bytes().fold(FNV_OFFSET, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME));
    format!("fnv1a64:{hash:016x}")
}

/// The undo stack as `history.list` reports it: its length and newest label ("" when it is empty).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryHead {
    pub length: u64,
    pub label: String,
}

/// The caller's side of `executeTyped(revision, activeSequence, command, params, {deadlineMs,
/// historyHead?, linkedSelection?, targeting?})`.
#[derive(Clone, Debug, PartialEq)]
pub struct Expect {
    /// The project revision the caller read (`context()`).
    pub revision: u64,
    /// The active sequence the caller read (`context()`); `None` when none is open.
    pub active_sequence: Option<u64>,
    /// The linked-selection setting the caller read (`watermark()`); `None` when the caller does not
    /// bind its edit to it.
    pub linked_selection: Option<bool>,
    /// The track targeting the caller read (`watermark().targeting`, a [`targeting_digest`]); `None`
    /// when the caller does not bind its edit to it (an entry that acts on the targeted tracks needs
    /// it).
    pub targeting: Option<String>,
    /// The latest dispatch time, in milliseconds since the epoch (`Date.now()`).
    pub deadline_ms: f64,
    /// The history head the caller read (`history.list`); undo and redo need it.
    pub history_head: Option<HistoryHead>,
}

/// What the hook reads in its UI-thread closure just before it would dispatch.
#[derive(Clone, Debug, PartialEq)]
pub struct Observed {
    /// The clock (`Date.now()`).
    pub now_ms: f64,
    pub revision: u64,
    pub active_sequence: Option<u64>,
    /// Linked selection (`sequence.linkedSelection`): whether an edit of the clips a call names takes
    /// the clips linked to them along (a trim, a move without `linked`, a cut through a clip, a ripple
    /// delete, an enable, a speed or gain change, an Essential Sound type or ducking). Turning it off
    /// or on bumps no revision.
    pub linked_selection: bool,
    /// The [`targeting_digest`] of the active sequence's track targeting: the tracks a lift, an
    /// extract or a transcript cut acts on, and on which a moved playhead selects clips; and where a
    /// source is patched. Changing it bumps no revision.
    pub targeting: String,
    /// The editor is busy with a person or with itself: a pointer held down, keyboard focus in a
    /// text field, an open dialog, modal or error window, a pen path, pen mask or monitor text edit
    /// in progress, a recording pass of the person's (a voice-over take, an automation pass of the
    /// Audio Track Mixer that writes automation, a multi-camera switching pass; not the automation
    /// pass that writes nothing, which the editor runs through every forward playback at normal
    /// speed), or trim playback (a dynamic trim or a trim-mode loop) that `Session::execute` would
    /// settle before anything else (`agent_view.rs` lists what it reads).
    pub busy: bool,
    pub history_head: HistoryHead,
}

/// Whether a dispatch at `now_ms` meets `deadline_ms`: in time up to and at the deadline; a clock
/// or a deadline that is not a finite number never is.
fn in_time(now_ms: f64, deadline_ms: f64) -> bool {
    now_ms.is_finite() && deadline_ms.is_finite() && now_ms <= deadline_ms
}

/// The pre-dispatch decision of `executeTyped` in `runtime`: the entry to dispatch, or the first
/// refusal of, in this order, `deadline_elapsed`, `revision_conflict`, `sequence_changed`,
/// `linked_selection_changed`, `targeting_changed`, `editor_busy`, `history_head_conflict`, and then
/// the table's `command_not_allowed`, `native_only` or `use_worker_export` ([`allowed`]). A
/// linked-selection setting or a track targeting the caller names must be the current one; one it
/// leaves out is not checked, except that a command acting on the targeted tracks needs the caller's
/// targeting (without it: `targeting_changed`). A history head the caller names must be the current
/// one, whatever the command; a missing one where the entry needs one is `history_head_conflict`
/// too. Nothing has run when this refuses.
pub fn admit(command: &str, params: &Value, expect: &Expect, observed: &Observed, runtime: Runtime) -> Result<&'static Entry, &'static str> {
    if !in_time(observed.now_ms, expect.deadline_ms) {
        return Err("deadline_elapsed");
    }
    if expect.revision != observed.revision {
        return Err("revision_conflict");
    }
    if expect.active_sequence != observed.active_sequence {
        return Err("sequence_changed");
    }
    if expect.linked_selection.is_some_and(|linked| linked != observed.linked_selection) {
        return Err("linked_selection_changed");
    }
    let targeting_holds = match &expect.targeting {
        Some(targeting) => *targeting == observed.targeting,
        None => !POLICY.iter().any(|entry| entry.command == command && entry.targeting),
    };
    if !targeting_holds {
        return Err("targeting_changed");
    }
    if observed.busy {
        return Err("editor_busy");
    }
    let head_holds = match &expect.history_head {
        Some(head) => head.length == observed.history_head.length && head.label == observed.history_head.label,
        None => !POLICY.iter().any(|entry| entry.command == command && entry.history_head),
    };
    if !head_holds {
        return Err("history_head_conflict");
    }
    allowed(command, params, runtime)
}

/// The clip `id` of the engine's serde view of a sequence: the first clip with that id on the video
/// tracks, then on the audio tracks, as the engine finds it (`Sequence::find_item`).
fn clip_of(sequence: &Value, id: u64) -> Option<&Value> {
    ["video_tracks", "audio_tracks"]
        .into_iter()
        .filter_map(|field| sequence.get(field)?.as_array())
        .flatten()
        .filter_map(|track| track.get("items")?.as_array())
        .flatten()
        .find(|item| item.get("id").and_then(Value::as_u64) == Some(id))
}

/// The media times of the keyframes of parameter `param` of the first effect `effect` on `clip` (a
/// clip of the serde view), as the engine's `keyframe_op` reaches them (`None` for a time that does
/// not read); `None` when the clip has no such effect or the effect no such parameter, which the
/// engine then refuses after dispatch.
fn keyframe_times(clip: &Value, effect: &str, param: &str) -> Option<Vec<Option<i64>>> {
    let effects = clip.get("effects")?.as_array()?;
    let instance = effects.iter().find(|instance| instance.get("effect").and_then(Value::as_str) == Some(effect))?;
    let keyframes = instance.get("params")?.get(param)?.get("keyframes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    Some(keyframes.iter().map(|keyframe| keyframe.get("time").and_then(Value::as_i64)).collect())
}

/// The media time (ticks) at which the engine's `keyframe_op` adds or removes a keyframe of `clip` (a
/// clip of the serde view) for sequence time `time`: the time held to the clip (from its start to
/// its last tick), then `TrackItem::source_time_at`, in the engine's own arithmetic: a frame hold's
/// time, else the source In point plus the elapsed time scaled by the speed, counted back from the
/// source Out point when the clip plays in reverse. `None` when the clip's timing does not read.
fn media_time(clip: &Value, time: i64) -> Option<i64> {
    if let Some(hold) = clip.get("frame_hold").filter(|hold| !hold.is_null()) {
        return hold.as_i64();
    }
    let start = clip.get("start")?.as_i64()?;
    let duration = clip.get("duration")?.as_i64()?;
    let source_in = clip.get("source_in")?.as_i64()?;
    let speed = clip.get("speed")?.as_f64()?.abs();
    let reverse = clip.get("reverse").and_then(Value::as_bool).unwrap_or(false);
    let last = start.checked_add(duration)?.checked_sub(1)?.max(start);
    let elapsed = time.clamp(start, last).checked_sub(start)?;
    let scaled = (elapsed as f64 * speed).round() as i64;
    if reverse {
        let span = (duration as f64 * speed).round() as i64;
        source_in.checked_add(span)?.checked_sub(scaled)?.checked_sub(1)
    } else {
        source_in.checked_add(scaled)
    }
}

/// The refusal, before dispatch, of a keyframe command whose keyframe the engine would not treat as
/// the call means it (slice R0, CC-25). `effects.addKeyframe` is a toggle in the engine: where a
/// keyframe already sits at the media time it resolves, it takes that keyframe away and still
/// succeeds, so an add there is `keyframe_exists` (also when the clip's timing or a keyframe's time
/// does not read: it cannot be ruled out). `effects.setKeyframe` and `effects.setInterpolation` fail
/// inside the command where no keyframe sits at `mediaTime`, and `effects.deleteKeyframe` there
/// changes nothing yet makes an undo step: those are `keyframe_not_found`. `sequence` gives the
/// engine's serde view of the active sequence; it is asked for only for these four commands. `None`
/// lets the call through, also when the view has no such clip, effect or parameter (the engine then
/// fails after dispatch; the adapter checks those over certified reads, MC §9.1).
pub fn keyframe_refusal(command: &str, params: &Value, sequence: impl FnOnce() -> Value) -> Option<&'static str> {
    let adds = command == "effects.addKeyframe";
    let edits = matches!(command, "effects.setKeyframe" | "effects.setInterpolation" | "effects.deleteKeyframe");
    if !(adds || edits) {
        return None;
    }
    let clip = params.get("clip").and_then(Value::as_u64)?;
    let effect = params.get("effect").and_then(Value::as_str)?;
    let param = params.get("param").and_then(Value::as_str)?;
    let sequence = sequence();
    let clip = clip_of(&sequence, clip)?;
    let times = keyframe_times(clip, effect, param)?;
    if adds {
        let at = params.get("time").and_then(Value::as_i64).and_then(|time| media_time(clip, time));
        let taken = at.is_none_or(|at| times.iter().any(|kept| kept.is_none_or(|kept| kept == at)));
        taken.then_some(KEYFRAME_EXISTS)
    } else {
        let at = params.get("mediaTime").and_then(Value::as_i64)?;
        (!times.contains(&Some(at))).then_some(KEYFRAME_NOT_FOUND)
    }
}

/// The refusal, before dispatch, of a template value the engine would read as another one (R0).
/// `graphics.template.set` passes a checkbox control's value on as it is given: the engine reads a
/// value other than `true` or `false` of the control that shows or hides a layer as `true`, and
/// fails on one of a control bound to another switch. So a call that gives a checkbox control,
/// named by its id or its name in any case as the engine finds it (a `control` replacing the same
/// name in `values`), anything but `true` or `false` is `control_kind_mismatch`. `sequence` gives
/// the engine's serde view of the active sequence; it is asked for only for this command. `None`
/// lets the call through, also when the view has no such clip, template or control (the engine then
/// fails after dispatch).
pub fn template_refusal(command: &str, params: &Value, sequence: impl FnOnce() -> Value) -> Option<&'static str> {
    if command != "graphics.template.set" {
        return None;
    }
    let clip = params.get("clip").and_then(Value::as_u64)?;
    let mut values = params.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(control) = params.get("control").and_then(Value::as_str) {
        values.insert(control.to_string(), params.get("value").cloned().unwrap_or_default());
    }
    let sequence = sequence();
    let controls = clip_of(&sequence, clip)?.get("graphic")?.get("template")?.get("controls")?.as_array()?;
    let checkbox = |key: &str| {
        let named = |control: &&Value| control.get("name").and_then(Value::as_str).is_some_and(|name| name.eq_ignore_ascii_case(key));
        let control = controls.iter().find(|control| control.get("id").and_then(Value::as_str) == Some(key)).or_else(|| controls.iter().find(named));
        control.is_some_and(|control| control.get("kind").and_then(Value::as_str) == Some("checkbox"))
    };
    values.iter().any(|(key, value)| checkbox(key.as_str()) && !value.is_boolean()).then_some(CONTROL_KIND_MISMATCH)
}

/// A clip (or a caption) of the serde view of a sequence: its id, where it starts and ends (ticks) and
/// its link.
struct Span {
    id: u64,
    start: i128,
    end: i128,
    link: Option<u64>,
}

/// A video, audio or caption track of the serde view: its id, and its name as the engine names it in
/// its messages (`V1`, `A2`, `C1`, by position).
struct Lane {
    id: Option<u64>,
    label: String,
    locked: bool,
    sync_lock: bool,
    spans: Vec<Span>,
}

/// One clip of the view, when it reads as one.
fn span(item: &Value) -> Option<Span> {
    let start = i128::from(item.get("start")?.as_i64()?);
    let duration = i128::from(item.get("duration")?.as_i64()?);
    Some(Span { id: item.get("id")?.as_u64()?, start, end: start + duration, link: item.get("link").and_then(Value::as_u64) })
}

/// The video tracks, then the audio tracks, of the engine's serde view of a sequence: its
/// `video_tracks` and `audio_tracks`, each with `id`, `locked`, `sync_lock` and `items` (clips with
/// `id`, `start`, `duration` and `link`). A track whose flags do not read counts as unlocked and
/// sync-locked, as a new track is, so the checks stay conservative; a view that does not read at
/// all has no tracks, and then nothing is found in any ripple's way.
fn lanes(sequence: &Value) -> Vec<Lane> {
    let mut lanes = Vec::new();
    for (field, prefix) in [("video_tracks", 'V'), ("audio_tracks", 'A')] {
        let tracks = sequence.get(field).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        for (index, track) in tracks.iter().enumerate() {
            let items = track.get("items").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            lanes.push(Lane {
                id: track.get("id").and_then(Value::as_u64),
                label: format!("{prefix}{}", index + 1),
                locked: track.get("locked").and_then(Value::as_bool).unwrap_or(false),
                sync_lock: track.get("sync_lock").and_then(Value::as_bool).unwrap_or(true),
                spans: items.iter().filter_map(span).collect(),
            });
        }
    }
    lanes
}

/// The caption tracks of the engine's serde view of a sequence (`caption_tracks`, from the top), named
/// as the engine names them (`C1` and on), each with `id`, `locked`, `sync_lock` and `captions` (with
/// `id`, `start` and `duration`), read as [`lanes`] reads a track.
fn caption_lanes(sequence: &Value) -> Vec<Lane> {
    let tracks = sequence.get("caption_tracks").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut lanes = Vec::new();
    for (index, track) in tracks.iter().enumerate() {
        let captions = track.get("captions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        lanes.push(Lane {
            id: track.get("id").and_then(Value::as_u64),
            label: format!("C{}", index + 1),
            locked: track.get("locked").and_then(Value::as_bool).unwrap_or(false),
            sync_lock: track.get("sync_lock").and_then(Value::as_bool).unwrap_or(true),
            spans: captions.iter().filter_map(span).collect(),
        });
    }
    lanes
}

/// The sync-locked tracks in the way of a ripple, found before dispatch (slice R0, MC §9.4): the
/// engine refuses such a ripple from inside the command (`SyncLockConflict`), where FF1 could only
/// call it `unknown`. The ripples of the table that can meet that refusal are a `timeline.trim` in
/// `ripple` mode that closes time, an `edit.rippleDelete` and a `sequence.closeGap`
/// (`filmcraft_edit` raises it nowhere else: an extract and a transcript cut never meet it). Since
/// slice R1 an insert (`timeline.place` with `insert`) is checked too: the engine never refuses one,
/// it cuts in two, in silence, the clip or caption that runs across the insert on every unlocked,
/// sync-locked track the call does not name ([`insert_in_the_way`]). `sequence` gives the engine's
/// serde view of the active sequence ([`lanes`]); it is asked for only for those commands. Empty
/// means nothing is in the way, or the view says the engine will refuse the call for another reason
/// (no such clip or track, nothing to close), which then ends `unknown`, as before.
///
/// It is conservative: it may refuse an edit the engine would allow (a split edit whose sound
/// follows its picture, an L cut, a delta the engine clamps to nothing), and the agent can then
/// turn the sync lock off or edit another way; an edit it lets through that the engine refuses
/// still ends `unknown`.
pub fn sync_locked_tracks(command: &str, params: &Value, linked_selection: bool, sequence: impl FnOnce() -> Value) -> Vec<String> {
    match command {
        "timeline.trim" => ripple_trim_in_the_way(params, linked_selection, sequence),
        "edit.rippleDelete" => ripple_delete_in_the_way(params, linked_selection, sequence),
        "sequence.closeGap" => close_gap_in_the_way(params, sequence),
        "timeline.place" => insert_in_the_way(params, sequence),
        _ => Vec::new(),
    }
}

/// The tracks in the way of a `timeline.trim` in `ripple` mode that closes time (a shorter tail, or
/// a shorter head), as `filmcraft_edit::ripple_trim_group` meets them. The clip trims with the clips
/// linked to it when `linked_selection` is on, as the engine trims it (`with_links`), and the
/// tracks that hold one of them ripple with it. On every other unlocked, sync-locked track the check
/// looks, from the requested delta (the engine may clamp it smaller, never larger), for either of
/// the engine's two conflicts: a clip in the stretch that closes, other than a clip linked to the
/// trimmed one; or a clip that starts before the cut and ends after the point the first later clip
/// moves back to (or that point before the sequence start).
fn ripple_trim_in_the_way(params: &Value, linked_selection: bool, sequence: impl FnOnce() -> Value) -> Vec<String> {
    if params.get("mode").and_then(Value::as_str) != Some("ripple") {
        return Vec::new();
    }
    let head = params.get("edge").and_then(Value::as_str) == Some("in");
    let delta = i128::from(params.get("delta").and_then(Value::as_i64).unwrap_or(0));
    // a longer head or tail moves the later clips away from the cut: nothing can be in its way
    let opens = if head { delta <= 0 } else { delta >= 0 };
    if opens {
        return Vec::new();
    }
    let Some(clip) = params.get("clip").and_then(|clip| clip.as_u64().or_else(|| clip.as_f64().map(|id| id as u64))) else {
        return Vec::new();
    };
    let lanes = lanes(&sequence());
    let Some(trimmed) = lanes.iter().flat_map(|lane| &lane.spans).find(|span| span.id == clip) else {
        return Vec::new();
    };
    // a clip linked to the trimmed one is its own: the engine never counts it as in the way
    let link = trimmed.link;
    let own = |span: &Span| link.is_some() && span.link == link;
    let group: Vec<u64> =
        lanes.iter().flat_map(|lane| &lane.spans).filter(|&span| span.id == clip || (linked_selection && own(span))).map(|span| span.id).collect();
    // the stretch that closes, where the later clips start to move and by how much: after the
    // trimmed tail, or after the latest head of the group (a sound that leads its picture starts
    // earlier)
    let (closing_start, closing_end, from, shift) = if head {
        let cut = lanes.iter().flat_map(|lane| &lane.spans).filter(|span| group.contains(&span.id)).map(|span| span.start).max().unwrap_or(trimmed.start);
        (cut, cut + delta, cut + 1, -delta)
    } else {
        (trimmed.end + delta, trimmed.end, trimmed.end, delta)
    };
    let mut in_the_way = Vec::new();
    for lane in &lanes {
        if lane.locked || !lane.sync_lock || lane.spans.iter().any(|span| group.contains(&span.id)) {
            continue;
        }
        let closed = lane.spans.iter().any(|span| !own(span) && span.start < closing_end && closing_start < span.end);
        let landing = lane.spans.iter().filter(|span| span.start >= from).map(|span| span.start + shift).min();
        let crowded = landing.is_some_and(|landing| landing < 0 || lane.spans.iter().any(|span| span.start < from && span.end > landing));
        if closed || crowded {
            in_the_way.push(lane.label.clone());
        }
    }
    in_the_way
}

/// `ranges` (start, end) in time order, with the ones that touch or overlap joined, as
/// `filmcraft_edit` joins them for a ripple delete.
fn joined(mut ranges: Vec<(i128, i128)>) -> Vec<(i128, i128)> {
    ranges.sort_unstable();
    let mut out: Vec<(i128, i128)> = Vec::new();
    for (start, end) in ranges {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// The tracks in the way of an `edit.rippleDelete` of `clips`, as `filmcraft_edit::ripple_delete_items`
/// meets them. The engine deletes the listed clips and, when `linked_selection` is on, every clip
/// linked to one of them (`with_links`). The stretches it closes are the time the deleted clips
/// covered, less whatever stays on the unlocked tracks that lose a clip; an unlocked, sync-locked
/// track that loses none is in the way when a clip of it overlaps one of those stretches. Empty also
/// when a listed clip is not in the view or nothing can close (the engine refuses those otherwise).
fn ripple_delete_in_the_way(params: &Value, linked_selection: bool, sequence: impl FnOnce() -> Value) -> Vec<String> {
    let Some(listed) = params.get("clips").and_then(Value::as_array) else {
        return Vec::new();
    };
    let listed: Vec<u64> = listed.iter().filter_map(Value::as_u64).collect();
    let lanes = lanes(&sequence());
    let found: Vec<&Span> = lanes.iter().flat_map(|lane| &lane.spans).filter(|span| listed.contains(&span.id)).collect();
    if listed.iter().any(|id| !found.iter().any(|span| span.id == *id)) {
        return Vec::new();
    }
    let links: Vec<u64> = found.iter().filter_map(|span| span.link).collect();
    let group: Vec<u64> = lanes
        .iter()
        .flat_map(|lane| &lane.spans)
        .filter(|span| listed.contains(&span.id) || (linked_selection && span.link.is_some_and(|link| links.contains(&link))))
        .map(|span| span.id)
        .collect();
    let (mut gone, mut kept) = (Vec::new(), Vec::new());
    for lane in lanes.iter().filter(|lane| !lane.locked && lane.spans.iter().any(|span| group.contains(&span.id))) {
        for span in &lane.spans {
            if group.contains(&span.id) {
                gone.push((span.start, span.end));
            } else {
                kept.push((span.start, span.end));
            }
        }
    }
    let kept = joined(kept);
    let mut closing = Vec::new();
    for (start, end) in joined(gone) {
        let mut at = start;
        for &(from, to) in kept.iter().filter(|&&(from, to)| from < end && start < to) {
            if from > at {
                closing.push((at, from));
            }
            at = at.max(to);
        }
        if at < end {
            closing.push((at, end));
        }
    }
    lanes
        .iter()
        .filter(|lane| !lane.locked && lane.sync_lock && !lane.spans.iter().any(|span| group.contains(&span.id)))
        .filter(|lane| lane.spans.iter().any(|span| closing.iter().any(|&(from, to)| span.start < to && from < span.end)))
        .map(|lane| lane.label.clone())
        .collect()
}

/// Whether `track` (an id, or a name such as `V2` or `a1`) names `lane`, as the engine's `track_p`
/// reads it.
fn names(track: &Value, lane: &Lane) -> bool {
    if let Some(id) = track.as_u64() {
        return lane.id == Some(id);
    }
    let Some(name) = track.as_str() else {
        return false;
    };
    let mut chars = name.chars();
    let prefix = match chars.next() {
        Some('v' | 'V') => 'V',
        Some('a' | 'A') => 'A',
        _ => return false,
    };
    chars.as_str().parse::<usize>().is_ok_and(|number| number >= 1 && lane.label == format!("{prefix}{number}"))
}

/// The tracks in the way of a `sequence.closeGap` at `time` on `track`, as `filmcraft_edit::close_gap`
/// meets them: the gap is the empty stretch of that track around `time`, from the end of the clip
/// before it to the start of the clip after it, and an unlocked, sync-locked track other than that
/// one is in the way when a clip of it overlaps the gap. Empty also when the track is not in the
/// view, a clip of it covers `time`, or no clip follows (the engine refuses those otherwise).
fn close_gap_in_the_way(params: &Value, sequence: impl FnOnce() -> Value) -> Vec<String> {
    let (Some(track), Some(time)) = (params.get("track"), params.get("time").and_then(Value::as_i64)) else {
        return Vec::new();
    };
    let time = i128::from(time);
    let lanes = lanes(&sequence());
    let Some(lane) = lanes.iter().find(|lane| names(track, lane)) else {
        return Vec::new();
    };
    if lane.spans.iter().any(|span| (span.start..span.end).contains(&time)) {
        return Vec::new();
    }
    let Some(next) = lane.spans.iter().filter(|span| span.start > time).map(|span| span.start).min() else {
        return Vec::new();
    };
    let previous = lane.spans.iter().filter(|span| span.end <= time).map(|span| span.end).max().unwrap_or(0);
    lanes
        .iter()
        .filter(|other| !other.locked && other.sync_lock && other.label != lane.label)
        .filter(|other| other.spans.iter().any(|span| span.start < next && previous < span.end))
        .map(|other| other.label.clone())
        .collect()
}

/// The tracks an insert (`timeline.place` with `insert: true`, slice R1) would cut in two without the
/// call naming them, as `filmcraft_edit::insert` treats them: at the time of the insert the engine
/// opens a gap on every unlocked track that takes the placed clips or is sync-locked, and on every
/// unlocked, sync-locked caption track, cutting in two the clip or caption that starts before that
/// time and ends after it (a music bed, an overlay, a caption), and raises no conflict. The tracks the
/// call names with `track` and `audioTrack` are its own: their clips are cut by design. Every other
/// unlocked, sync-locked track with a clip or caption across the time is in the way: video tracks,
/// then audio tracks, then caption tracks. A track the engine picks for the call (the source
/// patching, the further audio channels of an item) is one the call does not name. Empty for an
/// overwrite, and when `track` or `audioTrack` names no video or audio track of the view (the engine
/// refuses that from inside the command).
fn insert_in_the_way(params: &Value, sequence: impl FnOnce() -> Value) -> Vec<String> {
    let (Some(true), Some(time)) = (params.get("insert").and_then(Value::as_bool), params.get("time").and_then(Value::as_i64)) else {
        return Vec::new();
    };
    let view = sequence();
    let mut lanes = lanes(&view);
    let named: Vec<&Value> = ["track", "audioTrack"].into_iter().filter_map(|key| params.get(key)).collect();
    if named.iter().any(|track| !lanes.iter().any(|lane| names(track, lane))) {
        return Vec::new();
    }
    lanes.extend(caption_lanes(&view));
    let time = i128::from(time);
    lanes
        .iter()
        .filter(|lane| !lane.locked && lane.sync_lock && !named.iter().any(|track| names(track, lane)))
        .filter(|lane| lane.spans.iter().any(|span| span.start < time && time < span.end))
        .map(|lane| lane.label.clone())
        .collect()
}

/// The fields every envelope carries: the version, the outcome and both revisions as decimal
/// strings (INV-32).
fn envelope(outcome: &str, revision_before: u64, revision_after: u64) -> Map<String, Value> {
    let mut envelope = Map::new();
    envelope.insert("resultVersion".into(), json!(RESULT_VERSION));
    envelope.insert("outcome".into(), json!(outcome));
    envelope.insert("revisionBefore".into(), json!(revision_before.to_string()));
    envelope.insert("revisionAfter".into(), json!(revision_after.to_string()));
    envelope
}

/// The envelope of a refusal decided before dispatch: `not_applied` with its `code`, at `revision`.
pub fn refused(code: &str, revision: u64) -> Value {
    let mut refusal = envelope("not_applied", revision, revision);
    refusal.insert("code".into(), json!(code));
    Value::Object(refusal)
}

/// The envelope of a ripple refused before dispatch because sync-locked `tracks` are in its way
/// ([`sync_locked_tracks`]): `not_applied`, `sync_locked`, the tracks in `syncLockedTracks`
/// (MC §9.4), at `revision`.
pub fn sync_locked(revision: u64, tracks: Vec<String>) -> Value {
    let mut refusal = envelope("not_applied", revision, revision);
    refusal.insert("code".into(), json!(SYNC_LOCKED));
    refusal.insert("syncLockedTracks".into(), json!(tracks));
    Value::Object(refusal)
}

/// The envelope of an engine failure after dispatch: `unknown`, with the engine text in `detail`.
fn unknown(revision_before: u64, revision_after: u64, history_label: Option<&str>, detail: &str) -> Value {
    let mut failure = envelope("unknown", revision_before, revision_after);
    failure.insert("code".into(), json!(ENGINE_ERROR_AFTER_DISPATCH));
    failure.insert("detail".into(), json!(detail.chars().take(DETAIL_CHARS).collect::<String>()));
    if let Some(label) = history_label {
        failure.insert("historyLabel".into(), json!(label));
    }
    Value::Object(failure)
}

/// What `Session::execute` answered.
#[derive(Debug)]
pub enum Answer {
    /// The command's result.
    Ok(Value),
    /// `EngineError::Disabled`, with the command it names (a nested command may name another).
    Disabled(String),
    /// Any other engine error, as text.
    Failed(String),
}

/// The envelope of a dispatched command, from the revisions before and after it, the newest undo
/// label after it and the engine's answer.
///
/// Only the dispatched command's own `Disabled` error with an unchanged revision is a refusal
/// (`not_applied`, `command_disabled`): `Session::execute` raises it before the command runs.
/// Another command's `Disabled` error (from a nested `execute`), a moved revision or any other
/// error is `unknown`. A success is `applied`, with the engine's `result` and the undo label; a trim
/// adds `actual.deltaTicks` (the delta the engine reports it applied, a decimal string) and
/// `intentSatisfied` (that delta is the requested one), and names a clamp `clamped`. A read
/// ([`reads_only`]) takes no undo step, so its envelope names no history label: the newest undo label
/// is then an earlier edit's.
pub fn settle(command: &str, params: &Value, revision_before: u64, revision_after: u64, history_label: Option<&str>, answer: Answer) -> Value {
    let history_label = history_label.filter(|_| !reads_only(command, params));
    let result = match answer {
        Answer::Ok(result) => result,
        Answer::Disabled(named) if named == command && revision_after == revision_before => return refused("command_disabled", revision_before),
        Answer::Disabled(named) => {
            let detail = format!("`{named}` was not available while `{command}` ran");
            return unknown(revision_before, revision_after, history_label, &detail);
        }
        Answer::Failed(message) => return unknown(revision_before, revision_after, history_label, &message),
    };
    let mut applied = envelope("applied", revision_before, revision_after);
    if command == "timeline.trim"
        && let Some(moved) = result.get("delta").and_then(Value::as_i64)
    {
        // a trim without `delta` asks the engine for none
        let satisfied = params.get("delta").map_or(Some(0), Value::as_i64) == Some(moved);
        applied.insert("intentSatisfied".into(), json!(satisfied));
        applied.insert("actual".into(), json!({"deltaTicks": moved.to_string()}));
        if !satisfied {
            applied.insert("code".into(), json!(CLAMPED));
        }
    }
    applied.insert("result".into(), result);
    if let Some(label) = history_label {
        applied.insert("historyLabel".into(), json!(label));
    }
    Value::Object(applied)
}

/// What a typed marks edit reports as its `result` (slice R1; m22, BL item 3): the sequence's In
/// and Out marks after the edit, half-open, `{"marks": {"in", "outInclusive", "endExclusive"}}`,
/// each a decimal string of ticks, or null where that mark is not set: `in` and `outInclusive` as
/// the engine stores them (Out is the start of the last frame the range holds), and `endExclusive`
/// one `frame` later (null too when that is past `i64`). `None` for every other command, whose result is
/// the engine's.
pub fn marks_result(command: &str, mark_in: Option<i64>, mark_out: Option<i64>, frame: i64) -> Option<Value> {
    if !matches!(command, "markers.markIn" | "markers.markOut" | "markers.clearInOut") {
        return None;
    }
    let ticks = |tick: Option<i64>| tick.map(|tick| tick.to_string());
    let end = mark_out.and_then(|out| out.checked_add(frame));
    Some(json!({"marks": {"in": ticks(mark_in), "outInclusive": ticks(mark_out), "endExclusive": ticks(end)}}))
}

/// One verdict of the pending-media loop of `renderFrame` and `scopes`, after an attempt whose
/// render left the pending flag at `pending`, read at `now_ms`: `Some(true)` when the attempt is
/// complete (always the end, even after the deadline), `None` to wait for the host's fetches and
/// try again (up to and at the deadline), `Some(false)` to answer the incomplete attempt (past the
/// deadline, or a clock or deadline that is not a finite number).
pub fn attempt(pending: bool, now_ms: f64, deadline_ms: f64) -> Option<bool> {
    if !pending {
        Some(true)
    } else if in_time(now_ms, deadline_ms) {
        None
    } else {
        Some(false)
    }
}

/// What `renderFrame` (and `scopes`) report about the frame they rendered.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameMeta {
    pub width: usize,
    pub height: usize,
    /// The frame's start tick (the requested time snapped down to its frame).
    pub tick: i64,
    /// The frame index in the active sequence.
    pub frame: i64,
    pub revision: u64,
    pub active_sequence: Option<u64>,
}

/// The `renderFrame` fields besides the pixels: the size, the tick and the revision as decimal
/// strings (INV-32), the frame index, the active sequence (a decimal string), `complete` and the
/// number of attempts.
pub fn frame_report(meta: &FrameMeta, complete: bool, attempts: u32) -> Value {
    json!({
        "width": meta.width,
        "height": meta.height,
        "tick": meta.tick.to_string(),
        "frame": meta.frame,
        "revision": meta.revision.to_string(),
        "activeSequence": meta.active_sequence.map(|id| id.to_string()),
        "complete": complete,
        "attempts": attempts,
    })
}
