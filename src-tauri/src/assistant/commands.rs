//! The assistant host's `#[tauri::command]` surface.
//!
//! Five commands, all thin: the virtual REST call, start/stop/status, and the
//! trusted provider-configuration path (ADR-0017). Every
//! one of them carries [`Principal::Ui`] into the bridge, because the webview is
//! the only caller a `#[tauri::command]` can have — that is the whole basis on
//! which the principal is asserted (`docs/assistant/wire-protocol.md` §4).
//!
//! **No command here touches a document, the filesystem or the network**
//! (ADR-0018). `assistant_bridge_fetch` marshals one AgentKit REST operation to a
//! handler the sidecar holds in memory; it opens no socket and names no URL.
//!
//! The DTO shapes are pinned by the frontend transport
//! (`src/modules/assistant/client/createDesktopAgentKitFetch.ts`) and its tests.
//! They are camelCase, matching the house `dto.rs` convention, and the channel is
//! a FIELD of the request rather than a second command argument because that is
//! how the frontend sends it — one top-level `request` key.

use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::ipc::{Channel, JavaScriptChannelId};
use tauri::State;

use onecad_assistant_protocol::Principal;

use crate::error::ApiError;
use crate::state::AppState;

use super::bridge::{StreamEvent, StreamResponse};
use super::provider_gateway::ProviderConfig;
use super::supervisor::AssistantHost;

/// The `agentkit.fetch` verb (`docs/assistant/wire-protocol.md` §5).
const VERB_AGENTKIT_FETCH: &str = "agentkit.fetch";

/// How long the first assistant call waits for a cold child.
///
/// The sidecar opens a SQLite store and builds the whole AgentKit object graph
/// before it sends `hello`, and the host is started LAZILY (ADR-0015), so the
/// very first request in a session pays that cost. Generous on purpose: a
/// too-tight budget would report a cold start as a failure.
const READY_TIMEOUT: Duration = Duration::from_secs(20);

// ─────────────────────────────────────────────────────────────────────────────
// DTOs
// ─────────────────────────────────────────────────────────────────────────────

/// One virtual REST call, as the frontend transport sends it.
///
/// No `Debug`: [`JavaScriptChannelId`] has none, and a request DTO carrying a
/// live channel is not something to print anyway.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRequestDto {
    /// AgentKit's own `RestOperation` name. Carried for diagnostics; the sidecar
    /// routes on `path`.
    pub operation: String,
    pub method: String,
    /// Path plus query, rooted at `/` — never a host, never a scheme.
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSON text, or absent for a bodiless request.
    #[serde(default)]
    pub body: Option<String>,
    /// Present only for a streaming operation; the response body rides on it.
    ///
    /// A [`JavaScriptChannelId`] rather than a [`Channel`] because the frontend
    /// nests it inside the request object, and Tauri only resolves a `Channel`
    /// as a TOP-LEVEL command argument.
    #[serde(default)]
    pub on_chunk: Option<JavaScriptChannelId>,
}

/// The response head handed back to the frontend transport.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeResponseDto {
    pub status: u16,
    pub status_text: String,
    pub headers: BTreeMap<String, String>,
    /// The whole body, for a non-streaming answer; `None` when `streaming`.
    pub body: Option<String>,
    /// Whether the body follows on the channel.
    pub streaming: bool,
}

/// One message on a streaming response's channel.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum BridgeStreamMessage {
    /// Body bytes, base64-encoded (the channel carries JSON).
    Chunk { data: String },
    /// The body finished normally.
    End,
    /// The body failed. `code` is the bridge's own (`stream_overflow`, §7), so
    /// the client can tell a resumable overflow from a hard failure.
    Error { code: String, message: String },
}

/// The assistant host's lifecycle, as the webview sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantStatusDto {
    /// `stopped` | `starting` | `ready` | `restarting` | `failed` | `retired`.
    pub state: String,
    /// Whether a host binary resolved at all. `false` means this build has none,
    /// which is a different thing from one that failed to start.
    pub available: bool,
    pub pid: Option<u32>,
    pub host_version: Option<String>,
    pub agentkit_contract_version: Option<String>,
    /// The last failure reason. A `failed` with no explanation is not a diagnosis.
    pub error: Option<String>,
}

