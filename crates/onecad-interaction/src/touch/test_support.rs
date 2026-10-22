use super::*;

pub fn sample(
    contact: ContactId,
    kind: ContactKind,
    phase: Phase,
    provenance: Provenance,
    t: f64,
    x: f64,
    y: f64,
) -> Sample {
    Sample {
        contact,
        kind,
        phase,
        provenance,
        timestamp: t,
        position: Some(Point::new(x, y)),
        pose: Pose::default(),
        estimation_index: None,
        tap_count: 1,
    }
}

pub fn pencil(contact: ContactId, phase: Phase, t: f64, x: f64, y: f64) -> Sample {
    sample(
        contact,
        ContactKind::Pencil,
        phase,
        Provenance::Actual,
        t,
        x,
        y,
    )
}

pub fn finger(contact: ContactId, phase: Phase, t: f64, x: f64, y: f64) -> Sample {
    sample(
        contact,
        ContactKind::Finger,
        phase,
        Provenance::Actual,
        t,
        x,
        y,
    )
}

pub fn with(provenance: Provenance, base: Sample) -> Sample {
    Sample { provenance, ..base }
}

pub fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// Normalizer and router driven like a host: submit, drain, route.
#[derive(Default)]
pub struct Pipeline {
    pub normalizer: Normalizer,
    pub router: Router,
}

impl Pipeline {
    pub fn submit(&mut self, samples: &[Sample], ctx: &RouteContext<'_>) -> Vec<RoutedEvent> {
        self.submit_generation(samples, 0, ctx)
    }

    pub fn submit_generation(
        &mut self,
        samples: &[Sample],
        generation: u64,
        ctx: &RouteContext<'_>,
    ) -> Vec<RoutedEvent> {
        self.normalizer
            .submit(samples, generation)
            .expect("batch within bounds");
        self.router.route(self.normalizer.drain(), ctx)
    }

    pub fn cancel_all(&mut self, reason: CancelReason) -> Vec<RoutedEvent> {
        self.normalizer.cancel_all(reason);
        self.router
            .route(self.normalizer.drain(), &RouteContext::CANVAS)
    }
}

pub fn authoring() -> RouteContext<'static> {
    RouteContext {
        pencil_only_drawing: false,
        drag_authoring: true,
        over_control: &|_| false,
    }
}

/// Controls occupy x < 100.
pub fn with_controls() -> RouteContext<'static> {
    RouteContext {
        pencil_only_drawing: false,
        drag_authoring: false,
        over_control: &|point| point.x < 100.0,
    }
}

pub fn kinds(events: &[RoutedEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            RoutedEvent::Press { .. } => "press",
            RoutedEvent::Move { .. } => "move",
            RoutedEvent::Release { .. } => "release",
            RoutedEvent::Hover { .. } => "hover",
            RoutedEvent::HoverExit { .. } => "hover-exit",
            RoutedEvent::Orbit { .. } => "orbit",
            RoutedEvent::MultiTouch { .. } => "multitouch",
            RoutedEvent::Overlay(_) => "overlay",
            RoutedEvent::Cancel { .. } => "cancel",
            RoutedEvent::Activate { .. } => "activate",
        })
        .collect()
}
