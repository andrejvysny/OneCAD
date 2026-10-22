//! Native QA host over shared OneCAD application, interaction, scene and render services.
//! With the default `desktop` feature it runs under eframe; without it another host
//! (the iPadOS bridge) drives the same editor through [`host::Editor`].
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
// Test helpers serve suites that need the eframe/kittest harness (`desktop`).
#![cfg_attr(all(test, not(feature = "desktop")), allow(dead_code))]
mod app;
mod appearance;
mod bars;
#[cfg(test)]
mod body_inspection_tests_support;
mod bridge;
mod budget;
mod camera_navigation;
mod capture;
mod component_wizard;
mod constraint_authoring;
mod constraints_menu;
mod diagnostics;
#[cfg(feature = "desktop")]
mod dialogs;
mod dimension_chip;
mod display_menu;
mod document_editor;
mod document_flow;
mod document_items;
mod document_services;
mod document_state;
mod documents;
mod editor_facade;
mod event_mailbox;
mod explorer;
mod extension_pages;
mod extrude_handle;
mod extrude_ui;
mod feature_card;
mod gesture_services;
mod gizmo;
mod handoff;
pub mod host;
mod input;
mod input_trace;
mod inspector;
mod inspector_ext;
mod inspector_history;
mod inspector_props;
mod item_selection;
mod layout;
mod library_pages;
mod menu;
mod model_pick;
mod navigation;
mod navigation_cube;
mod operand_services;
mod outcome;
mod overlays;
mod parameter_drag;
mod parameter_ui;
mod pill;
mod profile_capture;
mod profile_diagnostics;
mod profiles;
mod projection_budget;
mod provisional;
mod qa;
mod record_budget;
mod render_receipt;
mod service_runtime;
mod services;
mod session_strip;
mod settings;
mod shell;
mod sketch_completion;
mod sketch_creation;
mod sketch_dimensions;
mod sketch_entry;
mod sketch_feedback;
mod sketch_gestures;
mod sketch_methods;
mod sketch_numeric;
mod sketch_pen_stroke;
mod sketch_repair;
mod sketch_services;
mod sketch_snapping;
mod sketch_ui;
mod sketch_view;
mod start_screen;
mod startup_recovery;
mod support_view;
mod tab_switcher;
mod tool_registry;
#[cfg(not(feature = "desktop"))]
mod touch_menu;
mod transform_ui;
mod ui_state;
mod viewport;
mod viewport_capture;
mod viewport_options;
mod viewport_selection;
mod viewport_updates;
mod viewport_widgets;
mod workspace_ui;

pub use qa::{HostLayout, QaPaths};

use onecad_widgets::{fonts, theme};

#[cfg(feature = "desktop")]
pub fn run() -> Result<(), String> {
    let paths = qa::QaPaths::from_args(std::env::args_os().skip(1))?;
    paths.require_launch_environment()?;
    let _profile_lifetime = paths.clone();
    let _logging = onecad_app::logging::init();
    let icon = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/native/app-icons/icon.png"
    ))
    .map_err(|error| format!("app icon: {error}"))?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("OneCAD")
        .with_icon(icon)
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([900.0, 560.0]);
    if cfg!(target_os = "macos") {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: viewport_gpu_configuration(),
        ..Default::default()
    };
    eframe::run_native(
        "OneCAD",
        options,
        Box::new(move |context| {
            app::App::new(context, paths)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
                .map_err(Into::into)
        }),
    )
    .map_err(|error| error.to_string())?;
    tracing::info!("Native host exited cleanly");
    Ok(())
}

#[cfg(feature = "desktop")]
fn viewport_gpu_configuration() -> egui_wgpu::WgpuConfiguration {
    let mut configuration = egui_wgpu::WgpuConfiguration::default();
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut configuration.wgpu_setup {
        let descriptor = setup.device_descriptor.clone();
        setup.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut descriptor = descriptor(adapter);
            descriptor.required_features |=
                onecad_render::required_device_features(adapter.features());
            descriptor
        });
    }
    configuration
}

#[cfg(all(test, feature = "desktop"))]
mod app_logic_tests;
mod sketch_candidates;