impl AssistantStatusDto {
    fn of(available: bool, host: Option<&AssistantHost>) -> Self {
        let Some(host) = host else {
            return AssistantStatusDto {
                state: "stopped".into(),
                available,
                pid: None,
                host_version: None,
                agentkit_contract_version: None,
                error: None,
            };
        };
        let hello = host.hello();
        AssistantStatusDto {
            state: host.state().as_str().to_string(),
            available,
            pid: hello.as_ref().map(|h| h.pid),
            host_version: hello.as_ref().map(|h| h.host_version.clone()),
            agentkit_contract_version: hello.as_ref().map(|h| h.agentkit_contract_version.clone()),
            error: host.last_error(),
        }
    }
}

/// The local provider OneCAD settings describe, as the settings UI sends it.
///
/// `deny_unknown_fields` is the point of the shape, not tidiness. There is **no
/// `apiKey`**: a loopback runtime does not need one, and a credential does not
/// belong in `localStorage` — if one is ever needed it comes from the OS
/// credential store, and until then an attempt to send one is a named refusal
/// rather than a field quietly dropped on the floor.
///
/// Nothing here is trusted geometry: `baseUrl` is a PROPOSAL. Rust decides, in
/// [`validate_provider_base`](super::provider_gateway::validate_provider_base).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSettingsDto {
    /// The id the sidecar names in `provider.fetch`.
    pub id: String,
    /// The configured base, e.g. `http://127.0.0.1:11434/v1`.
    pub base_url: String,
    /// The model this provider serves.
    pub model: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Commands
// ─────────────────────────────────────────────────────────────────────────────

/// One AgentKit REST operation, served by the supervised sidecar.
///
/// The head (status, headers) is the command's return value; the body either
/// comes back whole in `body` or streams over the request's channel. **The bridge
/// decides which**: a streaming operation whose status is not 2xx answers with a
/// plain body instead, so a client that asked for a stream and got an error can
/// read it without a stream at all.
///
/// Starts the host on first use (ADR-0015 lazy start) and waits for the
/// handshake; a host that cannot start surfaces as an error, never as a silent
/// empty answer.
#[tauri::command]
#[tracing::instrument(
    skip_all,
    fields(operation = %request.operation, method = %request.method),
    err(Display)
)]
pub async fn assistant_bridge_fetch(
    state: State<'_, AppState>,
    webview: tauri::Webview,
    request: BridgeRequestDto,
) -> Result<BridgeResponseDto, ApiError> {
    // The wire is a system boundary: the sidecar builds a `Request` from this
    // path and a rooted path is the one thing it cannot recover from.
    if !request.path.starts_with('/') {
        return Err(ApiError::InvalidCommand(format!(
            "assistant request path {:?} is not rooted at /",
            request.path
        )));
    }

    let host = state.assistant.start()?;
    let bridge = host.connected(READY_TIMEOUT).await?;

    let mut payload = json!({
        "method": request.method,
        "path": request.path,
        "headers": request.headers,
    });
    if let Some(body) = &request.body {
        payload["bodyBase64"] =
            Value::String(base64::engine::general_purpose::STANDARD.encode(body.as_bytes()));
    }

    // Always a streaming answer on the wire — the sidecar's `agentkit.fetch`
    // route returns head-then-chunks even for eight bytes of JSON, so there is
    // one code path here rather than a branch only the SSE route exercises.
    let stream = bridge
        .request_stream(VERB_AGENTKIT_FETCH, Principal::Ui, payload)
        .await?;
    let head = parse_head(&stream.head)?;

    let channel: Option<Channel<BridgeStreamMessage>> =
        request.on_chunk.map(|id| id.channel_on(webview));

    match channel {
        Some(channel) if (200..300).contains(&head.status) => {
            // `tauri::async_runtime::spawn`, matching the app layer's convention
            // for detached work started from a command.
            tauri::async_runtime::spawn(pump_body(stream, channel));
            Ok(BridgeResponseDto {
                status: head.status,
                status_text: head.status_text,
                headers: head.headers,
                body: None,
                streaming: true,
            })
        }
        // Either a non-streaming operation, or a streaming one that failed its
        // status check: the body is small and terminal, so collect it and let the
        // client read the error without a stream at all. Any channel is dropped
        // here unused, which is exactly what `streaming: false` tells the client.
        _ => {
            let body = collect_body(stream).await?;
            Ok(BridgeResponseDto {
                status: head.status,
                status_text: head.status_text,
                headers: head.headers,
                body: Some(body),
                streaming: false,
            })
        }
    }
}

