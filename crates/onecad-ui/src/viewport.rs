use crate::app::App;
use onecad_render::{BodyStyle, RenderStyle, Theme};
use onecad_scene::{Bounds, DVec2, ViewSnapshot, Viewport};
use std::sync::atomic::Ordering;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    crate::sketch_entry::show(app, ui.ctx());
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
            crate::sketch_numeric::capture_viewport(ui.ctx(), rect);
            crate::viewport_widgets::capture_canvas(ui.ctx(), rect, ui.layer_id());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Canvas,
                    true,
                    "Model viewport; select a closed profile",
                )
            });
            crate::automation::name(ui.ctx(), response.id, crate::automation::VIEWPORT);
            let Some(display) = app.doc.display.clone() else {
                crate::sketch_creation::clear_candidate(ui.ctx());
                return;
            };
            let viewport = Viewport {
                width: f64::from(rect.width()),
                height: f64::from(rect.height()),
                scale_factor: render_scale(rect, ui.ctx().pixels_per_point(), app.render_pixel_cap),
            };
            if !app.viewport_ready() {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Viewport unavailable. Save or close the document to recover.",
                    egui::FontId::proportional(16.0),
                    app.theme.ink3,
                );
                return;
            }
            if physical_size(viewport).contains(&0) {
                if let Err(error) = suspend(app, ui.ctx()) {
                    app.fail_viewport(error);
                }
                return;
            }
            navigation(app, ui, &response, viewport, &display);
            if let Err(error) = render(app, ui.ctx(), viewport, &display) {
                app.fail_viewport(error);
                return;
            }
            // A switched-in document never shows the previous document's image.
            if let Some(texture) = app.texture.filter(|_| !app.doc.awaiting_scene) {
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                app.sampled = true;
                crate::input_trace::app(app, "viewport.image_queued", serde_json::json!({"pass": ui.ctx().cumulative_pass_nr(), "frame": ui.ctx().cumulative_frame_nr()}));
            }
            if let Some((_, error)) = &app.resize_refusal {
                let message = format!("Viewport resize refused: {error}. Shrink the window or retry. Save remains available.");
                ui.painter().text(rect.center(), egui::Align2::CENTER_BOTTOM, message,
                    egui::FontId::proportional(13.0), app.theme.ink3);
                let retry = egui::Rect::from_center_size(rect.center() + egui::vec2(0.0, 24.0), egui::vec2(120.0, 30.0));
                if ui.put(retry, egui::Button::new("Retry viewport")).clicked() {
                    app.resize_refusal = None;
                    app.draw_dirty = true;
                    ui.ctx().request_repaint();
                }
                return;
            }
            let Some(view) = drawable_capture(app.doc.view, display.scene.identity(), viewport) else {
                crate::sketch_creation::clear_candidate(ui.ctx());
                app.cancel_sketch_gesture();
                return;
            };
            app.sync_sketch_gesture();
            crate::support_view::axes(app, ui.painter(), rect, &view);
            if crate::sketch_entry::viewport(app, ui, rect, &response, &view, &display) {
                crate::viewport_widgets::show(app, ui, rect, &view);
                return;
            }
            crate::viewport_selection::hover(app, ui, rect, &view, &display);
            if app.sketch_active() {
                if let Some(session) = app.doc.sketch.session.as_ref() {
                    let preview = app.doc.sketch_gesture.preview();
                    let pointer = ui.input(|input| input.pointer.hover_pos());
                    let hover = pointer
                        .filter(|point| {
                            preview.is_none()
                                && !app.doc.sketch_gesture.busy()
                                && crate::viewport_widgets::allows_pointer(ui, rect, *point)
                                && !crate::viewport_widgets::owns_pointer(app, ui, rect)
                        })
                        .and_then(|point| crate::sketch_view::hover_target(rect, &view, session, Some(point)));
                    if hover.is_some() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    crate::sketch_view::paint(ui.painter(), rect, &view, session, preview, hover, &app.theme);
                }
            } else {
                paint_profiles(app, ui, rect, &view, &display);
                crate::item_selection::paint(app, ui, rect, &view);
            }
            if crate::sketch_numeric::editor_open(ui.ctx()) {
                crate::sketch_numeric::viewport(app, ui, rect, &view);
            }
            if crate::input::overlay_owns_keyboard(app, ui.ctx()) {
                crate::sketch_creation::clear_candidate(ui.ctx());
                if crate::constraint_authoring::editor_open(ui.ctx()) {
                    crate::sketch_dimensions::show(app, ui, rect, &view);
                }
                crate::sketch_creation::chain_input::cancel(app, ui.ctx());
                app.cancel_sketch_gesture();
                // Menus and dialogs own input, but the cube and handles stay visible beneath them.
                ui.add_enabled_ui(false, |ui| {
                    crate::viewport_widgets::show(app, ui, rect, &view);
                });
                return;
            }
            let overlay_owns = crate::viewport_widgets::show(app, ui, rect, &view);
            if app.sketch_active() {
                let dimension_owns = !app.doc.sketch_gesture.busy() && crate::sketch_dimensions::show(app, ui, rect, &view);
                if app.doc.sketch_gesture.busy() {
                    crate::sketch_gestures::viewport(app, ui, rect, &response, &view);
                    return;
                }
                crate::sketch_pen_stroke::bar(app, ui, rect, &view);
                if !overlay_owns && !dimension_owns
                    && (crate::sketch_gestures::delivered_inside(app, ui, rect)
                        || crate::sketch_pen_stroke::drawing(ui.ctx())
                        || ui.input(|input| input.pointer.hover_pos()).is_some_and(|position| {
                            crate::viewport_widgets::allows_pointer(ui, rect, position)
                        }))
                {
                    crate::sketch_ui::viewport(app, ui, rect, &response, &view);
                } else {
                    crate::sketch_creation::clear_candidate(ui.ctx());
                    crate::sketch_creation::chain_input::cancel(app, ui.ctx());
                }
                return;
            }
            let mode = crate::model_pick::track(ui, rect);
            let double = response.double_clicked()
                || response.clicked() && crate::viewport_widgets::pencil_double_tap(ui.ctx());
            if (response.clicked() || response.double_clicked())
                && !overlay_owns
                && !crate::viewport_selection::sectioned(app)
                && app.doc.bridge.valid()
                && !app.busy()
                && crate::model_pick::allowed(app)
                && let Some(position) = response.interact_pointer_pos()
                && crate::viewport_widgets::allows_pointer(ui, rect, position)
            {
                crate::model_pick::pick_with_tolerance(
                    app,
                    &display,
                    &view,
                    DVec2::new(
                        f64::from(position.x - rect.min.x),
                        f64::from(position.y - rect.min.y),
                    ),
                    double,
                    mode,
                    crate::viewport_widgets::edge_tolerance(ui.ctx()),
                );
            }
        });
}

