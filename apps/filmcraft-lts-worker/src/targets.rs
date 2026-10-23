//! Target checks (wave native-acts, owner decisions OC-47 D1 and D11): what a typed edit names is checked against the
//! session it runs in, in the same call, before the engine runs anything. A target that is not there, or of another
//! kind than the act needs, is refused `invalid_params`, its `detail` naming the target, with nothing run: no
//! revision, no undo step and no Events line. Left to the engine, such a call fails inside the command, applies as an
//! undo step that changes nothing (a cut through no clip, a video effect for a sound's clip), applies to the targets
//! that are there and not to the others (an effect for two clips of which one is missing), commits a step before it
//! fails (a caption on a caption track that is not there, in a sequence that had none, after the engine made one), or
//! lands elsewhere (a shape for a clip that is no graphic starts a graphic clip of its own at the playhead, a title on
//! a video track past the last one takes a free track, a speaker renamed for an item that is no media item is renamed
//! in every transcript).
//!
//! In the active sequence:
//! - every clip a call names (`clip`, `clips`, each move's `clip`) is a clip of its video or audio tracks, of the kind
//!   the act needs: a picture's act (a video transition, Lumetri, fit and fill, a frame hold, scene detection, a mask,
//!   a video effect and its parameters and keyframes) a clip of a video track, a sound's act (an audio transition, an
//!   audio effect's parameters and keyframes) a clip of an audio track, and a graphic's edit a graphic clip;
//! - every video or audio track (`track` and `audioTrack`, by id or as `V1` and `A1` on, as the engine reads them; each
//!   move's `track`, of its clip's kind), caption track (by id or as `C1` on; the first caption of a sequence without a
//!   caption track may name `C1`, which the engine makes for it) and title track (by its index among the video
//!   tracks) is one of its own;
//! - every caption (`caption`, `captions`), the marker (`marker`: one of this sequence's own, as one session cuts
//!   several shorts, a sequence each) and the transition (`transition`) a call names are its own.
//!
//! In the project: the item a place or a subclip names, and the media item (or a subclip of one) a transcript is set
//! for or a speaker renamed in. The native-only entries have their own checks (`reach::precheck`). With no sequence
//! open, the engine refuses every command that needs one itself, before it runs (`command_disabled`), so nothing is
//! checked against a sequence then.

use filmcraft_engine::Session;
use filmcraft_engine::project::{ClipId, EffectKind, ItemId, Sequence, Track, TrackId, TrackKind, find_effect};
use serde_json::Value;

/// What an act needs of the clips it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Needs {
    /// A clip of the sequence, of either kind.
    Clip,
    /// A clip of a video track.
    Picture,
    /// A clip of an audio track.
    Sound,
    /// A graphic clip.
    Graphic,
}

impl Needs {
    /// What an act of this kind takes, as a refusal says it.
    fn wants(self) -> &'static str {
        match self {
            Needs::Clip => "acts on a clip of the active sequence",
            Needs::Picture => "acts on a clip of a video track",
            Needs::Sound => "acts on a clip of an audio track",
            Needs::Graphic => "edits a graphic clip",
        }
    }
}

/// Why a call the table admits names a target the session does not hold, or one of another kind than its act needs;
/// `Ok` when every target it names is there (see the module documentation).
pub(crate) fn check(session: &Session, command: &str, params: &Value) -> Result<(), String> {
    item(session, command, params)?;
    let Some(sequence) = session.active_sequence() else { return Ok(()) };
    let call = Call { session, sequence, command, params };
    call.clips()?;
    call.tracks()?;
    call.captions()?;
    call.marker()?;
    call.transition()
}

/// The item a call names (`item`) is in the project: any item for a place or a subclip, and a media item (or a subclip
/// of one) for a transcript set or a speaker renamed, where the engine reads an item that is none as no item (and
/// renames the speaker in every transcript).
fn item(session: &Session, command: &str, params: &Value) -> Result<(), String> {
    let Some(id) = params.get("item").and_then(Value::as_u64) else { return Ok(()) };
    let project = &session.project;
    match command {
        "timeline.place" | "clip.makeSubclip" if project.item(ItemId(id)).is_none() => Err(format!("item {id} is not in the project")),
        "transcript.set" | "transcript.renameSpeaker" if project.resolve_media(ItemId(id)).is_none() => Err(format!("item {id} is no media item")),
        _ => Ok(()),
    }
}

