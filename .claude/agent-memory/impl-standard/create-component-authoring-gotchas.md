---
name: create-component-authoring-gotchas
description: ONECAD-301 backend: configurations/static_save/attachment_from_pick/place-time re-bake — struct-literal blast radius, id forms, test fixture shape
metadata:
  type: project
---

Adding a field to `Metadata`/`ComponentPackage`/`IndexEntry`/`LibraryComponentDto`/`PlaceComponentRequest` breaks struct literals in onecad-library tests, onecad-app tests (component_ops, component_placement_pipeline, component_instances), `library_ingest.rs`, and two onecad-ui sites (`placement_gesture/mod.rs`, `library_catalog/testing.rs`). `NewComponentSpec` now derives Default; use `..Default::default()`.

`attachment_from_pick` takes the bare uuid body id (what `save_as_component` takes) or `body_<uuid>` (what `api::classify_element` needs) and normalizes.

Sketch constraint dims are NOT variable-expression driven (`regen/variables.rs` covers op Scalars only). To get a variable-driven bracket in tests, drive extrude distances (`Scalar::with_expr`) incl. a derived variable (`width / 2`); a circle-only sketch plus `BooleanMode::Add` fuses a post. Mesh bbox: `rt.get_mesh` + `onecad_protocol::mesh::validate_mesh_blob` (bbox_min/max f32).

The bake applies no re-centring: source-doc world frame == baked solid local frame.

Fenced reads in tests: `attachment_from_pick` needs a fence; a `DocumentRuntime::new_blank` must `bind_geometry_read_query(state.element_query())` first or `prepare_geometry_read` says "provider is not bound".
`Library` publishes packages by rename from `<root>/.tmp-*` (reindex skips dot-dirs); versions must be plain MAJOR.MINOR.PATCH.
