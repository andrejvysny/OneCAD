//! Conservative CPU admission, shared by a window's service runtime.
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Notify;
mod admission;
pub const DISPLAY_RESERVATION: usize = 160 << 20;
pub const PREVIEW_RESERVATION: usize = 192 << 20;
pub const DISPLAY_SOURCE_RESERVATION: usize = DISPLAY_RESERVATION - MAX_PREPARED_BYTES as usize;
pub const PREVIEW_SOURCE_RESERVATION: usize = PREVIEW_RESERVATION - MAX_PREPARED_BYTES as usize;
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

pub const ADMISSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const MAX_WAITERS: usize = onecad_app::reservations::MAX_OPEN_DOCUMENTS * 3;
pub type OwnerId = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    #[cfg(test)]
    Other,
    Display,
    DisplaySource,
    Preview,
    PreviewSource,
    Preparation,
    Contour,
    Highlight,
}
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub aggregate: usize,
    pub active_headroom: usize,
}
impl Policy {
    pub fn for_profile(profile: onecad_render::RenderProfile) -> Self {
        Self {
            aggregate: HOST_BYTES,
            active_headroom: if profile == onecad_render::RenderProfile::Mobile {
                PREVIEW_RESERVATION
            } else {
                0
            },
        }
    }
}
#[derive(Clone, Debug)]
pub struct Portion {
    pub id: u64,
    pub admission: u64,
    pub owner: OwnerId,
    pub kind: Kind,
    pub bytes: usize,
}
#[derive(Default, Debug)]
pub struct OwnerSnapshot {
    pub used: usize,
    pub kinds: BTreeMap<Kind, usize>,
}
pub struct Snapshot {
    pub used: usize,
    pub inactive: usize,
    pub active: Option<OwnerId>,
    pub portions: Vec<Portion>,
    pub owners: BTreeMap<OwnerId, OwnerSnapshot>,
    pub waiting: usize,
    pub degraded: bool,
}
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("used", &self.used)
            .field("inactive", &self.inactive)
            .field("active", &self.active)
            .field("portions", &self.portions)
            .field("owners", &self.owners)
            .field("waiting", &self.waiting)
            .field("degraded", &self.degraded)
            .finish()
    }
}
#[derive(Debug)]
struct Ticket {
    id: u64,
    owner: OwnerId,
    kind: Kind,
    bytes: usize,
    closed: Arc<AtomicBool>,
}
#[derive(Debug)]
struct State {
    policy: Policy,
    active: Option<OwnerId>,
    next: u64,
    degraded: bool,
    portions: BTreeMap<u64, Portion>,
    waiters: VecDeque<Ticket>,
}
impl State {
    fn next_id(&mut self) -> Result<u64, String> {
        if self.degraded {
            return Err("Native CPU budget ledger unavailable; document remains open".into());
        }
        let Some(next) = self.next.checked_add(1) else {
            self.degraded = true;
            return Err("Native CPU budget identity exhausted; document remains open".into());
        };
        self.next = next;
        Ok(next)
    }
    fn used(&self) -> usize {
        self.portions
            .values()
            .fold(0usize, |used, portion| used.saturating_add(portion.bytes))
    }
    fn inactive(&self) -> usize {
        self.portions
            .values()
            .filter(|portion| Some(portion.owner) != self.active)
            .map(|portion| portion.bytes)
            .sum()
    }
    fn eligible(&self, owner: OwnerId, bytes: usize) -> bool {
        !self.degraded
            && self
                .used()
                .checked_add(bytes)
                .is_some_and(|next| next <= self.policy.aggregate)
            && (self.active == Some(owner)
                || self.inactive().checked_add(bytes).is_some_and(|next| {
                    next <= self.policy.aggregate - self.policy.active_headroom
                }))
    }
    fn first_eligible(&self) -> Option<u64> {
        self.waiters
            .iter()
            .find(|ticket| {
                !ticket.closed.load(Ordering::Acquire) && self.eligible(ticket.owner, ticket.bytes)
            })
            .map(|ticket| ticket.id)
    }
    fn allocate(&mut self, owner: OwnerId, kind: Kind, bytes: usize) -> Result<u64, String> {
        let id = self.next_id()?;
        self.portions.insert(
            id,
            Portion {
                id,
                admission: id,
                owner,
                kind,
                bytes,
            },
        );
        Ok(id)
    }
}
#[derive(Debug)]
pub struct BudgetAuthority {
    state: Mutex<State>,
    changed: Notify,
}
impl BudgetAuthority {
    pub fn new(profile: onecad_render::RenderProfile) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                policy: Policy::for_profile(profile),
                active: None,
                next: 0,
                degraded: false,
                portions: BTreeMap::new(),
                waiters: VecDeque::new(),
            }),
            changed: Notify::new(),
        })
    }
    /// Poisoned accounting is inspectable and can retire known portions, but cannot admit work.
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| {
            let mut state = poisoned.into_inner();
            let first_failure = !state.degraded;
            state.degraded = true;
            if first_failure {
                self.changed.notify_waiters();
            }
            state
        })
    }
    pub fn owner(self: &Arc<Self>) -> Result<Arc<HostBudget>, String> {
        let owner = self.state().next_id()?;
        Ok(Arc::new(HostBudget {
            authority: self.clone(),
            owner,
            closed: Arc::new(AtomicBool::new(false)),
        }))
    }
    pub fn set_active(&self, owner: Option<OwnerId>) {
        let mut state = self.state();
        if state.active != owner {
            state.active = owner;
            drop(state);
            self.changed.notify_waiters();
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let state = self.state();
        let mut owners = BTreeMap::<OwnerId, OwnerSnapshot>::new();
        for portion in state.portions.values() {
            let owner = owners.entry(portion.owner).or_default();
            owner.used += portion.bytes;
            *owner.kinds.entry(portion.kind).or_default() += portion.bytes;
        }
        Snapshot {
            used: state.used(),
            inactive: state.inactive(),
            active: state.active,
            portions: state
                .portions
                .iter()
                .map(|(id, portion)| {
                    debug_assert_eq!(*id, portion.id);
                    portion.clone()
                })
                .collect(),
            owners,
            waiting: state.waiters.len(),
            degraded: state.degraded,
        }
    }
}
#[derive(Debug)]
pub struct HostBudget {
    authority: Arc<BudgetAuthority>,
    owner: OwnerId,
    closed: Arc<AtomicBool>,
}
#[cfg(test)]
impl Default for HostBudget {
    fn default() -> Self {
        let authority = BudgetAuthority::new(onecad_render::RenderProfile::Desktop);
        let owner = authority.state().next_id().unwrap();
        Self {
            authority,
            owner,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}
pub struct Lease {
    budget: Arc<HostBudget>,
    id: u64,
    bytes: usize,
}
impl std::fmt::Debug for Lease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Lease")
            .field("id", &self.id)
            .field("bytes", &self.bytes)
            .finish()
    }
}
impl HostBudget {
    pub fn owner_id(&self) -> OwnerId {
        self.owner
    }
    pub fn snapshot(&self) -> Snapshot {
        self.authority.snapshot()
    }
    /// Advisory capacity check only; callers still join the admission queue.
    pub fn has_capacity(&self, bytes: usize) -> bool {
        !self.closed.load(Ordering::Acquire) && self.authority.state().eligible(self.owner, bytes)
    }
    pub fn activate(&self) {
        let mut state = self.authority.state();
        if !self.closed.load(Ordering::Acquire) && state.active != Some(self.owner_id()) {
            state.active = Some(self.owner_id());
            drop(state);
            self.wake();
        }
    }
    pub fn wake(&self) {
        self.authority.changed.notify_waiters();
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let mut state = self.authority.state();
        if state.active == Some(self.owner) {
            state.active = None;
        }
        drop(state);
        self.wake();
    }
    #[cfg(test)]
    pub fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Lease, String> {
        self.reserve_kind(bytes, Kind::Other)
    }
    pub fn reserve_kind(self: &Arc<Self>, bytes: usize, kind: Kind) -> Result<Lease, String> {
        let mut state = self.authority.state();
        if state.degraded {
            return Err("Native CPU budget ledger unavailable; document remains open".into());
        }
        if self.closed.load(Ordering::Acquire)
            || !state.eligible(self.owner, bytes)
            || state.first_eligible().is_some()
        {
            return Err(exhausted());
        }
        let id = state.allocate(self.owner, kind, bytes)?;
        Ok(Lease {
            budget: self.clone(),
            id,
            bytes,
        })
    }
}
fn exhausted() -> String {
    "Native host CPU handoff budget exhausted; document remains open; display work was not admitted"
        .into()
}
impl Lease {
    #[cfg(test)]
    pub fn split(self, first_bytes: usize) -> Result<(Self, Self), String> {
        self.split_kinds(first_bytes, Kind::Other, Kind::Other)
    }
    /// Both portions retain one admission identity; no capacity is released or reacquired.
    pub fn split_kinds(
        mut self,
        first_bytes: usize,
        first_kind: Kind,
        second_kind: Kind,
    ) -> Result<(Self, Self), String> {
        if first_bytes > self.bytes {
            return Err("Native reservation split exceeds admitted bytes".into());
        }
        let mut state = self.budget.authority.state();
        if state.degraded {
            return Err("Native CPU budget ledger unavailable".into());
        }
        let Some(mut original) = state.portions.get(&self.id).cloned() else {
            state.degraded = true;
            return Err("Native CPU budget portion missing".into());
        };
        let second_id = state.next_id()?;
        let second = Portion {
            id: second_id,
            admission: original.admission,
            owner: original.owner,
            kind: second_kind,
            bytes: self.bytes - first_bytes,
        };
        original.bytes = first_bytes;
        original.kind = first_kind;
        state.portions.insert(self.id, original);
        state.portions.insert(second_id, second);
        let second = Self {
            budget: self.budget.clone(),
            id: second_id,
            bytes: self.bytes - first_bytes,
        };
        self.bytes = first_bytes;
        drop(state);
        Ok((self, second))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.budget.authority.state();
        if state.portions.remove(&self.id).is_none() {
            state.degraded = true;
        }
        drop(state);
        self.budget.wake();
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
mod tests;

#[cfg(test)]
mod admission_tests;
