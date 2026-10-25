//! Time annotations (spec section 4.9): intervals with a kind (speech, laughter, music, black, ...) that an agent or a
//! person writes. They are data, like candidates: stored, shown, measured against and mapped through edits, never created
//! from perception and never acted on by the editor. This module holds I17 and the writes of `vertical.setAnnotations`
//! (`add` content-idempotent, `update`, `remove`, `replace` by kind and range).

use std::collections::BTreeMap;

use serde_json::json;

use super::canon::{on_grid, q};
use super::plan::{Ctx, Draft, MAX_LIST, SetAnnotations, alloc, chars_within, check_len, not_found_id, too_many};
use super::types::{Annotation, Refusal};
use super::walls::counted;

/// The most annotations a set holds (I17).
pub const MAX_ANNOTATIONS: usize = 20_000;

/// The longest annotation label (I17).
const MAX_LABEL: usize = 64;

/// Whether `kind` is a legal annotation kind: 1 to 32 characters of `[a-z0-9_:-]` (I17).
pub fn valid_kind(kind: &str) -> bool {
    (1..=32).contains(&kind.len()) && kind.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b':' | b'-'))
}

/// Whether `id` is a legal box-track id, the syntax of a position's `subjects` too: 1 to 64 characters of
/// `[A-Za-z0-9_:.-]` (I18, I20).
pub fn valid_track_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'-'))
}

/// The content of an annotation a write names: kind, frames, label and score (on the 1/1000 grid).
#[derive(Clone, Debug, PartialEq)]
struct Content {
    kind: String,
    from: i64,
    to: i64,
    label: String,
    score: Option<f64>,
}

impl Content {
    /// Checked: `bad_annotation {index, reason}` for a bad kind or range, `too_many` for a long label, `bad_number {key:
    /// "score", index}` for a score whose grid value is not a finite number on the grid (spec section 2.6 stores
    /// `q(score)` and I17 checks that: a huge finite score overflows to infinity there). The score is put on the grid.
    fn checked(self, ctx: &Ctx, index: usize) -> Result<Self, Refusal> {
        let bad = |reason: &str| Refusal::new("bad_annotation", format!("annotation {index}: {reason}"), json!({"index": index, "reason": reason}));
        if !valid_kind(&self.kind) {
            return Err(bad("its kind is 1 to 32 characters of [a-z0-9_:-]"));
        }
        if self.from < 0 || self.from >= self.to || self.to > ctx.r.end_frame {
            return Err(bad("its frames lie inside [0, end], from before to"));
        }
        chars_within("label", &self.label, MAX_LABEL)?;
        let score = self.score.map(q);
        if score.is_some_and(|s| !on_grid(s)) {
            return Err(Refusal::new("bad_number", "a score is not a finite number on the 1/1000 grid", json!({"key": "score", "index": index})));
        }
        Ok(Self { score, ..self })
    }

    /// The content key: kind, frames, label and the score's bits.
    fn key(&self) -> Key {
        (self.kind.clone(), self.from, self.to, self.label.clone(), self.score.map(f64::to_bits))
    }

    /// Whether a stored annotation has this content.
    fn matches(&self, a: &Annotation, ctx: &Ctx) -> bool {
        let rate = ctx.r.rate;
        a.kind == self.kind && rate.frame_at(a.from) == self.from && rate.frame_at(a.to) == self.to && a.label == self.label && a.score == self.score
    }

    /// The stored annotation `id` with this content, written by the caller.
    fn stored(self, ctx: &Ctx, id: u32) -> Annotation {
        let rate = ctx.r.rate;
        let (from, to) = (rate.tick_of(self.from), rate.tick_of(self.to));
        Annotation { id, kind: self.kind, from, to, label: self.label, score: self.score, origin: ctx.origin(), modified_by: ctx.caller.writer }
    }
}

