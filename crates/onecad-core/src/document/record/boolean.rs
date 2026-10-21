//! Standalone Boolean parameters (SCHEMA §7.3 Boolean, ONECAD-362).
//!
//! Two canonical encodings share one typed value:
//! * **legacy** `{operation, targetBodyId, toolBodyId}` — exactly one consumed
//!   tool; byte-identical to every document written before multi-tool support;
//! * **array** `{operation, targetBodyId, toolBodyIds:[…], keepTools?:true}` —
//!   any other tool set or a retained tool set.
//!
//! The array form fences itself: a worker that predates it reads `toolBodyId` as
//! absent and refuses `REF_UNRESOLVED`, so it can never silently consume a
//! retained tool or ignore extra tools. An app that predates it refuses to open
//! the document (`toolBodyId` was a required field).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::BooleanOp;
use crate::document::refs::Extra;
use crate::ids::BodyId;

/// Inclusive upper bound on Boolean tool bodies; the worker enforces the same
/// constant (`kBooleanMaxTools`).
pub const BOOLEAN_MAX_TOOLS: usize = 32;

/// Standalone boolean parameters (OneCAD-CPP `BooleanParams`
/// `OperationRecord.h:127-132`; SCHEMA §7.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "BooleanParamsSerde", into = "BooleanParamsSerde")]
pub struct BooleanParams {
    pub operation: BooleanOp,
    pub target_body: BodyId,
    /// Ordered tool bodies; the legacy single tool is `len() == 1`. Order fixes
    /// the `inputs[]` slots, the `deleted` event order and the Intersect
    /// pre-combine order.
    pub tools: Vec<BodyId>,
    /// `true` ⇒ every tool survives unchanged (no lifecycle event). One global
    /// flag; mixed retention is two Booleans.
    pub keep_tools: bool,
    pub extra: Extra,
}

/// The persisted shape. Field order is the legacy byte order.
#[derive(Serialize, Deserialize)]
struct BooleanParamsSerde {
    operation: BooleanOp,
    #[serde(rename = "targetBodyId")]
    target_body: BodyId,
    #[serde(
        rename = "toolBodyId",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    tool_body: Option<BodyId>,
    #[serde(
        rename = "toolBodyIds",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    tool_bodies: Option<Vec<BodyId>>,
    #[serde(
        rename = "keepTools",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    keep_tools: Option<bool>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    extra: Extra,
}

/// A tool-set key that is present must carry a value: an explicit `null` would
/// otherwise read as absent (the worker's presence checks refuse it), so a
/// `keepTools:null` record would load as consume and delete its tools on regen.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl TryFrom<BooleanParamsSerde> for BooleanParams {
    type Error = String;

    fn try_from(raw: BooleanParamsSerde) -> Result<Self, String> {
        let (tools, keep_tools) = match (raw.tool_body, raw.tool_bodies) {
            (Some(_), Some(_)) => {
                return Err("Boolean carries both toolBodyId and toolBodyIds".into())
            }
            (None, None) => return Err("Boolean requires toolBodyId or toolBodyIds".into()),
            (Some(tool), None) => {
                if raw.keep_tools.is_some() {
                    return Err("Boolean keepTools requires toolBodyIds".into());
                }
                // A legacy self-tool record stays loadable: it predates
                // validation and the worker refuses it at regen.
                (vec![tool], false)
            }
            (None, Some(tools)) => (tools, raw.keep_tools.unwrap_or(false)),
        };
        let params = BooleanParams {
            operation: raw.operation,
            target_body: raw.target_body,
            tools,
            keep_tools,
            extra: raw.extra,
        };
        if params.uses_tools_array() {
            params.validate()?;
        }
        Ok(params)
    }
}

impl From<BooleanParams> for BooleanParamsSerde {
    fn from(p: BooleanParams) -> Self {
        let array = p.uses_tools_array();
        BooleanParamsSerde {
            operation: p.operation,
            target_body: p.target_body,
            tool_body: (!array).then(|| p.tools[0]),
            keep_tools: (array && p.keep_tools).then_some(true),
            tool_bodies: array.then_some(p.tools),
            extra: p.extra,
        }
    }
}

impl BooleanParams {
    /// The legacy single consumed tool.
    pub fn single(operation: BooleanOp, target: BodyId, tool: BodyId) -> Self {
        Self::new(operation, target, vec![tool], false)
    }

    /// Ordered tools with one global retention flag. Not validated; call
    /// [`Self::validate`] before admitting the record.
    pub fn new(operation: BooleanOp, target: BodyId, tools: Vec<BodyId>, keep_tools: bool) -> Self {
        Self {
            operation,
            target_body: target,
            tools,
            keep_tools,
            extra: Extra::new(),
        }
    }

    /// `true` when the record needs the array encoding (and worker capability
    /// `op.boolean.tools`): anything other than one consumed tool.
    pub fn uses_tools_array(&self) -> bool {
        self.tools.len() != 1 || self.keep_tools
    }

    /// The tools the operation deletes, in authored order.
    pub fn consumed_tools(&self) -> &[BodyId] {
        if self.keep_tools {
            &[]
        } else {
            &self.tools
        }
    }

