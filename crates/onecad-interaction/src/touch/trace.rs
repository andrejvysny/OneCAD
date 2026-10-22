//! Deterministic input traces (`fixtures/ios/traces/*.json`, schema in the
//! appendix of `docs/ios/INPUT.md`) and the canonical text of their routed
//! events. Test-hook replay parses samples with the same schema.
use super::{
    mask, CancelReason, Normalizer, Overlay, Point, RawSample, RouteContext, RoutedEvent, Router,
};
use serde_json::Value;

/// Routing flags and control rectangles (`[x0, y0, x1, y1]`, logical points).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceContext {
    pub pencil_only_drawing: bool,
    pub drag_authoring: bool,
    pub controls: Vec<[f64; 4]>,
}

impl TraceContext {
    pub fn over_control(&self, point: Point) -> bool {
        self.controls.iter().any(|[x0, y0, x1, y1]| {
            (*x0..=*x1).contains(&point.x) && (*y0..=*y1).contains(&point.y)
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Batch {
        generation: u64,
        samples: Vec<RawSample>,
    },
    CancelAll(CancelReason),
    Context(TraceContext),
    /// The host's native pan/pinch recognizer began.
    Navigation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    pub name: String,
    pub context: TraceContext,
    pub steps: Vec<Step>,
}

/// Routed events and normalizer counters, one canonical line each.
pub fn run(trace: &Trace) -> Vec<String> {
    let mut normalizer = Normalizer::default();
    let mut router = Router::default();
    let mut context = trace.context.clone();
    let mut lines = Vec::new();
    for step in &trace.steps {
        match step {
            Step::Batch {
                generation,
                samples,
            } => {
                if let Err(error) = normalizer.submit_raw(samples, *generation) {
                    lines.push(format!("refused {error}"));
                }
            }
            Step::CancelAll(reason) => normalizer.cancel_all(*reason),
            Step::Context(next) => context = next.clone(),
            Step::Navigation => {
                let (allowed, events) = router.native_navigation();
                lines.push(format!("navigation allowed={allowed}"));
                lines.extend(events.iter().map(describe));
            }
        }
        let over = |point: Point| context.over_control(point);
        let route = RouteContext {
            pencil_only_drawing: context.pencil_only_drawing,
            drag_authoring: context.drag_authoring,
            over_control: &over,
        };
        let events = router.route(normalizer.drain(), &route);
        lines.extend(events.iter().map(describe));
    }
    lines.push(format!("stats {:?}", normalizer.stats()));
    lines
}

/// FNV-1a (64-bit) of the canonical lines joined by `\n`.
pub fn digest(lines: &[String]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in lines.join("\n").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn point(p: Point) -> String {
    format!("({:.3},{:.3})", p.x, p.y)
}

/// Fixed-precision text of one routed event.
pub fn describe(event: &RoutedEvent) -> String {
    match event {
        RoutedEvent::Press {
            contact,
            owner,
            kind,
            position,
            tap_count,
        } => format!(
            "press c{contact} {owner:?} {kind:?} {} taps={tap_count}",
            point(*position)
        ),
        RoutedEvent::Move {
            contact,
            owner,
            position,
        } => format!("move c{contact} {owner:?} {}", point(*position)),
        RoutedEvent::Release {
            contact,
            owner,
            kind,
            position,
        } => format!("release c{contact} {owner:?} {kind:?} {}", point(*position)),
        RoutedEvent::Hover { kind, position } => format!("hover {kind:?} {}", point(*position)),
        RoutedEvent::HoverExit { kind } => format!("hover-exit {kind:?}"),
        RoutedEvent::Orbit {
            contact,
            phase,
            position,
            delta,
        } => format!(
            "orbit c{contact} {phase:?} {} d{}",
            point(*position),
            point(*delta)
        ),
        RoutedEvent::MultiTouch { contact, phase } => format!("multitouch c{contact} {phase:?}"),
        RoutedEvent::Overlay(Overlay::Prediction { contact, points }) => format!(
            "prediction c{contact} [{}]",
            points
                .iter()
                .map(|p| point(*p))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        RoutedEvent::Cancel { contact, reason } => format!("cancel c{contact} {reason:?}"),
    }
}

pub fn parse(text: &str) -> Result<Trace, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or("trace needs a name")?
        .to_owned();
    let context = match value.get("context") {
        Some(context) => parse_context(context)?,
        None => TraceContext::default(),
    };
    let steps = value
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("trace needs steps")?
        .iter()
        .map(parse_step)
        .collect::<Result<_, _>>()?;
    Ok(Trace {
        name,
        context,
        steps,
    })
}

fn parse_context(value: &Value) -> Result<TraceContext, String> {
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let controls = match value.get("controls") {
        None => Vec::new(),
        Some(list) => list
            .as_array()
            .ok_or("controls must be an array")?
            .iter()
            .map(|rect| {
                let numbers: Vec<f64> = rect
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                <[f64; 4]>::try_from(numbers).map_err(|_| "control is [x0, y0, x1, y1]".to_owned())
            })
            .collect::<Result<_, _>>()?,
    };
    Ok(TraceContext {
        pencil_only_drawing: flag("pencil_only_drawing"),
        drag_authoring: flag("drag_authoring"),
        controls,
    })
}

fn parse_step(value: &Value) -> Result<Step, String> {
    if let Some(batch) = value.get("batch") {
        let samples = batch
            .get("samples")
            .and_then(Value::as_array)
            .ok_or("batch needs samples")?
            .iter()
            .map(sample)
            .collect::<Result<_, _>>()?;
        let generation = batch.get("generation").and_then(Value::as_u64).unwrap_or(0);
        return Ok(Step::Batch {
            generation,
            samples,
        });
    }
    if let Some(reason) = value.get("cancel_all") {
        return cancel_reason(reason).map(Step::CancelAll);
    }
    if let Some(context) = value.get("context") {
        return parse_context(context).map(Step::Context);
    }
    if value.get("navigation").is_some() {
        return Ok(Step::Navigation);
    }
    Err(format!("unknown step {value}"))
}

fn cancel_reason(value: &Value) -> Result<CancelReason, String> {
    if let Some(code) = value.as_u64() {
        return u32::try_from(code)
            .ok()
            .and_then(CancelReason::from_cancel_all)
            .ok_or_else(|| format!("unknown cancel reason {code}"));
    }
    Ok(match value.as_str() {
        Some("system_gesture") => CancelReason::SystemGesture,
        Some("scene_inactive") => CancelReason::SceneInactive,
        Some("orientation") => CancelReason::Orientation,
        Some("text_focus") => CancelReason::TextFocus,
        Some("document_switch") => CancelReason::DocumentSwitch,
        _ => return Err(format!("unknown cancel reason {value}")),
    })
}

/// An ABI enum given as its number or its name.
fn enumerated(value: Option<&Value>, names: &[&str], default: u32) -> Result<u32, String> {
    match value {
        None => Ok(default),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| format!("bad enum value {n}")),
        Some(Value::String(s)) => names
            .iter()
            .position(|name| name == s)
            .map(|i| i as u32)
            .ok_or_else(|| format!("unknown name {s}")),
        Some(other) => Err(format!("bad enum value {other}")),
    }
}

const KINDS: &[&str] = &["finger", "pencil", "pointer"];
const PHASES: &[&str] = &[
    "began",
    "moved",
    "stationary",
    "ended",
    "cancelled",
    "hover_moved",
    "hover_exited",
];
const PROVENANCES: &[&str] = &["actual", "coalesced", "predicted", "estimation_update"];
const PROPERTIES: &[(&str, u32)] = &[
    ("force", mask::FORCE),
    ("altitude", mask::ALTITUDE),
    ("azimuth", mask::AZIMUTH),
    ("roll", mask::ROLL),
    ("hover_z", mask::HOVER_Z),
    ("location", mask::LOCATION),
];

fn property_mask(value: Option<&Value>) -> u32 {
    let names: Vec<&str> = value
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    PROPERTIES
        .iter()
        .filter(|(name, _)| names.contains(name))
        .fold(0, |bits, (_, bit)| bits | bit)
}

/// One sample: `contact`, `phase`, `t`, `x`, `y` required; `kind` defaults to
/// Pencil and `provenance` to actual. Optional properties are present only
/// when given; `"location": false` clears the location of an estimation update.
pub fn sample(value: &Value) -> Result<RawSample, String> {
    let number = |key: &str| value.get(key).and_then(Value::as_f64);
    let required = |key: &str| number(key).ok_or(format!("sample needs {key}"));
    let mut raw = RawSample {
        contact_id: value
            .get("contact")
            .and_then(Value::as_u64)
            .ok_or("sample needs contact")?,
        kind: enumerated(value.get("kind"), KINDS, 1)?,
        phase: enumerated(value.get("phase"), PHASES, u32::MAX)?,
        provenance: enumerated(value.get("provenance"), PROVENANCES, 0)?,
        timestamp_s: required("t")?,
        x_pt: required("x")?,
        y_pt: required("y")?,
        present: mask::LOCATION,
        estimated: property_mask(value.get("estimated")),
        expects_update: property_mask(value.get("expects_update")),
        tap_count: value
            .get("tap_count")
            .and_then(Value::as_u64)
            .map_or(1, |n| u32::try_from(n).unwrap_or(u32::MAX)),
        ..RawSample::default()
    };
    if value.get("phase").is_none() {
        return Err("sample needs phase".into());
    }
    if value.get("location").and_then(Value::as_bool) == Some(false) {
        raw.present &= !mask::LOCATION;
    }
    optional_properties(value, &mut raw);
    Ok(raw)
}

fn optional_properties(value: &Value, raw: &mut RawSample) {
    let number = |key: &str| value.get(key).and_then(Value::as_f64);
    let fields: [(&str, u32, &mut f64); 6] = [
        ("force", mask::FORCE, &mut raw.force),
        ("max_force", mask::FORCE, &mut raw.max_force),
        ("altitude", mask::ALTITUDE, &mut raw.altitude_rad),
        ("azimuth", mask::AZIMUTH, &mut raw.azimuth_rad),
        ("roll", mask::ROLL, &mut raw.roll_rad),
        ("hover_z", mask::HOVER_Z, &mut raw.hover_z),
    ];
    let mut present = 0;
    for (key, bit, field) in fields {
        if let Some(n) = number(key) {
            *field = n;
            present |= bit;
        }
    }
    raw.present |= present;
    if let Some(index) = value.get("estimation_index").and_then(Value::as_u64) {
        raw.estimation_index = index;
        raw.present |= mask::ESTIMATION_INDEX;
    }
}
