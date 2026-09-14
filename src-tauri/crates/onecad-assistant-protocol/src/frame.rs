//! OCAK1 wire framing.
//!
//! Frame layout (single frame): `magic "OCAK" + u32 jsonLen + u32 binLen + JSON
//! envelope + binary tail`. Both lengths are little-endian. The child's stdout
//! carries frames only; every log goes to stderr. There is NO resync after a bad
//! frame — the supervisor restarts the child. See
//! `../../../docs/assistant/wire-protocol.md` §1.
//!
//! This is NOT OCW1. OCAK1 borrows the geometry worker's frame *shape* because
//! that shape is proven in this codebase, but it has its own magic, its own caps
//! and no shared code: a frame from one protocol must never decode as the other,
//! which is exactly what a distinct magic buys.
//!
//! Two layers share one cap/magic check:
//! - pure [`encode_frame`] / [`decode_frame`] over byte slices (no async deps);
//! - the async [`OcakCodec`] in [`crate::codec`], behind the `codec` feature.
//!
//! There is deliberately no blocking read/write layer: both peers are async (a
//! tokio-supervised child on one end, Bun on the other), so a synchronous one
//! would be untested dead code.

use crate::error::ProtocolError;

/// Frame magic: the byte sequence `O C A K` (`0x4F 0x43 0x41 0x4B`) on the wire.
/// The BYTE SEQUENCE is normative per the wire contract §1 — always compare
/// bytes, never an endian-decoded integer.
pub const MAGIC_BYTES: [u8; 4] = *b"OCAK";

/// Maximum JSON envelope length: 1 MiB. Control frames are small; a megabyte is
/// the contract's stated starting limit and is far above any legal envelope.
pub const MAX_JSON_LEN: u32 = 1024 * 1024;

/// Maximum binary tail length: 8 MiB — one stream chunk. Larger payloads are
/// split across `chunk` frames rather than growing the cap.
pub const MAX_BIN_LEN: u32 = 8 * 1024 * 1024;

/// Fixed frame header size: `magic(4) + jsonLen(4) + binLen(4)`.
pub const HEADER_LEN: usize = 12;

/// The fixed-size frame header preceding the JSON envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Length of the JSON envelope in bytes. Never zero on a valid frame.
    pub json_len: u32,
    /// Length of the binary tail in bytes. Zero on every envelope but `chunk`.
    pub bin_len: u32,
}

impl FrameHeader {
    /// Total on-wire size of the frame this header describes.
    ///
    /// Cannot overflow: [`parse_header`] rejects both lengths above their caps
    /// first, so the sum is bounded by `12 + 1 MiB + 8 MiB`.
    pub fn total_len(&self) -> usize {
        HEADER_LEN + self.json_len as usize + self.bin_len as usize
    }
}

/// One decoded OCAK1 frame: the raw JSON envelope bytes and the raw binary tail.
///
/// The framing layer is envelope-agnostic — it never parses the JSON. Deciding
/// whether a tail is legal for the envelope that carried it (only `chunk` may
/// have one) belongs to the layer that understands [`crate::envelope::Envelope`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawFrame {
    /// UTF-8 JSON envelope bytes (no BOM, no trailing NUL).
    pub json: Vec<u8>,
    /// Raw binary tail bytes (may be empty).
    pub bin: Vec<u8>,
}

impl RawFrame {
    /// A frame with a JSON envelope and no binary tail — every envelope except
    /// `chunk`.
    pub fn json_only(json: Vec<u8>) -> Self {
        RawFrame {
            json,
            bin: Vec::new(),
        }
    }
}

/// Read the little-endian header lengths from the first [`HEADER_LEN`] bytes.
///
/// Validates the magic (as bytes), the empty-JSON rule and both caps. Callers
/// that already hold ≥12 bytes use this to learn the total frame size before
/// buffering a body.
pub(crate) fn parse_header(head: &[u8]) -> Result<FrameHeader, ProtocolError> {
    debug_assert!(head.len() >= HEADER_LEN);
    if head[0..4] != MAGIC_BYTES {
        return Err(ProtocolError::bad_magic([
            head[0], head[1], head[2], head[3],
        ]));
    }
    let json_len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    let bin_len = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
    if json_len == 0 {
        return Err(ProtocolError::EmptyJson);
    }
    if json_len > MAX_JSON_LEN {
        return Err(ProtocolError::TooLarge {
            what: "json",
            len: json_len,
            cap: MAX_JSON_LEN,
        });
    }
    if bin_len > MAX_BIN_LEN {
        return Err(ProtocolError::TooLarge {
            what: "bin",
            len: bin_len,
            cap: MAX_BIN_LEN,
        });
    }
    Ok(FrameHeader { json_len, bin_len })
}

