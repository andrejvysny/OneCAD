//! Native QA host over shared OneCAD application, interaction, scene and render services.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
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
mod transform_ui;
mod ui_state;
mod viewport;
mod viewport_capture;
mod viewport_options;
mod viewport_selection;
mod viewport_updates;
mod viewport_widgets;
mod workspace_ui;

use onecad_widgets::{fonts, theme};

pub fn run() -> Result<(), String> {
    let paths = qa::QaPaths::from_args(std::env::args_os().skip(1))?;
    paths.require_launch_environment()?;
    let _profile_lifetime = paths.clone();
    let _logging = onecad_app::logging::init();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("OneCAD")
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

#[cfg(test)]
mod app_logic_tests;
mod sketch_candidates;
