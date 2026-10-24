use super::*;
use onecad_core::{
    document::{
        record::{BooleanMode, DeterminismSettings, LoftParams},
        refs::SketchRegionRef,
    },
    ids::{RecordId, RegionId},
};

#[test]
fn loft_preview_and_plan_preserve_ordered_region_identity_and_flags() {
    let profiles: Vec<_> = (1..=3)
        .map(|i| SketchRegionRef {
            sketch: SketchId(Uuid::from_u128(i)),
            region: RegionId::new(format!("region-{i}")),
            region_identity_version: Some(2),
            region_anchor: Some([i as f64, -2.0]),
            extra: [("futureEvidence".into(), json!(i))].into_iter().collect(),
        })
        .collect();
    for is_ruled in [false, true] {
        let operation = Operation::Known(KnownOperation::Loft(LoftParams {
            profiles: profiles.clone(),
            is_solid: true,
            is_ruled,
            boolean_mode: BooleanMode::NewBody,
            extra: Default::default(),
        }));
        let planned = PlannedOp {
            record_id: RecordId(Uuid::from_u128(100)),
            step_index: 3,
            inputs: operation.derive_inputs(),
            operation: operation.clone(),
            determinism: DeterminismSettings::default(),
        };
        let plan = wire_op(&planned);
        let preview = preview_wire_op(&operation, &planned.record_id.to_string());
        assert_eq!(plan["params"], preview["params"]);
        assert_eq!(plan["inputs"], preview["inputs"]);
        assert_eq!(
            plan["params"]["profiles"],
            serde_json::to_value(&profiles).unwrap()
        );
        assert_eq!(plan["params"]["isSolid"], true);
        assert_eq!(plan["params"]["isRuled"], is_ruled);
        assert_eq!(plan["params"]["booleanMode"], "NewBody");
        assert!(plan["params"].get("profile").is_none());
    }
}
