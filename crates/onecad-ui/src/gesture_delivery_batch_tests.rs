//! Ordered input batches must preserve the same ordinary sketch gesture.
use super::*;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_multiple_primary_cycles_do_not_start_drag() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let before = host.sketch().clone();
    let start = host.project(0.0, 0.0);
    let end = host.project(22.0, 21.0);
    host.ui(vec![
        button(start, true),
        egui::Event::PointerMoved(end),
        button(end, false),
        button(start, true),
        button(start, false),
    ]);
    assert!(!host.app.sketch_gesture.busy());
    assert_eq!(host.sketch(), &before);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_click_ignores_motion_before_press() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let start = host.project(0.0, 0.0);
    let far = host.project(22.0, 21.0);
    host.ui(vec![
        egui::Event::PointerMoved(far),
        button(start, true),
        button(start, false),
    ]);
    assert!(
        !host.app.sketch_gesture.busy(),
        "ordinary click must not begin drag"
    );
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_batched_drag_uses_release_before_later_hover() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let start = host.project(0.0, 0.0);
    let end = host.project(22.0, 21.0);
    let later = egui::pos2(-10.0, -10.0);
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        egui::Event::PointerMoved(end),
        button(end, false),
        egui::Event::PointerMoved(later),
    ]);
    settled(&mut host).await;
    assert_rectangle(host.sketch(), [22.0, 21.0]);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_corner_press_and_move_batched_release_next_frame_commits_once() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let undo = host
        .app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .as_ref()
        .unwrap()
        .undo_depth;
    let start = host.project(0.0, 0.0);
    let end = host.project(22.0, 21.0);
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        egui::Event::PointerMoved(end),
    ]);
    host.ui(vec![button(end, false)]);
    settled(&mut host).await;
    assert_rectangle(host.sketch(), [22.0, 21.0]);
    assert_eq!(host.app.sketch_gesture.moved(), Some(true));
    assert_eq!(
        host.app
            .sketch
            .session
            .as_ref()
            .unwrap()
            .history
            .as_ref()
            .unwrap()
            .undo_depth,
        undo + 1
    );
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_drag_focus_loss_pointer_gone_escape_and_overlay_preserve_authority() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let accepted = host.sketch().clone();
    let history = host.app.sketch.session.as_ref().unwrap().history.clone();
    for condition in 0..4 {
        let start = host.project(0.0, 0.0);
        let end = host.project(22.0, 21.0);
        host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
        host.ui(vec![egui::Event::PointerMoved(end)]);
        assert!(host.app.sketch_gesture.busy());
        match condition {
            0 => host.ui_focused(vec![egui::Event::WindowFocused(false)], false),
            1 => host.ui(vec![egui::Event::PointerGone]),
            2 => host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]),
            _ => {
                host.app.ui.constraints_open = true;
                host.ui(vec![]);
            }
        }
        host.ui(vec![button(end, false)]);
        host.app.ui.constraints_open = false;
        settled(&mut host).await;
        assert_eq!(
            host.sketch(),
            &accepted,
            "cancellation condition {condition}"
        );
        assert_eq!(host.app.sketch.session.as_ref().unwrap().history, history);
    }
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn stationary_batched_release_completes_on_service_repaint_wakes() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let wakes = Arc::new(tokio::sync::Notify::new());
    let native_wakes = wakes.clone();
    host.context.set_repaint_observer(move |_| {
        if std::thread::current().name() == Some("onecad-native-services") {
            native_wakes.notify_one();
        }
    });
    let start = host.project(0.0, 0.0);
    let end = host.project(22.0, 21.0);
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        egui::Event::PointerMoved(end),
        button(end, false),
    ]);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            wakes.notified().await;
            logic_tick(&mut host.app, &host.context, &mut host.frame, false);
            if !host.app.sketch_gesture.busy() && active(&host.app) {
                break;
            }
        }
    })
    .await
    .expect("batched release must complete on service wakes without more input");
    assert_rectangle(host.sketch(), [22.0, 21.0]);
    assert_eq!(host.app.sketch_gesture.moved(), Some(true));
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_corner_drag_batched_in_one_frame_commits_one_edit() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let undo = host
        .app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .as_ref()
        .unwrap()
        .undo_depth;
    let start = host.project(0.0, 0.0);
    let end = host.project(22.0, 21.0);
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        egui::Event::PointerMoved(end),
        button(end, false),
    ]);
    settled(&mut host).await;
    assert_rectangle(host.sketch(), [22.0, 21.0]);
    assert_eq!(host.app.sketch_gesture.moved(), Some(true));
    assert_eq!(
        host.app
            .sketch
            .session
            .as_ref()
            .unwrap()
            .history
            .as_ref()
            .unwrap()
            .undo_depth,
        undo + 1
    );
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_release_over_chrome_refuses_even_after_pointer_returns() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let before = host.sketch().clone();
    let history = host.app.sketch.session.as_ref().unwrap().history.clone();
    let chrome = crate::layout::chrome(host.screen(), &host.app.ui);
    let cube_host = egui::Rect::from_min_max(
        egui::pos2(chrome.viewport.left(), chrome.view_cluster.y),
        egui::pos2(
            chrome.view_cluster.x,
            chrome.viewport.bottom().max(chrome.view_cluster.y),
        ),
    );
    for release in [
        egui::pos2(10.0, 10.0),
        crate::navigation_cube::rect(cube_host).center(),
    ] {
        let start = host.project(0.0, 0.0);
        host.ui(vec![button(start, true)]);
        host.ui(vec![egui::Event::PointerMoved(host.project(22.0, 21.0))]);
        assert!(host.app.sketch_gesture.busy());
        host.ui(vec![
            button(release, false),
            egui::Event::PointerMoved(start),
        ]);
        settled(&mut host).await;
        assert_eq!(host.sketch(), &before, "release over chrome at {release:?}");
        assert_eq!(host.app.sketch.session.as_ref().unwrap().history, history);
    }
    host.stop().await;
}