fn drawable_capture(
    view: Option<ViewSnapshot>,
    scene: onecad_scene::SceneIdentity,
    viewport: Viewport,
) -> Option<ViewSnapshot> {
    view.filter(|view| view.identity().scene == scene && view.viewport() == viewport)
}

/// Physical pixels per point for the scene image: the display scale, reduced
/// uniformly when the image would exceed `cap` pixels. Picking and the camera
/// work in points, so only the image resolution and line widths change.
fn render_scale(rect: egui::Rect, pixels_per_point: f32, cap: Option<f64>) -> f64 {
    let full = f64::from(pixels_per_point);
    let points = f64::from(rect.width()) * f64::from(rect.height());
    match cap {
        Some(cap) if points > 0.0 && points * full * full > cap => (cap / points).sqrt(),
        _ => full,
    }
}

fn physical_size(viewport: Viewport) -> [u32; 2] {
    [
        (viewport.width * viewport.scale_factor).round() as u32,
        (viewport.height * viewport.scale_factor).round() as u32,
    ]
}

fn navigation(
    app: &mut App,
    ui: &egui::Ui,
    response: &egui::Response,
    viewport: Viewport,
    display: &crate::bridge::Display,
) {
    if app.doc.fit_pending {
        let bounds = display.scene.bounds().or_else(|| profile_bounds(display));
        if let Some(bounds) = bounds
            && let Ok(fit) = onecad_scene::fit_camera(bounds, app.doc.camera, viewport, None)
        {
            app.doc.camera.target = fit.target;
            app.doc.camera.offset = app.doc.camera.offset.normalize() * fit.distance;
            app.draw_dirty = true;
        }
        app.doc.fit_pending = false;
    }
    navigate_input(app, ui, response, viewport, display.scene.identity());
    if app.doc.view.is_none_or(|view| {
        view.camera() != app.doc.camera
            || view.viewport() != viewport
            || view.identity().scene != display.scene.identity()
    }) {
        app.draw_dirty = true;
    }
}

