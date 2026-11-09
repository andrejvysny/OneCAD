//! Bounded serialization buffers used before filesystem publication.
use super::{IoError, IoResult};
use std::io::{self, Write};

pub(super) struct Buffer {
    pub bytes: Vec<u8>,
    cap: u64,
    exceeded: bool,
}

impl Buffer {
    pub fn new(cap: u64) -> Self {
        Self {
            bytes: Vec::new(),
            cap,
            exceeded: false,
        }
    }

    pub fn finish(self, result: Result<(), serde_json::Error>, name: &str) -> IoResult<Vec<u8>> {
        if self.exceeded {
            return Err(IoError::TooLarge(format!(
                "{name} serialization cap exceeded"
            )));
        }
        result.map_err(|error| IoError::Corrupt(format!("{name} serialize: {error}")))?;
        Ok(self.bytes)
    }
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if (self.bytes.len() as u64)
            .checked_add(bytes.len() as u64)
            .is_none_or(|n| n > self.cap)
        {
            self.exceeded = true;
            return Err(io::Error::other("serialization cap exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn pretty<T: serde::Serialize + ?Sized>(
    value: &T,
    cap: u64,
    name: &str,
) -> IoResult<Vec<u8>> {
    let mut buffer = Buffer::new(cap);
    let result = serde_json::to_writer_pretty(&mut buffer, value);
    buffer.finish(result, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_serialized_boundary_is_accepted_and_one_more_refused() {
        assert_eq!(pretty(&"x", 3, "test").unwrap(), b"\"x\"");
        assert!(matches!(
            pretty(&"xx", 3, "test"),
            Err(IoError::TooLarge(_))
        ));
    }

    #[test]
    fn failed_write_never_extends_buffer_past_cap() {
        let mut buffer = Buffer::new(2);
        buffer.write_all(b"ok").unwrap();
        assert!(buffer.write_all(b"x").is_err());
        assert_eq!(buffer.bytes, b"ok");
    }
}
