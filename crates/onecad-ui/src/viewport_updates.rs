//! Off-thread section preparation keeps the last drawable publication intact.
use crate::{app::App, bridge::Display};
use onecad_render::{
    PreparationBudget, PreparedScene, PreparedSectionContours, PreviewIdentity, SectionContourKey,
    SectionContourWorker,
};
use onecad_scene::{SceneIdentity, SceneSnapshot};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Source {
    pub scene: Arc<SceneSnapshot>,
    pub lease: Arc<crate::budget::Lease>,
}

pub(crate) struct Preview {
    pub prepared: PreparedScene,
    pub source: Source,
    pub suppressed: Vec<onecad_scene::BodyId>,
}

struct Job {
    worker: Arc<SectionContourWorker>,
    active: Arc<Mutex<Option<SectionContourKey>>>,
    key: Option<SectionContourKey>,
    ready: Option<PreparedSectionContours>,
    error: Option<String>,
    reservation: Option<Arc<crate::budget::Lease>>,
}

impl Job {
    fn new(
        worker: Arc<SectionContourWorker>,
        active: Arc<Mutex<Option<SectionContourKey>>>,
    ) -> Self {
        Self {
            worker,
            active,
            key: None,
            ready: None,
            error: None,
            reservation: None,
        }
    }

