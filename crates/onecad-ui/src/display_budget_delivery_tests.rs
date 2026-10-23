//! Render profile scale, process-wide display budget and target release through the real
//! host route, with the production worker and Metal. macOS evidence, not iPad evidence.
use super::model_selection_delivery_tests::{boxed, boxed_with, click, labels, targets};
use super::point_center_delivery_tests::support::Host;
use super::*;
use crate::display_budget::DisplayBudget;
use onecad_render::RenderProfile;
use onecad_scene::PickKind;
use std::sync::Arc;

/// Test-only readback of the renderer's output (RGBA8, rows packed).
fn read_output(host: &Host) -> Vec<u8> {
    let app = &host.app;
    let texture = app.renderer.output_view().unwrap().texture();
    let [width, height] = app.renderer.size();
    let stride = (width * 4).div_ceil(256) * 256;
    let buffer = app.gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test output readback only"),
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = app.gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    app.gpu.queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    app.gpu
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap();
    data.chunks(stride as usize)
        .flat_map(|row| row[..width as usize * 4].iter().copied())
        .collect()
}

/// The output texel under a screen point, through the view's scene scale.
fn texel(host: &Host, image: &[u8], at: egui::Pos2) -> [u8; 3] {
    let view = host.app.doc.view.unwrap();
    let rect = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
    let scale = view.viewport().scale_factor as f32;
    let width = host.app.renderer.size()[0] as usize;
    let x = ((at.x - rect.min.x) * scale) as usize;
    let y = ((at.y - rect.min.y) * scale) as usize;
    let i = (y * width + x) * 4;
    [image[i], image[i + 1], image[i + 2]]
}

