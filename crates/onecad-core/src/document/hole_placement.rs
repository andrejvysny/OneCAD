//! Ordered multi-point `Hole` placement (SCHEMA §7.3 `Hole.points`, ONECAD-361).
//!
//! A legacy/V2 record drills ONE hole at `point`. A `resultPolicyVersion: 3`
//! record carries an ordered `points` list instead; every entry has a
//! Rust-minted [`HolePointId`] that is never reused, so a downstream reference
//! to one hole's faces or edges keeps naming THAT hole across point edits (the
//! worker attributes each born element to its point id and filters ladder
//! candidates by it). `point` stays equal to `points[0].at` for old readers.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::document::record::{HoleParams, HoleType, HOLE_RESULT_POLICY_VERSION};
use crate::document::refs::Extra;
use crate::math::Vec3;

/// `resultPolicyVersion` that enables `points` (and keeps V2 single-solid
/// publication).
pub const HOLE_POINTS_POLICY_VERSION: u8 = 3;
/// Upper bound on placements in one Hole record.
pub const HOLE_MAX_POINTS: usize = 64;
/// The worker's coincidence/spacing gap on PROJECTED origins (SCHEMA §7.3).
pub const HOLE_WORKER_GAP_MM: f64 = 1e-3;
/// Authoring gap on 3D distance between frozen points. Larger than the worker's
/// because the worker measures after projecting each point (up to 1e-3 mm off
/// the face plane) onto the face: with 3e-3 a pair Rust accepts can never fall
/// under the worker's in-plane bound.
pub const HOLE_AUTHORING_GAP_MM: f64 = 3e-3;
/// `HoleTool.h kFaceOvershoot`: a countersink cone starts this far outside the
/// face, so its widest radius is `cs/2 + overshoot·tan(csAngle/2)`.
pub const HOLE_FACE_OVERSHOOT_MM: f64 = 1e-2;

/// Stable identity of one placement inside one Hole record. Minted by Rust
/// (UUID v4) when a point is added and never reassigned to another point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HolePointId(pub Uuid);

#[allow(clippy::new_without_default)]
impl HolePointId {
    /// Mints a fresh, globally unique point id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for HolePointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One persisted V3 placement: `{ "id": "<uuid>", "at": [x, y, z] }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HolePoint {
    pub id: HolePointId,
    /// World-space hole centre, frozen at authoring (same contract as `point`).
    pub at: Vec3,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Writer input for [`HoleParams::set_placements`]: an existing point keeps its
/// `id`; `None` asks for a freshly minted one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HolePlacement {
    pub id: Option<HolePointId>,
    pub at: Vec3,
}

impl HoleParams {
    /// The ordered placements; hole index `k` is entry `k`. A single-point
    /// legacy/V2 record yields `[point]` with no id.
    #[must_use]
    pub fn placements(&self) -> Vec<HolePlacement> {
        match &self.points {
            Some(points) => points
                .iter()
                .map(|p| HolePlacement {
                    id: Some(p.id),
                    at: p.at,
                })
                .collect(),
            None => vec![HolePlacement {
                id: None,
                at: self.point,
            }],
        }
    }

    /// Canonical writer for the placement list. A record without ids and one
    /// placement stays in the legacy shape (version untouched). Two or more
    /// placements, or any count on a record that already carries ids, writes
    /// V3: `points` with ids, `point = points[0].at`, `resultPolicyVersion: 3`.
    /// Keeping a 1-point V3 record at V3 preserves its id, so refs to the last
    /// remaining hole survive removing its siblings.
    ///
    /// Upgrading a legacy record also switches it to single-solid publication:
    /// a hole set that would split the host is then refused
    /// (`HOLE_DISJOINT_RESULT`) instead of keeping the legacy residual.
    ///
    /// # Errors
    /// An empty or over-long list, an id that is not a current placement of
    /// this record, or a duplicated id. Spacing is checked by
    /// [`HoleParams::validate`].
    pub fn set_placements(&mut self, placements: Vec<HolePlacement>) -> Result<(), String> {
        if placements.is_empty() || placements.len() > HOLE_MAX_POINTS {
            return Err(format!(
                "Hole needs 1..={HOLE_MAX_POINTS} points (got {})",
                placements.len()
            ));
        }
        let current: Vec<&HolePoint> = self.points.iter().flatten().collect();
        let mut seen = BTreeSet::new();
        for id in placements.iter().filter_map(|p| p.id) {
            if !current.iter().any(|p| p.id == id) {
                return Err(format!(
                    "Hole point id {id} is not a placement of this hole"
                ));
            }
            if !seen.insert(id) {
                return Err(format!("Hole point id {id} is listed twice"));
            }
        }
        if placements.len() == 1 && current.is_empty() {
            self.point = placements[0].at;
            return Ok(());
        }
        let points: Vec<HolePoint> = placements
            .into_iter()
            .map(|p| {
                let kept = p.id.and_then(|id| current.iter().find(|c| c.id == id));
                HolePoint {
                    id: p.id.unwrap_or_else(HolePointId::new),
                    at: p.at,
                    extra: kept.map(|c| c.extra.clone()).unwrap_or_default(),
                }
            })
            .collect();
        self.point = points[0].at;
        self.points = Some(points);
        self.result_policy_version = Some(HOLE_POINTS_POLICY_VERSION);
        Ok(())
    }

