//! Fixture loading: every `*.mesh1` in a fixture directory becomes one body.

use crate::mesh1::{self, Mesh1};
use std::path::{Path, PathBuf};

pub struct Body {
    pub label: String,
    pub file: String,
    pub mesh: Mesh1,
    pub visible: bool,
}

pub struct Scene {
    pub bodies: Vec<Body>,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub bytes: usize,
}

impl Scene {
    pub fn triangle_count(&self) -> u64 {
        self.bodies
            .iter()
            .map(|b| u64::from(b.mesh.triangle_count))
            .sum()
    }
    pub fn segment_count(&self) -> usize {
        self.bodies.iter().map(|b| b.mesh.edge_segments.len()).sum()
    }
    pub fn center(&self) -> [f32; 3] {
        std::array::from_fn(|i| 0.5 * (self.bbox_min[i] + self.bbox_max[i]))
    }
    pub fn radius(&self) -> f32 {
        let d: [f32; 3] = std::array::from_fn(|i| self.bbox_max[i] - self.bbox_min[i]);
        0.5 * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
}

/// The fixture directory: `SPIKE_FIXTURE`, else the repo's `spikes/fixtures/nema17`.
pub fn fixture_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SPIKE_FIXTURE") {
        return PathBuf::from(p);
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/nema17"))
}

pub fn load(dir: &Path) -> Result<Scene, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "mesh1"))
        .collect();
    files.sort();
    let mut bodies = Vec::with_capacity(files.len());
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut bytes = 0;
    for (i, path) in files.iter().enumerate() {
        let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        bytes += raw.len();
        let mesh = mesh1::parse(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        for k in 0..3 {
            lo[k] = lo[k].min(mesh.bbox_min[k]);
            hi[k] = hi[k].max(mesh.bbox_max[k]);
        }
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        bodies.push(Body {
            label: format!("Body {}", i + 1),
            file,
            mesh,
            visible: true,
        });
    }
    if bodies.is_empty() {
        return Err(format!("no .mesh1 files in {}", dir.display()));
    }
    Ok(Scene {
        bodies,
        bbox_min: lo,
        bbox_max: hi,
        bytes,
    })
}