    fn prepare(
        &mut self,
        source: Source,
        preview: Option<PreviewIdentity>,
        budget: PreparationBudget,
        section: [f32; 4],
        host: &Arc<crate::budget::HostBudget>,
    ) -> Result<bool, String> {
        let key = SectionContourKey {
            scene: source.scene.identity(),
            preview,
            section: section.map(f32::to_bits),
        };
        if self.key == Some(key) && self.ready.is_some() {
            return Ok(true);
        }
        let current = *self
            .active
            .lock()
            .map_err(|_| "section scheduling poisoned")?;
        if self.key != Some(key) || (current != Some(key) && self.error.is_none()) {
            self.cancel();
            self.key = Some(key);
            let reservation = Arc::new(host.reserve_kind(
                crate::budget::MAX_PREPARED_BYTES as usize,
                crate::budget::Kind::Contour,
            )?);
            if let Err(error) = self.worker.submit_retained(
                source.scene,
                preview,
                budget,
                section,
                Arc::new((source.lease, reservation.clone())),
            ) {
                self.error = Some(error.to_string());
            }
            self.reservation = Some(reservation);
            *self
                .active
                .lock()
                .map_err(|_| "section scheduling poisoned")? = Some(key);
        }
        if let Some(result) = self.worker.poll(key) {
            match result {
                Ok(contours) => self.ready = Some(contours),
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(self.ready.is_some())
    }

    fn cancel(&mut self) {
        self.worker.cancel();
        self.key = None;
        self.ready = None;
        self.error = None;
        self.reservation = None;
        if let Ok(mut active) = self.active.lock() {
            *active = None;
        }
    }
}

pub(crate) struct Sections {
    committed: Job,
    preview: Job,
    retained_preview: Option<(PreviewIdentity, Source)>,
    rejected_scene: Option<SceneIdentity>,
}

impl Sections {
    pub fn new() -> Result<Self, String> {
        let worker = Arc::new(SectionContourWorker::new().map_err(|error| error.to_string())?);
        let active = Arc::new(Mutex::new(None));
        Ok(Self {
            committed: Job::new(worker.clone(), active.clone()),
            preview: Job::new(worker, active),
            retained_preview: None,
            rejected_scene: None,
        })
    }

    pub fn cancel(&mut self) {
        self.committed.cancel();
        self.preview.cancel();
        self.retained_preview = None;
    }

    pub fn cancel_preview(&mut self) {
        self.preview.cancel();
    }
}

fn committed_source(display: &Display) -> Source {
    Source {
        scene: display.scene.clone(),
        lease: display._lease.clone(),
    }
}

fn sectioned(section: [f32; 4]) -> bool {
    section[..3].iter().any(|value| *value != 0.0)
}

/// Called only after CAD completion and the host sampling fence have drained.
pub(crate) fn apply(app: &mut App, display: &Display) -> Result<bool, String> {
    if app.renderer.scene_identity() != Some(display.scene.identity())
        && app
            .pending_display_options
            .is_some_and(|options| !sectioned(options.section))
    {
        apply_options(app, display)?;
    }
    if app.doc.sections.rejected_scene == Some(display.scene.identity()) {
        return Ok(false);
    }
    if app.renderer.scene_identity() != Some(display.scene.identity())
        && !replace_scene(app, display)?
    {
        return Ok(false);
    }
    if app.doc.clear_preview {
        clear_preview(app)?;
    }
    if !replace_preview(app, display)? {
        return Ok(false);
    }
    apply_options(app, display)
}

/// Retire cancelled preview payloads even when occlusion suppresses viewport drawing.
pub(crate) fn retire_cancelled_preview(app: &mut App, ctx: &egui::Context) -> Result<(), String> {
    if app.parked || !app.doc.clear_preview {
        return Ok(());
    }
    if !app
        .renderer
        .poll_idle()
        .map_err(|error| error.to_string())?
        || !app.sampling_done.load(std::sync::atomic::Ordering::Acquire)
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        return Ok(());
    }
    clear_preview(app)
}
fn clear_preview(app: &mut App) -> Result<(), String> {
    app.renderer
        .clear_preview()
        .map_err(|error| error.to_string())?;
    app.doc.sections.preview.cancel();
    app.doc.sections.retained_preview = None;
    app.doc.clear_preview = false;
    app.draw_dirty = true;
    Ok(())
}

fn replace_scene(app: &mut App, display: &Display) -> Result<bool, String> {
    let section = app.renderer.options().section;
    let contours = if sectioned(section) {
        match app.doc.sections.committed.prepare(
            committed_source(display),
            None,
            app.renderer.preparation_budget(),
            section,
            &app.doc.bridge.shared.host_budget,
        ) {
            Ok(true) => app.doc.sections.committed.ready.take(),
            Ok(false) => return Ok(false),
            Err(error) => {
                app.doc.status = format!("Section: {error}");
                return Ok(false);
            }
        }
    } else {
        None
    };
    let prepared = display
        .prepared
        .lock()
        .map_err(|_| "scene handoff poisoned")?
        .take()
        .ok_or("prepared scene already consumed")?;
    let result = match contours {
        Some(contours) => app
            .renderer
            .replace_prepared_scene_with_section_contours(prepared, contours),
        None => app.renderer.replace_prepared_scene(prepared),
    };
    if let Err(error) = result {
        app.doc.status = format!("Scene display: {error}");
        app.doc.sections.rejected_scene = Some(display.scene.identity());
        return Ok(false);
    }
    app.doc.sections.committed.cancel();
    app.doc.sections.preview.cancel();
    app.doc.clear_preview = true;
    app.doc.topology.invalidate(display.scene.identity());
    Ok(true)
}

fn replace_preview(app: &mut App, display: &Display) -> Result<bool, String> {
    let Some((tag, preview)) = app.doc.pending_preview.as_ref() else {
        return Ok(true);
    };
    if tag.context.scene != display.scene.identity()
        || app.doc.interaction.context.as_ref() != Some(&tag.context)
        || !app
            .doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| tool.displays_preview(tag))
    {
        app.doc.pending_preview = None;
        app.doc.sections.preview.cancel();
        return Ok(true);
    }
    let identity = PreviewIdentity {
        base: tag.context.scene,
        generation: tag.sequence,
    };
    let section = app.renderer.options().section;
    let contours = if sectioned(section) {
        match app.doc.sections.preview.prepare(
            preview.source.clone(),
            Some(identity),
            app.renderer.preparation_budget(),
            section,
            &app.doc.bridge.shared.host_budget,
        ) {
            Ok(true) => app.doc.sections.preview.ready.take(),
            Ok(false) => return Ok(false),
            Err(error) => {
                app.doc.status = format!("Preview section: {error}");
                return Ok(false);
            }
        }
    } else {
        None
    };
    let session = tag.context.scene.session;
    let metadata = [tag.sequence, tag.context.scene.scene, tag.gesture];
    let Some((_, preview)) = app.doc.pending_preview.take() else {
        return Ok(true);
    };
    let upload =
        crate::performance::span(crate::performance::Stage::PreviewUpload, session, metadata);
    let result = match contours {
        Some(contours) => app.renderer.replace_prepared_preview_with_section_contours(
            identity,
            preview.prepared,
            contours,
        ),
        None => app
            .renderer
            .replace_prepared_preview(identity, preview.prepared),
    };
    let result = result.and_then(|()| {
        app.renderer
            .suppress_preview_bodies(identity, &preview.suppressed)
    });
    drop(upload);
    match result {
        Ok(()) => {
            crate::performance::record(
                crate::performance::Stage::PreviewDisplayed,
                session,
                metadata,
            );
            app.doc.sections.retained_preview = Some((identity, preview.source));
        }
        Err(error) => app.doc.status = format!("Preview display: {error}"),
    }
    app.doc.sections.preview.cancel();
    Ok(true)
}

