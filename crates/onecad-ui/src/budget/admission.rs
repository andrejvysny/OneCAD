use super::*;

struct Waiting {
    authority: Arc<BudgetAuthority>,
    id: u64,
}
impl Drop for Waiting {
    fn drop(&mut self) {
        let mut state = self.authority.state();
        state.waiters.retain(|ticket| ticket.id != self.id);
        drop(state);
        self.authority.changed.notify_waiters();
    }
}
impl HostBudget {
    /// One actor serializes each document's capture, preparation and preview work.
    /// A dropped/expired future removes its ticket without retiring live allocations.
    pub async fn admit_while(
        self: &Arc<Self>,
        bytes: usize,
        kind: Kind,
        current: impl Fn() -> bool,
    ) -> Result<Lease, String> {
        let waiting = self.enqueue(bytes, kind)?;
        tokio::time::timeout(ADMISSION_TIMEOUT, self.wait(&waiting, bytes, kind, current))
            .await
            .map_err(|_| self.timed_out(bytes, kind))?
    }
    fn timed_out(&self, bytes: usize, kind: Kind) -> String {
        let snapshot = self.snapshot();
        tracing::warn!(
            requested_bytes = bytes,
            ?kind,
            owner = self.owner,
            ?snapshot,
            "Native CPU display admission timed out"
        );
        #[cfg(test)]
        eprintln!(
            "Native CPU admission timeout: owner={} kind={kind:?} requested={bytes} snapshot={snapshot:?}",
            self.owner
        );
        exhausted()
    }
    fn enqueue(&self, bytes: usize, kind: Kind) -> Result<Waiting, String> {
        let mut state = self.authority.state();
        if state.degraded {
            return Err("Native CPU budget ledger unavailable; document remains open".into());
        }
        if self.closed.load(Ordering::Acquire) || bytes > state.policy.aggregate {
            return Err(exhausted());
        }
        if state.waiters.len() >= MAX_WAITERS {
            return Err("Native display admission queue is full; document remains open".into());
        }
        if state
            .waiters
            .iter()
            .any(|ticket| ticket.owner == self.owner && ticket.kind == kind)
        {
            return Err("Native document already waits for this display work kind".into());
        }
        let id = state.next_id()?;
        state.waiters.push_back(Ticket {
            id,
            owner: self.owner,
            kind,
            bytes,
            closed: self.closed.clone(),
        });
        Ok(Waiting {
            authority: self.authority.clone(),
            id,
        })
    }
    async fn wait(
        self: &Arc<Self>,
        waiting: &Waiting,
        bytes: usize,
        kind: Kind,
        current: impl Fn() -> bool,
    ) -> Result<Lease, String> {
        loop {
            // Register before inspecting the queue so a release/cancel cannot be lost.
            let changed = self.authority.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !current() || self.closed.load(Ordering::Acquire) {
                return Err("Native display admission cancelled; document remains open".into());
            }
            if let Some(lease) = self.claim(waiting.id, bytes, kind)? {
                return Ok(lease);
            }
            changed.await;
        }
    }
    fn claim(
        self: &Arc<Self>,
        ticket: u64,
        bytes: usize,
        kind: Kind,
    ) -> Result<Option<Lease>, String> {
        let mut state = self.authority.state();
        if state.degraded {
            return Err("Native CPU budget ledger unavailable; document remains open".into());
        }
        if self.closed.load(Ordering::Acquire) || state.first_eligible() != Some(ticket) {
            return Ok(None);
        }
        state.waiters.retain(|waiting| waiting.id != ticket);
        let id = state.allocate(self.owner, kind, bytes)?;
        drop(state);
        self.wake();
        Ok(Some(Lease {
            budget: self.clone(),
            id,
            bytes,
        }))
    }
}
