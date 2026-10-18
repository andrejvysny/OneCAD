pub use onecad_app::api::*;
use onecad_app::{
    autosave, document_runtime::DocumentRuntime, dto::*, error::ApiError, state::AppState,
};
use onecad_core::edit::{EditCommand, SketchEditOp};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub async fn new_document(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DocumentSnapshotDto, ApiError> {
    onecad_app::api::new_document(&state, crate::host::context(&app)).await
}

#[tauri::command]
pub async fn open_document(
    state: State<'_, AppState>,
    app: AppHandle,
    path: String,
    on_recovery: Option<String>,
) -> Result<DocumentSnapshotDto, ApiError> {
    onecad_app::api::open_document(&state, crate::host::context(&app), path, on_recovery).await
}

#[tauri::command]
pub async fn import_step(
    state: State<'_, AppState>,
    app: AppHandle,
    path: String,
) -> Result<DocumentSnapshotDto, ApiError> {
    onecad_app::api::import_step(&state, crate::host::context(&app), path).await
}

#[tauri::command]
pub async fn insert_step(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<DocumentProjection>, ApiError> {
    onecad_app::api::insert_step(&state, crate::host::context(&app)).await
}

#[tauri::command]
pub async fn import_project(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<DocumentSnapshotDto>, ApiError> {
    onecad_app::api::import_project(&state, crate::host::context(&app)).await
}

#[tauri::command]
pub async fn save_document(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
    preview_png: Option<String>,
) -> Result<SaveOutcomeDto, ApiError> {
    onecad_app::api::save_document(&state, crate::host::context(&app), path, preview_png).await
}

#[tauri::command]
pub async fn export_step_file(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
) -> Result<Option<String>, ApiError> {
    onecad_app::api::export_step_file(&state, crate::host::context(&app), path).await
}

#[tauri::command]
pub async fn export_stl_file(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
) -> Result<Option<String>, ApiError> {
    onecad_app::api::export_stl_file(&state, crate::host::context(&app), path).await
}

#[tauri::command]
pub async fn export_obj_file(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
) -> Result<Option<String>, ApiError> {
    onecad_app::api::export_obj_file(&state, crate::host::context(&app), path).await
}

#[tauri::command]
pub async fn export_3mf_file(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
) -> Result<Option<String>, ApiError> {
    onecad_app::api::export_3mf_file(&state, crate::host::context(&app), path).await
}

#[tauri::command]
pub async fn close_document(state: State<'_, AppState>, app: AppHandle) -> Result<(), ApiError> {
    onecad_app::api::close_document(&state, crate::host::context(&app)).await
}

#[tauri::command]
pub async fn confirm_exit(app: AppHandle) {
    let document_id = {
        let state = app.state::<AppState>();
        let guard = state.runtime.lock().await;
        guard.as_ref().map(DocumentRuntime::document_uuid)
    };
    if let (Some(id), Some(root)) = (
        document_id,
        autosave::autosave_root(&crate::host::context(&app)),
    ) {
        autosave::clear_recovery_state(&root, id);
    }
    // The single funnel every real quit passes through (⌘Q and the window-close
    // button both route here via `events::CLOSE_REQUESTED`), so this is where both
    // worker slots are retired — once, and before the event loop unwinds.
    app.state::<AppState>().retire_all();
    app.state::<crate::ExitGuard>().clear();
    app.exit(0);
}

#[tauri::command]
pub async fn cancel_exit(app: AppHandle) {
    app.state::<crate::ExitGuard>().clear();
}

#[tauri::command]
pub async fn check_recovery(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<RecoveryInfoDto>, ApiError> {
    onecad_app::api::check_recovery(&state, crate::host::context(&app)).await
}

#[tauri::command]
pub async fn recover_document(
    state: State<'_, AppState>,
    app: AppHandle,
    document_id: String,
    accept: bool,
) -> Result<Option<DocumentSnapshotDto>, ApiError> {
    onecad_app::api::recover_document(&state, crate::host::context(&app), document_id, accept).await
}

#[tauri::command]
pub async fn apply_edit_command(
    state: State<'_, AppState>,
    app: AppHandle,
    command: EditCommand,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::apply_edit_command(&state, crate::host::context(&app), command).await
}

#[tauri::command]
pub async fn apply_operation(
    state: State<'_, AppState>,
    app: AppHandle,
    commands: Vec<EditCommand>,
    effects: Option<OperationEffects>,
) -> Result<ApplyOperationResponse, ApiError> {
    onecad_app::api::apply_operation(&state, crate::host::context(&app), commands, effects).await
}

#[tauri::command]
pub async fn rollback_failed_operation(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
) -> Result<RollbackFailedOperationResponse, ApiError> {
    onecad_app::api::rollback_failed_operation(&state, crate::host::context(&app), token).await
}

#[tauri::command]
pub async fn repair_mate_axis(
    state: State<'_, AppState>,
    app: AppHandle,
    record_id: String,
    keep_world_direction: bool,
    resolved_axis: [f64; 3],
    resolved_sidedness: Option<String>,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::repair_mate_axis(
        &state,
        crate::host::context(&app),
        record_id,
        keep_world_direction,
        resolved_axis,
        resolved_sidedness,
    )
    .await
}

#[tauri::command]
pub async fn get_module_state(
    state: State<'_, AppState>,
    module_id: String,
) -> Result<Option<ModuleStateDto>, ApiError> {
    onecad_app::api::get_module_state(&state, module_id).await
}

#[tauri::command]
pub async fn set_module_state(
    state: State<'_, AppState>,
    module_id: String,
    schema_version: Option<u32>,
    payload: Option<serde_json::Value>,
) -> Result<(), ApiError> {
    onecad_app::api::set_module_state(&state, module_id, schema_version, payload).await
}

#[tauri::command]
pub async fn list_document_modules(
    state: State<'_, AppState>,
) -> Result<Vec<DocumentModuleDto>, ApiError> {
    onecad_app::api::list_document_modules(&state).await
}

#[tauri::command]
pub async fn list_variables(state: State<'_, AppState>) -> Result<Vec<VariableDto>, ApiError> {
    onecad_app::api::list_variables(&state).await
}

#[tauri::command]
pub async fn upsert_variable(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
    value: f64,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::upsert_variable(&state, crate::host::context(&app), name, value).await
}

#[tauri::command]
pub async fn remove_variable(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::remove_variable(&state, crate::host::context(&app), name).await
}

#[tauri::command]
pub async fn upsert_variable_expr(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
    text: String,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::upsert_variable_expr(&state, crate::host::context(&app), name, text).await
}

#[tauri::command]
pub async fn rename_variable(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
    new_name: String,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::rename_variable(&state, crate::host::context(&app), name, new_name).await
}

#[tauri::command]
pub async fn evaluate_expression(
    state: State<'_, AppState>,
    expr: String,
    site: String,
) -> Result<EvaluatedExpressionDto, ApiError> {
    onecad_app::api::evaluate_expression(&state, expr, site).await
}

#[tauri::command]
pub async fn undo(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::undo(&state, crate::host::context(&app)).await
}

#[tauri::command]
pub async fn redo(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::redo(&state, crate::host::context(&app)).await
}

#[tauri::command]
pub async fn get_projection(state: State<'_, AppState>) -> Result<DocumentProjection, ApiError> {
    onecad_app::api::get_projection(&state).await
}

#[tauri::command]
pub async fn get_mesh(
    state: State<'_, AppState>,
    body_id: String,
    lod: String,
    generation: Option<u64>,
    expected_runtime_session: Option<String>,
) -> Result<tauri::ipc::Response, ApiError> {
    onecad_app::api::get_mesh(&state, body_id, lod, generation, expected_runtime_session)
        .await
        .map(tauri::ipc::Response::new)
}

#[tauri::command]
pub async fn enter_sketch(
    state: State<'_, AppState>,
    app: AppHandle,
    sketch_id: String,
    created_here: Option<bool>,
) -> Result<SketchSessionDto, ApiError> {
    onecad_app::api::enter_sketch(&state, crate::host::context(&app), sketch_id, created_here).await
}

#[tauri::command]
pub async fn get_sketch(
    state: State<'_, AppState>,
    sketch_id: String,
) -> Result<SketchSessionDto, ApiError> {
    onecad_app::api::get_sketch(&state, sketch_id).await
}

#[tauri::command]
pub async fn can_fold_transform(
    state: State<'_, AppState>,
    body_id: String,
) -> Result<Option<String>, ApiError> {
    onecad_app::api::can_fold_transform(&state, body_id).await
}

#[tauri::command]
pub async fn get_sketch_regions(
    state: State<'_, AppState>,
    sketch_id: String,
) -> Result<FinishSketchDto, ApiError> {
    onecad_app::api::get_sketch_regions(&state, sketch_id).await
}

#[tauri::command]
pub async fn sketch_upsert(
    state: State<'_, AppState>,
    app: AppHandle,
    sketch_id: String,
    ops: Vec<SketchEditOp>,
) -> Result<SketchUpsertDto, ApiError> {
    onecad_app::api::sketch_upsert(&state, crate::host::context(&app), sketch_id, ops).await
}

#[tauri::command]
pub async fn begin_gesture(
    state: State<'_, AppState>,
    sketch_id: String,
    drag_point: String,
    target: Option<GestureTargetArgs>,
) -> Result<BeginGestureDto, ApiError> {
    onecad_app::api::begin_gesture(&state, sketch_id, drag_point, target).await
}

#[tauri::command]
pub async fn cancel_gesture(state: State<'_, AppState>) -> Result<(), ApiError> {
    onecad_app::api::cancel_gesture(&state).await
}

#[tauri::command]
pub async fn solve_drag(
    state: State<'_, AppState>,
    target: [f64; 2],
) -> Result<DragSolveDto, ApiError> {
    onecad_app::api::solve_drag(&state, target).await
}

#[tauri::command]
pub async fn end_gesture(
    state: State<'_, AppState>,
    app: AppHandle,
    final_target: Option<[f64; 2]>,
) -> Result<SketchUpsertDto, ApiError> {
    onecad_app::api::end_gesture(&state, crate::host::context(&app), final_target).await
}

#[tauri::command]
pub async fn cancel_sketch(
    state: State<'_, AppState>,
    app: AppHandle,
    sketch_id: String,
    discard: Option<bool>,
) -> Result<CancelSketchDto, ApiError> {
    onecad_app::api::cancel_sketch(&state, crate::host::context(&app), sketch_id, discard).await
}

#[tauri::command]
pub async fn finish_sketch(
    state: State<'_, AppState>,
    app: AppHandle,
    sketch_id: String,
) -> Result<FinishSketchDto, ApiError> {
    onecad_app::api::finish_sketch(&state, crate::host::context(&app), sketch_id).await
}

#[tauri::command]
pub async fn promote_selection(
    state: State<'_, AppState>,
    app: AppHandle,
    snapshot_id: u64,
    body_id: String,
    picks: Vec<PickInput>,
    expected_runtime_session: Option<String>,
) -> Result<Vec<PromotedElementDto>, ApiError> {
    onecad_app::api::promote_selection(
        &state,
        crate::host::context(&app),
        snapshot_id,
        body_id,
        picks,
        expected_runtime_session,
    )
    .await
}

#[tauri::command]
pub async fn preview_op(
    state: State<'_, AppState>,
    op: serde_json::Value,
    sketch_id: Option<String>,
    expected_snapshot_id: Option<u64>,
    lod: Option<String>,
) -> Result<crate::dto::PreviewResultDto, ApiError> {
    onecad_app::api::preview_op(&state, op, sketch_id, expected_snapshot_id, lod).await
}

#[tauri::command]
pub async fn face_sketch_plane(
    state: State<'_, AppState>,
    snapshot_id: u64,
    body_id: String,
    element_id: String,
    topo_key: Option<String>,
) -> Result<crate::dto::SketchPlaneDto, ApiError> {
    onecad_app::api::face_sketch_plane(&state, snapshot_id, body_id, element_id, topo_key).await
}

#[tauri::command]
pub async fn add_sketch_on_face(
    state: State<'_, AppState>,
    snapshot_id: u64,
    body_id: String,
    element_id: String,
    topo_key: Option<String>,
    sketch_id: String,
    name: String,
) -> Result<SketchOnFaceDto, ApiError> {
    onecad_app::api::add_sketch_on_face(
        &state,
        snapshot_id,
        body_id,
        element_id,
        topo_key,
        sketch_id,
        name,
    )
    .await
}

#[tauri::command]
pub async fn project_to_sketch(
    state: State<'_, AppState>,
    snapshot_id: u64,
    sketch_id: String,
    mode: String,
    sources: Vec<ProjectionSourceInput>,
) -> Result<crate::dto::ProjectToSketchDto, ApiError> {
    onecad_app::api::project_to_sketch(&state, snapshot_id, sketch_id, mode, sources).await
}

#[tauri::command]
pub async fn update_projection(
    state: State<'_, AppState>,
    sketch_id: String,
) -> Result<crate::dto::ProjectToSketchDto, ApiError> {
    onecad_app::api::update_projection(&state, sketch_id).await
}

#[tauri::command]
pub async fn detach_projection(
    state: State<'_, AppState>,
    sketch_id: String,
    entity_ids: Option<Vec<String>>,
) -> Result<crate::dto::DetachProjectionDto, ApiError> {
    onecad_app::api::detach_projection(&state, sketch_id, entity_ids).await
}

#[tauri::command]
pub async fn element_info(
    state: State<'_, AppState>,
    snapshot_id: u64,
    body_id: String,
    element_id: String,
    topo_key: Option<String>,
    read_fence: Option<crate::dto::GeometryReadFenceDto>,
) -> Result<Option<crate::dto::ElementInfoDto>, ApiError> {
    onecad_app::api::element_info(
        &state,
        snapshot_id,
        body_id,
        element_id,
        topo_key,
        read_fence,
    )
    .await
}

#[tauri::command]
pub async fn query_mass_properties(
    state: State<'_, AppState>,
    body_id: String,
    read_fence: Option<crate::dto::GeometryReadFenceDto>,
) -> Result<crate::dto::MassPropertiesDto, ApiError> {
    onecad_app::api::query_mass_properties(&state, body_id, read_fence).await
}

#[tauri::command]
pub async fn classify_element(
    state: State<'_, AppState>,
    body_id: String,
    element_id: String,
    topo_key: Option<String>,
    read_fence: Option<crate::dto::GeometryReadFenceDto>,
) -> Result<Option<crate::dto::ClassifyElementDto>, ApiError> {
    onecad_app::api::classify_element(&state, body_id, element_id, topo_key, read_fence).await
}

#[tauri::command]
pub async fn query_body_topology(
    state: State<'_, AppState>,
    body_id: String,
) -> Result<crate::dto::BodyTopologyDto, ApiError> {
    onecad_app::api::query_body_topology(&state, body_id).await
}

#[tauri::command]
pub async fn prepare_offset_face(
    state: State<'_, AppState>,
    snapshot_id: u64,
    picked_faces: Vec<OffsetFacePickInput>,
    chain_tangent_faces: bool,
    distance_type: Option<String>,
) -> Result<crate::dto::PrepareOffsetFaceDto, ApiError> {
    onecad_app::api::prepare_offset_face(
        &state,
        snapshot_id,
        picked_faces,
        chain_tangent_faces,
        distance_type,
    )
    .await
}

#[tauri::command]
pub async fn prepare_edge_op(
    state: State<'_, AppState>,
    app: AppHandle,
    snapshot_id: u64,
    mode: String,
    picked_edges: Vec<EdgeOpPickInput>,
    chain_tangent_edges: bool,
) -> Result<crate::dto::PrepareEdgeOpDto, ApiError> {
    onecad_app::api::prepare_edge_op(
        &state,
        crate::host::context(&app),
        snapshot_id,
        mode,
        picked_edges,
        chain_tangent_edges,
    )
    .await
}

#[tauri::command]
pub async fn analyze_edge_op_range(
    state: State<'_, AppState>,
    snapshot_id: u64,
    mode: String,
    picked_edges: Vec<EdgeOpPickInput>,
    chain_tangent_edges: bool,
    range: Option<EdgeOpRangeInput>,
) -> Result<crate::dto::EdgeOpRangeDto, ApiError> {
    onecad_app::api::analyze_edge_op_range(
        &state,
        snapshot_id,
        mode,
        picked_edges,
        chain_tangent_edges,
        range,
    )
    .await
}

#[tauri::command]
pub async fn resolve_refs(
    state: State<'_, AppState>,
    snapshot_id: u64,
    refs: Vec<ResolveRefInput>,
) -> Result<Vec<ResolveRefDto>, ApiError> {
    onecad_app::api::resolve_refs(&state, snapshot_id, refs).await
}

#[tauri::command]
pub async fn get_operation_params(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<serde_json::Value, ApiError> {
    onecad_app::api::get_operation_params(&state, record_id).await
}

#[tauri::command]
pub async fn feature_dependencies(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<FeatureDependenciesDto, ApiError> {
    onecad_app::api::feature_dependencies(&state, record_id).await
}

#[tauri::command]
pub async fn clear_worker_circuit(state: State<'_, AppState>) -> Result<usize, ApiError> {
    onecad_app::api::clear_worker_circuit(&state).await
}

#[tauri::command]
pub async fn list_recents(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<RecentProjectDto>, ApiError> {
    onecad_app::api::list_recents(&state, crate::host::context(&app)).await
}

#[tauri::command]
pub async fn rename_recent_project(
    app: AppHandle,
    path: String,
    new_name: String,
) -> Result<(), ApiError> {
    onecad_app::api::rename_recent_project(crate::host::context(&app), path, new_name).await
}

#[tauri::command]
pub async fn delete_recent_project(app: AppHandle, path: String) -> Result<(), ApiError> {
    onecad_app::api::delete_recent_project(crate::host::context(&app), path).await
}

#[tauri::command]
pub async fn reveal_in_file_manager(path: String) -> Result<(), ApiError> {
    onecad_app::api::reveal_in_file_manager(path).await
}

#[tauri::command]
pub async fn open_file_dialog(app: AppHandle) -> Result<Option<String>, ApiError> {
    onecad_app::api::open_file_dialog(crate::host::context(&app)).await
}

#[tauri::command]
pub async fn import_file_dialog(app: AppHandle) -> Result<Option<String>, ApiError> {
    onecad_app::api::import_file_dialog(crate::host::context(&app)).await
}

#[tauri::command]
pub async fn save_file_dialog(app: AppHandle) -> Result<Option<String>, ApiError> {
    onecad_app::api::save_file_dialog(crate::host::context(&app)).await
}

#[tauri::command]
pub async fn step_file_dialog(app: AppHandle) -> Result<Option<String>, ApiError> {
    onecad_app::api::step_file_dialog(crate::host::context(&app)).await
}

#[tauri::command]
pub async fn pick_component_files(app: AppHandle) -> Result<Vec<String>, ApiError> {
    onecad_app::api::pick_component_files(crate::host::context(&app)).await
}

#[tauri::command]
pub async fn ingest_components(
    app: AppHandle,
    paths: Vec<String>,
    library_root: Option<String>,
    defaults: crate::dto::IngestDefaultsDto,
) -> Result<crate::dto::IngestComponentsReportDto, ApiError> {
    onecad_app::api::ingest_components(crate::host::context(&app), paths, library_root, defaults)
        .await
}

#[tauri::command]
pub fn mesh_render_completed(
    expectation_id: u64,
    document_id: String,
    revision: u64,
    snapshot_id: u64,
    body_id: String,
) {
    onecad_app::api::mesh_render_completed(
        expectation_id,
        document_id,
        revision,
        snapshot_id,
        body_id,
    )
}

#[tauri::command]
pub fn log_event(events: Vec<FeLogEvent>) {
    onecad_app::api::log_event(events)
}
