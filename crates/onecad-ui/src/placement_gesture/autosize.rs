//! Fastener auto-size: a hole, rim or bore picks the largest thread that fits it.
use super::snap::{Candidate, Geometry};
use crate::library_catalog::draft::{Field, Kind};
use onecad_core::document::record::ComponentParamValue as Value;
use onecad_library::tables::Iso4762Table;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const THREAD: &str = "thread";
const FIT_SLACK_MM: f64 = 1e-6;

/// The largest listed ISO 4762 size whose nominal diameter fits a hole of `radius`.
pub(crate) fn thread_for_radius<'a>(
    domain: impl Iterator<Item = &'a str>,
    radius: f64,
) -> Option<&'a str> {
    domain
        .filter_map(|name| Some((name, Iso4762Table::row(name)?.d1)))
        .filter(|(_, d1)| *d1 <= 2.0 * radius + FIT_SLACK_MM)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| name)
}

/// The complete parameter map to send when `candidate` implies a different thread.
/// None when nothing changes: the user typed `thread`, the target is a pin (a shaft
/// is not a hole), or the package has no ISO-sized `thread` parameter.
pub(crate) fn resized(
    thread: Option<&Field>,
    params: &BTreeMap<String, Value>,
    manual: &BTreeSet<String>,
    candidate: &Candidate,
) -> Option<BTreeMap<String, Value>> {
    let field = thread.filter(|f| f.key == THREAD && f.is_free() && f.kind == Kind::Text)?;
    if manual.contains(THREAD) || candidate.sidedness.as_deref() == Some("pin") {
        return None;
    }
    if !matches!(candidate.geometry, Geometry::Rim | Geometry::Cylinder) {
        return None;
    }
    let domain: Vec<String> = field.domain.iter().map(|d| d.text()).collect();
    let fit = thread_for_radius(domain.iter().map(String::as_str), candidate.radius?)?;
    let next = Value::Text(fit.to_owned());
    if params.get(THREAD) == Some(&next) {
        return None;
    }
    let mut map = params.clone();
    map.insert(THREAD.to_owned(), next);
    Some(map)
}
