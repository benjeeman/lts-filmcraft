//! The model and math tests of milestone M1 (spec section 11): T-M1 to T-M6, T-M8a, T-M12, T-M23, T-M24 (the round-trip
//! half) and T-M35, one file per topic. Synthetic data only; the property tests use a seeded generator with fixed case
//! counts, so every run checks the same cases.
//!
//! Milestone M2 (lane L1b) adds the planner, walls, parts, regions, annotations, the active area, the measures, the
//! derived values, the digests and validation: T-M7, T-M8c, T-M9, T-M10, T-M16 to T-M24 (the planner and bounds halves),
//! T-M26 (the centre-crop baseline), T-M27 to T-M29, T-M31, T-M32, T-M34 and T-M35 (the range and effective-walls
//! digests), with the fixtures of `fixtures.rs`.
//!
//! Its second stage (lane L1c) adds the edit hook and remap: T-M11, T-M14 (model half), T-M15, T-M30 (the person's
//! clauses), T-M32 (mapping half) and T-M33, with the before and after projects and oracle maps of `hook_fixtures.rs`;
//! hidden picture, transitions at every level, a second track, two plans, the nest depth, extended heads with walls on
//! them, and data that is not ordinary in `follow_layers.rs`, `follow_cases.rs` and `follow_robust.rs`.

mod active_area;
mod annotation_writes;
mod attention;
mod bitwise;
mod canonical;
mod containment;
mod curves;
mod custom_curves;
mod derived;
mod digests;
mod edge_rules;
mod fit_move;
mod fixtures;
mod follow_cases;
mod follow_data;
mod follow_fast_path;
mod follow_frame;
mod follow_gestures;
mod follow_hook;
mod follow_layers;
mod follow_robust;
mod framing_library;
mod hook_fixtures;
mod measures;
mod output_scope;
mod output_size;
mod part_rules;
mod phases;
mod planner;
mod plans;
mod ramp;
mod range_digests;
mod refusals;
mod region_checks;
mod remap;
mod remeasure;
mod seeding;
mod shapes;
mod smooth;
mod smooth_planner;
mod splits;
mod support;
mod validation;
mod wall_rules;
