//! Conservative CPU payload reservations include current, pending and building handoffs.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
pub const DISPLAY_RESERVATION: usize = 160 << 20;
pub const PREVIEW_RESERVATION: usize = 192 << 20;
pub const MAX_SOURCE_BYTES: usize = 32 << 20;
pub const MAX_PREPARED_BYTES: u64 = 64 << 20;
pub const MAX_PROFILE_BYTES: usize = 16 << 20;
pub const MAX_BODIES: usize = 256;
// Authored records (16 MiB), projection metadata (2 MiB), and host containers including profile refusals (2 MiB).
const DISPLAY_METADATA: usize = 20 << 20;
pub const MAX_DISPLAY_SCENE_BYTES: usize =
    DISPLAY_RESERVATION - MAX_PREPARED_BYTES as usize - MAX_PROFILE_BYTES - DISPLAY_METADATA;
// PreviewResult retains its original mesh payload while checked scene/preparation are built.
pub const MAX_PREVIEW_SCENE_BYTES: usize =
    PREVIEW_RESERVATION - MAX_SOURCE_BYTES - MAX_PREPARED_BYTES as usize - (2 << 20);
const HOST_BYTES: usize = 768 << 20;
#[derive(Default)]
pub struct HostBudget {
    used: AtomicUsize,
}
pub struct Lease {
    budget: Arc<HostBudget>,
    bytes: usize,
}
impl HostBudget {
    pub fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Lease, String> {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= HOST_BYTES)
            })
            .map_err(|_| {
                "Native host CPU handoff budget exhausted; wait for previous scene delivery"
                    .to_string()
            })?;
        Ok(Lease {
            budget: self.clone(),
            bytes,
        })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
pub fn charge(total: &mut usize, bytes: usize, limit: usize, kind: &str) -> Result<(), String> {
    *total = total
        .checked_add(bytes)
        .filter(|value| *value <= limit)
        .ok_or_else(|| format!("Native {kind} budget exceeded"))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_pending_and_building_payloads_share_one_reservation_limit() {
        let budget = Arc::new(HostBudget::default());
        let current = budget.reserve(DISPLAY_RESERVATION).unwrap();
        let pending = budget.reserve(DISPLAY_RESERVATION).unwrap();
        let building = budget.reserve(DISPLAY_RESERVATION).unwrap();
        let preview = budget.reserve(PREVIEW_RESERVATION).unwrap();
        assert!(budget.reserve(PREVIEW_RESERVATION).is_err());
        drop(pending);
        drop(building);
        assert!(budget.reserve(PREVIEW_RESERVATION).is_ok());
        drop(current);
        drop(preview);
        assert_eq!(budget.used.load(Ordering::Acquire), 0);
    }
    #[test]
    fn canceled_contour_job_keeps_its_host_reservation_until_execution_retires() {
        let budget = Arc::new(HostBudget::default());
        let _current = budget.reserve(DISPLAY_RESERVATION).unwrap();
        let _pending = budget.reserve(DISPLAY_RESERVATION).unwrap();
        let _building = budget.reserve(DISPLAY_RESERVATION).unwrap();
        let _preview = budget.reserve(PREVIEW_RESERVATION).unwrap();
        let host = Arc::new(budget.reserve(MAX_PREPARED_BYTES as usize).unwrap());
        let executing_job = host.clone();
        drop(host);
        assert!(budget.reserve(MAX_PREPARED_BYTES as usize).is_err());
        drop(executing_job);
        assert!(budget.reserve(MAX_PREPARED_BYTES as usize).is_ok());
    }
    #[test]
    fn nema_checked_scene_fits_display_reservation_with_bvh_storage() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render/nema17");
        let mut paths: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "mesh1")
            })
            .collect();
        paths.sort();
        let bodies = paths
            .into_iter()
            .map(|path| {
                onecad_scene::SceneBody::new(
                    onecad_scene::BodyId::new(),
                    1,
                    Arc::from(std::fs::read(path).unwrap()),
                    onecad_scene::MeshLimits::default(),
                )
                .unwrap()
            })
            .collect();
        let scene = onecad_scene::SceneSnapshot::new(
            onecad_scene::SceneIdentity {
                session: 1,
                scene: 1,
            },
            bodies,
            onecad_scene::SceneLimits {
                max_resident_bytes: MAX_DISPLAY_SCENE_BYTES,
                max_bodies: MAX_BODIES,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(scene.bodies().len(), 67);
        assert!(
            scene.resident_bytes()
                + MAX_PREPARED_BYTES as usize
                + MAX_PROFILE_BYTES
                + DISPLAY_METADATA
                <= DISPLAY_RESERVATION
        );
    }
}
