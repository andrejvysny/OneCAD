//! Bound retained authored records, including flattened extension data, without a JSON buffer.
use onecad_core::document::record::OperationRecord;
use std::io::{self, Write};

#[derive(Default)]
pub struct RecordBudget {
    bytes: usize,
}
impl RecordBudget {
    pub fn charge(&mut self, record: &OperationRecord) -> Result<(), String> {
        let mut counter = BoundedCounter::default();
        // OperationRecord's wire serializer builds an intermediate Value; count its
        // constituent typed fields directly to avoid that unbounded temporary.
        count(&mut counter, &record.op)?;
        count(&mut counter, &record.inputs)?;
        count(&mut counter, &record.outputs)?;
        count(&mut counter, &record.name)?;
        count(&mut counter, &record.extra)?;
        count(&mut counter, &record.determinism)?;
        let estimate = counter
            .bytes
            .checked_mul(32)
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<OperationRecord>()))
            .ok_or("authored record size overflow")?;
        crate::budget::charge(&mut self.bytes, estimate, 16 << 20, "authored records")
    }
}
pub fn check_payload(value: &impl serde::Serialize) -> Result<(), String> {
    count(&mut BoundedCounter::default(), value)
}
fn count(counter: &mut BoundedCounter, value: &impl serde::Serialize) -> Result<(), String> {
    serde_json::to_writer(counter, value).map_err(|error| error.to_string())
}
#[derive(Default)]
struct BoundedCounter {
    bytes: usize,
    depth: usize,
    string: bool,
    escaped: bool,
}
impl Write for BoundedCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|bytes| *bytes <= 1 << 20)
            .ok_or_else(|| io::Error::other("authored record exceeds native metadata budget"))?;
        for byte in bytes {
            if self.string {
                if self.escaped {
                    self.escaped = false;
                } else if *byte == b'\\' {
                    self.escaped = true;
                } else if *byte == b'"' {
                    self.string = false;
                }
            } else {
                match byte {
                    b'"' => self.string = true,
                    b'{' | b'[' => {
                        self.depth += 1;
                        if self.depth > 32 {
                            return Err(io::Error::other("authored metadata nesting exceeded"));
                        }
                    }
                    b'}' | b']' => self.depth = self.depth.saturating_sub(1),
                    _ => {}
                }
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::{
        document::record::{OpaqueOperation, Operation},
        ids::RecordId,
    };
    fn record() -> OperationRecord {
        OperationRecord::new(
            RecordId::new(),
            0,
            "Future feature",
            Operation::Opaque(OpaqueOperation {
                raw: serde_json::Map::new(),
            }),
        )
    }
    #[test]
    fn extension_bytes_and_deep_values_fail_before_retention() {
        let mut record = record();
        record.extra.insert(
            "large".into(),
            serde_json::Value::String("x".repeat(1 << 20)),
        );
        assert!(RecordBudget::default().charge(&record).is_err());
        let mut value = serde_json::Value::Null;
        for _ in 0..40 {
            value = serde_json::json!([value]);
        }
        record.extra.clear();
        record.extra.insert("deep".into(), value);
        assert!(RecordBudget::default().charge(&record).is_err());
        record.extra.clear();
        record.extra.insert(
            "quoted".into(),
            serde_json::json!("{[\\\"nested text\\\"]}"),
        );
        assert!(RecordBudget::default().charge(&record).is_ok());
    }
}
