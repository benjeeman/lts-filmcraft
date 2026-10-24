//! Vertical Edit: the framing of a 16:9 sequence for a vertical (or any other) output raster.
//!
//! A [`VerticalSet`] lives on [`Sequence::vertical`](crate::Sequence::vertical). Each of its plans holds positions: a window
//! `(cx, cy, h)` at an exact frame, reached by a cut or by a move along a curve. One function, [`window_at`], gives the
//! exact window at every tick, and every preview, read and export uses it. The 16:9 edit is never changed.
//!
//! Everything here is pure data and arithmetic (no session, UI or render state) and builds for wasm32. The rules are those
//! of the Vertical Edit specification v1.1 (final), sections 2 and 3.

pub mod canon;
pub mod curve;
pub mod digest;
pub mod eval;
pub mod path;
pub mod smooth;
pub mod split;
pub mod types;

pub use canon::{CentreBounds, ceil_g, centre_bounds, clamp_window, floor_g, floor_micro, h_from_zoom, on_grid, q, quantize_window, zoom_percent};
pub use curve::{DEFAULT_CURVE, MAX_CURVE_SLOPE, SlopePeak, check_curve, ease};
pub use digest::{data_digest, plan_digest, walls_digest};
pub use eval::{MoveSpan, centred_largest, limits, move_span, window_at};
pub use path::path;
pub use smooth::{screen_speed, smooth_through};
pub use split::{Split, insert_split, move_label, split_move};
pub use types::*;

#[cfg(test)]
mod tests;