/// What the act of `command` needs of the clips it names (an effect's act: by the effect `params` name).
fn needs(command: &str, params: &Value) -> Needs {
    match command {
        "sequence.applyVideoTransition" | "lumetri.setSection" | "clip.fillFrame" | "clip.fitToFrame" | "clip.frameHold" => Needs::Picture,
        "clip.sceneEditDetection" | "masks.add" | "masks.set" | "masks.translate" | "masks.remove" | "masks.track" => Needs::Picture,
        "sequence.applyAudioTransition" => Needs::Sound,
        "graphics.set" | "graphics.newShape" | "graphics.setTextType" | "graphics.setCharStyle" | "graphics.align" => Needs::Graphic,
        "graphics.distribute" | "graphics.arrangeLayer" | "graphics.template.set" => Needs::Graphic,
        "effects.apply" | "effects.setParam" | "effects.addKeyframe" | "effects.setKeyframe" | "effects.setInterpolation" => effect_needs(params),
        "effects.deleteKeyframe" => effect_needs(params),
        _ => Needs::Clip,
    }
}

/// What an effect's act needs of its clips, as the engine's catalogue makes the effect `params` name: a video effect a
/// clip of a video track, an audio effect one of an audio track.
fn effect_needs(params: &Value) -> Needs {
    match params.get("effect").and_then(Value::as_str).and_then(find_effect).map(|effect| effect.kind) {
        Some(EffectKind::Video) => Needs::Picture,
        Some(EffectKind::Audio) => Needs::Sound,
        _ => Needs::Clip,
    }
}

/// A target as a call names it, for a refusal's detail: a name as it is, an id as its number.
fn shown(named: &Value) -> String {
    match named {
        Value::String(name) => name.clone(),
        other => other.to_string(),
    }
}

/// The video or audio track `name` names as the engine reads it (`track_p`): `V` or `A`, in either case, then its
/// number from 1.
fn by_name<'a>(sequence: &'a Sequence, name: &str) -> Option<&'a Track> {
    let mut rest = name.chars();
    let tracks = match rest.next()? {
        'v' | 'V' => &sequence.video_tracks,
        'a' | 'A' => &sequence.audio_tracks,
        _ => return None,
    };
    let number: usize = rest.as_str().parse().ok()?;
    tracks.get(number.checked_sub(1)?)
}

/// A call, and the active sequence its targets are checked in.
struct Call<'a> {
    session: &'a Session,
    sequence: &'a Sequence,
    command: &'a str,
    params: &'a Value,
}

