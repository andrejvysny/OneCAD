use super::{
    api, apply_translation, document, mesh, mesh_compare, BodyId, Harness, OccurrencePath,
};
use onecad_app::resource_admission::{ArchiveAdmission, ArchiveBudget, ArchiveUse, Retention};
use onecad_app::state::AppState;
use std::sync::{Arc, Mutex};

const HOST_CAP: usize = 805_306_368;
pub(super) const CHECKPOINT: usize = 135_266_304;

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Snapshot {
    pub bytes: usize,
    pub peak_bytes: usize,
    pub guards: usize,
    pub scratch_guards: usize,
    pub checkpoint_guards: usize,
    pub checkpoint_denied: usize,
    pub checkpoint_admitted: usize,
    pub deny_checkpoint: bool,
}

#[derive(Clone, Default)]
pub(super) struct Probe(Arc<Mutex<Snapshot>>);

impl Probe {
    pub(super) fn install(&self, state: &AppState) {
        state
            .install_archive_budget(ArchiveBudget::new(Arc::new(self.clone())))
            .unwrap();
    }
    pub(super) fn snapshot(&self) -> Snapshot {
        *self.0.lock().unwrap()
    }
    pub(super) fn deny_checkpoint(&self, value: bool) {
        self.0.lock().unwrap().deny_checkpoint = value;
    }
}

struct Guard {
    probe: Probe,
    bytes: usize,
    scratch: bool,
    checkpoint: bool,
}
impl Drop for Guard {
    fn drop(&mut self) {
        let mut state = self.probe.0.lock().unwrap();
        state.bytes -= self.bytes;
        state.guards -= 1;
        state.scratch_guards -= usize::from(self.scratch);
        state.checkpoint_guards -= usize::from(self.checkpoint);
    }
}

impl ArchiveAdmission for Probe {
    fn reserve(&self, bytes: usize, purpose: ArchiveUse) -> Result<Retention, String> {
        let scratch = purpose == ArchiveUse::Scratch;
        let checkpoint = scratch && bytes == CHECKPOINT;
        let mut state = self.0.lock().unwrap();
        if checkpoint && state.deny_checkpoint {
            state.checkpoint_denied += 1;
            return Err("checkpoint admission denied by test".into());
        }
        let next = state
            .bytes
            .checked_add(bytes)
            .filter(|sum| *sum <= HOST_CAP)
            .ok_or_else(|| "host archive cap exceeded".to_string())?;
        state.bytes = next;
        state.peak_bytes = state.peak_bytes.max(next);
        state.guards += 1;
        state.scratch_guards += usize::from(scratch);
        state.checkpoint_guards += usize::from(checkpoint);
        state.checkpoint_admitted += usize::from(checkpoint);
        Ok(Arc::new(Guard {
            probe: self.clone(),
            bytes,
            scratch,
            checkpoint,
        }))
    }
}

pub(super) async fn budgeted_translation(
    h: &Harness,
    session: &str,
    probe: &Probe,
    independent: &[u8],
) {
    let before = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let part = before
        .definitions
        .iter()
        .find(|p| p.id == document::definition(2).to_string())
        .unwrap();
    let body: BodyId = part.bodies[0].id.parse().unwrap();
    let old_generation = part.mesh_generation.unwrap();
    probe.deny_checkpoint(true);
    apply_translation(&h.state, session, 0xA601, 1.0).await;
    let edited = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let baseline = probe.snapshot();
    assert_eq!(baseline.guards, 0, "{baseline:?}");
    assert_eq!(baseline.bytes, 0, "{baseline:?}");
    let error = api::regenerate_component(&h.state, session, document::definition(2))
        .await
        .unwrap_err();
    assert!(
        matches!(error, onecad_app::error::ApiError::Io(ref message) if message == "checkpoint admission denied by test"),
        "{error:?}"
    );
    let refused = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&refused).unwrap(),
        serde_json::to_value(&edited).unwrap()
    );
    assert!(api::component_mesh_for_path(
        &h.state,
        session,
        &OccurrencePath(vec![document::occurrence(2)]),
        body,
        old_generation
    )
    .await
    .is_err());
    mesh_compare::equal(
        "independent after admission refusal",
        independent,
        mesh(&h.state, session, 3, 3).await.as_slice(),
    );
    let denied = probe.snapshot();
    assert_eq!(
        denied.checkpoint_denied,
        baseline.checkpoint_denied + 1,
        "{denied:?}"
    );
    assert_eq!(denied.checkpoint_guards, 0, "{denied:?}");
    assert_eq!(denied.scratch_guards, 0, "{denied:?}");
    assert_eq!(denied.guards, 0, "{denied:?}");
    assert_eq!(denied.bytes, baseline.bytes, "{denied:?}");
    probe.deny_checkpoint(false);
    api::regenerate_component(&h.state, session, document::definition(2))
        .await
        .unwrap();
    let accepted = probe.snapshot();
    assert_eq!(
        accepted.checkpoint_admitted,
        denied.checkpoint_admitted + 1,
        "{accepted:?}"
    );
    assert_eq!(accepted.checkpoint_guards, 0, "{accepted:?}");
    assert_eq!(accepted.scratch_guards, 0, "{accepted:?}");
    assert_eq!(accepted.guards, 0, "{accepted:?}");
    assert_eq!(accepted.bytes, 0, "{accepted:?}");
    eprintln!("archive admission after checkpoint retry: {accepted:?}");
}
