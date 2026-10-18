//! Native QA host over shared OneCAD application, interaction, scene and render services.
mod app;
mod bridge;
mod budget;
mod camera_navigation;
mod capture;
mod event_mailbox;
mod fonts;
mod gizmo;
mod handoff;
mod icons;
mod input;
mod menu;
mod navigation;
mod navigation_cube;
mod outcome;
mod parameter_ui;
mod profiles;
mod projection_budget;
mod provisional;
mod qa;
mod record_budget;
mod render_receipt;
mod services;
mod shell;
mod sketch_completion;
mod sketch_services;
mod sketch_ui;
mod sketch_view;
mod theme;
mod transform_ui;
mod viewport;
mod viewport_options;
mod viewport_selection;
mod viewport_widgets;

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
    .map_err(|error| error.to_string())
}
