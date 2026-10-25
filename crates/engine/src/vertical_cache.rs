//! The 16:9 frame cache of the Vertical Edit (spec §6.3): the full-resolution, captionless 16:9 frames that
//! `filmcraft_render::render_vertical` resamples, keyed by [`FrameKey16x9`] (the project epoch, the sequence and frame,
//! the picture revision, the scale, the working space and the media pool generation), the [`CAPACITY`] most recently used
//! ones, behind the session's `Arc`. A plan-only edit keeps the picture revision, so the next vertical frame is a hit and
//! only the resample runs again (T-N19). [`Session::note_project_replaced`](crate::Session::note_project_replaced) clears
//! it.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use filmcraft_render::{FrameCache16x9, FrameKey16x9, Image};

/// How many 16:9 frames the session's cache holds (a 1920×1080 RGBA f32 frame is 33 MB).
pub const CAPACITY: usize = 4;

/// The frames held, the most recently used last.
type Frames = VecDeque<(FrameKey16x9, Arc<Image>)>;

/// The 16:9 frames the vertical renders reuse (spec §6.3), shared behind an `Arc` by the session and its frame jobs. It
/// counts its hits, misses and stores, so "a nudge causes zero 16:9 renders" can be checked: every 16:9 frame a vertical
/// frame needed and did not find is rendered and stored.
pub struct VerticalFrameCache {
    capacity: usize,
    frames: Mutex<Frames>,
    hits: AtomicU64,
    misses: AtomicU64,
    puts: AtomicU64,
    clears: AtomicU64,
}

impl Default for VerticalFrameCache {
    fn default() -> Self {
        Self::with_capacity(CAPACITY)
    }
}

impl fmt::Debug for VerticalFrameCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerticalFrameCache")
            .field("capacity", &self.capacity)
            .field("len", &self.len())
            .field("hits", &self.hits())
            .field("misses", &self.misses())
            .field("puts", &self.puts())
            .field("clears", &self.clears())
            .finish()
    }
}

impl VerticalFrameCache {
    /// A cache of the `capacity` most recently used frames (at least one).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            frames: Mutex::new(VecDeque::new()),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            puts: AtomicU64::new(0),
            clears: AtomicU64::new(0),
        }
    }

    fn frames(&self) -> MutexGuard<'_, Frames> {
        self.frames.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many frames it holds at most.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many frames it holds now.
    pub fn len(&self) -> usize {
        self.frames().len()
    }

    /// Whether it holds no frame.
    pub fn is_empty(&self) -> bool {
        self.frames().is_empty()
    }

    /// Whether it holds the frame of `key` (counts nothing, changes no recency).
    pub fn contains(&self, key: &FrameKey16x9) -> bool {
        self.frames().iter().any(|(held, _)| held == key)
    }

    /// How many lookups found their frame.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// How many lookups found nothing.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// How many frames were stored: the 16:9 renders a vertical frame needed.
    pub fn puts(&self) -> u64 {
        self.puts.load(Ordering::Relaxed)
    }

    /// Forget every frame: the session's project was replaced ([`crate::Session::note_project_replaced`]).
    pub fn clear(&self) {
        self.frames().clear();
        self.clears.fetch_add(1, Ordering::Relaxed);
    }

    /// How many times [`VerticalFrameCache::clear`] has run.
    pub fn clears(&self) -> u64 {
        self.clears.load(Ordering::Relaxed)
    }
}

impl FrameCache16x9 for VerticalFrameCache {
    /// The frame of `key`, now the most recently used one.
    fn get(&self, key: &FrameKey16x9) -> Option<Arc<Image>> {
        let mut frames = self.frames();
        let Some(at) = frames.iter().position(|(held, _)| held == key) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let entry = frames.remove(at)?;
        let frame = Arc::clone(&entry.1);
        frames.push_back(entry);
        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(frame)
    }

    /// Store the frame of `key` (replacing the one held for it) as the most recently used one, dropping the least
    /// recently used past the capacity. A frame rendered while media data was still loading (`filmcraft_media::pending`)
    /// shows a slate where the picture belongs: it is never kept.
    fn put(&self, key: FrameKey16x9, frame: Arc<Image>) {
        if filmcraft_media::pending::is_set() {
            return;
        }
        let mut frames = self.frames();
        frames.retain(|(held, _)| *held != key);
        frames.push_back((key, frame));
        while frames.len() > self.capacity {
            frames.pop_front();
        }
        self.puts.fetch_add(1, Ordering::Relaxed);
    }
}
