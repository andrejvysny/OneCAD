//! Shared error type for the OCAK1 assistant-bridge crate.
//!
//! `ProtocolError` spans framing (magic/caps/EOF), envelope JSON handling, and
//! the principal/verb authority check. Framing violations are fatal per
//! `../../../docs/assistant/wire-protocol.md` §1 — there is no resync, so the
//! supervisor tears the child down and restarts it. `BadMagic`, `TooLarge`,
//! `EmptyJson` and `ConnectionLost` are the variants the codec surfaces; a
//! desynchronised length prefix cannot be recovered from without guessing.

use thiserror::Error;

use crate::envelope::Principal;

/// Errors produced anywhere on the OCAK1 path.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// The 4 frame-magic bytes were not `OCAK`. Fatal per the wire contract §1:
    /// the reader tears down without resync. The magic is compared as BYTES,
    /// never as an endian-decoded integer, because the byte sequence is the
    /// normative form.
    #[error("bad frame magic: expected {expected:02x?}, got {got:02x?}")]
    BadMagic {
        /// The normative expected bytes (`OCAK`).
        expected: [u8; 4],
        /// The bytes actually seen at the frame head.
        got: [u8; 4],
    },

    /// A declared frame section length exceeded its cap (`jsonLen` ≤ 1 MiB,
    /// `binLen` ≤ 8 MiB). Fatal: an over-cap length is either a desynchronised
    /// stream or a peer that ignored the contract, and both are unrecoverable.
    #[error("frame {what} length {len} exceeds cap {cap}")]
    TooLarge {
        /// Which section (`"json"` or `"bin"`).
        what: &'static str,
        /// The declared length.
        len: u32,
        /// The cap it violated.
        cap: u32,
    },

    /// `jsonLen` was zero. Every OCAK1 frame carries a JSON object envelope
    /// (contract §1), so a zero-length JSON section is a framing violation and
    /// not an empty-but-legal frame.
    #[error("frame json section is empty; every OCAK1 frame carries an envelope")]
    EmptyJson,

    /// The stream ended (or errored) part-way through a frame, or the peer
    /// closed the connection. In-flight requests fail with this.
    #[error("connection lost: {0}")]
    ConnectionLost(&'static str),

    /// A `req` named a verb that is not in the receiving side's table. There is
    /// no wildcard entry (contract §4), so an unknown verb is refused rather
    /// than forwarded to a handler that might exist later.
    #[error("unknown verb {0:?}")]
    UnknownVerb(String),

    /// The verb exists but the asserting principal may not call it. The
    /// principal comes from the transport the frame arrived on, never from the
    /// frame's own claim.
    #[error("principal {principal} may not call verb {verb:?}")]
    VerbRefused {
        /// The verb that was refused.
        verb: String,
        /// The principal the transport stamped on the request.
        principal: Principal,
    },

    /// A `res` or `end` frame disagreed with itself: `ok: false` carrying no
    /// `error`, or `ok: true` carrying one. The contract states the invariant as
    /// "`error` iff `!ok`", and the TypeScript peer enforces it on decode — so
    /// accepting such a frame here would let the two sides disagree about which
    /// bytes are legal, which is exactly the class of drift a shared wire
    /// contract exists to prevent.
    #[error("{tag} frame for id {id} is inconsistent: ok={ok} but error is {}", if *.has_error { "present" } else { "absent" })]
    InconsistentOutcome {
        /// `"res"` or `"end"`.
        tag: &'static str,
        /// The request id the frame answers.
        id: u64,
        /// The `ok` flag as received.
        ok: bool,
        /// Whether an `error` object was present.
        has_error: bool,
    },

    /// A frame id exceeded [`crate::envelope::MAX_SAFE_ID`]. Contract §3: ids are
    /// `u64` on the wire but capped at 2^53−1, because the TypeScript peer
    /// cannot compare a larger JSON number against the map key it is supposed to
    /// match. Accepting one here would let Rust route a frame the sidecar refuses.
    #[error("{tag} frame id {id} exceeds the safe-integer cap {cap}")]
    IdOutOfRange {
        /// The envelope tag the id arrived on.
        tag: &'static str,
        /// The id as received.
        id: u64,
        /// The cap it violated.
        cap: u64,
    },

    /// A non-`chunk` envelope carried a non-empty binary tail, or a `chunk`
    /// carried an empty one (contract §2a). The framing layer yields the tail
    /// uninterpreted by design, so this is the dispatcher's check.
    #[error("{tag} frame for id {id} carries a {len}-byte binary tail, which it may not")]
    BadBinaryTail {
        /// The envelope tag that carried (or failed to carry) the tail.
        tag: &'static str,
        /// The request id the frame names.
        id: u64,
        /// The tail length as received.
        len: usize,
    },

    /// JSON (de)serialization of an envelope failed — including an unknown `t`
    /// tag, which the contract §2 makes an error rather than an ignorable frame.
    #[error("json (de)serialization error: {0}")]
    Json(#[from] serde_json::Error),

    /// Underlying transport IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl ProtocolError {
    /// Construct a [`ProtocolError::BadMagic`] with the normative expected bytes.
    pub(crate) fn bad_magic(got: [u8; 4]) -> Self {
        ProtocolError::BadMagic {
            expected: crate::frame::MAGIC_BYTES,
            got,
        }
    }
}
