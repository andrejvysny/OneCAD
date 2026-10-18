pub use onecad_app::library::*;
use onecad_app::{dto::*, error::ApiError, state::AppState};
use onecad_core::document::record::{ComponentParamValue, ComponentSourceRef, TransformRotation};
use std::collections::BTreeMap;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn list_library_components(app: AppHandle) -> Result<Vec<LibraryComponentDto>, ApiError> {
    onecad_app::library::list_library_components(crate::host::context(&app)).await
}

#[tauri::command]
pub async fn reindex_library(app: AppHandle) -> Result<ReindexReportDto, ApiError> {
    onecad_app::library::reindex_library(crate::host::context(&app)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn place_component(
    app: AppHandle,
    state: State<'_, AppState>,
    component_id: String,
    component_version: String,
    translate: [f64; 3],
    rotate: Option<TransformRotation>,
    params: Option<BTreeMap<String, ComponentParamValue>>,
    mate: Option<PlaceComponentMateInput>,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::library::place_component(
        crate::host::context(&app),
        &state,
        component_id,
        component_version,
        translate,
        rotate,
        params,
        mate,
    )
    .await
}

#[tauri::command]
pub async fn resolve_component_source(
    app: AppHandle,
    state: State<'_, AppState>,
    component_id: String,
    component_version: String,
) -> Result<ComponentSourceRef, ApiError> {
    onecad_app::library::resolve_component_source(
        crate::host::context(&app),
        &state,
        component_id,
        component_version,
    )
    .await
}

#[tauri::command]
pub async fn set_component_params(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: String,
    params: BTreeMap<String, ComponentParamValue>,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::library::set_component_params(crate::host::context(&app), &state, record_id, params)
        .await
}

#[tauri::command]
pub async fn component_preview_mesh(
    app: AppHandle,
    state: State<'_, AppState>,
    component_id: String,
    component_version: String,
    params: Option<BTreeMap<String, ComponentParamValue>>,
) -> Result<crate::dto::PreviewResultDto, ApiError> {
    onecad_app::library::component_preview_mesh(
        crate::host::context(&app),
        &state,
        component_id,
        component_version,
        params,
    )
    .await
}

#[tauri::command]
pub async fn list_templates(app: AppHandle) -> Result<Vec<ProjectTemplateDto>, ApiError> {
    onecad_app::library::list_templates(crate::host::context(&app)).await
}

#[tauri::command]
pub async fn save_as_template(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
    description: Option<String>,
    preview_png: Option<String>,
) -> Result<ProjectTemplateDto, ApiError> {
    onecad_app::library::save_as_template(
        crate::host::context(&app),
        &state,
        id,
        name,
        description,
        preview_png,
    )
    .await
}

#[tauri::command]
pub async fn new_from_template(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::dto::DocumentSnapshotDto, ApiError> {
    onecad_app::library::new_from_template(crate::host::context(&app), &state, id).await
}

#[tauri::command]
pub async fn save_as_component(
    app: AppHandle,
    state: State<'_, AppState>,
    body_id: String,
    spec: NewComponentSpec,
    preview_png: Option<String>,
    union_solids: Option<bool>,
) -> Result<LibraryComponentDto, ApiError> {
    onecad_app::library::save_as_component(
        crate::host::context(&app),
        &state,
        body_id,
        spec,
        preview_png,
        union_solids,
    )
    .await
}

#[tauri::command]
pub async fn replace_component(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: String,
    component_id: String,
    component_version: String,
    params: Option<BTreeMap<String, ComponentParamValue>>,
) -> Result<ReplaceComponentResultDto, ApiError> {
    onecad_app::library::replace_component(
        crate::host::context(&app),
        &state,
        record_id,
        component_id,
        component_version,
        params,
    )
    .await
}

#[tauri::command]
pub async fn component_upgrade_available(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: String,
) -> Result<Option<ComponentUpgradeDto>, ApiError> {
    onecad_app::library::component_upgrade_available(crate::host::context(&app), &state, record_id)
        .await
}

#[tauri::command]
pub async fn detach_component(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: String,
) -> Result<DocumentProjection, ApiError> {
    onecad_app::library::detach_component(crate::host::context(&app), &state, record_id).await
}
