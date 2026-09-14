//! OCAK1 JSON control envelopes, principals, the verb tables and request ids.
//!
//! One [`Envelope`] models every frame shape in
//! `../../../docs/assistant/wire-protocol.md` §2, internally tagged by the `t`
//! field (`hello`/`accept`/`reject`/`req`/`res`/`chunk`/`end`/`cancel`/`ping`/
//! `pong`). Object keys are camelCase, matching the contract's JSON examples
//! byte for byte. `u64` ids ride as JSON numbers.
//!
//! An unknown `t` deserializes to an ERROR, never a skipped frame: serde's
//! internally-tagged representation refuses an unrecognised tag by default, and
//! that default is the behaviour the contract §2 requires — silently dropping an
//! envelope is how a stream loses an event nobody notices.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ProtocolError;

/// Wire protocol version carried by `hello` and `accept`.
pub const PROTOCOL_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Principals
// ---------------------------------------------------------------------------

/// Who a `req` is acting as (contract §4).
///
/// Closed on purpose — there are exactly two authorities and adding a third is a
/// design decision, not a new enum arm. **The `principal` field on an inbound
/// frame is never trusted**: the reader stamps a sidecar-originated request as
/// [`Principal::Host`], overwriting whatever the frame claimed. A principal is
/// asserted by the transport a request arrived on, never by its own payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Principal {
    /// The request originated in the OneCAD webview, through a trusted
    /// `#[tauri::command]`.
    Ui,
    /// The request originated inside the assistant host — the AgentKit loop, a
    /// tool, or a provider callback.
    Host,
}

impl Principal {
    /// The wire spelling, identical to the serde representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Principal::Ui => "ui",
            Principal::Host => "host",
        }
    }
}

impl std::fmt::Display for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Verb tables
// ---------------------------------------------------------------------------

/// One row of a [`VerbTable`]: a verb and the principals allowed to call it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbEntry {
    /// The verb name as it appears in a `req`.
    pub verb: &'static str,
    /// Every principal permitted to call it. Never empty in practice; an empty
    /// row would refuse the verb outright, which is the safe direction.
    pub principals: &'static [Principal],
}

/// A verb → allowed-principals lookup.
///
/// **There is no wildcard entry.** A verb absent from the table is refused, so
/// the failure mode of forgetting to register a verb is a loud refusal rather
/// than a silently permitted call. The table is a `&'static [VerbEntry]` rather
/// than a map because it holds a handful of rows — a linear scan over three
/// entries is faster than hashing, and it keeps the tables `const`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbTable(&'static [VerbEntry]);

impl VerbTable {
    /// Wrap a static table.
    pub const fn new(entries: &'static [VerbEntry]) -> Self {
        VerbTable(entries)
    }

    /// Every row, in declaration order.
    pub const fn entries(&self) -> &'static [VerbEntry] {
        self.0
    }

    /// The principals allowed to call `verb`, or `None` if the verb is unknown.
    pub fn principals(&self, verb: &str) -> Option<&'static [Principal]> {
        self.0
            .iter()
            .find(|entry| entry.verb == verb)
            .map(|entry| entry.principals)
    }

    /// Whether `principal` may call `verb`. An unknown verb is never allowed.
    pub fn allows(&self, verb: &str, principal: Principal) -> bool {
        match self.principals(verb) {
            Some(allowed) => allowed.contains(&principal),
            None => false,
        }
    }

    /// The authority check a receiver runs before dispatching a `req`.
    ///
    /// Distinguishes "no such verb" from "not for you" because the two mean
    /// different things in a log: the first is a version skew between the peers,
    /// the second is a request crossing an authority boundary.
    pub fn check(&self, verb: &str, principal: Principal) -> Result<(), ProtocolError> {
        match self.principals(verb) {
            None => Err(ProtocolError::UnknownVerb(verb.to_string())),
            Some(allowed) if allowed.contains(&principal) => Ok(()),
            Some(_) => Err(ProtocolError::VerbRefused {
                verb: verb.to_string(),
                principal,
            }),
        }
    }
}