fn settle(host: &mut Host) {
    host.redraw();
    for _ in 0..200 {
        host.ui(vec![]);
        if host.app.doc.topology.settled() && !host.app.draw_dirty {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn a_sub_native_scene_image_keeps_picks_and_highlights_aligned() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    host.app.render_scale_limit = 0.5;
    host.redraw();
    let view = host.app.doc.view.unwrap();
    let rect = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
    assert_eq!(
        view.viewport().scale_factor,
        0.5,
        "half the 1x display scale"
    );
    assert_eq!(
        host.app.renderer.size(),
        [rect.width(), rect.height()].map(|v| (f64::from(v) * 0.5).round() as u32)
    );
    let faces = targets(&host, PickKind::Face);
    assert!(faces.len() >= 2, "{faces:?}");
    let (face, id) = faces[0].clone();
    let empty = rect.min + egui::vec2(4.0, 4.0);
    let before = read_output(&host);
    assert_ne!(
        texel(&host, &before, face),
        texel(&host, &before, empty),
        "the picked point shows the body in the scaled image"
    );
    click(&mut host, face, egui::Modifiers::NONE);
    assert_eq!(
        labels(&host),
        std::slice::from_ref(&id),
        "the pick hits the face"
    );
    settle(&mut host);
    assert!(host.app.doc.topology.uploaded >= 1);
    let after = read_output(&host);
    assert_ne!(
        texel(&host, &before, face),
        texel(&host, &after, face),
        "the face highlight covers the picked point in the scaled image"
    );
    host.stop().await;
}

/// The iPadOS route: a Mobile host on a 13-inch iPad viewport (1366x1024 points at 2x),
/// whose native image is over the Mobile scene cap.
async fn ipad(root: &std::path::Path) -> Host {
    let mut host = boxed_with(root, RenderProfile::Mobile).await;
    // The test host reports 1 native pixel per point; zoom makes it 2x. The pass that
    // applies a zoom rescales the previous screen, so the size is set after it.
    host.context.set_zoom_factor(2.0);
    host.ui(vec![]);
    host.size = egui::vec2(1366.0, 1024.0);
    host.redraw();
    assert_eq!(host.context.pixels_per_point(), 2.0);
    assert_eq!(host.app.renderer.profile(), RenderProfile::Mobile);
    host
}

/// Within the Mobile 3 Mpx scene cap (rounding of each side allowed).
fn capped(host: &Host) -> bool {
    let [width, height] = host.app.renderer.size().map(f64::from);
    host.app.scene_scale < 1.0 && width * height <= 3.0e6 + width + height + 1.0
}

fn cache_bytes(doc: &crate::document_editor::DocumentEditor) -> u64 {
    doc.bridge
        .shared
        .preparation_cache
        .lock()
        .unwrap()
        .cpu_bytes()
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn three_documents_and_a_rotation_stay_within_the_display_aggregate() {
    let root = tempfile::tempdir().unwrap();
    let mut host = ipad(root.path()).await;
    let budget = host.app.display_budget;
    assert_eq!(budget, DisplayBudget::for_profile(RenderProfile::Mobile));
    let view = host.app.doc.view.unwrap().viewport();
    let native = [view.width, view.height].map(|v| (v * 2.0).round() as u32);
    assert!(native[0] * native[1] > 5_000_000, "{native:?}");
    assert!(capped(&host), "{:?}", host.app.renderer.size());
    // The cap is what lets a replacement fit: native old + new targets alone would not.
    assert!(2 * host.app.renderer.target_bytes(native).unwrap() > budget.active);
    for parked in 1..=2 {
        host.app.document_request(Request::New);
        pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
            app.tabs.parked.len() == parked && app.document_open() && !app.busy()
        })
        .await;
        host.app.ui.start_open = false;
        host.redraw();
    }
    let ledger = host.app.display_ledger();
    let parked = &host.app.tabs.parked;
    assert!(parked.iter().all(|doc| doc.thumbnail.is_some()));
    assert!(
        cache_bytes(&parked[0]) > 0,
        "the parked box keeps its geometry"
    );
    assert!(ledger.parked > cache_bytes(&parked[0]), "{ledger:?}");
    assert!(ledger.peak_total() <= budget.aggregate, "{ledger:?}");

    // The shared renderer's replacement is charged with old and new targets at once.
    crate::input::fence_previous_frame(&mut host.app, &host.context);
    host.app
        .gpu
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let targets_before = host.app.renderer.memory_estimate().target_bytes;
    let [width, height] = host.app.renderer.size();
    host.app.renderer.resize([height, width]).unwrap();
    crate::viewport::replace_texture(&mut host.app);
    let rotated = host.app.display_ledger();
    assert_eq!(rotated.peak, rotated.resident + targets_before);
    assert!(rotated.peak <= budget.active, "{rotated:?}");
    assert!(rotated.peak_total() <= budget.aggregate, "{rotated:?}");

    // The host's own rotation stays within the aggregate and the cap on every frame.
    host.size = egui::vec2(host.size.y, host.size.x);
    host.app.draw_dirty = true;
    for _ in 0..200 {
        host.ui(vec![]);
        let ledger = host.app.display_ledger();
        assert!(ledger.peak_total() <= budget.aggregate, "{ledger:?}");
        if !host.app.draw_dirty && host.app.doc.view.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let rect = crate::layout::chrome(host.context.content_rect(), &host.app.ui).viewport;
    let view = host.app.doc.view.unwrap().viewport();
    assert_eq!(view.width, f64::from(rect.width()));
    assert!(view.height > view.width, "portrait after the rotation");
    assert!(capped(&host), "{:?}", host.app.renderer.size());

    // Room for one tab's caches: the least recently active tab loses its caches first,
    // its backend's preparation geometry included.
    let first = host.app.tabs.parked[0].tab;
    let one = rotated.parked / 2;
    host.app.display_budget.aggregate = budget.active + one;
    host.app.enforce_parked_budget();
    let parked = &host.app.tabs.parked;
    let kept: Vec<_> = parked.iter().map(|d| d.thumbnail.is_some()).collect();
    assert_eq!(kept, [false, true], "oldest parked tab evicted first");
    assert_eq!(cache_bytes(&parked[0]), 0);
    assert!(host.app.display_ledger().parked <= one);
    host.app.display_budget = budget;
    assert!(host.app.release_parked_caches() > 0);
    assert_eq!(host.app.display_ledger().parked, 0);

    // A tab whose caches went is prepared again from its own scene.
    let identity = host.app.tabs.parked[0]
        .display
        .as_ref()
        .unwrap()
        .scene
        .identity();
    host.app.activate_tab(first);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.prepare_in_flight.is_none()
    })
    .await;
    host.redraw();
    assert_eq!(host.app.renderer.scene_identity(), Some(identity));
    assert!(cache_bytes(&host.app.doc) > 0);
    host.stop().await;
}

