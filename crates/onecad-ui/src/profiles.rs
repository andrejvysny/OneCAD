//! Worker-owned region fills are projected and picked in their authoritative plane.
use onecad_app::dto::{FinishSketchDto, SketchRegionDto};
use onecad_core::{
    document::refs::{Extra, SketchRegionRef},
    ids::{RegionId, SketchId},
    sketch::SketchPlane,
};
use onecad_scene::{Bounds, DVec2, DVec3, ViewSnapshot};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct Profile {
    pub reference: SketchRegionRef,
    pub vertices: Vec<DVec3>,
    pub uv: Vec<DVec2>,
    pub triangles: Vec<[u32; 3]>,
    pub boundary: Vec<[u32; 2]>,
    pub bounds: Bounds,
    plane: SketchPlane,
    centroid: DVec3,
}

#[derive(Default)]
pub struct ProfileBudget {
    regions: usize,
    vertices: usize,
    indices: usize,
    bytes: usize,
}
impl ProfileBudget {
    pub(crate) fn retain_metadata(&mut self, bytes: usize) -> Result<(), String> {
        crate::budget::charge(
            &mut self.bytes,
            bytes,
            crate::budget::MAX_PROFILE_BYTES,
            "aggregate profile source/decoded/scratch/anchors",
        )
    }
    fn charge(&mut self, result: &FinishSketchDto) -> Result<(), String> {
        use crate::budget::charge;
        charge(
            &mut self.regions,
            result.regions.len(),
            256,
            "profile region count",
        )?;
        for region in &result.regions {
            let fill = region
                .preview_triangles
                .as_ref()
                .ok_or("worker region has no previewTriangles")?;
            charge(
                &mut self.vertices,
                fill.positions.len() / 2,
                100_000,
                "profile vertices",
            )?;
            charge(
                &mut self.indices,
                fill.indices.len(),
                300_000,
                "profile indices",
            )?;
            let metadata = region
                .outer_loop
                .iter()
                .chain(region.holes.iter().flatten())
                .map(|id| id.len() + 32)
                .try_fold(region.region_id.len(), |sum, value| sum.checked_add(value))
                .and_then(|sum| {
                    region
                        .holes
                        .len()
                        .checked_mul(32)
                        .and_then(|bytes| sum.checked_add(bytes))
                })
                .ok_or("profile metadata size overflow")?;
            let estimate = fill
                .positions
                .len()
                .checked_mul(256)
                .and_then(|v| {
                    fill.indices
                        .len()
                        .checked_mul(128)
                        .and_then(|i| v.checked_add(i))
                })
                .and_then(|value| value.checked_add(metadata))
                .ok_or("profile budget arithmetic overflow")?;
            charge(
                &mut self.bytes,
                estimate,
                crate::budget::MAX_PROFILE_BYTES,
                "aggregate profile source/decoded/scratch",
            )?;
        }
        Ok(())
    }
}
#[cfg(test)]
pub fn decode_profiles(
    sketch: SketchId,
    plane: SketchPlane,
    result: FinishSketchDto,
) -> Result<Vec<Profile>, String> {
    decode_profiles_budgeted(sketch, plane, result, &mut ProfileBudget::default())
}
pub fn decode_profiles_budgeted(
    sketch: SketchId,
    plane: SketchPlane,
    result: FinishSketchDto,
    budget: &mut ProfileBudget,
) -> Result<Vec<Profile>, String> {
    budget.charge(&result)?;
    validate_plane(plane)?;
    result
        .regions
        .into_iter()
        .map(|region| decode_profile(sketch, plane, result.region_identity_version, region))
        .collect()
}

fn validate_plane(plane: SketchPlane) -> Result<(), String> {
    if !onecad_interaction::sketch::validate_sketch_plane(plane) {
        return Err("invalid authoritative sketch plane".into());
    }
    Ok(())
}

fn vector(value: onecad_core::math::Vec3) -> DVec3 {
    DVec3::new(value.x, value.y, value.z)
}