/// Verbs the host may send to the sidecar (contract §5).
pub const HOST_TO_SIDECAR_VERBS: VerbTable = VerbTable::new(&[
    // One AgentKit REST operation; the reply is the response head plus, for
    // `streamRun`, a chunk stream.
    VerbEntry {
        verb: "agentkit.fetch",
        principals: &[Principal::Ui],
    },
    // Finish in-flight work and exit.
    VerbEntry {
        verb: "shutdown",
        principals: &[Principal::Ui],
    },
]);

/// Verbs the sidecar may send to the host (contract §5).
///
/// There is deliberately no verb by which the sidecar can mutate a document,
/// read the filesystem, spawn a process or open a socket. Adding one is a
/// work-package-sized decision with its own authorization design, not a new row.
pub const SIDECAR_TO_HOST_VERBS: VerbTable = VerbTable::new(&[
    // A model call routed through the Rust local-provider gateway. The sidecar
    // names a registered provider id, never a URL.
    VerbEntry {
        verb: "provider.fetch",
        principals: &[Principal::Host],
    },
]);

// ---------------------------------------------------------------------------
// Request ids
// ---------------------------------------------------------------------------

/// Per-originator request-id counter (contract §3).
///
/// The host allocates EVEN ids and the sidecar allocates ODD ids, so the two
/// spaces cannot collide and no envelope needs a direction field to disambiguate
/// a reply. Ids are monotonically increasing and never reused within a
/// connection; a reused id is a protocol error rather than a silently crossed
/// response.
///
/// The counter is atomic because both sides are duplex — the reader task keeps
/// draining while another task allocates an id for an outbound request (§6), so
/// `&self` is the receiver a shared allocator actually needs.
#[derive(Debug)]
pub struct IdAllocator {
    next: AtomicU64,
}

impl IdAllocator {
    /// The host's allocator: 0, 2, 4, …
    pub const fn host() -> Self {
        IdAllocator {
            next: AtomicU64::new(0),
        }
    }

    /// The sidecar's allocator: 1, 3, 5, …
    pub const fn sidecar() -> Self {
        IdAllocator {
            next: AtomicU64::new(1),
        }
    }

    /// The next id in this originator's space.
    ///
    /// `Relaxed` is sufficient: the only guarantee required is that no two calls
    /// return the same value, which `fetch_add` provides on its own. Exhausting
    /// 2^63 ids within one connection is not reachable.
    pub fn next(&self) -> u64 {
        self.next.fetch_add(2, Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Envelopes
// ---------------------------------------------------------------------------

/// Structured error body carried by a failed `res` or `end`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorObject {
    /// Machine-readable code, e.g. `stream_overflow`.
    pub code: String,
    /// Human-readable detail. Never the sole signal — callers branch on `code`.
    pub message: String,
}

/// `hello` — sidecar → host, unsolicited, exactly once, first.
///
/// The host MUST receive `hello` before anything else; anything else first is
/// fatal. `protocolVersion` has no default: a peer that does not state its
/// version must not be assumed compatible with ours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    /// OCAK1 version the sidecar speaks.
    pub protocol_version: u32,
    /// Version of the assistant host bundle.
    pub host_version: String,
    /// AgentKit contract version the bundle was built against.
    pub agentkit_contract_version: String,
    /// Child process id, for supervisor logging.
    pub pid: u32,
    /// Per-launch nonce, so a late frame from a previous child is detectable.
    pub session_nonce: String,
}

/// `accept` — host → sidecar, in reply to a `hello` it can serve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accept {
    /// OCAK1 version the host speaks.
    pub protocol_version: u32,
    /// OneCAD application version.
    pub app_version: String,
    /// Version of the host-side bridge implementation.
    pub bridge_version: String,
}

/// `reject` — host → sidecar, refusing the handshake.
///
/// Sent BEFORE any other frame is exchanged, so an incompatible pair can never
/// perform a partial operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reject {
    /// Why the handshake was refused, for the supervisor's log.
    pub reason: String,
}

