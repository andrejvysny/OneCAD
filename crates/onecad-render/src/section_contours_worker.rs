//! Latest-only contour preparation. Reservations include active and pending work.
use super::section_contours::Candidate;
use crate::{
    limits::{add, mul},
    PreparationBudget, PreparedSectionContours, PreviewIdentity, RenderError, SectionContourKey,
};
use onecad_scene::SceneSnapshot;
use std::sync::{Arc, Condvar, Mutex};

struct Request {
    scene: Arc<SceneSnapshot>,
    budget: PreparationBudget,
    key: SectionContourKey,
    serial: u64,
    cpu: u64,
    source: u64,
    _hold: Arc<dyn Send + Sync>,
}
#[derive(Default)]
struct Mailbox {
    pending: Option<Request>,
    result: Option<(
        SectionContourKey,
        Result<PreparedSectionContours, RenderError>,
    )>,
    serial: u64,
    stop: bool,
    active_cpu: u64,
    active_source: u64,
}
/// One active job, one replaceable pending job and one replaceable result. Canceled work
/// checks its generation every 256 triangles. Dropping the worker never blocks the UI.
pub struct SectionContourWorker {
    shared: Arc<(Mutex<Mailbox>, Condvar)>,
}
impl SectionContourWorker {
    pub fn new() -> Result<Self, RenderError> {
        let shared = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let thread = shared.clone();
        std::thread::Builder::new()
            .name("section-contours".into())
            .spawn(move || run(thread))
            .map_err(|error| RenderError::Device(error.to_string()))?;
        Ok(Self { shared })
    }
    pub fn submit(
        &self,
        scene: Arc<SceneSnapshot>,
        preview: Option<PreviewIdentity>,
        budget: PreparationBudget,
        section: [f32; 4],
    ) -> Result<SectionContourKey, RenderError> {
        self.submit_retained(scene, preview, budget, section, Arc::new(()))
    }
    pub fn submit_retained(
        &self,
        scene: Arc<SceneSnapshot>,
        preview: Option<PreviewIdentity>,
        mut budget: PreparationBudget,
        section: [f32; 4],
        hold: Arc<dyn Send + Sync>,
    ) -> Result<SectionContourKey, RenderError> {
        if preview.is_some_and(|p| p.base != scene.identity())
            || !section.iter().all(|v| v.is_finite())
        {
            return Err(RenderError::CaptureMismatch);
        }
        let key = SectionContourKey {
            scene: scene.identity(),
            preview,
            section: section.map(f32::to_bits),
        };
        let mut state = self
            .shared
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let cpu = reservation(&scene, budget, section, &state)?;
        budget.limits.max_cpu_prepared_bytes = add(cpu, budget.resident.cpu_prepared_bytes)?;
        state.serial = state
            .serial
            .checked_add(1)
            .ok_or(RenderError::Budget("section generation"))?;
        let serial = state.serial;
        let source = scene.resident_bytes() as u64;
        state.pending = Some(Request {
            scene,
            budget,
            key,
            serial,
            cpu,
            source,
            _hold: hold,
        });
        state.result = None;
        self.shared.1.notify_one();
        Ok(key)
    }
    pub fn poll(
        &self,
        expected: SectionContourKey,
    ) -> Option<Result<PreparedSectionContours, RenderError>> {
        let mut state = self
            .shared
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .result
            .as_ref()
            .is_some_and(|(key, _)| *key == expected)
        {
            state.result.take().map(|(_, result)| result)
        } else {
            None
        }
    }
    pub fn cancel(&self) {
        let mut state = self
            .shared
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.serial = state.serial.wrapping_add(1);
        state.pending = None;
        state.result = None;
    }
}
impl Drop for SectionContourWorker {
    fn drop(&mut self) {
        self.cancel();
        self.shared
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stop = true;
        self.shared.1.notify_one();
    }
}
fn reservation(
    scene: &SceneSnapshot,
    budget: PreparationBudget,
    section: [f32; 4],
    state: &Mailbox,
) -> Result<u64, RenderError> {
    let triangles = if section[..3].iter().any(|n| *n != 0.0) {
        scene.bodies().iter().try_fold(0u64, |count, body| {
            add(count, u64::from(body.mesh()?.header().triangle_count))
        })?
    } else {
        0
    };
    let worst = add(
        mul(
            triangles,
            3 * (std::mem::size_of::<Candidate>() as u64 + 24),
        )?,
        std::mem::size_of::<PreparedSectionContours>() as u64,
    )?;
    let source = add(scene.resident_bytes() as u64, state.active_source)?;
    let available = budget
        .limits
        .max_total_bytes
        .saturating_sub(add(source, budget.resident.peak_total_bytes)?);
    // Jobs execute sequentially: both source leases overlap, but CPU scratch does not.
    if state.active_cpu > available {
        return Err(RenderError::Budget("section worker overlap"));
    }
    let cpu = worst.min(available).min(
        budget
            .limits
            .max_cpu_prepared_bytes
            .saturating_sub(budget.resident.cpu_prepared_bytes),
    );
    if cpu < std::mem::size_of::<PreparedSectionContours>() as u64 {
        return Err(RenderError::Budget("section CPU preparation"));
    }
    Ok(cpu)
}
fn run(shared: Arc<(Mutex<Mailbox>, Condvar)>) {
    loop {
        let request = {
            let mut state = shared.0.lock().unwrap_or_else(|error| error.into_inner());
            while state.pending.is_none() && !state.stop {
                state = shared
                    .1
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.stop {
                return;
            }
            let Some(request) = state.pending.take() else {
                continue;
            };
            state.active_cpu = request.cpu;
            state.active_source = request.source;
            request
        };
        let current = || {
            let state = shared.0.lock().unwrap_or_else(|error| error.into_inner());
            !state.stop && state.serial == request.serial
        };
        let result = PreparedSectionContours::prepare(
            &request.scene,
            request.budget,
            request.key.section.map(f32::from_bits),
            &current,
        )
        .map(|mut prepared| {
            prepared.key = request.key;
            prepared
        });
        let mut state = shared.0.lock().unwrap_or_else(|error| error.into_inner());
        state.active_cpu = 0;
        state.active_source = 0;
        if !state.stop && state.serial == request.serial {
            state.result = Some((request.key, result));
        }
    }
}
