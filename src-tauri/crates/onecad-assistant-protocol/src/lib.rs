//! OCAK1 — the OneCAD assistant-bridge protocol.
//!
//! The source of truth for this wire contract is
//! `../../../docs/assistant/wire-protocol.md`, which is normative for both
//! implementations: this crate (Rust, host side) and `assistant-host/src/bridge/`
//! (TypeScript, sidecar side). OCAK1 carries traffic between the OneCAD Rust host
//! and the supervised Bun assistant host over the child's stdin/stdout.
//!
//! **This is not OCW1.** The geometry worker's protocol is the modeling module's
//! contract and nothing outside `onecad.modeling` speaks it. OCAK1 borrows OCW1's
//! *shape* — magic, two length prefixes, a JSON envelope and an opaque tail —
//! because that shape is proven here, but it shares no code, no magic and no
//! caps with it.
//!
//! Modules:
//! - [`frame`] — framing: the `OCAK` magic, the 12-byte header, caps, and the
//!   pure [`frame::encode_frame`] / [`frame::decode_frame`] pair;
//! - [`envelope`] — the [`envelope::Envelope`] tagged enum, [`envelope::Principal`],
//!   the per-direction [`envelope::VerbTable`]s and the [`envelope::IdAllocator`];
//! - [`codec`] — the async `tokio_util` [`codec::OcakCodec`] (feature `codec`);
//! - [`error`] — [`ProtocolError`], the one error type all of the above return.
//!
//! The crate is deliberately dependency-light: no `tauri`, no `onecad-core`, and
//! without the `codec` feature no async runtime at all. It is the shared
//! vocabulary of the bridge, so anything that can only be said by one side of it
//! belongs somewhere else.
//!
//! The child's stdout carries frames only; every log line goes to stderr, where
//! the supervisor forwards it into `tracing` under the target `assistant`. One
//! stray `console.log` on stdout corrupts the frame stream.

pub mod envelope;
pub mod error;
pub mod frame;

#[cfg(feature = "codec")]
pub mod codec;

pub use envelope::{
    Accept, Cancel, Chunk, End, Envelope, ErrorObject, Hello, IdAllocator, Ping, Pong, Principal,
    Reject, Req, Res, VerbEntry, VerbTable, HOST_TO_SIDECAR_VERBS, MAX_SAFE_ID, PROTOCOL_VERSION,
    SIDECAR_TO_HOST_VERBS,
};
pub use error::ProtocolError;
pub use frame::{
    decode_frame, encode_frame, FrameHeader, RawFrame, HEADER_LEN, MAGIC_BYTES, MAX_BIN_LEN,
    MAX_JSON_LEN,
};

#[cfg(feature = "codec")]
pub use codec::OcakCodec;