impl Call<'_> {
    /// The act a refusal names: the command, and the effect it names.
    fn act(&self) -> String {
        match self.params.get("effect").and_then(Value::as_str) {
            Some(effect) => format!("`{}` of {effect}", self.command),
            None => format!("`{}`", self.command),
        }
    }

    /// Every clip the call names (`clip`, `clips`, each move's `clip`) is one of the sequence's, of the kind its act needs.
    fn clips(&self) -> Result<(), String> {
        let needs = needs(self.command, self.params);
        let one = self.params.get("clip");
        let listed = self.params.get("clips").and_then(Value::as_array).into_iter().flatten();
        let moved = self.params.get("moves").and_then(Value::as_array).into_iter().flatten().filter_map(|step| step.get("clip"));
        for id in one.into_iter().chain(listed).chain(moved).filter_map(Value::as_u64) {
            self.clip(id, needs)?;
        }
        Ok(())
    }

    /// Clip `id` is one of the sequence's, of the kind `needs`.
    fn clip(&self, id: u64, needs: Needs) -> Result<(), String> {
        let Some((track, _)) = self.sequence.find_item(ClipId(id)) else { return Err(format!("clip {id} is not in the active sequence")) };
        let video = self.sequence.track(track).is_some_and(|found| found.kind == TrackKind::Video);
        let why = match needs {
            Needs::Picture if !video => "is on an audio track",
            Needs::Sound if video => "is on a video track",
            Needs::Graphic if !filmcraft_engine::graphics::is_graphic(self.session, self.sequence, ClipId(id)) => "is no graphic clip",
            _ => return Ok(()),
        };
        Err(format!("clip {id} {why}: {} {}", self.act(), needs.wants()))
    }

    /// Every track the call names is one of the sequence's (see the module documentation).
    fn tracks(&self) -> Result<(), String> {
        match self.command {
            "timeline.place" | "timeline.razor" | "sequence.closeGap" | "timeline.setTrack" => {
                for key in ["track", "audioTrack"] {
                    if let Some(named) = self.params.get(key) {
                        self.track(named)?;
                    }
                }
                Ok(())
            }
            "timeline.move" => self.moves(),
            "captions.add" | "captions.setStyle" => self.caption_track(),
            "graphics.newText" => self.title_track(),
            _ => Ok(()),
        }
    }

    /// The kind of the video or audio track `named` names, as the engine finds it (`track_p`): by its id, or as `V1` and
    /// `A1` on.
    fn track(&self, named: &Value) -> Result<TrackKind, String> {
        let found = match named.as_u64() {
            Some(id) => self.sequence.track(TrackId(id)),
            None => named.as_str().and_then(|name| by_name(self.sequence, name)),
        };
        found.map(|track| track.kind).ok_or_else(|| format!("track {} is not in the active sequence", shown(named)))
    }

    /// Each move's track is one of the sequence's, of its clip's kind: the engine would put a clip on a track of the other
    /// kind.
    fn moves(&self) -> Result<(), String> {
        for step in self.params.get("moves").and_then(Value::as_array).into_iter().flatten() {
            let Some(named) = step.get("track") else { continue };
            let to = self.track(named)?;
            let clip = step.get("clip").and_then(Value::as_u64).and_then(|id| self.sequence.find_item(ClipId(id)));
            let from = clip.and_then(|(track, _)| self.sequence.track(track)).map(|track| track.kind);
            if from.is_some_and(|from| from != to) {
                let id = step.get("clip").map(shown).unwrap_or_default();
                return Err(format!("clip {id} cannot move to track {}: a clip moves to a track of its own kind", shown(named)));
            }
        }
        Ok(())
    }

    /// The caption track the call names (`track`) is one of the sequence's, as the engine finds it: by its id, or as `C1`
    /// and on. The first caption of a sequence without a caption track may name `C1`: the engine makes that track for it.
    fn caption_track(&self) -> Result<(), String> {
        let Some(named) = self.params.get("track") else { return Ok(()) };
        let tracks = &self.sequence.caption_tracks;
        let made = self.command == "captions.add" && tracks.is_empty();
        let found = match named {
            Value::Number(id) => id.as_u64().is_some_and(|id| tracks.iter().any(|track| track.id.0 == id)),
            Value::String(name) => {
                let index = name.trim_start_matches(['C', 'c']).parse::<usize>().ok().and_then(|number| number.checked_sub(1));
                index.is_some_and(|at| at < tracks.len() || (made && at == 0))
            }
            _ => true,
        };
        if found {
            return Ok(());
        }
        Err(format!("caption track {} is not in the active sequence", shown(named)))
    }

    /// A title's track (`track`, its index among the video tracks from V1: the engine reads no name there) is one of the
    /// sequence's: past the last one, the engine would take a free track for the title instead.
    fn title_track(&self) -> Result<(), String> {
        let Some(index) = self.params.get("track").and_then(Value::as_u64) else { return Ok(()) };
        if usize::try_from(index).is_ok_and(|at| at < self.sequence.video_tracks.len()) {
            return Ok(());
        }
        Err(format!("track V{} (index {index}) is not in the active sequence", index.saturating_add(1)))
    }

    /// Every caption the call names (`caption`, `captions`) is one of the sequence's.
    fn captions(&self) -> Result<(), String> {
        let one = self.params.get("caption");
        let listed = self.params.get("captions").and_then(Value::as_array).into_iter().flatten();
        for id in one.into_iter().chain(listed).filter_map(Value::as_u64) {
            if self.sequence.find_caption(ClipId(id)).is_none() {
                return Err(format!("caption {id} is not in the active sequence"));
            }
        }
        Ok(())
    }

    /// The marker the call names (`marker`) is one of the active sequence's own: one session cuts several shorts, one
    /// sequence each, and another sequence's marker is not there.
    fn marker(&self) -> Result<(), String> {
        let Some(id) = self.params.get("marker").and_then(Value::as_u64) else { return Ok(()) };
        if self.sequence.markers.iter().any(|marker| marker.id.0 == id) {
            return Ok(());
        }
        Err(format!("marker {id} is not a marker of the active sequence"))
    }

    /// The transition the call names (`transition`) is on one of the sequence's tracks.
    fn transition(&self) -> Result<(), String> {
        let Some(id) = self.params.get("transition").and_then(Value::as_u64) else { return Ok(()) };
        if self.sequence.all_tracks().any(|track| track.transitions.iter().any(|transition| transition.id.0 == id)) {
            return Ok(());
        }
        Err(format!("transition {id} is not in the active sequence"))
    }
}
