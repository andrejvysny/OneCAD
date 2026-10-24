use super::Editor;
use crate::app_logic_tests::point_center_delivery_tests::support::Host;
use std::sync::{Arc, atomic::Ordering};
use std::time::{Duration, Instant};

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn post_host_submit_registers_completion_once_without_waiting_for_next_input() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        host.app.draw_dirty = true;
        host.ui(vec![]);
        if host.app.cad_needs_fence {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "CAD submission never became ready"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(host.app.sampled);
    let mut editor = Editor { app: host.app };
    // The helper has no egui GPU painter; this flushes its preceding CAD submission.
    // The production host calls the same hook after its real sampling submission.
    editor.app.gpu.queue.submit([]);
    editor.after_host_submit(&host.context);
    assert!(!editor.app.cad_needs_fence);
    assert!(!editor.app.sampled);
    let sampling = Arc::clone(&editor.app.sampling_done);
    editor.after_host_submit(&host.context);
    crate::input::fence_previous_frame(&mut editor.app, &host.context);
    assert!(
        Arc::ptr_eq(&sampling, &editor.app.sampling_done),
        "no duplicate host/fallback fence"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let idle = editor.app.renderer.poll_idle().unwrap();
        if idle && sampling.load(Ordering::Acquire) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "completion needed another editor/input frame"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    host.app = editor.app;
    host.stop().await;
}
