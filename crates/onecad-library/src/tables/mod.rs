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
//! credited in the repo-root `THIRD_PARTY_NOTICES`; a family that dataset does
//! not carry says so where it is defined.
//!
//! One mirror per seeded family (ISO 4762, 4032, 7089). A mirror of a table
//! nothing generates from pins nothing, so a family's table is removed with
//! its worker generator.

mod iso4032;
mod iso4762;
mod iso7089;

pub use iso4032::{Iso4032Row, Iso4032Table};
pub use iso4762::{Iso4762Row, Iso4762Table};
pub use iso7089::{Iso7089Row, Iso7089Table};