fn pending_copy(display: &crate::bridge::Display) -> crate::bridge::Display {
    crate::bridge::Display {
        epoch: display.epoch,
        preparation_generation: std::sync::atomic::AtomicU64::new(
            display
                .preparation_generation
                .load(std::sync::atomic::Ordering::Acquire),
        ),
        projection: display.projection.clone(),
        context: display.context.clone(),
        scene: display.scene.clone(),
        profiles: Vec::new(),
        profile_anchors: Vec::new(),
        profile_failures: Vec::new(),
        records: display.records.clone(),
        render_receipt: display.render_receipt.clone(),
        _lease: display._lease.clone(),
        prepared: Default::default(),
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn parked_eviction_fences_pending_cache_and_old_reply_then_rebuilds_on_desktop() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let first = host.app.doc.tab;
    let display = host.app.doc.display.as_ref().unwrap().clone();
    let shared = host.app.doc.bridge.shared.clone();
    let camera = host.app.doc.camera;
    let generation = shared.preparation_generation.current().unwrap();
    host.app.document_request(Request::New);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.tabs.parked.len() == 1 && app.document_open() && !app.busy()
    })
    .await;
    host.redraw();

    let mut executing_cache = shared.take_preparation_cache(generation);
    let allocation = Arc::new(
        shared
            .host_budget
            .reserve_kind(
                crate::budget::MAX_PREPARED_BYTES as usize,
                crate::budget::Kind::Preparation,
            )
            .unwrap(),
    );
    let executing_owner = Arc::downgrade(&allocation);
    let retention = onecad_render::PreparationRetention {
        source: Some(display._lease.clone()),
        preparation: Some(allocation),
    };
    let prepare = |cache: &mut onecad_render::PreparationCache| {
        onecad_render::PreparedScene::new_cached_retained(
            &display.scene,
            host.app.renderer.preparation_budget(),
            cache,
            retention.clone(),
        )
        .unwrap()
    };
    *display.prepared.lock().unwrap() = Some(prepare(&mut executing_cache));
    let pending = pending_copy(&display);
    *pending.prepared.lock().unwrap() = Some(prepare(&mut executing_cache));
    shared.publish_display(pending);
    let pending = shared.display.lock().unwrap().as_ref().unwrap().clone();
    let late = pending_copy(&display);
    *late.prepared.lock().unwrap() = Some(prepare(&mut executing_cache));
    drop(retention);

    // Desktop uses the same LRU share. The executing cache still owns allocations.
    host.app.display_budget.aggregate = host.app.display_budget.active;
    host.app.enforce_parked_budget();
    assert!(host.app.tabs.parked[0].thumbnail.is_none());
    assert!(display.prepared.lock().unwrap().is_none());
    assert!(pending.prepared.lock().unwrap().is_none());
    assert_eq!(cache_bytes(&host.app.tabs.parked[0]), 0);
    assert_eq!(host.app.tabs.parked[0].camera, camera);
    assert_eq!(
        host.app.tabs.parked[0].display.as_ref().unwrap().projection,
        display.projection
    );
    assert!(
        executing_owner.upgrade().is_some(),
        "executing payload remains charged"
    );
    let retained = shared.host_budget.snapshot().used;
    shared.restore_preparation_cache(generation, executing_cache);
    assert_eq!(
        cache_bytes(&host.app.tabs.parked[0]),
        0,
        "late restore was rejected"
    );
    assert!(shared.host_budget.snapshot().used <= retained);
    shared.publish_display(late);
    assert!(
        executing_owner.upgrade().is_none(),
        "last stale payload retired its lease"
    );
    assert!(
        shared
            .display
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .prepared
            .lock()
            .unwrap()
            .is_none()
    );

    let projection = display.projection.clone();
    // Simulate switching back before this tab's first source capture is delivered.
    host.app.tabs.parked[0].display = None;
    host.app.activate_tab(first);
    assert!(!host.app.doc.awaiting_scene);
    assert!(host.app.doc.prepare_in_flight.is_none());
    let late = host.app.doc.bridge.take_display().unwrap();
    assert!(shared.accept_display(&late));
    host.app.adopt_display(late);
    assert!(
        host.app.doc.awaiting_scene,
        "late source waits for its rebuild"
    );
    let requested = host.app.doc.prepare_in_flight;
    host.app.doc.awaiting_scene = false;
    host.app.request_preparation();
    assert!(
        host.app.doc.awaiting_scene,
        "an existing request also waits"
    );
    assert_eq!(
        host.app.doc.prepare_in_flight, requested,
        "one rebuild per generation"
    );
    let raw = egui::RawInput {
        screen_rect: Some(host.screen()),
        ..Default::default()
    };
    // Render without draining the queued reply: a missing preparation must wait.
    let mut output = host
        .context
        .run_ui(raw, |ui| crate::viewport::show(&mut host.app, ui));
    output.textures_delta.clear();
    assert!(host.app.viewport_failure.is_none());
    assert!(
        host.app.doc.view.is_none(),
        "old document cannot be picked while waiting"
    );
    // Initial source adoption may fit the camera; preparation completion must retain it.
    let camera = host.app.doc.camera;
    let newer = host.app.doc.prepare_in_flight;
    assert_eq!(
        newer.map(|request| request.0),
        Some(display.scene.identity())
    );
    assert_ne!(newer.unwrap().1, generation);
    let permits = tokio::sync::Semaphore::new(1);
    host.app.message(
        crate::bridge::Message::Prepared {
            identity: display.scene.identity(),
            generation,
            result: Err("obsolete preparation".into()),
            _permit: Arc::new(permits).acquire_owned().await.unwrap(),
        },
        &host.context,
    );
    assert_eq!(
        host.app.doc.prepare_in_flight, newer,
        "old reply kept newer request"
    );
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.prepare_in_flight.is_none()
    })
    .await;
    host.redraw();
    assert_eq!(
        host.app.renderer.scene_identity(),
        Some(display.scene.identity())
    );
    assert_eq!(host.app.doc.camera, camera);
    assert_eq!(
        host.app.doc.display.as_ref().unwrap().projection,
        projection
    );
    host.stop().await;
}

