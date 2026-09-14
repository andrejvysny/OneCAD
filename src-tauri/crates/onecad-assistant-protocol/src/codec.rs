//! Async OCAK1 codec (`tokio_util::codec`), behind the `codec` feature.
//!
//! The bridge is DUPLEX, not request/response lockstep: both sides must keep
//! draining inbound frames while an outbound request awaits its reply, or the
//! first provider callback that lands mid-request deadlocks the bridge (contract
//! §6). A `Framed` stream over this codec is what makes that draining
//! independent of any pending response.

use bytes::{Buf, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::error::ProtocolError;
use crate::frame::{decode_frame, encode_frame, parse_header, RawFrame, HEADER_LEN};

/// Custom OCAK1 codec (NOT the stock `LengthDelimitedCodec`, which knows nothing
/// about the magic, the two-section split or the caps).
///
/// `Decoder` yields [`RawFrame`]s and handles partial reads, cap violations
/// (→ [`ProtocolError::TooLarge`]), an empty JSON section
/// (→ [`ProtocolError::EmptyJson`]), bad magic (→ [`ProtocolError::BadMagic`])
/// and EOF mid-frame (→ [`ProtocolError::ConnectionLost`] via `decode_eof`).
/// Every one of those is fatal: the supervisor tears the child down and
/// restarts it rather than attempting a resync.
#[derive(Debug, Clone, Copy, Default)]
pub struct OcakCodec;

impl Decoder for OcakCodec {
    type Item = RawFrame;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match decode_frame(src)? {
            Some((frame, consumed)) => {
                src.advance(consumed);
                Ok(Some(frame))
            }
            None => {
                // Reserve the remainder we already know we need, so the framed
                // read grabs the rest of the frame in as few syscalls as
                // possible instead of growing the buffer a read at a time.
                if src.len() >= HEADER_LEN {
                    let header = parse_header(src)?; // caps already validated here
                    let total = header.total_len();
                    if total > src.len() {
                        src.reserve(total - src.len());
                    }
                }
                Ok(None)
            }
        }
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(src)? {
            Some(frame) => Ok(Some(frame)),
            None => {
                if src.is_empty() {
                    Ok(None) // clean end of stream, exactly at a frame boundary
                } else {
                    Err(ProtocolError::ConnectionLost("eof mid-frame"))
                }
            }
        }
    }
}

impl Encoder<RawFrame> for OcakCodec {
    type Error = ProtocolError;

    fn encode(&mut self, item: RawFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let bytes = encode_frame(&item.json, &item.bin)?;
        dst.extend_from_slice(&bytes);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{MAGIC_BYTES, MAX_BIN_LEN, MAX_JSON_LEN};

    fn encode(json: &[u8], bin: &[u8]) -> Vec<u8> {
        let mut codec = OcakCodec;
        let mut dst = BytesMut::new();
        codec
            .encode(
                RawFrame {
                    json: json.to_vec(),
                    bin: bin.to_vec(),
                },
                &mut dst,
            )
            .expect("encode");
        dst.to_vec()
    }

    #[test]
    fn codec_encode_matches_pure_encode() {
        assert_eq!(
            encode(br#"{"t":"ping","id":0}"#, &[5, 6]),
            encode_frame(br#"{"t":"ping","id":0}"#, &[5, 6]).unwrap()
        );
    }

    #[test]
    fn decoder_fed_one_byte_at_a_time_emits_exactly_one_frame() {
        let bytes = encode(br#"{"t":"chunk","id":1,"seq":0}"#, &[42]);
        let mut codec = OcakCodec;
        let mut buf = BytesMut::new();
        for &b in &bytes[..bytes.len() - 1] {
            buf.extend_from_slice(&[b]);
            assert!(codec.decode(&mut buf).unwrap().is_none());
        }
        buf.extend_from_slice(&[bytes[bytes.len() - 1]]);
        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.json, br#"{"t":"chunk","id":1,"seq":0}"#);
        assert_eq!(frame.bin, vec![42]);
        // Exactly one frame: the buffer is drained and yields nothing more.
        assert!(buf.is_empty());
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn codec_decodes_two_coalesced_frames() {
        let mut bytes = encode(br#"{"t":"ping","id":2}"#, &[]);
        bytes.extend(encode(br#"{"t":"pong","id":3}"#, &[]));
        let mut codec = OcakCodec;
        let mut buf = BytesMut::from(&bytes[..]);
        let f1 = codec.decode(&mut buf).unwrap().unwrap();
        let f2 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f1.json, br#"{"t":"ping","id":2}"#);
        assert_eq!(f2.json, br#"{"t":"pong","id":3}"#);
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn oversized_json_frame_errors() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&MAGIC_BYTES);
        buf.extend_from_slice(&(MAX_JSON_LEN + 1).to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        let mut codec = OcakCodec;
        match codec.decode(&mut buf) {
            Err(ProtocolError::TooLarge { what, cap, .. }) => {
                assert_eq!(what, "json");
                assert_eq!(cap, MAX_JSON_LEN);
            }
            other => panic!("expected TooLarge json, got {other:?}"),
        }
    }

    #[test]
    fn oversized_bin_frame_errors() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&MAGIC_BYTES);
        buf.extend_from_slice(&2u32.to_le_bytes());
        buf.extend_from_slice(&(MAX_BIN_LEN + 1).to_le_bytes());
        let mut codec = OcakCodec;
        assert!(matches!(
            codec.decode(&mut buf),
            Err(ProtocolError::TooLarge { what: "bin", .. })
        ));
    }

    #[test]
    fn codec_bad_magic_is_fatal() {
        let mut buf = BytesMut::from(&b"XXXX\x02\x00\x00\x00\x00\x00\x00\x00"[..]);
        let mut codec = OcakCodec;
        assert!(matches!(
            codec.decode(&mut buf),
            Err(ProtocolError::BadMagic { .. })
        ));
    }

    #[test]
    fn eof_mid_frame_is_connection_lost() {
        let bytes = encode(br#"{"t":"end","id":1,"ok":true}"#, &[9, 9, 9]);
        let mut codec = OcakCodec;
        let mut buf = BytesMut::from(&bytes[..bytes.len() - 2]);
        assert!(codec.decode(&mut buf).unwrap().is_none());
        match codec.decode_eof(&mut buf) {
            Err(ProtocolError::ConnectionLost(_)) => {}
            other => panic!("expected ConnectionLost, got {other:?}"),
        }
    }

    #[test]
    fn clean_eof_at_a_frame_boundary_is_none() {
        let mut codec = OcakCodec;
        let mut buf = BytesMut::new();
        assert!(codec.decode_eof(&mut buf).unwrap().is_none());
    }

    #[test]
    fn encoder_refuses_an_empty_envelope() {
        let mut codec = OcakCodec;
        let mut dst = BytesMut::new();
        assert!(matches!(
            codec.encode(RawFrame::default(), &mut dst),
            Err(ProtocolError::EmptyJson)
        ));
    }

    #[test]
    fn a_full_envelope_survives_the_codec_round_trip() {
        use crate::envelope::{Chunk, Envelope};
        let envelope = Envelope::Chunk(Chunk { id: 12, seq: 0 });
        let bytes = encode(&envelope.to_json_vec().unwrap(), b"payload bytes");
        let mut codec = OcakCodec;
        let mut buf = BytesMut::from(&bytes[..]);
        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(Envelope::from_json_slice(&frame.json).unwrap(), envelope);
        assert_eq!(frame.bin, b"payload bytes");
    }
}
