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
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
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
}
