//! Conservative admission estimates for owned JSON trees.
use serde_json::Value;

/// Estimate owned tree storage without cloning or serializing it. Containers
/// include spare capacity and conservative per-entry allocator/tree overhead.
/// Refuse nesting above 32 and stop as soon as the caller's bound is exceeded.
pub fn value_owned_bytes(value: &Value, limit: usize) -> Option<usize> {
    fn visit(value: &Value, depth: usize, left: &mut usize) -> Option<()> {
        if depth > 32 {
            return None;
        }
        *left = left.checked_sub(size_of::<Value>())?;
        match value {
            Value::String(text) => *left = left.checked_sub(text.capacity())?,
            Value::Array(items) => {
                *left = left.checked_sub(items.capacity().checked_mul(size_of::<Value>())?)?;
                for item in items {
                    visit(item, depth + 1, left)?;
                }
            }
            Value::Object(items) => {
                for (key, item) in items {
                    *left = left.checked_sub(key.capacity().checked_add(256)?)?;
                    visit(item, depth + 1, left)?;
                }
            }
            _ => {}
        }
        Some(())
    }
    let mut remaining = limit;
    visit(value, 0, &mut remaining)?;
    Some(limit - remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_json_and_deep_trees_refuse_before_clone() {
        let wide = Value::Array(vec![Value::Null; 40_000]);
        assert!(serde_json::to_vec(&wide).unwrap().len() < 256 << 10);
        assert!(value_owned_bytes(&wide, 2 << 20).is_none());
        let mut deep = Value::Null;
        for _ in 0..33 {
            deep = Value::Array(vec![deep]);
        }
        assert!(value_owned_bytes(&deep, 2 << 20).is_none());
    }
}