/// `vertical.setAnnotations`: `remove`, then `update`, then `replace`, then `add` (an add identical to an existing
/// annotation, kind, range, label and score, returns its id). Annotations stay sorted by `(from, id)`.
pub(crate) fn set_annotations(ctx: &Ctx, draft: &mut Draft, w: &SetAnnotations) -> Result<(), Refusal> {
    check_len("add", w.add.len(), MAX_ANNOTATIONS)?;
    check_len("update", w.update.len(), MAX_ANNOTATIONS)?;
    check_len("remove", w.remove.len(), MAX_LIST)?;
    if let Some(r) = &w.replace {
        check_len("set", r.set.len(), MAX_ANNOTATIONS)?;
    }
    let rate = ctx.r.rate;
    let before = draft.set.annotations.len();
    draft.set.annotations.retain(|a| !w.remove.contains(&a.id));
    let removed = before - draft.set.annotations.len();
    for (index, u) in w.update.iter().enumerate() {
        let Some(slot) = draft.set.annotations.iter_mut().find(|a| a.id == u.id) else { return Err(not_found_id("annotation", u.id)) };
        let content = Content {
            kind: u.kind.clone().unwrap_or_else(|| slot.kind.clone()),
            from: u.from_frame.unwrap_or(rate.frame_at(slot.from)),
            to: u.to_frame.unwrap_or(rate.frame_at(slot.to)),
            label: u.label.clone().unwrap_or_else(|| slot.label.clone()),
            score: u.score.unwrap_or(slot.score),
        };
        let content = content.checked(ctx, index)?;
        if !content.matches(slot, ctx) {
            *slot = Annotation { origin: slot.origin, ..content.stored(ctx, slot.id) };
        }
    }
    let top = top_id(draft);
    let mut label = None;
    if let Some(r) = &w.replace {
        if !valid_kind(&r.kind) || r.from_frame < 0 || r.from_frame >= r.to_frame || r.to_frame > ctx.r.end_frame {
            let detail = json!({"key": "replace", "reason": "a kind of [a-z0-9_:-] over frames inside [0, end]"});
            return Err(Refusal::new("bad_annotation", "replace names a kind and frames inside the sequence", detail));
        }
        let inside = |a: &Annotation| a.kind == r.kind && rate.frame_at(a.from) >= r.from_frame && rate.frame_at(a.to) <= r.to_frame;
        let (gone, kept): (Vec<Annotation>, Vec<Annotation>) = std::mem::take(&mut draft.set.annotations).into_iter().partition(inside);
        draft.set.annotations = kept;
        // the replaced annotations by content, each list reversed so that `pop` gives back the first id of a content
        let mut pool: BTreeMap<Key, Vec<Annotation>> = BTreeMap::new();
        for a in gone.into_iter().rev() {
            pool.entry(key_of(&a, ctx)).or_default().push(a);
        }
        for (index, item) in r.set.iter().enumerate() {
            let content = Content { kind: r.kind.clone(), from: item.from_frame, to: item.to_frame, label: item.label.clone(), score: item.score };
            let content = content.checked(ctx, index)?;
            if content.from < r.from_frame || content.to > r.to_frame {
                let detail = json!({"index": index, "reason": "outside the replaced range"});
                return Err(Refusal::new("bad_annotation", format!("annotation {index} lies outside the replaced range"), detail));
            }
            match pool.get_mut(&content.key()).and_then(Vec::pop) {
                Some(same) => draft.set.annotations.push(same),
                None => {
                    let id = alloc(&mut draft.set.next_annotation_id, top)?;
                    draft.set.annotations.push(content.stored(ctx, id));
                }
            }
        }
        label = Some(format!("Vertical: annotations {} {}\u{2013}{}", r.kind, ctx.tc(r.from_frame), ctx.tc(r.to_frame)));
    }
    let mut added = 0;
    let mut known: BTreeMap<Key, u32> = BTreeMap::new();
    for a in &draft.set.annotations {
        known.entry(key_of(a, ctx)).or_insert(a.id);
    }
    for (index, a) in w.add.iter().enumerate() {
        let content = Content { kind: a.kind.clone(), from: a.from_frame, to: a.to_frame, label: a.label.clone(), score: a.score };
        let content = content.checked(ctx, index)?;
        let key = content.key();
        if let Some(&id) = known.get(&key) {
            draft.ids.push(id);
            continue;
        }
        let id = alloc(&mut draft.set.next_annotation_id, top)?;
        draft.set.annotations.push(content.stored(ctx, id));
        known.insert(key, id);
        draft.ids.push(id);
        added += 1;
    }
    if draft.set.annotations.len() > MAX_ANNOTATIONS {
        return Err(too_many("annotations", draft.set.annotations.len(), MAX_ANNOTATIONS));
    }
    draft.set.annotations.sort_by_key(|a| (rate.frame_at(a.from), a.id));
    draft.label = label.unwrap_or_else(|| counted("annotations", added, removed));
    Ok(())
}

/// What makes two annotations the same content: kind, frames, label and the score's bits.
type Key = (String, i64, i64, String, Option<u64>);

/// The content key of a stored annotation.
fn key_of(a: &Annotation, ctx: &Ctx) -> Key {
    let rate = ctx.r.rate;
    (a.kind.clone(), rate.frame_at(a.from), rate.frame_at(a.to), a.label.clone(), a.score.map(f64::to_bits))
}

/// The largest annotation id in the draft.
fn top_id(draft: &Draft) -> u32 {
    draft.set.annotations.iter().map(|a| a.id).max().unwrap_or(0)
}