fn decode_profile(
    sketch: SketchId,
    plane: SketchPlane,
    version: u32,
    region: SketchRegionDto,
) -> Result<Profile, String> {
    if region.region_id.is_empty() {
        return Err("worker region lacks an identity".into());
    }
    let fill = region
        .preview_triangles
        .ok_or("worker region has no previewTriangles")?;
    if fill.holes_subtracted as usize != region.holes.len()
        || fill.positions.len() % 2 != 0
        || fill.indices.len() % 3 != 0
        || fill.positions.len() > 200_000
        || fill.indices.len() > 300_000
    {
        return Err("worker region fill is incomplete or exceeds QA limits".into());
    }
    let uv: Vec<_> = fill
        .positions
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| DVec2::new(p[0], p[1]))
        .collect();
    if uv.is_empty() || uv.iter().any(|p| !p.is_finite()) {
        return Err("invalid worker profile positions".into());
    }
    let triangles = fill.indices.as_chunks::<3>().0.to_vec();
    validate_triangles(&uv, &triangles)?;
    let boundary = boundary_edges(&triangles)?;
    let (origin, x, y) = (
        vector(plane.origin),
        vector(plane.x_axis),
        vector(plane.y_axis),
    );
    let vertices: Vec<_> = uv.iter().map(|p| origin + x * p.x + y * p.y).collect();
    let min = vertices
        .iter()
        .fold(DVec3::splat(f64::INFINITY), |a, p| a.min(*p));
    let max = vertices
        .iter()
        .fold(DVec3::splat(f64::NEG_INFINITY), |a, p| a.max(*p));
    let bounds = Bounds::new(min, max).map_err(|e| e.to_string())?;
    let centroid = area_centroid(&uv, &triangles, plane, &bounds);
    Ok(Profile {
        reference: SketchRegionRef {
            sketch,
            region: RegionId::new(region.region_id),
            region_identity_version: Some(version),
            region_anchor: None,
            extra: Extra::new(),
        },
        vertices,
        uv,
        triangles,
        boundary,
        bounds,
        plane,
        centroid,
    })
}

/// Area centroid of the filled region (holes excluded by the worker fill); bounds center
/// when the fill has no usable area.
fn area_centroid(
    uv: &[DVec2],
    triangles: &[[u32; 3]],
    plane: SketchPlane,
    bounds: &Bounds,
) -> DVec3 {
    let (mut area, mut moment) = (0.0, DVec2::ZERO);
    for triangle in triangles {
        let [a, b, c] = triangle.map(|i| uv[i as usize]);
        let weight = (b - a).perp_dot(c - a) * 0.5;
        area += weight;
        moment += (a + b + c) / 3.0 * weight;
    }
    if !area.is_finite() || area.abs() < 1e-12 {
        return (bounds.min + bounds.max) * 0.5;
    }
    let uv = moment / area;
    vector(plane.origin) + vector(plane.x_axis) * uv.x + vector(plane.y_axis) * uv.y
}

fn validate_triangles(uv: &[DVec2], triangles: &[[u32; 3]]) -> Result<(), String> {
    if triangles.is_empty() || triangles.iter().flatten().any(|i| *i as usize >= uv.len()) {
        return Err("invalid worker profile indices".into());
    }
    for triangle in triangles {
        let [a, b, c] = triangle.map(|i| uv[i as usize]);
        if (b - a).perp_dot(c - a).abs() <= 1e-12 {
            return Err("degenerate worker profile triangle".into());
        }
    }
    Ok(())
}
fn boundary_edges(triangles: &[[u32; 3]]) -> Result<Vec<[u32; 2]>, String> {
    let mut counts = BTreeMap::<[u32; 2], usize>::new();
    for [a, b, c] in triangles {
        for mut edge in [[*a, *b], [*b, *c], [*c, *a]] {
            edge.sort();
            *counts.entry(edge).or_default() += 1;
        }
    }
    if counts.values().any(|count| *count > 2) {
        return Err("nonmanifold worker profile fill".into());
    }
    Ok(counts
        .into_iter()
        .filter_map(|(edge, count)| (count == 1).then_some(edge))
        .collect())
}
impl Profile {
    pub(crate) fn plane(&self) -> SketchPlane {
        self.plane
    }
    pub(crate) fn centroid(&self) -> DVec3 {
        self.centroid
    }
    pub fn pick(&self, view: &ViewSnapshot, logical: DVec2) -> Option<([f64; 2], f64)> {
        let ray = view.ray(logical).ok()?;
        let point = onecad_interaction::sketch::intersect_sketch_plane(self.plane, ray)?;
        view.project(point)?;
        let local = onecad_interaction::sketch::sketch_plane_coordinates(self.plane, point)?;
        let uv = DVec2::new(local.x, local.y);
        let distance = point.distance(ray.origin);
        if self
            .boundary
            .iter()
            .any(|edge| near_segment(uv, self.uv[edge[0] as usize], self.uv[edge[1] as usize]))
        {
            return None;
        }
        self.triangles
            .iter()
            .any(|t| {
                inside_triangle(
                    uv,
                    self.uv[t[0] as usize],
                    self.uv[t[1] as usize],
                    self.uv[t[2] as usize],
                )
            })
            .then_some((uv.to_array(), distance))
    }
}
fn near_segment(p: DVec2, a: DVec2, b: DVec2) -> bool {
    let direction = b - a;
    let parameter = ((p - a).dot(direction) / direction.length_squared()).clamp(0.0, 1.0);
    p.distance_squared(a + parameter * direction) < 1e-14
}
fn inside_triangle(p: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    let determinant = (b - a).perp_dot(c - a);
    let u = (p - a).perp_dot(c - a) / determinant;
    let v = (b - a).perp_dot(p - a) / determinant;
    u >= -1e-10 && v >= -1e-10 && u + v <= 1.0 + 1e-10
}
#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
