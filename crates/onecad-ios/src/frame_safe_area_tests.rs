use super::*;

#[test]
fn installed_theme_opaquely_covers_the_safe_area_background() {
    let context = egui::Context::default();
    for dark in [false, true] {
        let theme = onecad_widgets::theme::load(dark).unwrap();
        let [r, g, b, _] = theme.canvas.to_normalized_gamma_f32();
        onecad_widgets::theme::install(&context, theme, dark);
        assert_eq!(
            surface_clear_color(&context),
            wgpu::Color {
                r: r.into(),
                g: g.into(),
                b: b.into(),
                a: 1.0,
            }
        );
    }
}

#[test]
fn resolved_appearance_flag_uses_installed_editor_theme_not_system_traits() {
    let context = egui::Context::default();
    let existing = crate::ffi::FRAME_REDRAW_REQUESTED
        | crate::ffi::FRAME_TEXT_INPUT_ACTIVE
        | crate::ffi::FRAME_A11Y_CHANGED
        | crate::ffi::FRAME_HOST_REQUESTS
        | crate::ffi::FRAME_KERNEL_TERMINAL
        | crate::ffi::FRAME_PRESENTED;
    assert_eq!(existing, 0x3f);
    assert_eq!(crate::ffi::FRAME_DARK_APPEARANCE, 1 << 6);
    for dark in [false, true] {
        onecad_widgets::theme::install(&context, onecad_widgets::theme::load(dark).unwrap(), dark);
        let system = if dark {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        };
        let mut output = context.run_ui(
            egui::RawInput {
                system_theme: Some(system),
                ..Default::default()
            },
            |_| {},
        );
        output.textures_delta.clear();
        assert_eq!(context.system_theme(), Some(system));
        let flags = appearance_flags(&context);
        assert_eq!(flags, if dark { 1 << 6 } else { 0 });
        assert_eq!((existing | flags) & existing, existing);
    }
}

#[test]
#[ignore = "requires Metal; run with ONECAD_REQUIRE_GPU=1"]
fn portrait_landscape_controls_keep_safe_and_keyboard_insets_in_both_themes() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let gpu = crate::gpu::create_render_state().unwrap();
    assert_eq!(gpu.adapter.get_info().backend, wgpu::Backend::Metal);
    for dark in [false, true] {
        for size_px in [[1640, 2360], [2360, 1640]] {
            for keyboard_overlap in [0.0, 320.0] {
                assert_content_geometry(&gpu, dark, size_px, keyboard_overlap);
            }
        }
    }
}

fn assert_content_geometry(
    gpu: &RenderState,
    dark: bool,
    size_px: [u32; 2],
    keyboard_overlap: f32,
) {
    let context = egui::Context::default();
    onecad_widgets::theme::install(&context, onecad_widgets::theme::load(dark).unwrap(), dark);
    let view = View {
        size_px,
        scale: 2.0,
        safe_area: Insets {
            top: 32.0,
            left: 8.0,
            bottom: 20.0,
            right: 12.0,
        },
        keyboard_overlap,
    };
    let raw = raw_input(
        &view,
        gpu,
        Inputs {
            time: 1.0,
            focused: true,
            theme: None,
            events: vec![],
        },
    );
    let size = egui::vec2(size_px[0] as f32, size_px[1] as f32) / 2.0;
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let safe = egui::Rect::from_min_max(
        egui::pos2(8.0, 32.0),
        egui::pos2(size.x - 12.0, size.y - 20.0f32.max(keyboard_overlap)),
    );
    let mut output = context.run_ui(raw, |ui| {
        assert_eq!(ui.ctx().viewport_rect(), viewport);
        assert_eq!(ui.ctx().content_rect(), safe);
        let button = ui
            .scope_builder(egui::UiBuilder::new().max_rect(safe), |ui| {
                ui.button("Safe-area control")
            })
            .inner;
        assert!(safe.contains_rect(button.rect));
    });
    output.textures_delta.clear();
    assert_eq!(surface_clear_color(&context).a, 1.0);
    assert_eq!(appearance_flags(&context) != 0, dark);
}