    /// Tool-set rules: 1..=[`BOOLEAN_MAX_TOOLS`] distinct tools, none the target.
    pub fn validate(&self) -> Result<(), String> {
        if self.tools.is_empty() {
            return Err("Boolean requires at least one tool body".into());
        }
        if self.tools.len() > BOOLEAN_MAX_TOOLS {
            return Err(format!(
                "Boolean supports at most {BOOLEAN_MAX_TOOLS} tool bodies"
            ));
        }
        let mut seen = BTreeSet::new();
        for tool in &self.tools {
            if *tool == self.target_body {
                return Err(format!("Boolean tool {tool} is the target body"));
            }
            if !seen.insert(*tool) {
                return Err(format!("Boolean tools contain a duplicate body {tool}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ids(n: usize) -> Vec<BodyId> {
        (0..n).map(|_| BodyId::new()).collect()
    }

    fn parse(value: serde_json::Value) -> Result<BooleanParams, String> {
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    #[test]
    fn legacy_bytes_round_trip_unchanged() {
        let b = ids(2);
        let text = format!(
            r#"{{"operation":"Cut","targetBodyId":"{}","toolBodyId":"{}","alienKey":7}}"#,
            b[0], b[1]
        );
        let p: BooleanParams = serde_json::from_str(&text).unwrap();
        assert_eq!(p.tools, vec![b[1]]);
        assert!(!p.keep_tools && !p.uses_tools_array());
        assert_eq!(serde_json::to_string(&p).unwrap(), text);
    }

    #[test]
    fn array_form_round_trips_and_omits_false_keep() {
        let b = ids(3);
        let kept = BooleanParams::new(BooleanOp::Union, b[0], vec![b[1], b[2]], true);
        let v = serde_json::to_value(&kept).unwrap();
        assert_eq!(
            v,
            json!({"operation":"Union","targetBodyId":b[0],"toolBodyIds":[b[1],b[2]],"keepTools":true})
        );
        assert_eq!(parse(v).unwrap(), kept);
        let consumed = BooleanParams::new(BooleanOp::Cut, b[0], vec![b[1], b[2]], false);
        let v = serde_json::to_value(&consumed).unwrap();
        assert!(v.get("keepTools").is_none() && v.get("toolBodyId").is_none());
        assert_eq!(parse(v).unwrap(), consumed);
        let single_kept = BooleanParams::new(BooleanOp::Cut, b[0], vec![b[1]], true);
        let v = serde_json::to_value(&single_kept).unwrap();
        assert_eq!(v["toolBodyIds"], json!([b[1]]));
        assert_eq!(single_kept.consumed_tools(), &[] as &[BodyId]);
    }

    #[test]
    fn single_unkept_array_normalizes_to_legacy() {
        let b = ids(2);
        for keep in [json!(null), json!(false)] {
            let mut v = json!({"operation":"Cut","targetBodyId":b[0],"toolBodyIds":[b[1]]});
            if !keep.is_null() {
                v["keepTools"] = keep;
            }
            let p = parse(v).unwrap();
            assert_eq!(p, BooleanParams::single(BooleanOp::Cut, b[0], b[1]));
            assert_eq!(
                serde_json::to_value(&p).unwrap(),
                json!({"operation":"Cut","targetBodyId":b[0],"toolBodyId":b[1]})
            );
        }
    }

    #[test]
    fn structural_errors_are_hard() {
        let b = ids(3);
        let cases = [
            (
                json!({"operation":"Cut","targetBodyId":b[0],"toolBodyId":b[1],"toolBodyIds":[b[2]]}),
                "both",
            ),
            (json!({"operation":"Cut","targetBodyId":b[0]}), "requires"),
            (
                json!({"operation":"Cut","targetBodyId":b[0],"toolBodyId":b[1],"keepTools":true}),
                "keepTools requires",
            ),
            (
                json!({"operation":"Cut","targetBodyId":b[0],"toolBodyId":b[1],"keepTools":false}),
                "keepTools requires",
            ),
            (
                json!({"operation":"Cut","targetBodyId":b[0],"toolBodyIds":[]}),
                "at least one",
            ),
            (
                json!({"operation":"Cut","targetBodyId":b[0],"toolBodyIds":[b[1],b[1]]}),
                "duplicate",
            ),
            (
                json!({"operation":"Cut","targetBodyId":b[0],"toolBodyIds":[b[1],b[0]]}),
                "is the target",
            ),
        ];
        for (value, needle) in cases {
            let error = parse(value.clone()).unwrap_err();
            assert!(error.contains(needle), "{value}: {error}");
        }
        // An explicit null is a present, ill-typed key, never "absent".
        for value in [
            json!({"operation":"Cut","targetBodyId":b[0],"toolBodyId":b[1],"keepTools":null}),
            json!({"operation":"Cut","targetBodyId":b[0],"toolBodyId":b[1],"toolBodyIds":null}),
            json!({"operation":"Cut","targetBodyId":b[0],"toolBodyId":null,"toolBodyIds":[b[1],b[2]]}),
            json!({"operation":"Cut","targetBodyId":b[0],"toolBodyIds":[b[1],b[2]],"keepTools":null}),
        ] {
            let error = parse(value.clone()).unwrap_err();
            assert!(error.contains("null"), "{value}: {error}");
            let record = json!({"opType":"Boolean","params":value});
            assert!(serde_json::from_value::<super::super::Operation>(record).is_err());
        }
        let many: Vec<BodyId> = ids(BOOLEAN_MAX_TOOLS + 1);
        let error =
            parse(json!({"operation":"Cut","targetBodyId":b[0],"toolBodyIds":many})).unwrap_err();
        assert!(error.contains("at most 32"), "{error}");
        let max: Vec<BodyId> = ids(BOOLEAN_MAX_TOOLS);
        assert!(parse(json!({"operation":"Cut","targetBodyId":b[0],"toolBodyIds":max})).is_ok());
    }

    #[test]
    fn legacy_self_tool_stays_loadable_but_invalid() {
        let b = ids(1);
        let p = parse(json!({"operation":"Cut","targetBodyId":b[0],"toolBodyId":b[0]})).unwrap();
        assert!(p.validate().unwrap_err().contains("is the target"));
    }
}
