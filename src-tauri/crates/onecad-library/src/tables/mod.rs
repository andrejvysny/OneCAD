//! Parametric dimension tables (spec §6) — the authoring/metadata mirror of
//! the worker's own generator tables.
//!
//! **This side is never a geometry authority.** Spec §6 puts the generators
//! (and therefore the dimensions they build from) in the worker;
//! `worker/src/ops/FastenerTables.cpp` is that copy. These tables exist for
//! what the worker never sees: `component.toml`'s `[parameters] role="table"`
//! resolution, designation strings, and the `thread` domains a seed package
//! offers. Both copies were transcribed independently from the same sources,
//! and each family's tests pin the geometry-relevant columns against the
//! worker's values — so a drift in either copy fails loud rather than shipping
//! two different M6 screws.
//!
//! Provenance is recorded per family, at the family. BOLTS-sourced tables are
//! credited in the repo-root `THIRD_PARTY_NOTICES`; ISO 7380-1 and ISO 7093-1
//! are NOT in that dataset and say so where they are defined.

mod fasteners;
mod iso4762;

pub use fasteners::{
    HexScrewRow, Iso4014Table, Iso4017Table, Iso4032Table, Iso7089Table, Iso7093Table,
    Iso7380Table, NutRow, ScrewRow, WasherRow,
};
pub use iso4762::{Iso4762Row, Iso4762Table};