fn navigate_input(
    app: &mut App,
    ui: &egui::Ui,
    response: &egui::Response,
    viewport: Viewport,
    scene: onecad_scene::SceneIdentity,
) {
    let cursor = ui.input(|input| input.pointer.hover_pos()).map(|position| {
        let position = position - response.rect.min;
        DVec2::new(f64::from(position.x), f64::from(position.y))
    });
    // Floating chrome sits in higher layers; the camera must not react beneath it.
    let over_chrome = ui
        .input(|input| input.pointer.hover_pos())
        .is_none_or(|position| {
            !crate::viewport_widgets::allows_pointer(ui, response.rect, position)
        });
    let allowed = !crate::input::navigation_blocked(app, ui.ctx())
        && !crate::sketch_dimensions::owns_pointer(app, ui)
        && crate::viewport_widgets::gesture_allowed(ui)
        && ui.input(|input| !input.pointer.primary_down())
        && !crate::viewport_widgets::owns_pointer(app, ui, response.rect)
        && !egui::Popup::is_any_open(ui.ctx())
        && ui.is_enabled();
    ui.input(|input| {
        for event in &input.events {
            let motion = app
                .doc
                .navigation
                .event(event, input.time, viewport.height as f32);
            if let Some(mut motion) = motion
                && allowed
                && !over_chrome
                && response.hovered()
            {
                if let crate::navigation::Motion::Orbit(delta) = &mut motion
                    && !cfg!(any(target_os = "macos", target_os = "ios"))
                {
                    *delta = -*delta;
                }
                app.draw_dirty |= crate::camera_navigation::apply(
                    &mut app.doc.camera,
                    viewport,
                    scene,
                    cursor,
                    motion,
                );
            }
        }
    });
    // A drag that started on the viewport keeps running over chrome.
    if allowed && (response.dragged() || !over_chrome) {
        navigate_drag(app, ui, response, viewport, scene);
    }
}

fn navigate_drag(
    app: &mut App,
    ui: &egui::Ui,
    response: &egui::Response,
    viewport: Viewport,
    scene: onecad_scene::SceneIdentity,
) {
    let secondary = response.dragged_by(egui::PointerButton::Secondary);
    let middle = response.dragged_by(egui::PointerButton::Middle);
    if !secondary && !middle {
        return;
    }
    let delta = ui.input(|input| input.pointer.delta());
    let motion = if secondary && !middle && ui.input(|input| input.modifiers.shift) {
        crate::navigation::Motion::Orbit(delta)
    } else {
        crate::navigation::Motion::Pan(delta)
    };
    app.draw_dirty |=
        crate::camera_navigation::apply(&mut app.doc.camera, viewport, scene, None, motion);
}

