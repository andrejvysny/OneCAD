//! Render profile scale, process-wide display budget and target release through the real
//! host route, with the production worker and Metal. macOS evidence, not iPad evidence.
use super::model_selection_delivery_tests::{boxed, click, labels, targets};
use super::point_center_delivery_tests::support::Host;
use super::*;
use onecad_scene::PickKind;

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

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn three_documents_and_a_rotation_stay_within_the_display_aggregate() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    for parked in 1..=2 {
        host.app.document_request(Request::New);
        pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
            app.tabs.parked.len() == parked && app.document_open() && !app.busy()
        })
        .await;
        host.app.ui.start_open = false;
        host.redraw();
    }
    let budget = host.app.display_budget;
    let ledger = host.app.display_ledger();
    assert!(
        host.app
            .tabs
            .parked
            .iter()
            .all(|doc| doc.thumbnail.is_some()),
        "parked tabs keep switcher thumbnails"
    );
    assert!(
        ledger.parked > 0 && ledger.peak_total() <= budget.aggregate,
        "{ledger:?}"
    );

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
    assert!(rotated.peak_total() <= budget.aggregate, "{rotated:?}");

    // The host's own rotation stays within the aggregate on every frame.
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
    assert_eq!(
        host.app.doc.view.unwrap().viewport().width,
        f64::from(
            crate::layout::chrome(host.screen(), &host.app.ui)
                .viewport
                .width()
        )
    );

    // Room for one thumbnail: the least recently active tab loses its caches first.
    let first = host.app.tabs.parked[0].tab;
    let one = rotated.parked / 2;
    host.app.display_budget.aggregate = budget.active + one;
    host.app.enforce_parked_budget();
    let kept: Vec<_> = host
        .app
        .tabs
        .parked
        .iter()
        .map(|d| d.thumbnail.is_some())
        .collect();
    assert_eq!(kept, [false, true], "oldest parked tab evicted first");
    assert!(host.app.display_ledger().parked <= one);
    host.app.display_budget = budget;
    assert!(host.app.release_parked_caches() > 0);
    assert_eq!(host.app.display_ledger().parked, 0);

    // A tab whose caches went is drawn again from its own scene.
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