    /// The `(points, resultPolicyVersion)` fence plus the per-point shape rules.
    pub(crate) fn validate_placement_policy(&self) -> Result<(), String> {
        let version = self.result_policy_version;
        let Some(points) = &self.points else {
            return match version {
                None | Some(HOLE_RESULT_POLICY_VERSION) => Ok(()),
                Some(HOLE_POINTS_POLICY_VERSION) => {
                    Err("Hole resultPolicyVersion 3 requires points".into())
                }
                Some(v) => Err(format!(
                    "Hole resultPolicyVersion {v} is unsupported (expected {HOLE_RESULT_POLICY_VERSION} or {HOLE_POINTS_POLICY_VERSION})"
                )),
            };
        };
        if version != Some(HOLE_POINTS_POLICY_VERSION) {
            return Err("Hole points require resultPolicyVersion 3".into());
        }
        if points.is_empty() || points.len() > HOLE_MAX_POINTS {
            return Err(format!(
                "Hole needs 1..={HOLE_MAX_POINTS} points (got {})",
                points.len()
            ));
        }
        let mut seen = BTreeSet::new();
        for (k, p) in points.iter().enumerate() {
            if !p.at.is_finite() {
                return Err(format!("Hole points[{k}] has a non-finite component"));
            }
            if !seen.insert(p.id) {
                return Err(format!("Hole points[{k}] repeats id {}", p.id));
            }
        }
        if points[0].at != self.point {
            return Err("Hole point must equal points[0]".into());
        }
        Ok(())
    }

    /// Pairwise spacing: tools must be disjoint so one compound cut drills
    /// every hole and each born element belongs to exactly one point.
    pub(crate) fn validate_spacing(&self) -> Result<(), String> {
        let Some(points) = &self.points else {
            return Ok(());
        };
        let min = 2.0 * self.seat_radius() + HOLE_AUTHORING_GAP_MM;
        for i in 0..points.len() {
            for j in i + 1..points.len() {
                let d = distance(points[i].at, points[j].at);
                if d < HOLE_WORKER_GAP_MM {
                    return Err(format!("Hole points[{i}] and points[{j}] coincide"));
                }
                if d < min {
                    return Err(format!(
                        "Hole points[{i}] and points[{j}] overlap (distance {d:.3} < {min:.3})"
                    ));
                }
            }
        }
        Ok(())
    }

    /// The widest radius of one hole's cutting tool (SCHEMA §7.3): the drill,
    /// the counterbore, or the countersink cone at its overshoot start.
    #[must_use]
    pub fn seat_radius(&self) -> f64 {
        let drill = self.diameter.value / 2.0;
        match self.hole_type {
            HoleType::Simple => drill,
            HoleType::Counterbore => {
                let cb = self.cb_diameter.as_ref().map_or(0.0, |s| s.value / 2.0);
                drill.max(cb)
            }
            HoleType::Countersink => {
                let cs = self.cs_diameter.as_ref().map_or(0.0, |s| s.value / 2.0);
                let half = self
                    .cs_angle_deg
                    .as_ref()
                    .map_or(0.0, |s| s.value.to_radians() / 2.0);
                drill.max(cs + HOLE_FACE_OVERSHOOT_MM * half.tan())
            }
        }
    }
}

fn distance(a: Vec3, b: Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

#[cfg(test)]
#[path = "hole_placement_tests.rs"]
mod tests;
