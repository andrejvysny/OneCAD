//! Optional captures of the actual production extension renderer, using egui-wgpu.
//! Software Vulkan can inspect 2D layout; this does not qualify Metal or CAD rendering.
use super::{DetailTab, ManagerTab, PresentationContext, Reply, State, render};
use crate::ui_state::StartPage;
use onecad_extension_host::{ExtensionRegistry, PackageState};
use std::{collections::BTreeMap, path::Path};

#[path = "visual_tests/gpu.rs"]
mod gpu;

fn installed_state() -> (tempfile::TempDir, State) {
    let root = tempfile::tempdir().unwrap();
    let mut registry = ExtensionRegistry::open(root.path()).unwrap();
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging/binaries/addons/dev.onecad.bambu-studio");
    assert!(
        package.join("extension.wasm").is_file(),
        "build addons first"
    );
    registry.install_package(&package).unwrap();
    let mut state = State::default();
    state.receive(Reply {
        generation: 1,
        packages: registry.packages(),
        commands: registry.commands(),
        applications: BTreeMap::new(),
        status: String::new(),
        warning: false,
        discovery_errors: Vec::new(),
        review_package: None,
    });
    (root, state)
}

#[tokio::test]
#[ignore = "explicit visual review: requires a wgpu adapter and ONECAD_EXTENSION_CAPTURE_DIR"]
async fn capture_native_extension_surfaces() {
    let output = std::env::var_os("ONECAD_EXTENSION_CAPTURE_DIR")
        .expect("set ONECAD_EXTENSION_CAPTURE_DIR to a capture directory");
    let output = Path::new(&output);
    std::fs::create_dir_all(output).unwrap();
    let gpu = gpu::Gpu::new().await;
    let (_root, state) = installed_state();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        for (name, page, view) in scenarios(&state) {
            let path = output.join(format!("{theme}-{name}.png"));
            if matches!(name, "detail" | "settings" | "consent" | "reset") {
                capture(
                    &gpu,
                    &output.join(format!("{theme}-{name}-narrow.png")),
                    view.clone(),
                    page,
                    dark,
                    [660, 516],
                );
            }
            capture(&gpu, &path, view, page, dark, [1040, 756]);
        }
        capture(
            &gpu,
            &output.join(format!("{theme}-narrow.png")),
            state.clone(),
            StartPage::Extensions,
            dark,
            [660, 516],
        );
    }
}

fn scenarios(state: &State) -> Vec<(&'static str, StartPage, State)> {
    let mut views = vec![("installed", StartPage::Extensions, state.clone())];
    let mut discover = state.clone();
    discover.tab = ManagerTab::Discover;
    views.push(("discover", StartPage::Extensions, discover));
    let mut updates = state.clone();
    updates.tab = ManagerTab::Updates;
    views.push(("updates", StartPage::Extensions, updates));
    let mut detail = state.clone();
    detail.detail = Some(detail.packages[0].manifest.id.clone());
    views.push(("detail", StartPage::ExtensionDetail, detail.clone()));
    detail.detail_tab = DetailTab::Settings;
    views.push(("settings", StartPage::ExtensionDetail, detail.clone()));
    detail.reset_confirmation = true;
    views.push(("reset", StartPage::ExtensionDetail, detail));
    let mut consent = state.clone();
    consent.review = Some(consent.packages[0].clone());
    views.push(("consent", StartPage::Extensions, consent));
    let mut diagnostics = state.clone();
    diagnostics.developer_open = true;
    diagnostics.packages[0].state = PackageState::Failed(
        "The extension exceeded its execution budget. Review permissions to enable it again."
            .into(),
    );
    views.push(("failure", StartPage::Extensions, diagnostics));
    let mut empty = state.clone();
    empty.search = "no matching package".into();
    views.push(("empty-search", StartPage::Extensions, empty));
    let mut long = state.clone();
    long.tab = ManagerTab::Discover;
    long.packages[0].manifest.name =
        "Manufacturing export integration with a deliberately long package title".into();
    long.packages[0].manifest.description =
        "Exports selected bodies and preserves their names and colors. ".repeat(12);
    views.push(("long-content", StartPage::Extensions, long));
    let loading = State {
        attempted: true,
        pending: true,
        pending_request: Some(super::Request::InstallFolder),
        ..Default::default()
    };
    views.push(("loading", StartPage::Extensions, loading));
    views
}

fn capture(
    gpu: &gpu::Gpu,
    path: &Path,
    mut state: State,
    page: StartPage,
    dark: bool,
    size: [u32; 2],
) {
    let ctx = egui::Context::default();
    onecad_widgets::fonts::install(&ctx);
    let theme = onecad_widgets::theme::load(dark).unwrap();
    onecad_widgets::theme::install(&ctx, theme.clone(), dark);
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0] as f32, size[1] as f32),
        )),
        ..Default::default()
    };
    let mut painter = gpu.painter();
    for frame in 0..3 {
        input.time = Some(f64::from(frame) * 0.25);
        let mut output = ctx.run_ui(input.clone(), |ui| {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(theme.surface)
                        .inner_margin(egui::Margin {
                            left: 40,
                            right: 40,
                            top: 32,
                            bottom: 16,
                        }),
                )
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("start-content")
                        .show(ui, |ui| {
                            render(
                                ui,
                                &mut state,
                                page,
                                PresentationContext {
                                    idle: true,
                                    document: true,
                                    selection: false,
                                },
                            );
                        });
                });
        });
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                painter.update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        if frame == 2 {
            let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
            gpu.save(&mut painter, &jobs, size, path);
        }
        for id in &output.textures_delta.free {
            painter.free_texture(id);
        }
        output.textures_delta.clear();
    }
}