/// The assistant host's current lifecycle state. Never starts anything, so a
/// status poll cannot spawn a 99 MB child.
#[tauri::command]
pub async fn assistant_status(state: State<'_, AppState>) -> Result<AssistantStatusDto, ApiError> {
    let slot = &state.assistant;
    Ok(AssistantStatusDto::of(
        slot.is_configured(),
        slot.host().as_ref(),
    ))
}

/// Starts the assistant host (or returns the running one) and waits for its
/// handshake. This is the explicit half of the lazy start; the first
/// [`assistant_bridge_fetch`] does the same thing implicitly.
#[tauri::command]
#[tracing::instrument(skip_all, err(Display))]
pub async fn assistant_start(state: State<'_, AppState>) -> Result<AssistantStatusDto, ApiError> {
    let host = state.assistant.start()?;
    // A host that does not come up within the budget is reported through the
    // status DTO rather than as a command error: the supervisor is still trying,
    // and the state plus `error` say exactly where it got to.
    let _ = host.wait_ready(READY_TIMEOUT).await;
    Ok(AssistantStatusDto::of(true, Some(&host)))
}

/// Installs the local provider the user configured, or clears it with `null`.
///
/// **The one trusted path by which a provider is registered** (ADR-0017): a
/// `#[tauri::command]`'s only caller is the webview's settings surface, and no
/// frame arriving from the sidecar can reach this.
///
/// Takes effect on the next `provider.fetch` — the bridge reads the registry per
/// request — so a user editing their endpoint never restarts the app.
///
/// A refused base comes back as an error naming the gateway's own code and the
/// reason, and leaves any working provider installed. A misconfiguration the
/// user is told about is worth more than a silent drop that resurfaces later as
/// "unknown provider" on every answer.
#[tauri::command]
#[tracing::instrument(skip_all, err(Display))]
pub async fn assistant_configure_provider(
    state: State<'_, AppState>,
    provider: Option<ProviderSettingsDto>,
) -> Result<(), ApiError> {
    let config = provider.map(|dto| ProviderConfig {
        id: dto.id,
        base_url: dto.base_url,
        model: dto.model,
        api_key: None,
    });
    state.assistant.configure_provider(config).map_err(|err| {
        ApiError::InvalidCommand(format!(
            "assistant provider refused ({}): {err}",
            err.code()
        ))
    })
}

