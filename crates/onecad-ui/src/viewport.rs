use crate::app::App;
use onecad_interaction::{Action, ProfileSelection};
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
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Image,
                    true,
                    "Model viewport; select a closed profile",
                )
            });
            let Some(display) = app.display.clone() else {
                crate::sketch_creation::clear_candidate(ui.ctx());
                return;
            };
            let viewport = Viewport {
                width: f64::from(rect.width()),
                height: f64::from(rect.height()),
                scale_factor: f64::from(ui.ctx().pixels_per_point()),
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
            if let Some(texture) = app.texture {
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
            let Some(view) = drawable_capture(app.view, display.scene.identity(), viewport) else {
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
                if let Some(session) = app.sketch.session.as_ref() {
                    let preview = app.sketch_gesture.preview();
                    let pointer = ui.input(|input| input.pointer.hover_pos());
                    let hover = pointer
                        .filter(|point| {
                            preview.is_none()
                                && !app.sketch_gesture.busy()
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
                return;
            }
            let overlay_owns = crate::viewport_widgets::show(app, ui, rect, &view);
            if app.sketch_active() {
                let dimension_owns = !app.sketch_gesture.busy() && crate::sketch_dimensions::show(app, ui, rect, &view);
                if app.sketch_gesture.busy() {
                    crate::sketch_gestures::viewport(app, ui, rect, &response, &view);
                    return;
                }
                if !overlay_owns && !dimension_owns
                    && (crate::sketch_gestures::delivered_inside(app, ui, rect)
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
            if (response.clicked() || response.double_clicked())
                && !overlay_owns
                && !crate::viewport_selection::sectioned(app)
                && app.bridge.valid()
                && !app.busy()
                && app.interaction.tool.is_none()
                && let Some(position) = response.interact_pointer_pos()
                && crate::viewport_widgets::allows_pointer(ui, rect, position)
            {
                pick(
                    app,
                    &display,
                    &view,
                    DVec2::new(
                        f64::from(position.x - rect.min.x),
                        f64::from(position.y - rect.min.y),
                    ),
                    response.double_clicked(),
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
    if app.fit_pending {
        let bounds = display.scene.bounds().or_else(|| profile_bounds(display));
        if let Some(bounds) = bounds
            && let Ok(fit) = onecad_scene::fit_camera(bounds, app.camera, viewport, None)
        {
            app.camera.target = fit.target;
            app.camera.offset = app.camera.offset.normalize() * fit.distance;
            app.draw_dirty = true;
        }
        app.fit_pending = false;
    }
    navigate_input(app, ui, response, viewport, display.scene.identity());
    if app.view.is_none_or(|view| {
        view.camera() != app.camera
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
                .navigation
                .event(event, input.time, viewport.height as f32);
            if let Some(mut motion) = motion
                && allowed
                && !over_chrome
                && response.hovered()
            {
                if let crate::navigation::Motion::Orbit(delta) = &mut motion
                    && !cfg!(target_os = "macos")
                {
                    *delta = -*delta;
                }
                app.draw_dirty |= crate::camera_navigation::apply(
                    &mut app.camera,
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
        crate::camera_navigation::apply(&mut app.camera, viewport, scene, None, motion);
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
    if let Ok(mut budget) = app.bridge.shared.budget.lock() {
        *budget = app.renderer.preparation_budget();
    }
    if !renderer_idle {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        return Ok(());
    }
    if !app.draw_dirty
        && !needs_scene
        && app.pending_preview.is_none()
        && !app.clear_preview
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
                app.view = None;
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
        app.view,
        app.view_presentation,
        crate::viewport_capture::Request {
            scene: display.scene.identity(),
            camera: app.camera,
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
        app.bridge.shared.input_trace.gpu_submitted(
            &app.gpu.queue,
            display,
            &view,
            app.renderer.preview_identity().is_some(),
        );
        if app.renderer.preview_identity().is_none() {
            app.bridge.render_submitted(&app.gpu.queue, display);
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
    app.view = Some(view);
    app.view_presentation = Some(presentation);
    if let Ok(mut budget) = app.bridge.shared.budget.lock() {
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
    app.view = None;
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
        let selected = app.interaction.selection.as_ref().is_some_and(|selection| {
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

pub(crate) fn pick(
    app: &mut App,
    display: &crate::bridge::Display,
    view: &ViewSnapshot,
    logical: DVec2,
    whole_body: bool,
) {
    let hit = display
        .profiles
        .iter()
        .filter_map(|profile| profile.pick(view, logical).map(|hit| (profile, hit)))
        .min_by(|a, b| a.1.1.total_cmp(&b.1.1));
    let body = onecad_scene::pick_with_policy(
        &display.scene,
        view,
        view.identity(),
        logical,
        pick_policy(app.renderer.options().display_mode),
    )
    .ok()
    .flatten();
    if !whole_body
        && let Some((profile, (anchor, distance))) = hit
        && profile_visible(body.as_ref(), distance)
        && let Some(context) = app.interaction.context.clone()
    {
        let mut reference = profile.reference.clone();
        reference.region_anchor = Some(anchor);
        app.dispatch(Action::SelectProfile(Box::new(ProfileSelection {
            profile: reference,
            context,
            capture: view.identity(),
        })));
        app.status = "Profile selected".into();
        app.ui.selected_sketch = Some(profile.reference.sketch);
        app.selected_body = None;
        app.selected_feature = None;
        app.topology.select(None);
        app.draw_dirty = true;
        return;
    }
    app.dispatch(Action::ClearSelection);
    app.selected_feature = None;
    app.selected_body = body.as_ref().map(|body| body.body_id);
    app.topology.select(body.as_ref());
    if whole_body {
        app.topology.select(None);
    }
    app.draw_dirty = true;
}

pub(crate) fn pick_policy(mode: onecad_render::DisplayMode) -> onecad_scene::PickPolicy {
    match mode {
        onecad_render::DisplayMode::Wireframe => onecad_scene::PickPolicy::EdgesOnly,
        _ => onecad_scene::PickPolicy::FacesAndEdges,
    }
}

fn profile_visible(body: Option<&onecad_scene::PickHit<'_>>, distance: f64) -> bool {
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