/// Serialize one frame to a fresh `Vec<u8>`.
///
/// Enforces the caps and the empty-JSON rule on the SEND side too, so a local
/// bug is caught at its own call site instead of as an unexplained teardown at
/// the peer. Pure — no async or `bytes` dependency.
pub fn encode_frame(json: &[u8], bin: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if json.is_empty() {
        return Err(ProtocolError::EmptyJson);
    }
    // A slice longer than u32::MAX cannot be a legal length; report it as the
    // cap violation it is rather than truncating the conversion.
    let json_len = u32::try_from(json.len()).unwrap_or(u32::MAX);
    if json_len > MAX_JSON_LEN {
        return Err(ProtocolError::TooLarge {
            what: "json",
            len: json_len,
            cap: MAX_JSON_LEN,
        });
    }
    let bin_len = u32::try_from(bin.len()).unwrap_or(u32::MAX);
    if bin_len > MAX_BIN_LEN {
        return Err(ProtocolError::TooLarge {
            what: "bin",
            len: bin_len,
            cap: MAX_BIN_LEN,
        });
    }
    let mut out = Vec::with_capacity(HEADER_LEN + json.len() + bin.len());
    out.extend_from_slice(&MAGIC_BYTES);
    out.extend_from_slice(&json_len.to_le_bytes());
    out.extend_from_slice(&bin_len.to_le_bytes());
    out.extend_from_slice(json);
    out.extend_from_slice(bin);
    Ok(out)
}