/// Stops the assistant host: a graceful `shutdown`, then a kill after the grace
/// window. Idempotent, and safe with nothing running.
#[tauri::command]
#[tracing::instrument(skip_all, err(Display))]
pub async fn assistant_stop(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.assistant.stop();
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Body plumbing
// ─────────────────────────────────────────────────────────────────────────────

struct ResponseHead {
    status: u16,
    status_text: String,
    headers: BTreeMap<String, String>,
}

/// Parses the `res` payload the sidecar's `agentkit.fetch` route sends.
fn parse_head(head: &Value) -> Result<ResponseHead, ApiError> {
    let status = head
        .get("status")
        .and_then(Value::as_u64)
        .ok_or_else(|| ApiError::Internal("assistant reply carried no status".into()))?;
    let headers = head
        .get("headers")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Ok(ResponseHead {
        status: status as u16,
        status_text: head
            .get("statusText")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        headers,
    })
}

/// Drains a body into one string (the non-streaming answer).
async fn collect_body(mut stream: StreamResponse) -> Result<String, ApiError> {
    let mut bytes = Vec::new();
    loop {
        match stream.next().await {
            Some(StreamEvent::Chunk(chunk)) => bytes.extend_from_slice(&chunk),
            Some(StreamEvent::End(None)) | None => break,
            Some(StreamEvent::End(Some(error))) => {
                return Err(ApiError::Internal(format!(
                    "assistant response body failed ({}): {}",
                    error.code, error.message
                )))
            }
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| ApiError::Internal("assistant response body is not UTF-8".into()))
}

/// Forwards a body to the webview, chunk by chunk, until the stream terminates.
///
/// A failed `send` means the webview dropped the channel (navigated away, closed
/// the panel), which is this lane's only backpressure signal: the loop stops and
/// dropping `stream` sends `cancel`, so an abandoned SSE run does not keep the
/// sidecar pumping into nothing.
async fn pump_body(mut stream: StreamResponse, channel: Channel<BridgeStreamMessage>) {
    loop {
        let message = match stream.next().await {
            Some(StreamEvent::Chunk(bytes)) => BridgeStreamMessage::Chunk {
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            },
            Some(StreamEvent::End(None)) => BridgeStreamMessage::End,
            Some(StreamEvent::End(Some(error))) => BridgeStreamMessage::Error {
                code: error.code,
                message: error.message,
            },
            None => BridgeStreamMessage::End,
        };
        let terminal = !matches!(message, BridgeStreamMessage::Chunk { .. });
        if let Err(err) = channel.send(message) {
            tracing::debug!(target: "assistant", error = %err, "assistant stream consumer went away");
            return;
        }
        if terminal {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_messages_match_the_frontend_transport_contract() {
        // Pinned by `createDesktopAgentKitFetch.ts`: the discriminator is
        // `event`, and the three shapes are exactly these.
        assert_eq!(
            serde_json::to_string(&BridgeStreamMessage::Chunk {
                data: "aGk=".into()
            })
            .unwrap(),
            r#"{"event":"chunk","data":"aGk="}"#
        );
        assert_eq!(
            serde_json::to_string(&BridgeStreamMessage::End).unwrap(),
            r#"{"event":"end"}"#
        );
        assert_eq!(
            serde_json::to_string(&BridgeStreamMessage::Error {
                code: "stream_overflow".into(),
                message: "per-stream buffer exceeded".into()
            })
            .unwrap(),
            r#"{"event":"error","code":"stream_overflow","message":"per-stream buffer exceeded"}"#
        );
    }

    #[test]
    fn the_response_dto_is_camel_case() {
        let dto = BridgeResponseDto {
            status: 200,
            status_text: "OK".into(),
            headers: BTreeMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: Some("{}".into()),
            streaming: false,
        };
        let value = serde_json::to_value(&dto).unwrap();
        assert_eq!(value["statusText"], "OK");
        assert_eq!(value["headers"]["content-type"], "application/json");
        assert_eq!(value["streaming"], false);
    }

    #[test]
    fn the_request_dto_deserializes_what_the_frontend_sends() {
        let request: BridgeRequestDto = serde_json::from_value(json!({
            "operation": "submitMessage",
            "method": "POST",
            "path": "/v1/chats/c1/messages",
            "headers": {"content-type": "application/json"},
            "body": "{\"content\":\"hi\"}"
        }))
        .expect("the transport's request shape");
        assert_eq!(request.operation, "submitMessage");
        assert_eq!(request.headers["content-type"], "application/json");
        assert!(request.on_chunk.is_none());
    }

    #[test]
    fn a_head_without_a_status_is_refused_rather_than_guessed() {
        let head = parse_head(&json!({"status": 204, "statusText": "No Content"})).unwrap();
        assert_eq!(head.status, 204);
        assert!(head.headers.is_empty());
        assert!(parse_head(&json!({"statusText": "OK"})).is_err());
    }

    #[test]
    fn the_provider_dto_is_the_camel_case_shape_and_refuses_a_credential() {
        let dto: ProviderSettingsDto = serde_json::from_value(json!({
            "id": "local",
            "baseUrl": "http://127.0.0.1:11434/v1",
            "model": "qwen3:8b",
        }))
        .expect("the settings surface's shape");
        assert_eq!(dto.id, "local");
        assert_eq!(dto.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(dto.model, "qwen3:8b");

        // A credential must not ride in through a field nobody reads: an attempt
        // is a named refusal, not a silent drop.
        let err = serde_json::from_value::<ProviderSettingsDto>(json!({
            "id": "local",
            "baseUrl": "http://127.0.0.1:11434/v1",
            "model": "qwen3:8b",
            "apiKey": "sk-secret",
        }))
        .expect_err("apiKey is not part of this DTO");
        assert!(err.to_string().contains("apiKey"), "{err}");
    }

    #[test]
    fn the_status_dto_reports_a_missing_binary_distinctly_from_a_stopped_host() {
        let unavailable = AssistantStatusDto::of(false, None);
        assert_eq!(unavailable.state, "stopped");
        assert!(!unavailable.available);
        let value = serde_json::to_value(&unavailable).unwrap();
        assert_eq!(value["hostVersion"], Value::Null);
        assert_eq!(value["agentkitContractVersion"], Value::Null);
    }
}