/// `req` — either direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Req {
    /// Originator-allocated id (see [`IdAllocator`]).
    pub id: u64,
    /// The asserted authority. On an INBOUND frame this field is overwritten by
    /// the reader with the principal the transport implies; it is never trusted
    /// as received.
    pub principal: Principal,
    /// The verb, checked against the receiving side's [`VerbTable`].
    pub verb: String,
    /// Verb-specific body. Absent on the wire deserializes as `null` so a
    /// payload-free verb such as `shutdown` need not send an empty object.
    #[serde(default)]
    pub payload: Value,
}

/// `res` — either direction, answering exactly one `req`.
///
/// `payload` iff `ok`, `error` iff `!ok`. For a streaming request this carries
/// the response head (status, headers) and is followed by `chunk`s and one `end`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Res {
    /// The id of the `req` being answered.
    pub id: u64,
    /// Whether the request succeeded.
    pub ok: bool,
    /// Success body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Failure body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

/// `chunk` — one piece of a streaming response body. Bytes ride in the bin tail.
///
/// `seq` starts at 0 and increases by one per chunk of that request. A gap is
/// fatal: the receiver cannot tell a dropped chunk from a reordered one, and
/// guessing would let the stream and its durable log disagree undetectably.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    /// The id of the `req` being streamed.
    pub id: u64,
    /// Zero-based, contiguous per request.
    pub seq: u64,
}

/// `end` — exactly one per streamed request, terminating its chunk sequence.
///
/// On a bounded-buffer overflow the stream ends with `ok: false` and
/// `error.code == "stream_overflow"`, never with silently dropped chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct End {
    /// The id of the `req` whose stream is ending.
    pub id: u64,
    /// Whether the stream completed normally.
    pub ok: bool,
    /// Failure body, present iff `!ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

/// `cancel` — either direction, best effort.
///
/// A `res`/`end` for a cancelled id MAY still arrive and MUST be discarded by
/// the originator. Cancelling an unknown id is a no-op, not an error: that race
/// is normal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cancel {
    /// The id to cancel.
    pub id: u64,
}

/// `ping` — liveness probe, either direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ping {
    /// Echoed back in the matching [`Pong`].
    pub id: u64,
}

/// `pong` — the reply to a [`Ping`], carrying its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pong {
    /// The id of the `ping` being answered.
    pub id: u64,
}

/// Every OCAK1 control envelope, tagged by `t` (contract §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Envelope {
    /// Sidecar handshake; must be the first frame on the connection.
    Hello(Hello),
    /// Host accepts the handshake.
    Accept(Accept),
    /// Host refuses the handshake.
    Reject(Reject),
    /// A request in either direction.
    Req(Req),
    /// The terminal reply to one request.
    Res(Res),
    /// One piece of a streaming response body.
    Chunk(Chunk),
    /// The terminator of a chunk stream.
    End(End),
    /// Best-effort cancellation.
    Cancel(Cancel),
    /// Liveness probe.
    Ping(Ping),
    /// Liveness reply.
    Pong(Pong),
}

