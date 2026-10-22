//! Synthetic section lines. They carry no body, face, or edge identifiers.
use crate::{
    limits::{add, mul},
    PreparationBudget, PreviewIdentity, RenderError,
};
use onecad_scene::{SceneIdentity, SceneSnapshot};

pub(crate) use crate::section_contours_gpu::GpuSectionContours;
type OwnedContours = (Vec<[f32; 6]>, Vec<[u64; 4]>);
type OwnedSegment = ([f32; 6], [u64; 4]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionContourKey {
    pub scene: SceneIdentity,
    pub preview: Option<PreviewIdentity>,
    pub section: [u32; 4],
}

pub struct PreparedSectionContours {
    pub(super) key: SectionContourKey,
    segments: Vec<[f32; 6]>,
    pub(super) owners: Vec<[u64; 4]>,
}

#[derive(Clone, Copy)]
pub(super) struct Candidate {
    segment: [f32; 6],
    coplanar: bool,
    body: u32,
}

fn compare(a: &[f32; 6], b: &[f32; 6]) -> std::cmp::Ordering {
    a.iter()
        .zip(b)
        .map(|(a, b)| a.total_cmp(b))
        .find(|c| !c.is_eq())
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn segment(a: [f32; 3], b: [f32; 3]) -> Option<[f32; 6]> {
    let a = a.map(|v| if v == 0.0 { 0.0 } else { v });
    let b = b.map(|v| if v == 0.0 { 0.0 } else { v });
    if a == b {
        return None;
    }
    let mut result = [a[0], a[1], a[2], b[0], b[1], b[2]];
    let reversed = [b[0], b[1], b[2], a[0], a[1], a[2]];
    if compare(&reversed, &result).is_lt() {
        result = reversed;
    }
    Some(result)
}

// Canonical endpoint ordering gives identical rounded intersections on shared edges.
fn intersection(mut a: [f32; 3], mut b: [f32; 3], plane: [f32; 4]) -> [f32; 3] {
    if a.map(f32::to_bits) > b.map(f32::to_bits) {
        std::mem::swap(&mut a, &mut b);
    }
    let da = distance(a, plane);
    let db = distance(b, plane);
    let t = da / (da - db);
    std::array::from_fn(|i| (f64::from(a[i]) + t * (f64::from(b[i]) - f64::from(a[i]))) as f32)
}
fn distance(p: [f32; 3], plane: [f32; 4]) -> f64 {
    p.iter()
        .zip(plane)
        .map(|(v, n)| f64::from(*v) * f64::from(n))
        .sum::<f64>()
        - f64::from(plane[3])
}

fn triangle(points: [[f32; 3]; 3], plane: [f32; 4], mut emit: impl FnMut(Candidate)) {
    let [a, b, c] = points.map(|p| glam::Vec3::from_array(p).as_dvec3());
    if (b - a).cross(c - a).length_squared() == 0.0 {
        return;
    }
    let d = points.map(|p| distance(p, plane));
    if d.iter().all(|d| *d == 0.0) {
        for i in 0..3 {
            if let Some(segment) = segment(points[i], points[(i + 1) % 3]) {
                emit(Candidate {
                    segment,
                    coplanar: true,
                    body: 0,
                });
            }
        }
        return;
    }
    let mut hits = [[0.0; 3]; 3];
    let mut count = 0;
    for i in 0..3 {
        let j = (i + 1) % 3;
        let hit = if d[i] == 0.0 {
            Some(points[i])
        } else if (d[i] < 0.0 && d[j] > 0.0) || (d[i] > 0.0 && d[j] < 0.0) {
            Some(intersection(points[i], points[j], plane))
        } else {
            None
        };
        if let Some(p) = hit {
            if !hits[..count].contains(&p) {
                hits[count] = p;
                count += 1;
            }
        }
    }
    if count == 2 {
        if let Some(segment) = segment(hits[0], hits[1]) {
            emit(Candidate {
                segment,
                coplanar: false,
                body: 0,
            });
        }
    }
}

fn visit(
    scene: &SceneSnapshot,
    plane: [f32; 4],
    current: &impl Fn() -> bool,
    mut emit: impl FnMut(Candidate),
) -> Result<(), RenderError> {
    for (ordinal, body) in scene.bodies().iter().enumerate() {
        let mesh = body.mesh()?;
        for i in 0..mesh.header().triangle_count {
            if i % 256 == 0 && !current() {
                return Err(RenderError::CaptureMismatch);
            }
            let indices = mesh
                .triangle(i as usize)
                .ok_or(RenderError::Mesh("section triangle"))?;
            let mut points = [[0.0; 3]; 3];
            for (point, index) in points.iter_mut().zip(indices) {
                *point = mesh
                    .position(index as usize)
                    .ok_or(RenderError::Mesh("section position"))?;
            }
            triangle(points, plane, |mut item| {
                item.body = ordinal as u32;
                emit(item)
            });
        }
    }
    Ok(())
}

impl PreparedSectionContours {
    pub fn new(
        scene: &SceneSnapshot,
        budget: PreparationBudget,
        section: [f32; 4],
    ) -> Result<Self, RenderError> {
        Self::prepare(scene, budget, section, &|| true)
    }
    pub(super) fn prepare(
        scene: &SceneSnapshot,
        budget: PreparationBudget,
        section: [f32; 4],
        current: &impl Fn() -> bool,
    ) -> Result<Self, RenderError> {
        if !section.iter().all(|v| v.is_finite()) {
            return Err(RenderError::Mesh("invalid section plane"));
        }
        if scene.bodies().len() > 255 && section[..3].iter().any(|v| *v != 0.0) {
            return Err(RenderError::Budget("section winding body count"));
        }
        let key = SectionContourKey {
            scene: scene.identity(),
            preview: None,
            section: section.map(f32::to_bits),
        };
        let mut count = 0u64;
        if section[..3].iter().any(|v| *v != 0.0) {
            visit(scene, section, current, |_| count += 1)?;
        }
        let bytes = add(
            mul(count, (std::mem::size_of::<Candidate>() + 112) as u64)?,
            std::mem::size_of::<Self>() as u64,
        )?;
        check_budget(budget, bytes, mul(count, 24)?)?;
        let count = usize::try_from(count).map_err(|_| RenderError::Budget("section count"))?;
        let mut candidates = Vec::new();
        candidates
            .try_reserve_exact(count)
            .map_err(|_| RenderError::Budget("section scratch"))?;
        if count != 0 {
            visit(scene, section, current, |item| candidates.push(item))?;
        }
        let (segments, owners) = resolve_owned(candidates)?;
        Ok(Self {
            key,
            segments,
            owners,
        })
    }
    pub fn with_preview(mut self, preview: PreviewIdentity) -> Result<Self, RenderError> {
        if preview.base != self.key.scene {
            return Err(RenderError::CaptureMismatch);
        }
        self.key.preview = Some(preview);
        Ok(self)
    }
    pub fn key(&self) -> SectionContourKey {
        self.key
    }
    pub fn identity(&self) -> SceneIdentity {
        self.key.scene
    }
    pub fn section(&self) -> [f32; 4] {
        self.key.section.map(f32::from_bits)
    }
    pub fn segments(&self) -> &[[f32; 6]] {
        &self.segments
    }
    pub fn cpu_bytes(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
            + self.segments.capacity() as u64 * 24
            + self.owners.capacity() as u64 * 32
    }
    pub(super) fn gpu_metadata_bytes(&self) -> u64 {
        std::mem::size_of::<GpuSectionContours>() as u64 + self.owners.capacity() as u64 * 32
    }
}
#[cfg(test)]
fn resolve(candidates: Vec<Candidate>) -> Result<Vec<[f32; 6]>, RenderError> {
    resolve_owned(candidates).map(|(segments, _)| segments)
}
fn resolve_owned(mut candidates: Vec<Candidate>) -> Result<OwnedContours, RenderError> {
    let count = candidates.len();
    candidates.sort_unstable_by(|a, b| {
        a.body
            .cmp(&b.body)
            .then_with(|| compare(&a.segment, &b.segment))
    });
    let mut resolved = Vec::new();
    resolved
        .try_reserve_exact(count)
        .map_err(|_| RenderError::Budget("section contours"))?;
    let mut first = 0;
    while first < candidates.len() {
        let mut end = first + 1;
        while end < candidates.len()
            && candidates[end].body == candidates[first].body
            && candidates[end].segment == candidates[first].segment
        {
            end += 1;
        }
        let coplanar = candidates[first..end].iter().filter(|c| c.coplanar).count();
        if coplanar < 2 || candidates[first..end].iter().any(|c| !c.coplanar) {
            let mut owners = [0u64; 4];
            let body = candidates[first].body as usize;
            if body >= 256 {
                return Err(RenderError::Budget("section winding body count"));
            }
            owners[body / 64] = 1 << (body % 64);
            resolved.push((candidates[first].segment, owners));
        }
        first = end;
    }
    merge_owners(resolved)
}
fn merge_owners(mut resolved: Vec<OwnedSegment>) -> Result<OwnedContours, RenderError> {
    resolved.sort_unstable_by(|a, b| compare(&a.0, &b.0));
    let mut segments = Vec::new();
    let mut owners: Vec<[u64; 4]> = Vec::new();
    segments
        .try_reserve_exact(resolved.len())
        .map_err(|_| RenderError::Budget("section contours"))?;
    owners
        .try_reserve_exact(resolved.len())
        .map_err(|_| RenderError::Budget("section contour ownership"))?;
    for (segment, mask) in resolved {
        if segments.last() == Some(&segment) {
            if let Some(previous) = owners.last_mut() {
                for (word, incoming) in previous.iter_mut().zip(mask) {
                    *word |= incoming;
                }
            }
        } else {
            segments.push(segment);
            owners.push(mask);
        }
    }
    Ok((segments, owners))
}
pub(super) fn check_budget(
    budget: PreparationBudget,
    cpu: u64,
    gpu: u64,
) -> Result<(), RenderError> {
    if add(cpu, budget.resident.cpu_prepared_bytes)? > budget.limits.max_cpu_prepared_bytes {
        return Err(RenderError::Budget("section CPU preparation"));
    }
    if gpu.max(64) > budget.max_buffer || gpu > u64::from(u32::MAX) * 24 {
        return Err(RenderError::Budget("section buffer"));
    }
    budget.limits.check(
        crate::MemoryEstimate {
            cpu_prepared_bytes: add(cpu, budget.resident.cpu_prepared_bytes)?,
            geometry_bytes: add(gpu.max(64), budget.resident.geometry_bytes)?,
            upload_bytes: gpu,
            ..budget.resident
        },
        crate::MemoryEstimate::default(),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "section_contours_tests.rs"]
mod tests;
