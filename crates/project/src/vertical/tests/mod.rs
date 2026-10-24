//! The model and math tests of milestone M1 (spec section 11): T-M1 to T-M6, T-M8a, T-M12, T-M23, T-M24 (the round-trip
//! half) and T-M35, one file per topic. Synthetic data only; the property tests use a seeded generator with fixed case
//! counts, so every run checks the same cases.

mod bitwise;
mod canonical;
mod containment;
mod curves;
mod custom_curves;
mod digests;
mod phases;
mod ramp;
mod shapes;
mod smooth;
mod splits;
mod support;