impl Envelope {
    /// Serialize to JSON envelope bytes for a frame's json section.
    pub fn to_json_vec(&self) -> Result<Vec<u8>, ProtocolError> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Parse a frame's json section. An unknown `t` is an error, not a skip.
    ///
    /// Also enforces the outcome invariant on `res` and `end` — see
    /// [`Envelope::validate`]. Serde alone cannot express "`error` iff `!ok`"
    /// across two independent fields, so the check is explicit and runs on every
    /// decode rather than living only in a doc comment.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Envelope, ProtocolError> {
        let envelope: Envelope = serde_json::from_slice(bytes)?;
        envelope.validate()?;
        Ok(envelope)
    }

    /// Reject a frame that disagrees with itself.
    ///
    /// `res` and `end` both carry a boolean outcome beside an optional error
    /// body, and the contract binds them: `error` is present iff `ok` is false.
    /// The TypeScript peer rejects a violation on decode; without this, Rust
    /// would accept frames the other side refuses and the "same protocol" would
    /// quietly mean two different things.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let (tag, id, ok, has_error) = match self {
            Envelope::Res(r) => ("res", r.id, r.ok, r.error.is_some()),
            Envelope::End(e) => ("end", e.id, e.ok, e.error.is_some()),
            _ => return Ok(()),
        };
        if ok == has_error {
            return Err(ProtocolError::InconsistentOutcome {
                tag,
                id,
                ok,
                has_error,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- the res/end outcome invariant -------------------------------------
    //
    // These pin the ONE place the two implementations could silently diverge on
    // which bytes are legal. The TypeScript peer rejects a self-contradicting
    // frame on decode; if Rust ever stops doing the same, these fail here rather
    // than as a confusing mid-stream error in a packaged app.

    #[test]
    fn res_claiming_failure_without_an_error_is_rejected() {
        let wire = br#"{"t":"res","id":4,"ok":false}"#;
        let err = Envelope::from_json_slice(wire).expect_err("must reject");
        assert!(
            matches!(
                err,
                ProtocolError::InconsistentOutcome {
                    tag: "res",
                    id: 4,
                    ok: false,
                    has_error: false
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn res_claiming_success_while_carrying_an_error_is_rejected() {
        let wire = br#"{"t":"res","id":4,"ok":true,"error":{"code":"x","message":"y"}}"#;
        let err = Envelope::from_json_slice(wire).expect_err("must reject");
        assert!(
            matches!(err, ProtocolError::InconsistentOutcome { tag: "res", .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn end_claiming_failure_without_an_error_is_rejected() {
        let wire = br#"{"t":"end","id":9,"ok":false}"#;
        let err = Envelope::from_json_slice(wire).expect_err("must reject");
        assert!(
            matches!(
                err,
                ProtocolError::InconsistentOutcome {
                    tag: "end",
                    id: 9,
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn consistent_outcomes_decode_in_both_directions() {
        for wire in [
            &br#"{"t":"res","id":1,"ok":true,"payload":{"a":1}}"#[..],
            &br#"{"t":"res","id":1,"ok":false,"error":{"code":"c","message":"m"}}"#[..],
            &br#"{"t":"end","id":2,"ok":true}"#[..],
            &br#"{"t":"end","id":2,"ok":false,"error":{"code":"stream_overflow","message":"m"}}"#[..],
        ] {
            Envelope::from_json_slice(wire).unwrap_or_else(|e| {
                panic!("{} should decode: {e:?}", String::from_utf8_lossy(wire))
            });
        }
    }

    #[test]
    fn a_stream_overflow_end_is_the_shape_the_contract_names() {
        // Contract §7: overflow ends the stream loudly with this exact code.
        // Silently dropping chunks instead would leave the client and the
        // durable event log disagreeing with nothing able to detect it.
        let wire = br#"{"t":"end","id":3,"ok":false,"error":{"code":"stream_overflow","message":"buffer full"}}"#;
        let Envelope::End(end) = Envelope::from_json_slice(wire).expect("decodes") else {
            panic!("expected an end envelope");
        };
        assert!(!end.ok);
        assert_eq!(end.error.expect("error present").code, "stream_overflow");
    }

    use serde_json::json;

    /// Assert the exact on-wire bytes in BOTH directions. The literal is the
    /// point: a rename, a reordered field or a lost camelCase key breaks this
    /// test instead of breaking the TypeScript peer at run time.
    fn assert_wire(envelope: &Envelope, wire: &str) {
        assert_eq!(serde_json::to_string(envelope).unwrap(), wire);
        assert_eq!(
            &Envelope::from_json_slice(wire.as_bytes()).unwrap(),
            envelope
        );
    }

    #[test]
    fn protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn hello_round_trips() {
        assert_wire(
            &Envelope::Hello(Hello {
                protocol_version: 1,
                host_version: "0.1.0".into(),
                agentkit_contract_version: "2026-05-01".into(),
                pid: 1234,
                session_nonce: "n0nce".into(),
            }),
            r#"{"t":"hello","protocolVersion":1,"hostVersion":"0.1.0","agentkitContractVersion":"2026-05-01","pid":1234,"sessionNonce":"n0nce"}"#,
        );
    }

    #[test]
    fn accept_round_trips() {
        assert_wire(
            &Envelope::Accept(Accept {
                protocol_version: 1,
                app_version: "0.1.0".into(),
                bridge_version: "0.1.0".into(),
            }),
            r#"{"t":"accept","protocolVersion":1,"appVersion":"0.1.0","bridgeVersion":"0.1.0"}"#,
        );
    }

    #[test]
    fn reject_round_trips() {
        assert_wire(
            &Envelope::Reject(Reject {
                reason: "unsupported protocol version 2".into(),
            }),
            r#"{"t":"reject","reason":"unsupported protocol version 2"}"#,
        );
    }

    #[test]
    fn req_round_trips() {
        assert_wire(
            &Envelope::Req(Req {
                id: 12,
                principal: Principal::Ui,
                verb: "agentkit.fetch".into(),
                payload: json!({}),
            }),
            r#"{"t":"req","id":12,"principal":"ui","verb":"agentkit.fetch","payload":{}}"#,
        );
    }

    #[test]
    fn req_with_host_principal_round_trips() {
        assert_wire(
            &Envelope::Req(Req {
                id: 7,
                principal: Principal::Host,
                verb: "provider.fetch".into(),
                payload: json!({"providerId": "local"}),
            }),
            r#"{"t":"req","id":7,"principal":"host","verb":"provider.fetch","payload":{"providerId":"local"}}"#,
        );
    }

    #[test]
    fn req_without_payload_defaults_to_null() {
        let env =
            Envelope::from_json_slice(br#"{"t":"req","id":4,"principal":"ui","verb":"shutdown"}"#)
                .unwrap();
        match env {
            Envelope::Req(req) => assert_eq!(req.payload, Value::Null),
            other => panic!("expected req, got {other:?}"),
        }
    }

    #[test]
    fn res_ok_round_trips() {
        assert_wire(
            &Envelope::Res(Res {
                id: 12,
                ok: true,
                payload: Some(json!({})),
                error: None,
            }),
            r#"{"t":"res","id":12,"ok":true,"payload":{}}"#,
        );
    }

    #[test]
    fn res_error_round_trips() {
        assert_wire(
            &Envelope::Res(Res {
                id: 12,
                ok: false,
                payload: None,
                error: Some(ErrorObject {
                    code: "provider_unavailable".into(),
                    message: "no such provider id".into(),
                }),
            }),
            r#"{"t":"res","id":12,"ok":false,"error":{"code":"provider_unavailable","message":"no such provider id"}}"#,
        );
    }

    #[test]
    fn chunk_round_trips() {
        assert_wire(
            &Envelope::Chunk(Chunk { id: 12, seq: 0 }),
            r#"{"t":"chunk","id":12,"seq":0}"#,
        );
    }

    #[test]
    fn end_ok_round_trips() {
        assert_wire(
            &Envelope::End(End {
                id: 12,
                ok: true,
                error: None,
            }),
            r#"{"t":"end","id":12,"ok":true}"#,
        );
    }

    #[test]
    fn end_overflow_round_trips() {
        assert_wire(
            &Envelope::End(End {
                id: 12,
                ok: false,
                error: Some(ErrorObject {
                    code: "stream_overflow".into(),
                    message: "per-stream buffer exceeded".into(),
                }),
            }),
            r#"{"t":"end","id":12,"ok":false,"error":{"code":"stream_overflow","message":"per-stream buffer exceeded"}}"#,
        );
    }

    #[test]
    fn cancel_round_trips() {
        assert_wire(
            &Envelope::Cancel(Cancel { id: 12 }),
            r#"{"t":"cancel","id":12}"#,
        );
    }

    #[test]
    fn ping_round_trips() {
        assert_wire(&Envelope::Ping(Ping { id: 7 }), r#"{"t":"ping","id":7}"#);
    }

    #[test]
    fn pong_round_trips() {
        assert_wire(&Envelope::Pong(Pong { id: 7 }), r#"{"t":"pong","id":7}"#);
    }

    #[test]
    fn envelope_to_json_vec_matches_to_string() {
        let env = Envelope::Ping(Ping { id: 3 });
        assert_eq!(
            env.to_json_vec().unwrap(),
            br#"{"t":"ping","id":3}"#.to_vec()
        );
    }

    #[test]
    fn unknown_tag_is_an_error_not_a_skip() {
        match Envelope::from_json_slice(br#"{"t":"nope","id":1}"#) {
            Err(ProtocolError::Json(_)) => {}
            other => panic!("expected a Json error for an unknown tag, got {other:?}"),
        }
    }

    #[test]
    fn missing_tag_is_an_error() {
        assert!(matches!(
            Envelope::from_json_slice(br#"{"id":1}"#),
            Err(ProtocolError::Json(_))
        ));
    }

    #[test]
    fn unknown_principal_is_an_error() {
        // `principal` is a closed enum; "admin" must not deserialize to anything.
        assert!(matches!(
            Envelope::from_json_slice(
                br#"{"t":"req","id":1,"principal":"admin","verb":"shutdown","payload":{}}"#
            ),
            Err(ProtocolError::Json(_))
        ));
    }

    #[test]
    fn principal_wire_spelling_is_lowercase() {
        assert_eq!(serde_json::to_string(&Principal::Ui).unwrap(), "\"ui\"");
        assert_eq!(serde_json::to_string(&Principal::Host).unwrap(), "\"host\"");
        assert_eq!(Principal::Ui.as_str(), "ui");
        assert_eq!(Principal::Host.as_str(), "host");
        assert_eq!(Principal::Host.to_string(), "host");
    }

    // -- verb tables --------------------------------------------------------

    #[test]
    fn host_to_sidecar_table_matches_the_contract() {
        let rows: Vec<_> = HOST_TO_SIDECAR_VERBS
            .entries()
            .iter()
            .map(|e| (e.verb, e.principals))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("agentkit.fetch", &[Principal::Ui][..]),
                ("shutdown", &[Principal::Ui][..]),
            ]
        );
    }

    #[test]
    fn sidecar_to_host_table_matches_the_contract() {
        let rows: Vec<_> = SIDECAR_TO_HOST_VERBS
            .entries()
            .iter()
            .map(|e| (e.verb, e.principals))
            .collect();
        assert_eq!(rows, vec![("provider.fetch", &[Principal::Host][..])]);
    }

    #[test]
    fn each_host_to_sidecar_verb_allows_exactly_its_listed_principals() {
        for verb in ["agentkit.fetch", "shutdown"] {
            assert!(HOST_TO_SIDECAR_VERBS.allows(verb, Principal::Ui), "{verb}");
            assert!(
                !HOST_TO_SIDECAR_VERBS.allows(verb, Principal::Host),
                "{verb}"
            );
            assert!(HOST_TO_SIDECAR_VERBS.check(verb, Principal::Ui).is_ok());
        }
    }

    #[test]
    fn each_sidecar_to_host_verb_allows_exactly_its_listed_principals() {
        assert!(SIDECAR_TO_HOST_VERBS.allows("provider.fetch", Principal::Host));
        assert!(!SIDECAR_TO_HOST_VERBS.allows("provider.fetch", Principal::Ui));
    }

    #[test]
    fn ui_principal_cannot_call_a_host_only_verb() {
        match SIDECAR_TO_HOST_VERBS.check("provider.fetch", Principal::Ui) {
            Err(ProtocolError::VerbRefused { verb, principal }) => {
                assert_eq!(verb, "provider.fetch");
                assert_eq!(principal, Principal::Ui);
            }
            other => panic!("expected VerbRefused, got {other:?}"),
        }
    }

    #[test]
    fn host_principal_cannot_call_a_ui_only_verb() {
        match HOST_TO_SIDECAR_VERBS.check("shutdown", Principal::Host) {
            Err(ProtocolError::VerbRefused { verb, principal }) => {
                assert_eq!(verb, "shutdown");
                assert_eq!(principal, Principal::Host);
            }
            other => panic!("expected VerbRefused, got {other:?}"),
        }
    }

    #[test]
    fn a_verb_from_the_other_direction_is_unknown_here() {
        // The tables are per-direction: the host's verbs are not the sidecar's.
        match HOST_TO_SIDECAR_VERBS.check("provider.fetch", Principal::Host) {
            Err(ProtocolError::UnknownVerb(verb)) => assert_eq!(verb, "provider.fetch"),
            other => panic!("expected UnknownVerb, got {other:?}"),
        }
        match SIDECAR_TO_HOST_VERBS.check("agentkit.fetch", Principal::Ui) {
            Err(ProtocolError::UnknownVerb(verb)) => assert_eq!(verb, "agentkit.fetch"),
            other => panic!("expected UnknownVerb, got {other:?}"),
        }
    }

    #[test]
    fn an_unlisted_verb_is_refused_for_every_principal() {
        for table in [HOST_TO_SIDECAR_VERBS, SIDECAR_TO_HOST_VERBS] {
            for verb in ["", "fs.read", "process.spawn", "AGENTKIT.FETCH", "agentkit"] {
                for principal in [Principal::Ui, Principal::Host] {
                    assert!(!table.allows(verb, principal), "{verb} / {principal}");
                    assert!(matches!(
                        table.check(verb, principal),
                        Err(ProtocolError::UnknownVerb(_))
                    ));
                }
            }
        }
        // There is no wildcard row hiding in either table.
        assert!(HOST_TO_SIDECAR_VERBS.principals("*").is_none());
        assert!(SIDECAR_TO_HOST_VERBS.principals("*").is_none());
    }

    #[test]
    fn an_empty_table_refuses_everything() {
        let empty = VerbTable::new(&[]);
        assert!(!empty.allows("agentkit.fetch", Principal::Ui));
        assert!(matches!(
            empty.check("agentkit.fetch", Principal::Ui),
            Err(ProtocolError::UnknownVerb(_))
        ));
    }

    // -- request ids --------------------------------------------------------

    #[test]
    fn host_allocator_yields_only_even_ids_strictly_increasing() {
        let alloc = IdAllocator::host();
        let mut prev = None;
        for _ in 0..1_000 {
            let id = alloc.next();
            assert_eq!(id % 2, 0, "host ids must be even");
            if let Some(p) = prev {
                assert!(id > p, "ids must strictly increase");
            }
            prev = Some(id);
        }
    }

    #[test]
    fn sidecar_allocator_yields_only_odd_ids_strictly_increasing() {
        let alloc = IdAllocator::sidecar();
        let mut prev = None;
        for _ in 0..1_000 {
            let id = alloc.next();
            assert_eq!(id % 2, 1, "sidecar ids must be odd");
            if let Some(p) = prev {
                assert!(id > p, "ids must strictly increase");
            }
            prev = Some(id);
        }
    }

    #[test]
    fn the_two_id_spaces_never_collide() {
        let host = IdAllocator::host();
        let sidecar = IdAllocator::sidecar();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..500 {
            assert!(seen.insert(host.next()), "host id reused");
            assert!(seen.insert(sidecar.next()), "sidecar id reused");
        }
        assert_eq!(seen.len(), 1_000);
    }

    #[test]
    fn allocators_start_where_the_contract_says() {
        assert_eq!(IdAllocator::host().next(), 0);
        assert_eq!(IdAllocator::sidecar().next(), 1);
    }

    #[test]
    fn a_shared_allocator_never_hands_out_a_duplicate() {
        use std::sync::Arc;
        // Duplex operation means two tasks may allocate concurrently (§6).
        let alloc = Arc::new(IdAllocator::host());
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let alloc = Arc::clone(&alloc);
                std::thread::spawn(move || (0..250).map(|_| alloc.next()).collect::<Vec<_>>())
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        for handle in handles {
            for id in handle.join().expect("thread") {
                assert!(seen.insert(id), "duplicate id {id}");
            }
        }
        assert_eq!(seen.len(), 1_000);
    }
}
