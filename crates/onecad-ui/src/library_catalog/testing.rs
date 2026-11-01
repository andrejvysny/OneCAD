//! Fixtures shared by the catalog and library-page tests.
use super::{Catalog, Reply, draft::ParamDraft};
use onecad_app::dto::{ComponentDetailDto, LibraryComponentDto};
use onecad_library::package::ParameterSpec;
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) fn spec(json: serde_json::Value) -> ParameterSpec {
    serde_json::from_value(json).expect("parameter spec")
}

/// The seeded ISO 4762 signature, reduced to the parts the draft reads.
pub(crate) fn iso_specs() -> BTreeMap<String, ParameterSpec> {
    let threads = ["M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"];
    BTreeMap::from([
        (
            "thread".into(),
            spec(json!({"role": "free", "value": "M6", "domain": threads})),
        ),
        (
            "length".into(),
            spec(json!({"role": "free", "value": 20, "snap": "preferred", "min": 2})),
        ),
        (
            "thread_detail".into(),
            spec(json!({"role": "free", "value": "cosmetic",
                "domain": ["cosmetic", "simplified", "modeled"]})),
        ),
        (
            "pitch".into(),
            spec(json!({"role": "table", "from": "pitch_mm"})),
        ),
    ])
}

pub(crate) fn component(
    id: &str,
    version: &str,
    specs: BTreeMap<String, ParameterSpec>,
) -> LibraryComponentDto {
    LibraryComponentDto {
        id: id.into(),
        version: version.into(),
        name: format!("Part {id}"),
        category: vec!["fasteners".into()],
        tags: vec![],
        source_kind: "generator".into(),
        revision: "sha256:00".into(),
        generator_id: Some("iso4762".into()),
        generator_version: Some(1),
        attachments: BTreeMap::new(),
        parameters: specs,
        designation: Some("ISO 4762 {thread}x{length}".into()),
        standard: Some("ISO 4762".into()),
        unit: Some("mm".into()),
        preview_path: None,
        configurations: BTreeMap::new(),
        default_configuration: None,
    }
}

/// Applies a search reply the way the service lane would.
pub(crate) fn load(catalog: &mut Catalog, hits: Vec<LibraryComponentDto>) {
    catalog.prepare();
    let sequence = catalog.take_outbox()[0].sequence();
    catalog.accept(
        sequence,
        Ok(Reply::Searched(hits)),
        &egui::Context::default(),
    );
}

/// Opens `component` and applies its detail reply.
pub(crate) fn open(catalog: &mut Catalog, component: LibraryComponentDto) {
    catalog.open_detail(&component.id, &component.version);
    let sequence = catalog.take_outbox()[0].sequence();
    let versions = vec![component.version.clone()];
    let detail = ComponentDetailDto {
        component,
        versions,
    };
    catalog.accept(
        sequence,
        Ok(Reply::Detail(Box::new(detail))),
        &egui::Context::default(),
    );
}

pub(crate) fn iso_draft() -> ParamDraft {
    ParamDraft::from_component(&component("iso", "1.0.0", iso_specs()))
}