fn apply_options(app: &mut App, display: &Display) -> Result<bool, String> {
    let Some(options) = app.pending_display_options else {
        return Ok(true);
    };
    let changed_section = options.section != app.renderer.options().section;
    let result = if sectioned(options.section) {
        match prepare_options(app, display, options.section) {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(error) => {
                app.doc.status = format!("Display: {error}");
                app.pending_display_options = None;
                app.doc.sections.committed.cancel();
                app.doc.sections.preview.cancel();
                return Ok(true);
            }
        }
        let contours = app
            .doc
            .sections
            .committed
            .ready
            .take()
            .ok_or("missing section contours")?;
        app.renderer.set_options_with_section_contours(
            options,
            contours,
            app.doc.sections.preview.ready.take(),
        )
    } else {
        app.renderer.set_options(options).or_else(|error| {
            if error.recoverable_target_refusal() && !changed_section {
                app.renderer.set_options_reclaiming(options)
            } else {
                Err(error)
            }
        })
    };
    app.pending_display_options = None;
    app.doc.sections.committed.cancel();
    app.doc.sections.preview.cancel();
    match result {
        Ok(()) => {
            app.resize_refusal = None;
            crate::viewport::replace_texture(app);
            if changed_section {
                // Sectioned geometry is not pickable; drop model picks, keep rows.
                crate::item_selection::retain(app, |item| {
                    matches!(
                        item,
                        onecad_interaction::selection_set::SelectionItem::Feature(_)
                            | onecad_interaction::selection_set::SelectionItem::Sketch(_)
                    )
                });
                app.doc.interaction.selection = None;
            }
        }
        Err(error) => {
            if !app.renderer.has_targets() && error.recoverable_target_refusal() {
                app.resize_refusal = Some((app.renderer.size(), error.to_string()));
                app.doc.view = None;
            }
            app.doc.status = format!("Display: {error}");
        }
    }
    Ok(true)
}

fn prepare_options(app: &mut App, display: &Display, section: [f32; 4]) -> Result<bool, String> {
    let budget = app
        .renderer
        .preparation_budget()
        .limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES);
    let committed = app.doc.sections.committed.prepare(
        committed_source(display),
        None,
        budget,
        section,
        &app.doc.bridge.shared.host_budget,
    )?;
    if !committed {
        return Ok(false);
    }
    let retained = app
        .doc
        .sections
        .committed
        .ready
        .as_ref()
        .map_or(0, |ready| ready.cpu_bytes());
    let budget =
        budget.limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES.saturating_sub(retained));
    let preview = match &app.doc.sections.retained_preview {
        Some((identity, source)) => app.doc.sections.preview.prepare(
            source.clone(),
            Some(*identity),
            budget,
            section,
            &app.doc.bridge.shared.host_budget,
        )?,
        None => true,
    };
    Ok(committed && preview)
}
