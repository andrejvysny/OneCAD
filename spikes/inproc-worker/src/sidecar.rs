//! The sidecar baseline: the built `onecad-worker` binary over stdio, driven by
//! the same `ProtocolClient` and the same requests as the in-process host.
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use onecad_protocol::client::ProtocolClient;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};

pub struct SidecarWorker {
    pub client: Option<ProtocolClient>,
    pub handshake: Duration,
    child: Child,
}

impl SidecarWorker {
    pub async fn start(binary: &Path) -> std::io::Result<SidecarWorker> {
        let t0 = Instant::now();
        let mut child = Command::new(binary)
            .env("ONECAD_WORKER_LOG", "info")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        // Drain stderr so a chatty worker can never block on a full pipe.
        let stderr = child.stderr.take().expect("stderr");
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
        let client = ProtocolClient::connect(stdout, stdin)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(SidecarWorker {
            client: Some(client),
            handshake: t0.elapsed(),
            child,
        })
    }

    pub fn client(&self) -> &ProtocolClient {
        self.client.as_ref().expect("client open")
    }

    pub async fn shutdown(mut self) -> Option<i32> {
        if let Some(client) = self.client.take() {
            let _ = client
                .request_timeout("Shutdown", serde_json::json!({}), Duration::from_secs(5))
                .await;
            client.close().await;
        }
        let status = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
        status.ok().and_then(|s| s.ok()).and_then(|s| s.code())
    }
}
