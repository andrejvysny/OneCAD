---
name: edge-overflow-app-test-gotchas
description: fillet_overflow.rs app-lane harness gotchas — re-edit must clone live op, reason_code is a Diagnostic field, preview==commit needs Lod::Fine
metadata:
  type: project
---

- Re-edit a Fillet via `UpdateOperationParams`: clone `rt.operation_record(id).op` and change only radius. Re-sending a stripped `fillet_op` (no stamped descriptors) goes needsRepair "ambiguous" on the 3rd replay (native too, not overflow-specific).
- `EDGE_OP_OVERFLOW_UNSUPPORTED` reasonCode lives in `Diagnostic.reason_code`, evidence is None. Refused/failed regen shows in `report.failed_steps[].diagnostics`; successes in `projection().features[].diagnostics`.
- Curved-solid preview==commit: compare `Lod::Fine` both sides (Coarse differs 2072 vs 2085). Exact volume via `ElementQuery::query_mass_properties`.
- Unreferenced promoted ids don't survive `query_element` after a fillet; probe downstream refs with `rt.resolve_refs`.
