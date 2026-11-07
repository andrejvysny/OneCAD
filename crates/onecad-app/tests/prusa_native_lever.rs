//! Source-derived native lever history; same-kernel diagnostics are not an independent oracle.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
#[path = "prusa_native_lever_support/mod.rs"]
mod model;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_lever_history_and_fresh_process_reopen() {
    model::native_lever_history_and_fresh_process_reopen().await;
}