fn render(
    app: &mut App,
    ctx: &egui::Context,
    viewport: Viewport,
    display: &crate::bridge::Display,
) -> Result<(), String> {
    let support_grid = crate::support_view::grid(app);
    let grid_visible = crate::support_view::grid_visible(ctx);
    let grid_id = egui::Id::unique("viewport-support-grid");
    ctx.data_mut(|data| {
        if data.get_temp::<(Option<onecad_render::GridFrame>, bool)>(grid_id)
            != Some((support_grid, grid_visible))
        {
            app.draw_dirty = true;
            data.insert_temp(grid_id, (support_grid, grid_visible));
        }
    });
    let needs_scene = app.renderer.scene_identity() != Some(display.scene.identity());
    // Poll even without new drawing: completion callbacks retire resources and acknowledge scenes.
    let renderer_idle = app.renderer.poll_idle().map_err(|e| e.to_string())?;
    if app.doc.awaiting_scene {
        if !needs_scene {
            app.doc.awaiting_scene = false;
        } else if display.prepared.lock().map_or(true, |p| p.is_none()) {
            // Switched-in document: its re-prepared scene is on its way, and its
            // arrival (or failure) wakes the UI; nothing to draw until then.
            return Ok(());
        }
    }
    if let Ok(mut budget) = app.doc.bridge.shared.budget.lock() {
        *budget = app.renderer.preparation_budget();
    }
    if !renderer_idle {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        return Ok(());
    }
    if !app.draw_dirty
        && !needs_scene
        && app.doc.pending_preview.is_none()
        && !app.doc.clear_preview
        && app.pending_display_options.is_none()
    {
        crate::viewport_selection::upload(app, &display.scene, ctx)?;
        return Ok(());
    }
    if !app.sampling_done.load(Ordering::Acquire) {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        return Ok(());
    }
    if !crate::viewport_updates::apply(app, display)? {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        return Ok(());
    }
    let size = physical_size(viewport);
    if app.renderer.size() != size || !app.renderer.has_targets() || app.resize_refusal.is_some() {
        if app
            .resize_refusal
            .as_ref()
            .is_some_and(|(rejected, _)| *rejected == size)
        {
            return Ok(());
        }
        let result = app.renderer.resize(size).or_else(|error| {
            if error.recoverable_target_refusal() {
                app.renderer.resize_reclaiming(size)
            } else {
                Err(error)
            }
        });
        match result {
            Ok(()) => {
                app.resize_refusal = None;
                replace_texture(app);
            }
            Err(error) if error.recoverable_target_refusal() => {
                app.resize_refusal = Some((size, error.to_string()));
                app.doc.view = None;
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let presentation = crate::viewport_capture::Presentation {
        options: *app.renderer.options(),
        dark: app.dark,
        preview: app.renderer.preview_identity(),
    };
    let view = crate::viewport_capture::next(
        app.doc.view,
        app.doc.view_presentation,
        crate::viewport_capture::Request {
            scene: display.scene.identity(),
            camera: app.doc.camera,
            viewport,
            presentation,
        },
        &mut app.view_number,
    )?;
    for body in display.scene.bodies() {
        app.renderer.set_body_style(
            body.body_id(),
            BodyStyle {
                selected: crate::item_selection::whole_body_selected(app, body.body_id()),
            },
        );
    }
    crate::viewport_selection::upload(app, &display.scene, ctx)?;
    let mut encoder = app
        .gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("native CAD viewport"),
        });
    if app
        .renderer
        .encode(
            &mut encoder,
            &view,
            RenderStyle {
                theme: if app.dark { Theme::Dark } else { Theme::Light },
                support_grid,
                grid_visible,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?
    {
        app.gpu.queue.submit([encoder.finish()]);
        app.doc.bridge.shared.input_trace.gpu_submitted(
            &app.gpu.queue,
            display,
            &view,
            app.renderer.preview_identity().is_some(),
        );
        if app.renderer.preview_identity().is_none() {
            app.doc.bridge.render_submitted(&app.gpu.queue, display);
        }
        app.cad_needs_fence = true;
        ctx.request_repaint();
    }
    if app.texture.is_none()
        && let Some(view) = app.renderer.output_view()
    {
        app.texture = Some(app.gpu.renderer.write().register_native_texture(
            &app.gpu.device,
            view,
            wgpu::FilterMode::Linear,
        ));
    }
    app.doc.view = Some(view);
    app.doc.view_presentation = Some(presentation);
    if let Ok(mut budget) = app.doc.bridge.shared.budget.lock() {
        *budget = app.renderer.preparation_budget();
    }
    app.draw_dirty = false;
    Ok(())
}

pub(crate) fn replace_texture(app: &mut App) {
    let next = app.renderer.output_view().map(|view| {
        app.gpu.renderer.write().register_native_texture(
            &app.gpu.device,
            view,
            wgpu::FilterMode::Linear,
        )
    });
    let previous = std::mem::replace(&mut app.texture, next);
    if let Some(previous) = previous {
        app.gpu.renderer.write().free_texture(&previous);
    }
}

fn suspend(app: &mut App, ctx: &egui::Context) -> Result<(), String> {
    if !app
        .renderer
        .poll_idle()
        .map_err(|error| error.to_string())?
        || !app.sampling_done.load(Ordering::Acquire)
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        return Ok(());
    }
    app.renderer
        .resize([0, 0])
        .map_err(|error| error.to_string())?;
    replace_texture(app);
    app.doc.view = None;
    app.draw_dirty = true;
    Ok(())
}

fn paint_profiles(
    app: &App,
    ui: &egui::Ui,
    rect: egui::Rect,
    view: &ViewSnapshot,
    display: &crate::bridge::Display,
) {
    if app.renderer.preview_identity().is_some() {
        return;
    }
    for profile in &display.profiles {
        let selected = app
            .doc
            .selection
            .profiles()
            .chain(app.doc.interaction.selection.as_ref())
            .any(|selection| {
                selection.profile.sketch == profile.reference.sketch
                    && selection.profile.region == profile.reference.region
            });
        let color = if selected {
            app.theme.accent.linear_multiply(0.42)
        } else {
            app.theme.accent.linear_multiply(0.20)
        };
        let projected: Vec<_> = profile
            .vertices
            .iter()
            .map(|vertex| {
                view.project(*vertex).map(|point| {
                    rect.min + egui::vec2(point.logical.x as f32, point.logical.y as f32)
                })
            })
            .collect();
        ui.painter()
            .add(profile_fill(&projected, &profile.triangles, color));
        for edge in &profile.boundary {
            if let (Some(a), Some(b)) = (
                view.project(profile.vertices[edge[0] as usize]),
                view.project(profile.vertices[edge[1] as usize]),
            ) {
                let logical = |point: onecad_scene::ProjectedPoint| {
                    rect.min + egui::vec2(point.logical.x as f32, point.logical.y as f32)
                };
                ui.painter().line_segment(
                    [logical(a), logical(b)],
                    egui::Stroke::new(1.2, app.theme.accent),
                );
            }
        }
    }
}

fn profile_fill(
    points: &[Option<egui::Pos2>],
    triangles: &[[u32; 3]],
    color: egui::Color32,
) -> egui::Mesh {
    // Independent stroked triangles introduce internal edges and acute-join spikes.
    let mut mesh = egui::Mesh::default();
    for point in points {
        mesh.colored_vertex(point.unwrap_or(egui::Pos2::ZERO), color);
    }
    for triangle in triangles {
        if triangle
            .iter()
            .all(|index| points.get(*index as usize).is_some_and(Option::is_some))
        {
            mesh.add_triangle(triangle[0], triangle[1], triangle[2]);
        }
    }
    mesh
}

pub(crate) fn pick_policy(mode: onecad_render::DisplayMode) -> onecad_scene::PickPolicy {
    match mode {
        onecad_render::DisplayMode::Wireframe => onecad_scene::PickPolicy::EdgesOnly,
        _ => onecad_scene::PickPolicy::FacesAndEdges,
    }
}

pub(crate) fn profile_visible(body: Option<&onecad_scene::PickHit<'_>>, distance: f64) -> bool {
    body.is_none_or(|body| body.ray_distance >= distance - 1e-6)
}

fn profile_bounds(display: &crate::bridge::Display) -> Option<Bounds> {
    display
        .profiles
        .iter()
        .map(|profile| profile.bounds)
        .reduce(|a, b| Bounds {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
        })
}

#[cfg(test)]
#[path = "viewport_tests.rs"]
mod tests;

#[cfg(test)]
mod render_scale_tests {
    use super::render_scale;

    fn rect(w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))
    }

    #[test]
    fn display_scale_is_kept_without_cap_or_below_it() {
        assert_eq!(render_scale(rect(1032.0, 1376.0), 2.0, None), 2.0);
        assert_eq!(
            render_scale(rect(100.0, 100.0), 2.0, Some(3_000_000.0)),
            2.0
        );
    }

    #[test]
    fn large_viewports_are_reduced_to_the_cap() {
        let scale = render_scale(rect(1032.0, 1332.0), 2.0, Some(3_000_000.0));
        let pixels = 1032.0 * scale * 1332.0 * scale;
        assert!(scale < 2.0);
        assert!((pixels - 3_000_000.0).abs() < 1.0, "{pixels}");
    }
}