/// Try to decode one frame from the front of `buf`.
///
/// - `Ok(Some((frame, consumed)))` — a full frame; `consumed` bytes may be
///   dropped from the front of the buffer.
/// - `Ok(None)` — not enough bytes yet; the caller should read more.
/// - `Err(_)` — a fatal framing violation (bad magic / empty json / over-cap).
///   No resync: the connection is torn down.
///
/// Caps are checked from the header BEFORE requiring the body, so a header
/// claiming `u32::MAX` errors immediately instead of waiting for four gigabytes
/// that will never arrive. This function never panics on any input.
pub fn decode_frame(buf: &[u8]) -> Result<Option<(RawFrame, usize)>, ProtocolError> {
    if buf.len() < HEADER_LEN {
        return Ok(None);
    }
    let header = parse_header(buf)?;
    let json_len = header.json_len as usize;
    let total = header.total_len();
    if buf.len() < total {
        return Ok(None);
    }
    let json = buf[HEADER_LEN..HEADER_LEN + json_len].to_vec();
    let bin = buf[HEADER_LEN + json_len..total].to_vec();
    Ok(Some((RawFrame { json, bin }, total)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_bytes(json: &[u8], bin: &[u8]) -> Vec<u8> {
        encode_frame(json, bin).expect("encode")
    }

    fn header_bytes(json_len: u32, bin_len: u32) -> Vec<u8> {
        let mut head = Vec::with_capacity(HEADER_LEN);
        head.extend_from_slice(&MAGIC_BYTES);
        head.extend_from_slice(&json_len.to_le_bytes());
        head.extend_from_slice(&bin_len.to_le_bytes());
        head
    }

    #[test]
    fn magic_bytes_are_normative_ocak() {
        assert_eq!(&MAGIC_BYTES, b"OCAK");
        // Distinct from OCW1 on purpose: a geometry frame must never decode here.
        assert_ne!(&MAGIC_BYTES, b"OCW1");
    }

    #[test]
    fn caps_match_the_wire_contract() {
        assert_eq!(MAX_JSON_LEN, 1024 * 1024);
        assert_eq!(MAX_BIN_LEN, 8 * 1024 * 1024);
        assert_eq!(HEADER_LEN, 12);
    }

    #[test]
    fn header_lengths_are_little_endian() {
        let bytes = frame_bytes(br#"{"t":"ping","id":7}"#, &[0xAA, 0xBB]);
        assert_eq!(&bytes[0..4], b"OCAK");
        assert_eq!(&bytes[4..8], &[19, 0, 0, 0]); // jsonLen = 19, LE
        assert_eq!(&bytes[8..12], &[2, 0, 0, 0]); // binLen  = 2,  LE
    }

    #[test]
    fn round_trip_json_and_bin() {
        let bytes = frame_bytes(br#"{"t":"chunk","id":12,"seq":0}"#, &[1, 2, 3, 4]);
        let (frame, consumed) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(frame.json, br#"{"t":"chunk","id":12,"seq":0}"#);
        assert_eq!(frame.bin, vec![1, 2, 3, 4]);
    }

    #[test]
    fn round_trip_no_bin() {
        let bytes = frame_bytes(br#"{"t":"pong","id":7}"#, &[]);
        let (frame, consumed) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(consumed, bytes.len());
        assert!(frame.bin.is_empty());
        assert_eq!(frame.json, br#"{"t":"pong","id":7}"#);
    }

    #[test]
    fn json_only_constructor_has_empty_tail() {
        let frame = RawFrame::json_only(b"{}".to_vec());
        assert!(frame.bin.is_empty());
    }

    #[test]
    fn empty_json_rejected_on_encode() {
        assert!(matches!(
            encode_frame(&[], &[1, 2, 3]),
            Err(ProtocolError::EmptyJson)
        ));
    }

    #[test]
    fn empty_json_rejected_on_decode() {
        assert!(matches!(
            decode_frame(&header_bytes(0, 0)),
            Err(ProtocolError::EmptyJson)
        ));
    }

    #[test]
    fn bad_magic_is_fatal() {
        let mut bytes = frame_bytes(b"{}", &[]);
        bytes[0] = b'X';
        match decode_frame(&bytes) {
            Err(ProtocolError::BadMagic { got, expected }) => {
                assert_eq!(expected, MAGIC_BYTES);
                assert_eq!(got, [b'X', b'C', b'A', b'K']);
            }
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }

    #[test]
    fn ocw1_magic_does_not_decode_as_ocak1() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OCW1");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"{}");
        assert!(matches!(
            decode_frame(&bytes),
            Err(ProtocolError::BadMagic { .. })
        ));
    }

    #[test]
    fn oversize_json_rejected() {
        match decode_frame(&header_bytes(MAX_JSON_LEN + 1, 0)) {
            Err(ProtocolError::TooLarge { what, len, cap }) => {
                assert_eq!(what, "json");
                assert_eq!(len, MAX_JSON_LEN + 1);
                assert_eq!(cap, MAX_JSON_LEN);
            }
            other => panic!("expected TooLarge json, got {other:?}"),
        }
    }

    #[test]
    fn oversize_bin_rejected() {
        match decode_frame(&header_bytes(2, MAX_BIN_LEN + 1)) {
            Err(ProtocolError::TooLarge { what, len, cap }) => {
                assert_eq!(what, "bin");
                assert_eq!(len, MAX_BIN_LEN + 1);
                assert_eq!(cap, MAX_BIN_LEN);
            }
            other => panic!("expected TooLarge bin, got {other:?}"),
        }
    }

    #[test]
    fn encode_rejects_over_cap_json() {
        // 1 MiB + 1 is cheap to allocate and proves the send-side cap branch.
        let huge = vec![b'a'; MAX_JSON_LEN as usize + 1];
        match encode_frame(&huge, &[]) {
            Err(ProtocolError::TooLarge { what, cap, .. }) => {
                assert_eq!(what, "json");
                assert_eq!(cap, MAX_JSON_LEN);
            }
            other => panic!("expected TooLarge json, got {other:?}"),
        }
    }

    #[test]
    fn declared_length_that_would_overflow_usize_is_rejected() {
        // A header claiming u32::MAX for both sections: the caps must reject it
        // from the header alone, so `HEADER_LEN + json + bin` is never computed
        // (and on a 32-bit target never wraps).
        match decode_frame(&header_bytes(u32::MAX, u32::MAX)) {
            Err(ProtocolError::TooLarge { what, len, .. }) => {
                assert_eq!(what, "json");
                assert_eq!(len, u32::MAX);
            }
            other => panic!("expected TooLarge json, got {other:?}"),
        }
        // Same for an in-cap json with a u32::MAX tail.
        match decode_frame(&header_bytes(2, u32::MAX)) {
            Err(ProtocolError::TooLarge { what, .. }) => assert_eq!(what, "bin"),
            other => panic!("expected TooLarge bin, got {other:?}"),
        }
    }

    #[test]
    fn split_at_every_byte_boundary_needs_more() {
        let bytes = frame_bytes(br#"{"t":"chunk","id":4,"seq":1}"#, &[7, 8, 9]);
        // Every proper prefix is incomplete, including all 12 header prefixes.
        for n in 0..bytes.len() {
            match decode_frame(&bytes[..n]) {
                Ok(None) => {}
                other => panic!("prefix of {n} bytes should need more, got {other:?}"),
            }
        }
        let (frame, consumed) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(frame.bin, vec![7, 8, 9]);
    }

    #[test]
    fn truncated_payload_needs_more_not_an_error() {
        let bytes = frame_bytes(br#"{"t":"end","id":3,"ok":true}"#, &[1, 2, 3, 4, 5]);
        // One byte short of the tail: still Ok(None), never a partial frame.
        assert!(matches!(decode_frame(&bytes[..bytes.len() - 1]), Ok(None)));
        // Header complete, JSON half-arrived.
        assert!(matches!(decode_frame(&bytes[..HEADER_LEN + 4]), Ok(None)));
    }

    #[test]
    fn two_frames_coalesced_in_one_buffer() {
        let first = frame_bytes(br#"{"t":"ping","id":2}"#, &[]);
        let second = frame_bytes(br#"{"t":"chunk","id":2,"seq":0}"#, &[0xFE]);
        let mut buf = first.clone();
        buf.extend_from_slice(&second);

        let (f1, c1) = decode_frame(&buf).unwrap().unwrap();
        assert_eq!(c1, first.len());
        assert_eq!(f1.json, br#"{"t":"ping","id":2}"#);
        assert!(f1.bin.is_empty());

        let (f2, c2) = decode_frame(&buf[c1..]).unwrap().unwrap();
        assert_eq!(c2, second.len());
        assert_eq!(f2.json, br#"{"t":"chunk","id":2,"seq":0}"#);
        assert_eq!(f2.bin, vec![0xFE]);

        assert_eq!(c1 + c2, buf.len());
        assert!(matches!(decode_frame(&buf[c1 + c2..]), Ok(None)));
    }

    /// Fuzz-ish: decode must never panic on arbitrary input and must return
    /// `Ok(None)` / `Ok(Some)` / `Err`, never diverge. Cheap xorshift PRNG so the
    /// test has no external dependency.
    #[test]
    fn decode_never_panics_on_random_bytes() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..5_000 {
            let len = (next() % 64) as usize;
            let mut buf = vec![0u8; len];
            for b in &mut buf {
                *b = (next() & 0xFF) as u8;
            }
            let _ = decode_frame(&buf);
        }
    }

    /// Random *valid-magic* headers with random declared lengths: still must not
    /// panic, and an over-cap length must be rejected rather than buffered.
    #[test]
    fn decode_never_panics_with_valid_magic_random_lengths() {
        let mut state: u64 = 0x1234_5678_9ABC_DEF0;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            state
        };
        for _ in 0..5_000 {
            let json_len = (next() & 0xFFFF_FFFF) as u32;
            let bin_len = (next() & 0xFFFF_FFFF) as u32;
            let mut buf = header_bytes(json_len, bin_len);
            let extra = (next() % 8) as usize;
            buf.extend(std::iter::repeat_n(0xABu8, extra));
            match decode_frame(&buf) {
                Ok(_) => {}
                Err(ProtocolError::TooLarge { .. }) | Err(ProtocolError::EmptyJson) => {}
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }
    }
}