/// Accessible labels of the Display menu body with "More settings" opened.
fn display_menu_labels(host: &mut Host) -> Vec<String> {
    let pass = |host: &mut Host, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(host.screen()),
            events,
            ..Default::default()
        };
        let app = &mut host.app;
        let mut output = host.context.run_ui(raw, |ui| {
            egui::Area::new(egui::Id::unique("display-menu-test"))
                .show(ui.ctx(), |ui| crate::viewport_options::contents(app, ui));
        });
        output.textures_delta.clear();
        super::point_center_delivery_tests::support::widgets(&output)
    };
    // A new area spends its first pass sizing itself, invisible.
    pass(host, vec![]);
    let widgets = pass(host, vec![]);
    let header = widgets
        .iter()
        .find(|(label, _)| label == "More settings")
        .unwrap_or_else(|| panic!("no More settings in {widgets:?}"))
        .1
        .center();
    let press = super::point_center_delivery_tests::support::button;
    pass(
        host,
        vec![egui::Event::PointerMoved(header), press(header, true)],
    );
    pass(host, vec![press(header, false)]);
    let mut labels = vec![];
    for _ in 0..30 {
        labels = pass(host, vec![]);
    }
    labels.into_iter().map(|(label, _)| label).collect()
}

fn idle_delay(host: &mut Host) -> std::time::Duration {
    let raw = egui::RawInput {
        screen_rect: Some(host.screen()),
        ..Default::default()
    };
    let mut output = host.app.frame(&host.context, raw);
    output.textures_delta.clear();
    output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay)
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn the_mobile_route_shows_xray_lazily_recreates_targets_and_idles() {
    use onecad_render::{DisplayMode, RenderOptions};
    let root = tempfile::tempdir().unwrap();
    let mut host = ipad(root.path()).await;
    let labels = display_menu_labels(&mut host);
    assert!(labels.iter().any(|label| label == "X-ray"), "{labels:?}");
    assert!(!labels.iter().any(|l| l.starts_with("SSAA")), "{labels:?}");

    let shaded = *host.app.renderer.options();
    let shaded_bytes = host.app.renderer.memory_estimate().target_bytes;
    let xray = RenderOptions {
        display_mode: DisplayMode::Xray,
        ..shaded
    };
    for (options, grows) in [(xray, true), (shaded, false)] {
        host.app.pending_display_options = Some(options);
        host.redraw();
        assert_eq!(
            host.app.renderer.options().display_mode,
            options.display_mode
        );
        let bytes = host.app.renderer.memory_estimate().target_bytes;
        assert_eq!(bytes > shaded_bytes, grows, "{bytes} vs {shaded_bytes}");
        let ledger = host.app.display_ledger();
        assert!(ledger.peak <= host.app.display_budget.active, "{ledger:?}");
    }

    let (size, scale) = (host.app.renderer.size(), host.app.scene_scale);
    assert!(host.app.release_render_targets(&host.context));
    assert!(!host.app.renderer.has_targets());
    assert_eq!(host.app.renderer.memory_estimate().target_bytes, 0);
    host.redraw();
    assert!(
        host.app.renderer.has_targets(),
        "the next frame recreates them"
    );
    assert_eq!(
        (host.app.renderer.size(), host.app.scene_scale),
        (size, scale)
    );
    assert_eq!(
        host.app.renderer.memory_estimate().target_bytes,
        shaded_bytes
    );

    // Rendering on demand: once settled, the host frame asks egui for no repaint.
    settle(&mut host);
    let mut idle = 0;
    for _ in 0..200 {
        idle = if idle_delay(&mut host) == std::time::Duration::MAX {
            idle + 1
        } else {
            0
        };
        if idle == 5 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(idle, 5, "the idle editor never stopped asking for frames");
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn released_targets_and_memory_pressure_keep_scene_and_document_state() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let identity = host.app.renderer.scene_identity().unwrap();
    let projection = &host.app.doc.display.as_ref().unwrap().projection;
    let (revision, features) = (projection.revision, projection.features.len());
    let camera = host.app.doc.camera;
    let size = host.app.renderer.size();

    assert!(host.app.release_render_targets(&host.context));
    assert!(
        !host.app.renderer.has_targets(),
        "background released the targets"
    );
    assert_eq!(host.app.renderer.memory_estimate().target_bytes, 0);
    assert_eq!(
        host.app.renderer.scene_identity(),
        Some(identity),
        "geometry kept"
    );
    let relief = host.app.relieve_memory_pressure();
    assert_eq!(relief.lowered_scale, None, "within its allowance");
    host.redraw();
    assert!(
        host.app.renderer.has_targets(),
        "the next frame recreates them"
    );
    assert_eq!(host.app.renderer.size(), size);
    assert_eq!(host.app.renderer.scene_identity(), Some(identity));
    assert_eq!(host.app.doc.view.unwrap().identity().scene, identity);

    // An active document over its allowance drops one scale step and keeps its scene.
    host.app.display_budget.active = host.app.display_ledger().resident;
    let relief = host.app.relieve_memory_pressure();
    assert_eq!(relief.lowered_scale, Some(0.875));
    host.redraw();
    assert_eq!(host.app.scene_scale, 0.875);
    assert_eq!(host.app.renderer.scene_identity(), Some(identity));
    let display = host.app.doc.display.as_ref().unwrap();
    assert_eq!(display.scene.identity(), identity);
    assert_eq!(display.projection.revision, revision);
    assert_eq!(display.projection.features.len(), features);
    assert_eq!(host.app.doc.camera, camera);
    host.stop().await;
}
