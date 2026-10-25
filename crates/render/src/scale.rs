//! Preview scales (Vertical Edit, spec §6.1): previews render a sequence at one of a few fixed scales, so a frame cached
//! at one size serves every nearby size while a panel is resized. The Program monitor's buckets
//! (`crates/ui-egui/src/panels/monitor.rs`) live here so that the vertical frame, which `crates/render` makes, and the
//! monitors, which call this function, can never disagree.

/// The preview scales, smallest first.
pub const SCALE_BUCKETS: [f32; 9] = [1.0 / 32.0, 1.0 / 16.0, 1.0 / 8.0, 0.1875, 0.25, 0.375, 0.5, 0.75, 1.0];

/// The scale to render at for a display that needs `s`: the smallest bucket of [`SCALE_BUCKETS`] at or above `s`, with a
/// tolerance of 1e-4 (a bucket at most 1e-4 below `s` counts), else 1. A NaN gives 1; zero, a negative value and minus
/// infinity give the smallest bucket.
pub fn quantize_scale(s: f32) -> f32 {
    SCALE_BUCKETS.iter().copied().find(|b| *b >= s - 1e-4).unwrap_or(1.0)
}
