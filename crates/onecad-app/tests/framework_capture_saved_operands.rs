//! Private diagnostic export; not native workflow qualification.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
use onecad_app::{api, worker::SolverEngine};
#[tokio::test(flavor = "multi_thread")]
async fn capture_saved_operands() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("onecad_app=debug,onecad_protocol=debug,worker=info")
        .with_test_writer()
        .try_init();
    host::required_worker();
    let source = std::env::var("ONECAD_CAPTURE_SOURCE").unwrap();
    let output = std::path::PathBuf::from(std::env::var_os("ONECAD_CAPTURE_OUTPUT").unwrap());
    std::fs::create_dir(&output).unwrap();
    let area = tempfile::tempdir().unwrap();
    let h = host::Harness::new(area.path()).await;
    api::open_document(&h.state, h.host.clone(), source, None)
        .await
        .unwrap();
    h.settled(&h.projection().await).await;
    let worker = h.state.live_worker().unwrap();
    let snapshot = api::preview_context(&h.state).await.unwrap().snapshot_id;
    for (index, text) in std::env::var("ONECAD_CAPTURE_BODIES")
        .unwrap()
        .split(',')
        .enumerate()
    {
        let body = text.parse().unwrap();
        let path = output.join(format!("input-{index}.brep"));
        let result = worker
            .export_geometry(
                path.to_str().unwrap(),
                &[body],
                "brep",
                false,
                Some(snapshot),
            )
            .await
            .unwrap();
        eprintln!("exported {text}: {} bytes", result.bytes_written);
        assert_eq!(result.solid_count, 1);
    }
    h.close().await;
}
